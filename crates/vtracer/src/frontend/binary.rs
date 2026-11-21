use visioncortex::{BinaryImage, Color, ColorImage, PointI32, SummedAreaTable};

use crate::error::Error;
use crate::ir::{Layer, Paint, RegionMask, Segmentation};

use super::Frontend;

/// Grayscale intensity (0..=255) used by every thresholding method. Matches the
/// metric `SummedAreaTable::from_color_image` sums, so fixed and adaptive
/// thresholds agree on what "dark" means.
#[inline]
fn intensity(c: Color) -> u32 {
    (c.r as u32 + c.g as u32 + c.b as u32) / 3
}

/// How the binary frontend separates foreground (dark) from background pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Threshold {
    /// Global cutoff: a pixel is foreground when its intensity is below this
    /// value (0..=255). Fast and predictable; best for clean, evenly-lit input.
    Fixed(u8),
    /// Bradley–Roth adaptive threshold: a pixel is foreground when its
    /// intensity is more than `t` percent below the mean of the surrounding
    /// `window`×`window` block. Handles uneven lighting and shadows that defeat
    /// a single global cutoff. Computed in one pass with a summed-area table,
    /// so it stays O(pixels) regardless of window size.
    Adaptive {
        /// Window side length in pixels; `0` auto-derives ~1/8 of the shorter
        /// image dimension (the value suggested by the paper).
        window: u32,
        /// Sensitivity, as a percentage below the local mean (paper default 15).
        t: f64,
    },
    /// Sauvola adaptive threshold: a pixel is foreground when its intensity is
    /// below `mean * (1 + k * (stddev / R - 1))` over the surrounding
    /// `window`×`window` block (Sauvola et al. 2000; R = 128 is the standard
    /// dynamic range of the standard-deviation term). Like Bradley–Roth it
    /// handles uneven lighting, but the stddev normalization makes it far less
    /// trigger-happy on flat low-contrast areas: on a perfectly uniform region
    /// stddev = 0 and the threshold drops below every pixel, so nothing fires.
    /// Computed in one pass with two integral images (sum and sum-of-squares),
    /// so it stays O(pixels) regardless of window size.
    Sauvola {
        /// Window side length in pixels; `0` auto-derives ~1/8 of the shorter
        /// image dimension (matching [`Threshold::Adaptive`]).
        window: u32,
        /// Contrast sensitivity k (paper default 0.2; larger = more ink).
        k: f64,
    },
}

impl Threshold {
    /// Bradley–Roth adaptive thresholding with the paper's defaults
    /// (auto window, `t = 15`).
    pub const fn adaptive() -> Self {
        Threshold::Adaptive {
            window: 0,
            t: 15.0,
        }
    }
}

impl Default for Threshold {
    fn default() -> Self {
        Threshold::Fixed(128)
    }
}

/// Binary (black/white) frontend: threshold the image then cluster the
/// foreground. Every region is painted black.
///
/// The [`Threshold`] chooses how foreground is separated from background — a
/// fixed global cutoff ([`Threshold::Fixed`]) or Bradley–Roth adaptive
/// thresholding ([`Threshold::Adaptive`]) for scans and photos with uneven
/// lighting.
///
/// Speckle removal drops clusters smaller than `min_area` px as the clusters
/// are collected, matching the pre-1.0 binary path (`cluster.size() >= area`).
#[derive(Debug, Clone)]
pub struct BinaryFrontend {
    /// How foreground pixels are selected (see [`Threshold`]).
    pub threshold: Threshold,
    /// Whether to connect clusters diagonally.
    pub diagonal: bool,
    /// Discard clusters smaller than this many pixels (0 = keep all).
    pub min_area: usize,
}

impl Default for BinaryFrontend {
    fn default() -> Self {
        Self {
            threshold: Threshold::default(),
            diagonal: false,
            min_area: 0,
        }
    }
}

impl BinaryFrontend {
    /// Binarize `img` into a foreground mask according to [`Self::threshold`].
    fn binarize(&self, img: &ColorImage) -> BinaryImage {
        match self.threshold {
            Threshold::Fixed(value) => {
                let value = value as u32;
                img.to_binary_image(|c| intensity(c) < value)
            }
            Threshold::Adaptive { window, t } => adaptive_bradley_roth(img, window, t),
            Threshold::Sauvola { window, k } => sauvola(img, window, k),
        }
    }
}

/// Bradley–Roth adaptive thresholding via a summed-area table.
///
/// For each pixel, compare its intensity to the mean of a surrounding window:
/// it is foreground when `value <= mean * (1 - t/100)`, i.e. more than `t`
/// percent darker than its neighborhood.
fn adaptive_bradley_roth(img: &ColorImage, window: u32, t: f64) -> BinaryImage {
    let (w, h) = (img.width, img.height);
    let sat = SummedAreaTable::from_color_image(img);

    // Window: 0 => auto (~1/8 of the shorter side, per the paper), min 1.
    let side = if window == 0 {
        (w.min(h) / 8).max(1)
    } else {
        window as usize
    };
    let half = side / 2;
    let factor = 1.0 - t.clamp(0.0, 100.0) / 100.0;

    let mut out = BinaryImage::new_w_h(w, h);
    for y in 0..h {
        let y0 = y.saturating_sub(half);
        let y1 = (y + half).min(h - 1);
        for x in 0..w {
            let x0 = x.saturating_sub(half);
            let x1 = (x + half).min(w - 1);

            let count = ((x1 - x0 + 1) * (y1 - y0 + 1)) as f64;
            let sum = sat.get_region_sum_x_y_w_h(x0, y0, x1 - x0 + 1, y1 - y0 + 1) as f64;
            let value = intensity(img.get_pixel(x, y)) as f64;

            // value <= mean * factor  ⇔  value * count <= sum * factor
            out.set_pixel(x, y, value * count <= sum * factor);
        }
    }
    out
}

/// Sauvola adaptive thresholding via integral images.
///
/// For each pixel, compute the mean `m` and stddev `s` of its surrounding
/// window with two summed-area tables (intensity and intensity²), and mark it
/// foreground when `value < m * (1 + k * (s / R - 1))` with R = 128. The
/// stddev term adapts to local contrast: on clean paper (high s between ink
/// and background) it stays close to a midpoint cut; on washed-out or
/// unevenly-lit areas it sinks toward the local mean so only real strokes
/// survive.
fn sauvola(img: &ColorImage, window: u32, k: f64) -> BinaryImage {
    let (w, h) = (img.width, img.height);

    // Integral images: I[i][j] sums the i×j top-left block; I2 sums squares.
    // u64 covers the worst case (255² × w × h) at any practical size.
    let mut i_sum: Vec<u64> = vec![0; (w + 1) * (h + 1)];
    let mut i_sq: Vec<u64> = vec![0; (w + 1) * (h + 1)];
    for y in 0..h {
        for x in 0..w {
            let v = intensity(img.get_pixel(x, y)) as u64;
            let here = (y + 1) * (w + 1) + (x + 1);
            let acc = i_sum[y * (w + 1) + (x + 1)] + i_sum[(y + 1) * (w + 1) + x]
                - i_sum[y * (w + 1) + x];
            let acc2 = i_sq[y * (w + 1) + (x + 1)] + i_sq[(y + 1) * (w + 1) + x]
                - i_sq[y * (w + 1) + x];
            i_sum[here] = acc + v;
            i_sq[here] = acc2 + v * v;
        }
    }

    // Window: 0 => auto (~1/8 of the shorter side, matching the Bradley arm).
    let side = if window == 0 {
        (w.min(h) / 8).max(1)
    } else {
        window as usize
    };
    let half = side / 2;
    let k = k.clamp(0.0, 1.0);

    let region_sum = |i: &[u64], x: usize, y: usize| -> u64 {
        let x1 = (x + half + 1).min(w);
        let y1 = (y + half + 1).min(h);
        let x0 = x.saturating_sub(half);
        let y0 = y.saturating_sub(half);
        // (a + d) − b − c ordering keeps every intermediate non-negative for
        // the u64 tables.
        i[y1 * (w + 1) + x1] + i[y0 * (w + 1) + x0]
            - i[y0 * (w + 1) + x1]
            - i[y1 * (w + 1) + x0]
    };

    let mut out = BinaryImage::new_w_h(w, h);
    for y in 0..h {
        for x in 0..w {
            let count = ((x + half + 1).min(w) - x.saturating_sub(half)) as f64
                * ((y + half + 1).min(h) - y.saturating_sub(half)) as f64;
            let sum = region_sum(&i_sum, x, y) as f64;
            let mean = sum / count;
            let sq = region_sum(&i_sq, x, y) as f64;
            // E[x²] − m², clamped for float error on flat regions.
            let std = (sq / count - mean * mean).max(0.0).sqrt();
            let t = mean * (1.0 + k * (std / 128.0 - 1.0));
            let value = intensity(img.get_pixel(x, y)) as f64;
            out.set_pixel(x, y, value < t);
        }
    }
    out
}

impl Frontend for BinaryFrontend {
    fn segment(&self, img: &ColorImage) -> Result<Segmentation, Error> {
        if img.width == 0 || img.height == 0 {
            return Err(Error::EmptyImage);
        }

        let width = img.width;
        let height = img.height;
        let binary = self.binarize(img);
        let clusters = binary.to_clusters(self.diagonal);

        let mut seg = Segmentation::new(width as u32, height as u32);
        let black = Color::new(0, 0, 0);
        for i in 0..clusters.len() {
            let cluster = clusters.get_cluster(i);
            if cluster.size() < self.min_area {
                continue;
            }
            let mask = RegionMask::new(
                cluster.to_binary_image(),
                PointI32 {
                    x: cluster.rect.left,
                    y: cluster.rect.top,
                },
            );
            seg.layers.push(Layer {
                paint: Paint::Solid(black),
                mask,
            });
        }

        Ok(seg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gray(v: u8, w: usize, h: usize) -> ColorImage {
        let mut img = ColorImage::new_w_h(w, h);
        for y in 0..h {
            for x in 0..w {
                img.set_pixel(x, y, &Color::new(v, v, v));
            }
        }
        img
    }

    fn foreground_count(b: &BinaryImage) -> usize {
        let mut n = 0;
        for y in 0..b.height {
            for x in 0..b.width {
                if b.get_pixel(x, y) {
                    n += 1;
                }
            }
        }
        n
    }

    /// Uniform areas must never fire: stddev is 0, the threshold sinks below
    /// every pixel. (A Fixed cutoff would misclassify a uniform dark field.)
    #[test]
    fn sauvola_ignores_uniform_fields() {
        let img = gray(200, 64, 64);
        let b = BinaryFrontend {
            threshold: Threshold::Sauvola { window: 15, k: 0.2 },
            ..Default::default()
        }
        .binarize(&img);
        assert_eq!(foreground_count(&b), 0, "flat field must produce no ink");
    }

    /// A thin dark stroke on flat paper is recovered in full; interior
    /// windows still see background, so the stddev term keeps them ink.
    #[test]
    fn sauvola_recovers_thin_strokes_on_flat_paper() {
        let w = 64;
        let h = 64;
        let mut img = gray(200, w, h);
        for y in 30..35 {
            for x in 16..48 {
                img.set_pixel(x, y, &Color::new(40, 40, 40));
            }
        }
        let b = BinaryFrontend {
            threshold: Threshold::Sauvola { window: 15, k: 0.2 },
            ..Default::default()
        }
        .binarize(&img);
        for y in 30..35 {
            for x in 16..48 {
                assert!(b.get_pixel(x, y), "stroke pixel ({x},{y}) missed");
            }
        }
        // Far background rows must stay background.
        for x in 0..w {
            assert!(!b.get_pixel(x, 10), "background row fired at x={x}");
        }
    }

    /// Under strong left-to-right lighting that defeats a single global
    /// cutoff, Sauvola keeps the stroke on both the dark and the bright side
    /// while a Fixed threshold floods the dark half with background ink.
    #[test]
    fn sauvola_beats_fixed_threshold_under_uneven_lighting() {
        let w = 128;
        let h = 64;
        let mut img = ColorImage::new_w_h(w, h);
        for y in 0..h {
            for x in 0..w {
                let bg = (30 + 200 * x / w) as u8; // gradient 30..229
                let v = if (30..35).contains(&y) && (16..112).contains(&x) {
                    bg / 4
                } else {
                    bg
                };
                img.set_pixel(x, y, &Color::new(v, v, v));
            }
        }
        let sauv = BinaryFrontend {
            threshold: Threshold::Sauvola { window: 15, k: 0.2 },
            ..Default::default()
        }
        .binarize(&img);
        let fixed = BinaryFrontend {
            threshold: Threshold::Fixed(128),
            ..Default::default()
        }
        .binarize(&img);
        // Stroke recall in the dark half, where the global cutoff drowns.
        let sauv_dark = (30..35)
            .flat_map(|y| (16..64).map(move |x| (x, y)))
            .filter(|(x, y)| sauv.get_pixel(*x, *y))
            .count();
        assert!(
            sauv_dark > 40,
            "sauvola should keep the stroke on the dark side: {sauv_dark}"
        );
        // Background false-positives: the dark half is all sub-128 gray, so
        // the fixed threshold marks it all; Sauvola must not. The true stroke
        // is 96 px wide over 5 rows (480 px); anything else is a false ink.
        let stroke = 96 * 5;
        let fixed_fp = foreground_count(&fixed).saturating_sub(stroke);
        let sauv_fp = foreground_count(&sauv).saturating_sub(stroke);
        assert!(
            sauv_fp < fixed_fp,
            "sauvola FP {sauv_fp} should be far below fixed FP {fixed_fp}"
        );
    }
}
