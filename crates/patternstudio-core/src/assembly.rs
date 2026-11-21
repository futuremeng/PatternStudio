//! QWP7: symmetry-aware assembly planning.
//!
//! Turns a [`crate::symmetry`] detection into a product path: trace the
//! fundamental sector of the pattern once, then rebuild the full artwork by
//! rotating/mirroring that one sector. The SVG side references the sector
//! through `<use>` groups, so editing one copy syncs all of them and the
//! symmetry is exact by construction. Translation symmetry is deliberately
//! out of this path's scope (tiling is a separate mechanism).

use std::f64::consts::TAU;

use vtracer::ColorImage;

use crate::smart::downsample;
use crate::symmetry::{analyze_symmetry, dilate, foreground_mask, SYM_MAX_SIDE};

/// Box dilation radius for the fold verification, matching the detector's.
const FOLD_DILATE_R: usize = 2;

/// Minimum dilated-F1 between the fold projection and the source for a plan
/// to be taken. Calibrated on the 8-sample eval set: true folds score about
/// 0.9995 or better, false-positive mirrors on chiral or translation-only
/// patterns at most 0.92; 0.97 sits in the gap so a doubtful assembly falls
/// back to the plain trace instead of degrading it.
pub const FOLD_F1_MIN: f64 = 0.97;

/// How to rebuild the artwork from one traced sector: `n` rotations of
/// `360/n`° about `center`, each optionally folded by a mirror across the
/// axis at `base_deg` through the center.
#[derive(Debug, Clone)]
pub struct AssemblyPlan {
    /// Symmetry center in full-resolution pixel coordinates.
    pub center: (f64, f64),
    /// Rotation order; 1 for the mirror-only fold.
    pub n: u32,
    /// Whether the sector is also mirrored.
    pub mirror: bool,
    /// Start angle of the fundamental sector in degrees. With a detected
    /// mirror this is the mirror axis angle; otherwise 0.
    pub base_deg: f64,
    /// Dilated-F1 between the source foreground and this plan's fold
    /// projection: how exactly the assembly reproduces the source. Reported
    /// so a run can show why a plan was taken or rejected.
    pub fold_f1: f64,
}

impl AssemblyPlan {
    /// Angular width of the fundamental sector in radians: the plane splits
    /// into `n` wedges, each split again by the mirror fold.
    pub fn sector_width(&self) -> f64 {
        TAU / ((self.n.max(1) * if self.mirror { 2 } else { 1 }) as f64)
    }

    /// Group label for metadata: dihedral D_n, mirror-only C1v, cyclic C_n.
    pub fn kind(&self) -> String {
        match (self.n, self.mirror) {
            (1, true) => "C1v".to_string(),
            (1, false) => "C1".to_string(),
            (n, true) => format!("D{}", n),
            (n, false) => format!("C{}", n),
        }
    }
}

/// Detect a symmetry this path can assemble; `None` falls back to the plain
/// trace. The mirror-only case plans as a single half-plane fold (n = 1).
pub fn plan_for(img: &ColorImage) -> Option<AssemblyPlan> {
    let report = analyze_symmetry(img);
    let (n, base_deg) = match (&report.rotation, &report.mirror) {
        (Some(r), Some(m)) => (r.n, m.angle_deg),
        (Some(r), None) => (r.n, 0.0),
        (None, Some(m)) => (1, m.angle_deg),
        (None, None) => return None,
    };
    let mirror = report.mirror.is_some();
    // Symmetry center: the same foreground centroid the detector aligned on,
    // mapped from analysis pixels back to full-resolution coordinates.
    let thumb = downsample(img, SYM_MAX_SIDE);
    let mask = foreground_mask(&thumb);
    let (mut sx, mut sy) = (0.0f64, 0.0f64);
    let mut cnt = 0usize;
    for y in 0..thumb.height {
        for x in 0..thumb.width {
            if mask[y * thumb.width + x] {
                sx += x as f64;
                sy += y as f64;
                cnt += 1;
            }
        }
    }
    if cnt < 16 {
        return None;
    }
    let cx = sx / cnt as f64 * (img.width as f64 / thumb.width as f64);
    let cy = sy / cnt as f64 * (img.height as f64 / thumb.height as f64);
    let mut plan = AssemblyPlan {
        center: (cx, cy),
        n,
        mirror,
        base_deg,
        fold_f1: 0.0,
    };
    plan.fold_f1 = round4(fold_consistency(img, &plan));
    // Detection proposed the group; the fold decides. A near-miss here means
    // rebuilding from one sector would visibly diverge from the source.
    if plan.fold_f1 < FOLD_F1_MIN {
        return None;
    }
    Some(plan)
}

/// Dilated-F1 between the source foreground mask and the mask the assembly
/// would paint: every pixel is folded back into the fundamental sector
/// (rotation `mod TAU/n`, then the mirror lobe reflected into the base lobe)
/// and sampled from the source. Detection proposes a group; this verifies
/// that rebuilding from one sector actually reproduces this image, so a
/// false-positive mirror on a chiral or translation-only pattern can fall
/// back to the plain trace instead of degrading it.
pub fn fold_consistency(img: &ColorImage, plan: &AssemblyPlan) -> f64 {
    let thumb = downsample(img, SYM_MAX_SIDE);
    let (w, h) = (thumb.width, thumb.height);
    let src = foreground_mask(&thumb);
    let cx = plan.center.0 * (w as f64 / img.width as f64);
    let cy = plan.center.1 * (h as f64 / img.height as f64);
    let base = plan.base_deg.to_radians();
    let wedge = TAU / plan.n.max(1) as f64;
    let width = plan.sector_width();
    let mut asm = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let dx = x as f64 + 0.5 - cx;
            let dy = y as f64 + 0.5 - cy;
            let r = (dx * dx + dy * dy).sqrt();
            let mut rel = (dy.atan2(dx) - base) % TAU;
            if rel < 0.0 {
                rel += TAU;
            }
            rel %= wedge;
            if plan.mirror && rel >= width {
                rel = 2.0 * width - rel;
            }
            let ang = base + rel;
            let fx = (cx + r * ang.cos() - 0.5)
                .round()
                .clamp(0.0, w as f64 - 1.0) as usize;
            let fy = (cy + r * ang.sin() - 0.5)
                .round()
                .clamp(0.0, h as f64 - 1.0) as usize;
            asm[y * w + x] = src[fy * w + fx];
        }
    }
    // Symmetric dilated F1, the same tolerance the detector scores with.
    let dil_asm = dilate(w, h, &asm, FOLD_DILATE_R);
    let dil_src = dilate(w, h, &src, FOLD_DILATE_R);
    let (mut na, mut ns, mut hit_a, mut hit_s) = (0usize, 0usize, 0usize, 0usize);
    for i in 0..w * h {
        if asm[i] {
            na += 1;
            if dil_src[i] {
                hit_a += 1;
            }
        }
        if src[i] {
            ns += 1;
            if dil_asm[i] {
                hit_s += 1;
            }
        }
    }
    if na == 0 && ns == 0 {
        return 1.0;
    }
    if na == 0 || ns == 0 {
        return 0.0;
    }
    let p = hit_a as f64 / na as f64;
    let r = hit_s as f64 / ns as f64;
    if p + r == 0.0 {
        0.0
    } else {
        2.0 * p * r / (p + r)
    }
}

fn round4(v: f64) -> f64 {
    (v * 1e4).round() / 1e4
}

/// The image clipped to the fundamental sector: pixels outside the wedge
/// become fully transparent, so the tracer sees the sector content on its
/// own and the copies tile the plane without painting over each other.
pub fn sector_image(img: &ColorImage, plan: &AssemblyPlan) -> ColorImage {
    let w = img.width;
    let h = img.height;
    let base = plan.base_deg.to_radians();
    let width = plan.sector_width();
    let mut pixels = img.pixels.clone();
    for y in 0..h {
        for x in 0..w {
            let dx = x as f64 + 0.5 - plan.center.0;
            let dy = y as f64 + 0.5 - plan.center.1;
            let mut a = dy.atan2(dx) - base;
            a %= TAU;
            if a < 0.0 {
                a += TAU;
            }
            if a >= width {
                let i = (y * w + x) * 4;
                pixels[i] = 0;
                pixels[i + 1] = 0;
                pixels[i + 2] = 0;
                pixels[i + 3] = 0;
            }
        }
    }
    ColorImage {
        pixels,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_image(w: usize, h: usize, mut f: impl FnMut(usize, usize) -> bool) -> ColorImage {
        let mut pixels = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let v = if f(x, y) { 0u8 } else { 255u8 };
                pixels.extend_from_slice(&[v, v, v, 255]);
            }
        }
        ColorImage {
            pixels,
            width: w,
            height: h,
        }
    }

    /// Dots on a ring: n-fold rotation, and mirror-symmetric about the axes.
    fn ring(n: usize, w: usize) -> ColorImage {
        let (cx, cy, rr) = (w / 2, w / 2, w / 3);
        make_image(w, w, |x, y| {
            if (x as f64 - cx as f64).abs() < 3.0 && (y as f64 - cy as f64).abs() < 3.0 {
                return true;
            }
            (0..n).any(|i| {
                let a = TAU * i as f64 / n as f64;
                // Snap dot centers to integers: at exact right angles the
                // float cos/sin epsilon would shift the dot window by a
                // pixel and make the image not exactly D_n.
                let px = (cx as f64 + rr as f64 * a.cos()).round();
                let py = (cy as f64 + rr as f64 * a.sin()).round();
                (x as f64 - px).abs() < 4.0 && (y as f64 - py).abs() < 4.0
            })
        })
    }

    #[test]
    fn plans_four_fold_dihedral() {
        let p = plan_for(&ring(4, 256)).expect("4-fold ring must plan");
        assert_eq!(p.n, 4);
        assert!(p.mirror, "ring dots are axis-symmetric");
        assert_eq!(p.kind(), "D4");
        let w = p.sector_width();
        assert!((w - std::f64::consts::FRAC_PI_4).abs() < 1e-9);
    }

    #[test]
    fn plans_mirror_only_fold() {
        // Two differently shaped blobs on the left, mirrored on the right:
        // only the vertical fold axis survives (asymmetric in y as well).
        let img = make_image(128, 128, |x, y| {
            let left = ((20..44).contains(&x) && (40..52).contains(&y))
                || ((28..36).contains(&x) && (60..92).contains(&y));
            let mx = 127 - x;
            let right = ((20..44).contains(&mx) && (40..52).contains(&y))
                || ((28..36).contains(&mx) && (60..92).contains(&y));
            left || right
        });
        let p = plan_for(&img).expect("mirror fold must plan");
        assert_eq!(p.n, 1);
        assert!(p.mirror);
        assert!(
            (p.base_deg - 90.0).abs() < 5.0,
            "axis ~vertical, got {}",
            p.base_deg
        );
    }

    #[test]
    fn plans_nothing_for_asymmetric() {
        // Six 10x10 squares at incoherent positions: no order, no axis.
        let spots = [
            (26usize, 28usize),
            (61, 24),
            (40, 62),
            (85, 52),
            (24, 88),
            (72, 72),
        ];
        let img = make_image(128, 128, |x, y| {
            spots
                .iter()
                .any(|&(sx, sy)| x >= sx && x < sx + 10 && y >= sy && y < sy + 10)
        });
        assert!(plan_for(&img).is_none());
    }

    #[test]
    fn fold_scores_exact_for_a_true_mirror() {
        // Blob + a differently shaped blob on the left, exact mirror right.
        let img = make_image(128, 128, |x, y| {
            let (x, y) = (x as i64, y as i64);
            (x < 64 && (x - 30).abs() + (y - 40).abs() < 12)
                || (x < 52 && (x - 30).abs() / 2 + (y - 90).abs() < 8)
                || (x > 64 && (127 - x - 30).abs() + (y - 40).abs() < 12)
                || (x > 76 && (127 - x - 30).abs() / 2 + (y - 90).abs() < 8)
        });
        let p = AssemblyPlan {
            center: (64.0, 64.0),
            n: 1,
            mirror: true,
            base_deg: 90.0, // image coords: the line x = cx
            fold_f1: 0.0,
        };
        assert!(fold_consistency(&img, &p) > 0.95, "true fold must score ~1");
    }

    #[test]
    fn fold_flags_a_false_mirror() {
        // Left half mirrored on the right, plus one blob only on the left:
        // the fold invents an extra blob on the right, so it must score low.
        let img = make_image(128, 128, |x, y| {
            let (x, y) = (x as i64, y as i64);
            (x < 64 && (x - 30).abs() + (y - 40).abs() < 12)
                || (x < 52 && (x - 30).abs() / 2 + (y - 90).abs() < 8)
                || (x > 64 && (127 - x - 30).abs() + (y - 40).abs() < 12)
                || (x > 76 && (127 - x - 30).abs() / 2 + (y - 90).abs() < 8)
                || (x < 64 && (x - 24).abs() + (y - 64).abs() < 24)
        });
        let p = AssemblyPlan {
            center: (64.0, 64.0),
            n: 1,
            mirror: true,
            base_deg: 90.0, // image coords: the line x = cx
            fold_f1: 0.0,
        };
        let f = fold_consistency(&img, &p);
        assert!(f < 0.9, "one-sided blob must not pass the fold ({f})");
    }

    #[test]
    fn sector_keeps_only_its_wedge() {
        // Solid disk at the center: the D4 sector must keep ~1/8 of its pixels.
        let img = make_image(256, 256, |x, y| {
            let dx = x as f64 - 128.0;
            let dy = y as f64 - 128.0;
            dx * dx + dy * dy < 90.0 * 90.0
        });
        let p = plan_for(&img).expect("disk plans at least the mirror fold");
        let sector = sector_image(&img, &p);
        let total = img.pixels.chunks_exact(4).filter(|p| p[0] == 0).count();
        let kept = sector
            .pixels
            .chunks_exact(4)
            .filter(|p| p[3] > 0 && p[0] == 0)
            .count();
        let frac = kept as f64 / total as f64;
        let want = p.sector_width() / TAU;
        assert!(
            (frac - want).abs() < 0.06,
            "sector kept {kept}/{total} = {frac:.3}, want {want:.3}"
        );
    }
}
