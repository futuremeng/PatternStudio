//! QWP6 spike: symmetry detection for repeat patterns.
//!
//! Scores rotational, mirror and translational self-similarity of an image's
//! foreground mask, to decide whether a pattern is worth the symmetry-aware
//! trace path (sector tracing + mirrored assembly). Rotational/mirror scores
//! align the mask by inverse sampling around the foreground centroid and use
//! a dilated F1 so thin-stroke sampling error does not dominate. Translation
//! sweeps integer pixel periods with that dilated F1 (computed through the
//! dilation-shift identity) and uses a projection-profile DFT at the
//! continuous frequency to choose among the periods that align.

use serde::Serialize;
use vtracer::ColorImage;

use crate::smart::downsample;

/// Analysis resolution cap: the generated samples are 512 px, so keeping
/// full detail means no resample at all and thin strokes stay above the
/// foreground cut. Warps cost ~4x vs 256 but the spike is offline.
pub(crate) const SYM_MAX_SIDE: u32 = 512;
/// Box dilation radius used as point-set tolerance when comparing masks.
const DILATE_R: usize = 2;
/// Rotational orders to try, per the plan (n in 3..=12).
const ROT_MIN_N: u32 = 3;
const ROT_MAX_N: u32 = 12;
/// Smallest translation period tried, in analysis pixels; below this the
/// dilation tolerance already covers ordinary shifts.
const TRANS_MIN_P: i32 = 8;

// Detection thresholds. Calibrated on the 8 synthetic samples (spike report
// in docs/design/QUALITY-IMPROVEMENT-PLAN.md); 0.7 sits in the gap between
// symmetric detections (>=0.85) and asymmetric scores (<0.62).
pub const ROTATION_MIN: f64 = 0.7;
pub const MIRROR_MIN: f64 = 0.7;
/// Profile DFT amplitude and 2D alignment gates for translation. The DFT
/// amplitude only proposes candidate periods; the 2D F1 at the period is
/// what rejects single-bump profiles and incoherent patterns. A sparse
/// stroke train bottoms out near 0.49 amplitude by Parseval, so the gate
/// must stay at the candidate level only.
pub const TRANS_PROFILE_MIN: f64 = 0.4;
pub const TRANS_F1_MIN: f64 = 0.75;
/// A 2D alignment this strong is a period even with a flat profile.
pub const TRANS_F1_STRONG: f64 = 0.9;
/// F1 gap that counts as a real preference: shifts inside the dilation
/// tolerance align within ~1e-3 of the true period, so differences below
/// this are treated as ties and the profile amplitude decides between them.
const F1_TIE: f64 = 2.5e-3;

#[derive(Debug, Clone, Serialize)]
pub struct RotationSymmetry {
    /// Largest order whose alignment score passes [`ROTATION_MIN`].
    pub n: u32,
    /// Best alignment F1 in 0..=1.
    pub score: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MirrorSymmetry {
    /// Mirror axis direction in degrees [0,180): 90 = vertical axis.
    pub angle_deg: f64,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TranslationSymmetry {
    /// Axis of the detected period: "h" (columns) or "v" (rows).
    pub axis: &'static str,
    /// Period in analysis pixels.
    pub period: i32,
    /// Cycles across the image on that axis (may be fractional).
    pub cycles: f64,
    /// Projection-profile DFT amplitude at that period, 0..=1.
    pub profile: f64,
    /// 2D alignment F1 at the candidate period, 0..=1.
    pub f1: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymmetryReport {
    pub width: usize,
    pub height: usize,
    pub foreground_ratio: f64,
    pub rotation: Option<RotationSymmetry>,
    pub mirror: Option<MirrorSymmetry>,
    pub translation: Option<TranslationSymmetry>,
    /// Calibration detail: best alignment F1 for every n in 3..=12.
    pub rotation_curve: Vec<(u32, f64)>,
    /// Best mirror-axis F1 found in the coarse+refine sweep.
    pub mirror_peak: f64,
    /// Translation candidates (axis, period, profile, f1) that passed either
    /// gate, sorted by profile amplitude; detection picks the passing
    /// candidate with the best alignment F1, breaking alignment ties by
    /// profile amplitude. This separates the true period from shifts riding
    /// the dilation tolerance and from its multiples.
    pub translation_candidates: Vec<(&'static str, i32, f64, f64)>,
}

/// Score rotational, mirror and translational symmetry of `img`.
pub fn analyze_symmetry(img: &ColorImage) -> SymmetryReport {
    let thumb = downsample(img, SYM_MAX_SIDE);
    let w = thumb.width;
    let h = thumb.height;
    let mask = foreground_mask(&thumb);
    let empty = || SymmetryReport {
        width: w,
        height: h,
        foreground_ratio: 0.0,
        rotation: None,
        mirror: None,
        translation: None,
        rotation_curve: Vec::new(),
        mirror_peak: 0.0,
        translation_candidates: Vec::new(),
    };
    let fg: Vec<(usize, usize)> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|&(x, y)| mask[y * w + x])
        .collect();
    if fg.len() < 16 {
        return empty();
    }
    let cx = fg.iter().map(|p| p.0 as f64).sum::<f64>() / fg.len() as f64;
    let cy = fg.iter().map(|p| p.1 as f64).sum::<f64>() / fg.len() as f64;
    let fg_ratio = fg.len() as f64 / (w * h) as f64;
    let base = Mask::new(w, h, mask, &fg);

    // Rotation: a pattern that is n-fold symmetric maps onto itself under a
    // 2*pi/n rotation about the symmetry center, independent of its absolute
    // phase, so one alignment trial per n suffices.
    let mut rotation_curve = Vec::new();
    let mut rotation: Option<RotationSymmetry> = None;
    for n in ROT_MIN_N..=ROT_MAX_N {
        let th = 2.0 * std::f64::consts::PI / n as f64;
        let rotated = base.warp(|x, y| {
            let dx = x - cx;
            let dy = y - cy;
            (
                cx + th.cos() * dx + th.sin() * dy,
                cy - th.sin() * dx + th.cos() * dy,
            )
        });
        let score = base.f1_against(&rotated);
        rotation_curve.push((n, round3(score)));
        if score >= ROTATION_MIN {
            rotation = Some(RotationSymmetry {
                n,
                score: round3(score),
            });
        }
    }

    // Mirror: sweep axis direction, coarse then two refinements; only the
    // score and its angle are kept.
    let mut best_a = 0.0f64;
    let mut best_s = warp_mirror(&base, cx, cy, 0.0).f1_against(&base);
    for i in 0..45 {
        let a = i as f64 * 4.0;
        let s = warp_mirror(&base, cx, cy, a).f1_against(&base);
        if s > best_s {
            best_s = s;
            best_a = a;
        }
    }
    for step in [2.0, 0.4] {
        for i in -4..=4 {
            let a = best_a + i as f64 * step;
            let s = warp_mirror(&base, cx, cy, a).f1_against(&base);
            if s > best_s {
                best_s = s;
                best_a = a;
            }
        }
    }
    let mirror_angle = ((best_a % 180.0) + 180.0) % 180.0;
    let mirror = if best_s >= MIRROR_MIN {
        Some(MirrorSymmetry {
            angle_deg: round2(mirror_angle),
            score: round3(best_s),
        })
    } else {
        None
    };

    // Translation: sweep integer pixel periods; the 2D alignment F1 (cheap
    // via the dilation-shift identity) is the primary signal, the
    // projection-profile DFT amplitude at the continuous frequency picks
    // between candidates that align: the true period scores ~1, shifts
    // riding the dilation tolerance score partial amplitude, and multiples
    // sit at half-frequency subharmonics where any p-periodic profile has
    // no energy, so they score 0.
    let mut candidates: Vec<(&'static str, i32, f64, f64)> = Vec::new();
    let mut translation: Option<TranslationSymmetry> = None;
    let col: Vec<f64> = (0..w)
        .map(|x| (0..h).filter(|&y| base.px[y * w + x]).count() as f64)
        .collect();
    let row: Vec<f64> = (0..h)
        .map(|y| (0..w).filter(|&x| base.px[y * w + x]).count() as f64)
        .collect();
    for (axis, prof, size, vertical) in [("h", &col, w, false), ("v", &row, h, true)] {
        let mut best: Option<(i32, f64, f64)> = None;
        for p in TRANS_MIN_P..=(size as i32 / 2) {
            let (dx, dy) = if vertical { (0, p) } else { (p, 0) };
            let f1 = base.trans_f1(dx, dy);
            if f1 < TRANS_F1_MIN {
                continue;
            }
            let amp = profile_amp(prof, p as f64);
            if amp >= TRANS_PROFILE_MIN || f1 >= TRANS_F1_STRONG {
                candidates.push((axis, p, round3(amp), round3(f1)));
            }
            if (amp >= TRANS_PROFILE_MIN && f1 >= TRANS_F1_MIN) || f1 >= TRANS_F1_STRONG {
                // Perfect alignment is the strongest evidence: pick the
                // best-F1 candidate, and when several align equally well
                // break ties by profile amplitude (which kills the
                // tolerance-riding p-1 and the subharmonic multiples).
                let better = match best {
                    None => true,
                    Some((_, a, af)) => {
                        f1 > af + F1_TIE
                            || (f1 >= af - F1_TIE && amp >= TRANS_PROFILE_MIN && amp > a + 1e-9)
                    }
                };
                if better {
                    best = Some((p, amp, f1));
                }
            }
        }
        if let Some((p, amp, f1)) = best {
            if translation.as_ref().is_none_or(|t| amp > t.profile + 1e-9) {
                translation = Some(TranslationSymmetry {
                    axis,
                    period: p,
                    cycles: round2(size as f64 / p as f64),
                    profile: round3(amp),
                    f1: round3(f1),
                });
            }
        }
    }
    candidates.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    candidates.truncate(16);

    SymmetryReport {
        width: w,
        height: h,
        foreground_ratio: round4(fg_ratio),
        rotation,
        mirror,
        translation,
        rotation_curve,
        mirror_peak: round3(best_s),
        translation_candidates: candidates,
    }
}

/// Foreground = pixels darker than an Otsu threshold of the luminance
/// histogram (ink on paper). Falls back to the 128 cut when the split is
/// degenerate, and to nothing when even that leaves a near-empty mask.
pub(crate) fn foreground_mask(img: &ColorImage) -> Vec<bool> {
    let n = img.width * img.height;
    let mut lum = vec![0u8; n];
    let mut hist = [0f64; 256];
    for (i, px) in img.pixels.chunks_exact(4).enumerate() {
        let l = (0.2126 * px[0] as f64 + 0.7152 * px[1] as f64 + 0.0722 * px[2] as f64)
            .round()
            .clamp(0.0, 255.0) as usize;
        lum[i] = l as u8;
        hist[l] += 1.0;
    }
    let mut cut = otsu_cut(&hist, n as f64) as i32;
    let count = |cut: i32| lum.iter().filter(|&&v| (v as i32) < cut).count();
    let c1 = count(cut);
    if c1 < n / 500 || c1 > n * 49 / 50 {
        let c128 = count(128);
        if c128 >= n / 500 && c128 <= n * 49 / 50 {
            cut = 128;
        }
    }
    if count(cut) < n / 500 || count(cut) > n * 49 / 50 {
        return vec![false; n];
    }
    lum.iter().map(|&v| (v as i32) < cut).collect()
}

/// Otsu's between-class variance maximum; returns the cut value (0..=255)
/// just below which pixels count as foreground.
fn otsu_cut(hist: &[f64; 256], total: f64) -> usize {
    let sum: f64 = hist.iter().enumerate().map(|(t, &h)| t as f64 * h).sum();
    let (mut sum_b, mut w_b, mut best, mut best_t) = (0.0f64, 0.0f64, -1.0f64, 128usize);
    for (t, &h) in hist.iter().enumerate() {
        w_b += h;
        if w_b == 0.0 {
            continue;
        }
        let w_f = total - w_b;
        if w_f == 0.0 {
            break;
        }
        sum_b += t as f64 * h;
        let m_b = sum_b / w_b;
        let m_f = (sum - sum_b) / w_f;
        let var = w_b * w_f * (m_b - m_f) * (m_b - m_f);
        if var > best {
            best = var;
            best_t = t;
        }
    }
    best_t + 1
}

struct Mask {
    w: usize,
    h: usize,
    px: Vec<bool>,
    dil: Vec<bool>,
    fg: Vec<(usize, usize)>,
}

impl Mask {
    fn new(w: usize, h: usize, px: Vec<bool>, fg: &[(usize, usize)]) -> Mask {
        let dil = dilate(w, h, &px, DILATE_R);
        Mask {
            w,
            h,
            px,
            dil,
            fg: fg.to_vec(),
        }
    }

    /// Bilinear 0.5-threshold sample of the mask at float coordinates.
    fn sample(&self, x: f64, y: f64) -> bool {
        // Out-of-bounds is background: the mask has nothing beyond the frame
        // (clamping would smear edge ink across every warp).
        if x < 0.0 || y < 0.0 || x > self.w as f64 - 1.0 || y > self.h as f64 - 1.0 {
            return false;
        }
        let x0 = x.floor() as usize;
        let y0 = y.floor() as usize;
        let x1 = (x0 + 1).min(self.w - 1);
        let y1 = (y0 + 1).min(self.h - 1);
        let (fx, fy) = (x - x0 as f64, y - y0 as f64);
        let v = |x: usize, y: usize| self.px[y * self.w + x] as u8 as f64;
        (v(x0, y0) * (1.0 - fx) + v(x1, y0) * fx) * (1.0 - fy)
            + (v(x0, y1) * (1.0 - fx) + v(x1, y1) * fx) * fy
            >= 0.5
    }

    /// The mask mapped through `f` (a gather warp: destination pixel takes
    /// the source position returned by f).
    fn warp(&self, f: impl Fn(f64, f64) -> (f64, f64)) -> Mask {
        let mut px = vec![false; self.w * self.h];
        for y in 0..self.h {
            for x in 0..self.w {
                let (sx, sy) = f(x as f64, y as f64);
                px[y * self.w + x] = self.sample(sx, sy);
            }
        }
        let fg: Vec<(usize, usize)> = (0..self.h)
            .flat_map(|y| (0..self.w).map(move |x| (x, y)))
            .filter(|&(x, y)| px[y * self.w + x])
            .collect();
        let dil = dilate(self.w, self.h, &px, DILATE_R);
        Mask {
            w: self.w,
            h: self.h,
            px,
            dil,
            fg,
        }
    }

    /// Fraction of `self`'s foreground pixels covered by the dilated `other`.
    fn coverage_of(&self, other: &Mask) -> f64 {
        if self.fg.is_empty() {
            return 0.0;
        }
        let hit = self
            .fg
            .iter()
            .filter(|&&(x, y)| other.dil[y * self.w + x])
            .count();
        hit as f64 / self.fg.len() as f64
    }

    /// Symmetric dilated F1 between two masks.
    fn f1_against(&self, other: &Mask) -> f64 {
        let p = self.coverage_of(other);
        let q = other.coverage_of(self);
        if p + q == 0.0 {
            0.0
        } else {
            2.0 * p * q / (p + q)
        }
    }

    /// Alignment F1 between the mask and itself translated by (dx, dy),
    /// computed through the dilation-shift identity: foreground pixel q is
    /// covered by the dilated shift iff dil[q + s] is set. Alignment is
    /// measured on the overlap window (shifted copy inside the image) on
    /// both sides, so window truncation cannot prop up a near-full-width
    /// shift that does not actually repeat the pattern.
    fn trans_f1(&self, dx: i32, dy: i32) -> f64 {
        let w = self.w as i32;
        let h = self.h as i32;
        let (mut hit_p, mut tot_p, mut hit_q, mut tot_q) = (0usize, 0usize, 0usize, 0usize);
        for &(x, y) in &self.fg {
            let (x, y) = (x as i32, y as i32);
            // Pixels whose +s image stays in bounds: same count as the
            // -s side by symmetry, so both F1 halves share a denominator.
            let px = x + dx;
            let py = y + dy;
            if px >= 0 && py >= 0 && px < w && py < h {
                tot_p += 1;
                hit_p += self.dil[(py * w + px) as usize] as usize;
                let qx = x - dx;
                let qy = y - dy;
                if qx >= 0 && qy >= 0 && qx < w && qy < h {
                    tot_q += 1;
                    hit_q += self.dil[(qy * w + qx) as usize] as usize;
                }
            }
        }
        if tot_p == 0 || tot_q == 0 {
            return 0.0;
        }
        let p = hit_p as f64 / tot_p as f64;
        let q = hit_q as f64 / tot_q as f64;
        if p + q == 0.0 {
            return 0.0;
        }
        2.0 * p * q / (p + q)
    }
}

/// Reflect `m` about the line through (cx, cy) with direction angle `a`
/// (degrees, image coordinates: y grows down, so 90 is a vertical axis).
fn warp_mirror(m: &Mask, cx: f64, cy: f64, a: f64) -> Mask {
    let rad = a.to_radians();
    let (ux, uy) = (rad.cos(), rad.sin());
    m.warp(|x, y| {
        let dx = x - cx;
        let dy = y - cy;
        let t = 2.0 * (dx * ux + dy * uy);
        (cx + t * ux - dx, cy + t * uy - dy)
    })
}

/// Box dilation (two separable 1D sliding ORs).
pub(crate) fn dilate(w: usize, h: usize, px: &[bool], r: usize) -> Vec<bool> {
    let mut tmp = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let hit = ((x.saturating_sub(r))..=((x + r).min(w - 1))).any(|xx| px[y * w + xx]);
            tmp[y * w + x] = hit;
        }
    }
    let mut out = vec![false; w * h];
    for x in 0..w {
        for y in 0..h {
            let hit = ((y.saturating_sub(r))..=((y + r).min(h - 1))).any(|yy| tmp[yy * w + x]);
            out[y * w + x] = hit;
        }
    }
    out
}

/// DFT amplitude of a detrended projection profile at the continuous frequency
/// for a pixel period of `period_px`: 1.0 for a perfectly periodic profile at
/// that period, much lower for a single smooth bump or incoherent noise.
/// Evaluating the continuous frequency (instead of an integer-cycles grid)
/// lets a pattern with a non-integer cycle count in the window, e.g. seven
/// waves in 512 px, score at its true period without quantization loss.
fn profile_amp(prof: &[f64], period_px: f64) -> f64 {
    let n = prof.len() as f64;
    let mean = prof.iter().sum::<f64>() / n;
    let p: Vec<f64> = prof.iter().map(|&v| v - mean).collect();
    let rms = (p.iter().map(|v| v * v).sum::<f64>() / n).sqrt();
    if rms < 1e-9 {
        return 0.0;
    }
    let w = std::f64::consts::TAU / period_px;
    let re: f64 = p
        .iter()
        .enumerate()
        .map(|(x, v)| v * (w * x as f64).cos())
        .sum();
    let im: f64 = p
        .iter()
        .enumerate()
        .map(|(x, v)| v * (w * x as f64).sin())
        .sum();
    (2.0 * (re * re + im * im).sqrt() / n / rms).min(1.0)
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}
fn round4(v: f64) -> f64 {
    (v * 10000.0).round() / 10000.0
}
fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
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

    /// A dot pattern on a ring: n-fold rotation by construction.
    fn ring(n: usize, w: usize) -> ColorImage {
        let (cx, cy, rr) = (w / 2, w / 2, w / 3);
        make_image(w, w, |x, y| {
            if (x as f64 - cx as f64).abs() < 3.0 && (y as f64 - cy as f64).abs() < 3.0 {
                return true;
            }
            (0..n).any(|i| {
                let a = std::f64::consts::TAU * i as f64 / n as f64;
                let px = cx as f64 + rr as f64 * a.cos();
                let py = cy as f64 + rr as f64 * a.sin();
                (x as f64 - px).abs() < 4.0 && (y as f64 - py).abs() < 4.0
            })
        })
    }

    #[test]
    fn detects_eight_fold_rotation() {
        let r = analyze_symmetry(&ring(8, 256));
        let rot = r.rotation.expect("8-fold ring must be detected");
        assert_eq!(rot.n, 8, "order must be 8, curve {:?}", r.rotation_curve);
        assert!(rot.score > 0.9, "score {:#?}", rot);
    }

    #[test]
    fn detects_vertical_mirror_axis() {
        // Asymmetric blobs on the left, mirrored on the right.
        let img = make_image(128, 128, |x, y| {
            let left = ((20..44).contains(&x) && (40..52).contains(&y))
                || ((28..36).contains(&x) && (60..92).contains(&y));
            let mx = 127 - x;
            let right = ((20..44).contains(&mx) && (40..52).contains(&y))
                || ((28..36).contains(&mx) && (60..92).contains(&y));
            left || right
        });
        let r = analyze_symmetry(&img);
        let m = r.mirror.expect("mirrored blobs must be detected");
        assert!(m.angle_deg > 85.0 && m.angle_deg < 95.0, "axis {:?}", m);
    }

    #[test]
    fn detects_horizontal_period() {
        // 2-px vertical strokes every 32 px across a 256-px band.
        let img = make_image(256, 64, |x, y| y > 20 && y < 44 && x % 32 < 2);
        let r = analyze_symmetry(&img);
        let t = r
            .translation
            .unwrap_or_else(|| panic!("stripes: cands {:?}", r.translation_candidates));
        assert_eq!((t.axis, t.period), ("h", 32), "{:?}", t);
    }

    #[test]
    fn asymmetric_blobs_report_nothing() {
        // Six 10x10 squares at incoherent positions.
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
        let r = analyze_symmetry(&img);
        assert!(r.rotation.is_none(), "curve {:?}", r.rotation_curve);
        assert!(r.mirror.is_none(), "mirror peak {}", r.mirror_peak);
        assert!(
            r.translation.is_none(),
            "cands {:?}",
            r.translation_candidates
        );
    }

    #[test]
    fn blank_image_is_all_clear() {
        let img = make_image(64, 64, |_, _| false);
        let r = analyze_symmetry(&img);
        assert_eq!(r.foreground_ratio, 0.0);
        assert!(r.rotation.is_none() && r.mirror.is_none() && r.translation.is_none());
    }
}
