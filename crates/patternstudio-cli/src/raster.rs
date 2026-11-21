//! Minimal deterministic scanline rasterizer for the quality gate.
//!
//! Renders a [`LayeredDoc`] back to an RGB bitmap exactly as the SVG writer
//! emits it — layers in order, shapes in paint order, default nonzero fill —
//! so the gate can compare the vector result with the source bitmap
//! (`ink_f1`, `psnr`, see QWP1) without an external renderer: the numbers
//! must be bit-stable between the machine that blessed the baseline and CI.
//!
//! Curve flattening subdivides cubics to a fixed flatness tolerance, so the
//! geometry error is far below the ±2 px dilation the F1 metric forgives.
//!
//! Limitation: every path is filled; the lineart writer mode (which strokes
//! the paths) is approximated by filling the same geometry. Gate samples are
//! traced in fill mode.

use patternstudio_core::LayeredDoc;
use vtracer::ir::{MultiPath, PathCmd};

/// Flatness tolerance for cubic flattening, in canvas pixels.
const FLATTEN_TOL: f64 = 0.25;
/// Maximum cubic subdivision depth (2^12 segments per curve at most).
const FLATTEN_DEPTH: u32 = 12;

/// A rasterized RGB canvas.
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    /// Row-major RGB.
    pub rgb: Vec<u8>,
}

impl Canvas {
    /// Rec.709 luma per pixel.
    pub fn luma(&self) -> Vec<f64> {
        self.rgb
            .chunks(3)
            .map(|px| 0.2126 * px[0] as f64 + 0.7152 * px[1] as f64 + 0.0722 * px[2] as f64)
            .collect()
    }
}

type Pt = (f64, f64);

/// Rasterize a layered document: white background, then every shape of
/// every layer filled in its solid color, in document (paint) order.
pub fn rasterize(doc: &LayeredDoc) -> Canvas {
    let w = doc.width as usize;
    let h = doc.height as usize;
    let mut rgb = vec![255u8; w * h * 3];
    for layer in &doc.layers {
        for shape in &layer.shapes {
            fill_path(&mut rgb, w, h, &shape.path, shape.paint.color().rgb_u8());
        }
    }
    Canvas {
        width: w,
        height: h,
        rgb,
    }
}

/// Nonzero scanline fill of one multi-path.
fn fill_path(rgb: &mut [u8], w: usize, h: usize, path: &MultiPath, color: [u8; 3]) {
    if w == 0 || h == 0 {
        return;
    }
    // Collect non-horizontal edges: (x0, y0, x1, y1).
    let mut edges: Vec<(f64, f64, f64, f64)> = Vec::new();
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for sub in &path.subpaths {
        let ring = flatten_subpath(sub.commands.as_slice());
        if ring.len() < 2 {
            continue;
        }
        for i in 0..ring.len() {
            let (x0, y0) = ring[i];
            let (x1, y1) = ring[(i + 1) % ring.len()];
            if y0 == y1 {
                continue;
            }
            min_y = min_y.min(y0).min(y1);
            max_y = max_y.max(y0).max(y1);
            edges.push((x0, y0, x1, y1));
        }
    }
    if edges.is_empty() {
        return;
    }
    let y_first = min_y.floor().max(0.0) as usize;
    let y_last = (max_y.ceil() as usize + 1).min(h);
    for y in y_first..y_last {
        let yc = y as f64 + 0.5; // pixel-center scanline
                                 // Crossings with this scanline: x position + winding direction.
        let mut xs: Vec<(f64, i32)> = Vec::with_capacity(edges.len());
        for &(x0, y0, x1, y1) in &edges {
            let (lo, hi) = if y0 < y1 { (y0, y1) } else { (y1, y0) };
            if yc < lo || yc >= hi {
                continue;
            }
            let t = (yc - y0) / (y1 - y0);
            xs.push((x0 + t * (x1 - x0), if y1 > y0 { 1 } else { -1 }));
        }
        if xs.is_empty() {
            continue;
        }
        xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let row = y * w;
        let mut winding = 0i32;
        let mut span_start = 0.0f64;
        for &(x, dir) in &xs {
            if winding == 0 {
                span_start = x;
            }
            winding += dir;
            if winding == 0 {
                // Fill pixels whose center x+0.5 lies strictly in the span.
                let x0 = ((span_start - 0.5).floor() + 1.0).max(0.0) as usize;
                let x1 = (x - 0.5).ceil().max(0.0) as usize;
                for x in x0..x1.min(w) {
                    let i = (row + x) * 3;
                    rgb[i] = color[0];
                    rgb[i + 1] = color[1];
                    rgb[i + 2] = color[2];
                }
            }
        }
    }
}

/// Flatten one subpath's commands to a closed ring of points.
fn flatten_subpath(commands: &[PathCmd]) -> Vec<Pt> {
    let mut ring: Vec<Pt> = Vec::with_capacity(commands.len());
    let mut start: Pt = (0.0, 0.0);
    for cmd in commands {
        match *cmd {
            PathCmd::MoveTo(p) => {
                start = (p.x, p.y);
                ring.push(start);
            }
            PathCmd::LineTo(p) => ring.push((p.x, p.y)),
            PathCmd::CubicTo(c1, c2, p) => {
                let p0 = *ring.last().unwrap_or(&start);
                flatten_cubic(
                    p0,
                    (c1.x, c1.y),
                    (c2.x, c2.y),
                    (p.x, p.y),
                    FLATTEN_DEPTH,
                    &mut ring,
                );
            }
            PathCmd::Close => {
                // Closing edge is implicit: the ring wraps back to the start.
                ring.push(start);
            }
        }
    }
    // Drop a duplicated closing point so the wrap edge is not degenerate.
    while ring.len() > 1 && ring.first() == ring.last() {
        ring.pop();
    }
    ring
}

/// Subdivide a cubic until flat within [`FLATTEN_TOL`], pushing endpoints.
fn flatten_cubic(p0: Pt, p1: Pt, p2: Pt, p3: Pt, depth: u32, out: &mut Vec<Pt>) {
    if depth == 0 || is_flat(p0, p1, p2, p3) {
        out.push(p3);
        return;
    }
    let p01 = mix(p0, p1);
    let p12 = mix(p1, p2);
    let p23 = mix(p2, p3);
    let p012 = mix(p01, p12);
    let p123 = mix(p12, p23);
    let p0123 = mix(p012, p123);
    flatten_cubic(p0, p01, p012, p0123, depth - 1, out);
    flatten_cubic(p0123, p123, p23, p3, depth - 1, out);
}

/// True when both control points are within the tolerance of the chord.
fn is_flat(p0: Pt, p1: Pt, p2: Pt, p3: Pt) -> bool {
    let (dx, dy) = (p3.0 - p0.0, p3.1 - p0.1);
    let len2 = dx * dx + dy * dy;
    if len2 < 1e-9 {
        // Degenerate chord: compare against the point itself.
        let d1 = (p1.0 - p0.0).hypot(p1.1 - p0.1);
        let d2 = (p2.0 - p0.0).hypot(p2.1 - p0.1);
        return d1.max(d2) < FLATTEN_TOL;
    }
    let d1 = ((p1.0 - p0.0) * dy - (p1.1 - p0.1) * dx).abs() / len2.sqrt();
    let d2 = ((p2.0 - p0.0) * dy - (p2.1 - p0.1) * dx).abs() / len2.sqrt();
    d1.max(d2) < FLATTEN_TOL
}

fn mix(a: Pt, b: Pt) -> Pt {
    ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5)
}

/// Peak-signal-to-noise ratio in dB between two luma buffers, capped at
/// 100 dB so a pixel-perfect match serializes as a finite number in the
/// baseline JSON.
pub fn psnr_db(src: &[f64], out: &[f64]) -> f64 {
    let n = src.len().min(out.len());
    if n == 0 {
        return 100.0;
    }
    let mse = src
        .iter()
        .zip(out.iter())
        .map(|(a, b)| (a - b) * (a - b))
        .sum::<f64>()
        / n as f64;
    if mse <= 1e-9 {
        return 100.0;
    }
    (10.0 * ((255.0 * 255.0) / mse).log10()).min(100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_path(x0: f64, y0: f64, x1: f64, y1: f64) -> MultiPath {
        use vtracer::ir::SubPath;
        let mut mp = MultiPath::new();
        mp.subpaths.push(SubPath {
            commands: vec![
                PathCmd::MoveTo(vtracer::PointF64 { x: x0, y: y0 }),
                PathCmd::LineTo(vtracer::PointF64 { x: x1, y: y0 }),
                PathCmd::LineTo(vtracer::PointF64 { x: x1, y: y1 }),
                PathCmd::LineTo(vtracer::PointF64 { x: x0, y: y1 }),
                PathCmd::Close,
            ],
        });
        mp
    }

    #[test]
    fn fill_path_paints_a_rect() {
        let (w, h) = (10usize, 8usize);
        let mut rgb = vec![255u8; w * h * 3];
        fill_path(&mut rgb, w, h, &rect_path(2.0, 2.0, 6.0, 5.0), [0, 0, 0]);
        let dark = |x: usize, y: usize| rgb[(y * w + x) * 3] == 0;
        assert!(dark(3, 3) && dark(5, 4));
        assert!(!dark(1, 3) && !dark(7, 3) && !dark(3, 6) && !dark(3, 1));
        // Boundaries: pixel centers inside [2,6)x[2,5) are painted.
        assert!(dark(2, 2) && dark(5, 4));
        assert!(!dark(6, 2));
    }
}
