//! Smart preset: recommend the best trace parameter combination for a source
//! image.
//!
//! Two-stage approach:
//!
//! 1. **Analyze** the original image (on a downsampled thumbnail): effective
//!    color count, colorfulness, grayscale-histogram bimodality, Otsu
//!    threshold, texture/noise, and uneven-lighting statistics. These pick a
//!    short list of candidate parameter families (line art / black-white /
//!    poster / photo / gradient / low-poly / flat).
//! 2. **Trial and score**: each candidate is traced on a downsampled copy of
//!    the image. Fidelity is measured by compositing the segmentation back
//!    into a per-pixel color map and comparing it with the source (RMSE);
//!    complexity is measured from the resulting vector document (SVG size on
//!    a log bytes-per-pixel scale). The candidate with the best
//!    `fidelity − λ·complexity` wins.
//!
//! Pixel-unit parameters (filter speckle, length threshold, adaptive window)
//! are scaled down for the trial resolution and reported back at the original
//! image scale, so the winning params can be applied as-is.

use std::collections::HashMap;
use std::time::Instant;

use serde::Serialize;
use vtracer::config::{Clustering, FitMode, Hierarchical, Preset, SegmentKey};
use vtracer::ir::Segmentation;
use vtracer::{ColorImage, VectorDoc};

use crate::error::Error;

/// Longest side of the analysis thumbnail.
const ANALYSIS_MAX_SIDE: u32 = 512;
/// Longest side of the trial-trace image.
const TEST_MAX_SIDE: u32 = 1024;
/// Bytes-per-pixel at which the complexity metric saturates to 1.0.
const COMPLEXITY_REF_BPP: f64 = 32.0;
/// A variation must beat the preset baseline by at least this score margin to
/// replace it — otherwise the (known-good) preset is kept.
const BASELINE_MARGIN: f64 = 0.02;

/// Per-family evaluation criteria — what "good" means for each preset style.
///
/// Different presets optimize for different things, so a single
/// `fidelity − λ·complexity` score is wrong for all of them:
/// - line art cares about *capturing every stroke* (ink recall), not color;
/// - gradients need *many* colors to avoid banding, so complexity is barely
///   penalized;
/// - low-poly should stay *simple*, so complexity is heavily penalized.
#[derive(Debug, Clone, Copy)]
struct Criteria {
    /// Weight of the complexity penalty in the score.
    lambda: f64,
    /// Measure fidelity as ink recall/precision (binary styles) instead of
    /// plain color RMSE.
    ink_aware: bool,
    /// On line-art-like sources (`stroke_dominant`) judge by dilated stroke
    /// F1 instead of color RMSE, so candidates that drop thin strokes are
    /// penalized (a lost stroke barely moves RMSE; see `stroke_fidelity`).
    stroke_aware: bool,
}

/// The evaluation criteria for a preset family.
fn criteria_for(family: &str) -> Criteria {
    match family {
        // Line art: completeness of the strokes matters most; light penalty
        // for a little extra ink so faint lines aren't dropped. Judged by ink
        // capture (F1), which is far more sensitive to missing strokes than
        // color RMSE (dominated by the white background).
        "lineArt" => Criteria {
            lambda: 0.15,
            ink_aware: true,
            stroke_aware: false,
        },
        // Black & white: whole-image binarization (filled regions), so judge
        // by overall color fidelity rather than stroke capture.
        "bw" => Criteria {
            lambda: 0.2,
            ink_aware: false,
            stroke_aware: true,
        },
        // Color poster: faithful, clean quantization.
        "color" => Criteria {
            lambda: 0.25,
            ink_aware: false,
            stroke_aware: true,
        },
        // Gradient: needs many colors to avoid banding — barely penalize
        // complexity so detail is preserved.
        "gradient" => Criteria {
            lambda: 0.08,
            ink_aware: false,
            stroke_aware: false,
        },
        // Low-poly: should stay simple — heavy complexity penalty.
        "lowPoly" => Criteria {
            lambda: 0.5,
            ink_aware: false,
            stroke_aware: false,
        },
        // Auto-mode families (not user-selectable presets).
        "photo" => Criteria {
            lambda: 0.1,
            ink_aware: false,
            stroke_aware: true,
        },
        "flat" => Criteria {
            lambda: 0.3,
            ink_aware: false,
            stroke_aware: true,
        },
        _ => Criteria {
            lambda: 0.25,
            ink_aware: false,
            stroke_aware: true,
        },
    }
}

// ── Image analysis ──────────────────────────────────────────────────────

/// Stroke-F1 dilation radius in pixels: a stroke may shrink by up to this
/// much during tracing without being counted as lost, but a *dropped*
/// stroke still collapses the score (its dilated source band has no match).
pub const STROKE_DILATE_PX: usize = 2;

/// A source whose content is sparse dark strokes over a light field: the
/// candidates that win for it must *keep* those strokes, so fidelity is
/// judged by stroke F1 rather than color RMSE (which the large white
/// background dominates). Rosette/meander-like patterns hit this; photo and
/// flat color-field images do not.
pub fn stroke_dominant(stats: &ImageStats) -> bool {
    stats.ink_coverage < 0.15 && stats.texture >= 0.05
}

/// Stroke-like *routing* test: this image should get line-art / black-and-
/// white candidates even when the two-tone bimodality gate misses it.
///
/// The failure it fixes: a paper scan's JPEG noise smears the grayscale
/// histogram so bimodality collapses toward 0 (measured: clean scans and
/// their originals alike report bimodality 0 for sparse ink), the two-tone
/// gate misses them, and no binarization is ever trialed. Line-art-like
/// content — fine strokes over a light field, hence at least a faint
/// texture (`texture ≥ 0.025`) — with a small effective palette qualifies;
/// the colorfulness ceiling only keeps vivid photos out (measured grayscale
/// scans sit at 0, so the noise correction there is belt-and-braces).
pub fn stroke_like(stats: &ImageStats) -> bool {
    stats.texture >= 0.025
        && stats.effective_colors <= 24
        && stats.colorfulness - 2.0 * stats.noise < 60.0
}

/// Whether the source is degraded enough (pixel noise or uneven lighting)
/// that enhancing it before analysis is worth the latency. Only meaningful
/// for stroke-like content: photo and flat color-field images inflate
/// `illumination_var` with their own color blocks and must not be taken for
/// unevenly-lit scans. Callers run the Real-ESRGAN preprocess step and fall
/// back to the original on failure.
pub fn needs_preprocess(stats: &ImageStats) -> bool {
    (stroke_dominant(stats) || stroke_like(stats))
        && (stats.noise > 12.0 || stats.illumination_var > 20.0)
}

/// Statistics computed from a downsampled copy of the source image.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageStats {
    /// Original image width (px).
    pub width: u32,
    /// Original image height (px).
    pub height: u32,
    /// Distinct 4-bit-per-channel colors covering ≥ 0.1% of the pixels.
    pub effective_colors: usize,
    /// Hasler–Süsstrunk colorfulness (0 = achromatic, ~100 = vivid).
    pub colorfulness: f64,
    /// Grayscale histogram bimodality coefficient (0..=1; > 0.45 ≈ two-tone).
    pub bimodality: f64,
    /// Otsu threshold of the grayscale histogram (0..=255).
    pub otsu_threshold: u8,
    /// Fraction of pixels with a strong gradient (edge/texture density).
    pub texture: f64,
    /// Fraction of dark pixels (luminance < 128): "ink" coverage. Line art
    /// has low coverage over a light field; photo/flat color images do not.
    pub ink_coverage: f64,
    /// Mean gradient magnitude over near-flat pixels (noise proxy, 0..255).
    pub noise: f64,
    /// Mean grayscale luminance (0..=255).
    pub mean_luminance: f64,
    /// Std-dev of 8×8 block-mean luminance (uneven-lighting proxy, 0..255).
    pub illumination_var: f64,
}

/// Area-average (box) downsample so the longest side is ≤ `max_side`.
/// Returns the image unchanged when it already fits.
pub fn downsample(img: &ColorImage, max_side: u32) -> ColorImage {
    let (w, h) = (img.width, img.height);
    if w.max(h) <= max_side as usize {
        return img.clone();
    }
    let scale = max_side as f64 / w.max(h) as f64;
    let nw = ((w as f64) * scale).round().max(1.0) as usize;
    let nh = ((h as f64) * scale).round().max(1.0) as usize;
    let mut out = vec![0u8; nw * nh * 4];
    for dy in 0..nh {
        let y0 = (dy as f64 / scale) as usize;
        let y1 = (((dy + 1) as f64) / scale) as usize;
        let y1 = y1.max(y0 + 1).min(h);
        for dx in 0..nw {
            let x0 = (dx as f64 / scale) as usize;
            let x1 = (((dx + 1) as f64) / scale) as usize;
            let x1 = x1.max(x0 + 1).min(w);
            let (mut r, mut g, mut b, mut a) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            for y in y0..y1 {
                let row = y * w * 4;
                for x in x0..x1 {
                    let i = row + x * 4;
                    r += img.pixels[i] as f64;
                    g += img.pixels[i + 1] as f64;
                    b += img.pixels[i + 2] as f64;
                    a += img.pixels[i + 3] as f64;
                }
            }
            let n = ((y1 - y0) as f64 * (x1 - x0) as f64).max(1.0);
            let o = (dy * nw + dx) * 4;
            out[o] = (r / n).round() as u8;
            out[o + 1] = (g / n).round() as u8;
            out[o + 2] = (b / n).round() as u8;
            out[o + 3] = (a / n).round() as u8;
        }
    }
    ColorImage {
        pixels: out,
        width: nw,
        height: nh,
    }
}

/// Compute [`ImageStats`] from a downsampled copy of the image.
pub fn analyze_image(img: &ColorImage) -> ImageStats {
    let thumb = downsample(img, ANALYSIS_MAX_SIDE);
    let (w, h) = (thumb.width, thumb.height);
    let n = (w * h).max(1);
    let n_f = n as f64;

    let mut lum = vec![0f64; n];
    let mut hist = [0f64; 256];
    let (mut sum_rg, mut sum_yb, mut sum_rg2, mut sum_yb2) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut buckets: HashMap<u32, usize> = HashMap::new();
    let mut dark = 0usize;

    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let (r, g, b) = (
                thumb.pixels[i] as f64,
                thumb.pixels[i + 1] as f64,
                thumb.pixels[i + 2] as f64,
            );
            let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            lum[y * w + x] = l;
            if l < 128.0 {
                dark += 1;
            }
            hist[l.round().clamp(0.0, 255.0) as usize] += 1.0;
            let rg = r - g;
            let yb = 0.5 * (r + g) - b;
            sum_rg += rg;
            sum_yb += yb;
            sum_rg2 += rg * rg;
            sum_yb2 += yb * yb;
            // 4 bits per channel -> 12-bit bucket key.
            let key = ((thumb.pixels[i] >> 4) as u32) << 8
                | ((thumb.pixels[i + 1] >> 4) as u32) << 4
                | (thumb.pixels[i + 2] >> 4) as u32;
            *buckets.entry(key).or_insert(0) += 1;
        }
    }

    let mean_luminance = lum.iter().sum::<f64>() / n_f;

    // Colorfulness (Hasler–Süsstrunk).
    let mean_rg = sum_rg / n_f;
    let mean_yb = sum_yb / n_f;
    let std_rg = (sum_rg2 / n_f - mean_rg * mean_rg).max(0.0).sqrt();
    let std_yb = (sum_yb2 / n_f - mean_yb * mean_yb).max(0.0).sqrt();
    let colorfulness = (std_rg * std_rg + std_yb * std_yb).sqrt()
        + 0.3 * (mean_rg * mean_rg + mean_yb * mean_yb).sqrt();

    // Effective colors: quantized buckets covering ≥ 0.1% of the pixels.
    let min_count = (n_f * 0.001).max(1.0) as usize;
    let effective_colors = buckets.values().filter(|&&c| c >= min_count).count();

    let (bimodality, otsu_threshold) = histogram_stats(&hist);
    let (texture, noise) = gradient_stats(&lum, w, h);
    let illumination_var = block_luminance_std(&lum, w, h);

    ImageStats {
        width: img.width as u32,
        height: img.height as u32,
        effective_colors,
        colorfulness,
        bimodality,
        otsu_threshold,
        texture,
        ink_coverage: dark as f64 / n_f,
        noise,
        mean_luminance,
        illumination_var,
    }
}

/// Bimodality score + Otsu threshold from a grayscale histogram.
///
/// The score is peak-based (robust for symmetric two-tone images, where
/// moment-based coefficients vanish): smooth the histogram, find the two
/// strongest well-separated peaks, and score by the relative valley depth
/// between them. The second peak only needs ≥ 5% of the main peak's mass
/// (line art's dark peak is legitimately small), and the height ratio
/// saturates at 15%, so asymmetric two-tone images still score ≈1.
/// ≈0 for unimodal or flat histograms.
fn histogram_stats(hist: &[f64; 256]) -> (f64, u8) {
    let total = hist.iter().sum::<f64>();
    if total == 0.0 {
        return (0.0, 128);
    }

    // 5-bin box smoothing.
    let mut smooth = [0f64; 256];
    for (i, slot) in smooth.iter_mut().enumerate() {
        let (mut s, mut c) = (0.0f64, 0usize);
        for j in (i as i32 - 2)..=(i as i32 + 2) {
            if (0..256).contains(&j) {
                s += hist[j as usize];
                c += 1;
            }
        }
        *slot = s / c.max(1) as f64;
    }

    // Peak 1: global max. Peak 2: strongest bin at least 16 away from it.
    let p1 = (0..256usize)
        .max_by(|a, b| smooth[*a].total_cmp(&smooth[*b]))
        .unwrap();
    let bimodality = if smooth[p1] > 0.0 {
        let p2 = (0..256usize)
            .filter(|&i| i.abs_diff(p1) >= 16)
            .max_by(|a, b| smooth[*a].total_cmp(&smooth[*b]))
            .filter(|&i| smooth[i] >= 0.05 * smooth[p1]);
        match p2 {
            Some(p2) => {
                let (lo, hi) = if p1 < p2 { (p1, p2) } else { (p2, p1) };
                let valley = (lo..=hi).map(|i| smooth[i]).fold(f64::INFINITY, f64::min);
                let peak = smooth[p1].max(smooth[p2]);
                let second = smooth[p1].min(smooth[p2]);
                let depth = 1.0 - valley / peak.max(1e-9);
                // The relative-height factor saturates at 15%: line art
                // legitimately has a small dark peak (thin strokes cover only
                // a few percent of the image), so asymmetry beyond that is
                // not penalized further. The 5% gate above already rejects
                // pure noise bumps.
                let ratio = (second / peak) / 0.15;
                (depth * ratio.min(1.0)).clamp(0.0, 1.0)
            }
            None => 0.0,
        }
    } else {
        0.0
    };

    // Otsu's method: maximize the between-class variance. Perfectly separated
    // classes give a flat plateau of equal maxima — take its midpoint so the
    // threshold lands between the two peaks instead of at the first bin.
    let sum_total: f64 = (0..256).map(|i| i as f64 * hist[i]).sum();
    let (mut sum_b, mut w_b) = (0.0f64, 0.0f64);
    let (mut best, mut best_lo, mut best_hi) = (-1.0f64, 128, 128);
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
        let m_f = (sum_total - sum_b) / w_f;
        let between = w_b * w_f * (m_b - m_f) * (m_b - m_f);
        if between > best {
            best = between;
            best_lo = t;
            best_hi = t;
        } else if (between - best).abs() < 1e-9 {
            best_hi = t;
        }
    }
    (bimodality, ((best_lo + best_hi) / 2) as u8)
}

/// Sobel-based texture (strong-edge fraction) and noise (mean gradient over
/// near-flat pixels) statistics.
fn gradient_stats(lum: &[f64], w: usize, h: usize) -> (f64, f64) {
    if w < 3 || h < 3 {
        return (0.0, 0.0);
    }
    let (mut strong, mut flat_sum, mut flat_count) = (0usize, 0.0f64, 0usize);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            let gx = -lum[i - w - 1] - 2.0 * lum[i - 1] - lum[i + w - 1]
                + lum[i - w + 1]
                + 2.0 * lum[i + 1]
                + lum[i + w + 1];
            let gy = -lum[i - w - 1] - 2.0 * lum[i - w] - lum[i - w + 1]
                + lum[i + w - 1]
                + 2.0 * lum[i + w]
                + lum[i + w + 1];
            let mag = (gx * gx + gy * gy).sqrt();
            if mag > 64.0 {
                strong += 1;
            } else if mag < 32.0 {
                flat_sum += mag;
                flat_count += 1;
            }
        }
    }
    let inner = ((w - 2) * (h - 2)).max(1) as f64;
    let texture = strong as f64 / inner;
    let noise = if flat_count > 0 {
        flat_sum / flat_count as f64
    } else {
        0.0
    };
    (texture, noise)
}

/// Std-dev of 8×8 block-mean luminance (uneven-lighting proxy).
fn block_luminance_std(lum: &[f64], w: usize, h: usize) -> f64 {
    const B: usize = 8;
    let mut sums: Vec<f64> = Vec::new();
    for by in (0..h).step_by(B) {
        for bx in (0..w).step_by(B) {
            let (mut s, mut c) = (0.0f64, 0usize);
            for y in by..(by + B).min(h) {
                for x in bx..(bx + B).min(w) {
                    s += lum[y * w + x];
                    c += 1;
                }
            }
            if c > 0 {
                sums.push(s / c as f64);
            }
        }
    }
    if sums.len() < 2 {
        return 0.0;
    }
    let mean = sums.iter().sum::<f64>() / sums.len() as f64;
    (sums.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / sums.len() as f64).sqrt()
}

// ── Candidate parameters ────────────────────────────────────────────────────

/// A full trace parameter set, serialized in the same camelCase shape as the
/// web editor's `DEFAULT_PARAMS` so the UI can apply it directly.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartParams {
    pub preset: String,
    pub clustering: String,
    pub hierarchical: String,
    pub color_precision: u8,
    pub layer_difference: u8,
    pub filter_speckle: u32,
    pub mode: String,
    pub simplify: f64,
    pub path_precision: u8,
    pub max_colors: Option<usize>,
    pub line_art: bool,
    pub binary_adaptive: bool,
    pub binary_threshold: u8,
    pub binary_adaptive_t: f64,
    pub binary_adaptive_window: u32,
    pub watershed_detail: u32,
    pub palette: String,
    pub layers: usize,
    pub layer_strategy: String,
    pub complexity_threshold: u8,
    pub corner_threshold: i32,
    pub length_threshold: f64,
    pub optimize: u8,
}

impl Default for SmartParams {
    /// Mirrors the web editor's `DEFAULT_PARAMS`.
    fn default() -> Self {
        Self {
            preset: "poster".into(),
            clustering: "color".into(),
            hierarchical: "stacked".into(),
            color_precision: 6,
            layer_difference: 16,
            filter_speckle: 4,
            mode: "spline".into(),
            simplify: 0.1,
            path_precision: 2,
            max_colors: None,
            line_art: false,
            binary_adaptive: false,
            binary_threshold: 128,
            binary_adaptive_t: 15.0,
            binary_adaptive_window: 0,
            watershed_detail: 128,
            palette: String::new(),
            layers: 0,
            layer_strategy: "none".into(),
            complexity_threshold: 70,
            corner_threshold: 60,
            length_threshold: 4.0,
            optimize: 1,
        }
    }
}

impl SmartParams {
    /// Convert to a concrete VTracer config (same mapping as the server's
    /// `build_vtracer_config`).
    pub fn to_vtracer_config(&self) -> vtracer::Config {
        let preset = match self.preset.as_str() {
            "bw" => Preset::Bw,
            "photo" => Preset::Photo,
            _ => Preset::Poster,
        };
        let mut config = vtracer::Config::from_preset(preset);
        config.clustering = match self.clustering.as_str() {
            "binary" => Clustering::Binary,
            "watershed" => Clustering::Watershed,
            _ => Clustering::ColorCluster,
        };
        config.hierarchical = match self.hierarchical.as_str() {
            "cutout" => Hierarchical::Cutout,
            _ => Hierarchical::Stacked,
        };
        config.color_precision = self.color_precision as i32;
        config.layer_difference = self.layer_difference as i32;
        config.filter_speckle = self.filter_speckle as usize;
        config.mode = match self.mode.as_str() {
            "pixel" => FitMode::Pixel,
            "polygon" => FitMode::Polygon,
            _ => FitMode::Spline,
        };
        config.simplify = Some(self.simplify);
        config.path_precision = Some(self.path_precision as u32);
        if let Some(mc) = self.max_colors {
            config.max_colors = Some(mc);
        }
        config.binary_adaptive = self.binary_adaptive;
        config.binary_threshold = self.binary_threshold;
        config.binary_adaptive_t = self.binary_adaptive_t;
        config.binary_adaptive_window = self.binary_adaptive_window;
        config.watershed_detail = self.watershed_detail;
        config.corner_threshold = self.corner_threshold;
        config.length_threshold = self.length_threshold;
        config.optimize = self.optimize;
        config
    }
}

/// One trial candidate: a named parameter family with its full params at the
/// original image scale. `label` is an i18n key (a preset family id, or
/// "photo" / "flat" / "presetDefault") translated by the frontend.
#[derive(Debug, Clone)]
pub struct SmartCandidate {
    /// Machine family id ("lineArt", "bw", "color", "photo", "gradient",
    /// "lowPoly", "flat").
    pub family: &'static str,
    /// Human label (Chinese) for the report.
    pub label: &'static str,
    /// Full parameter set at the original image scale.
    pub params: SmartParams,
    /// True for the preset-default baseline candidate (the exact web preset).
    pub is_baseline: bool,
}

/// Pick the candidate families to trial from the image statistics.
///
/// The families mirror the editor's existing presets (so the recommendation
/// stays within familiar territory) with noise-adaptive tweaks:
/// - two-tone / line-art image  -> 线稿 / 黑白 / 彩色填充
/// - flat illustration          -> 彩色填充 / 扁平色块 / 低多边形
/// - photo-like                  -> 照片 / 渐变 / 彩色填充
/// - mixed                       -> 彩色填充 / 照片 / 低多边形 / 扁平色块
pub fn candidate_params(stats: &ImageStats) -> Vec<SmartCandidate> {
    // Noise-adaptive speckle: clean images keep the base, noisy ones grow it.
    let speckle = |base: u32| (base as f64 + stats.noise / 6.0).round().clamp(0.0, 20.0) as u32;
    let adaptive = stats.illumination_var > 20.0;

    let two_tone = stats.bimodality >= 0.45 && stats.colorfulness < 20.0;
    let mut out = Vec::new();
    if two_tone {
        out.push(line_art_candidate(stats, adaptive, speckle(4)));
        out.push(bw_candidate(stats, adaptive, speckle(4)));
        out.push(poster_candidate(speckle(4)));
    } else if stats.effective_colors <= 12 && stats.texture < 0.2 {
        out.push(poster_candidate(speckle(4)));
        out.push(flat_candidate(speckle(4)));
        out.push(low_poly_candidate(speckle(4)));
    } else if stats.effective_colors >= 48 || stats.texture >= 0.35 {
        out.push(photo_candidate(speckle(8)));
        out.push(gradient_candidate(speckle(8)));
        out.push(poster_candidate(speckle(4)));
    } else {
        out.push(poster_candidate(speckle(4)));
        out.push(photo_candidate(speckle(8)));
        out.push(low_poly_candidate(speckle(4)));
        out.push(flat_candidate(speckle(4)));
    }
    // Stroke-like sources deserve binarization trials regardless of how the
    // branch above routed: a scanned pattern's JPEG noise smears the
    // grayscale histogram so bimodality collapses and the two-tone gate
    // misses it, and even when the gate does hit, the Bradley–Roth pairs
    // survive uneven paper lighting better than global-Otsu variants.
    // Additive: nothing from the branch above is dropped, trial scoring
    // decides.
    if stroke_like(stats) {
        for t in [15.0, 25.0] {
            out.push(bradley_line_art_candidate(stats, speckle(4), t));
            out.push(bradley_bw_candidate(stats, speckle(4), t));
        }
    }
    out
}

/// Valid family ids — mirrors the web editor's `MODE_PRESETS` keys.
pub const FAMILIES: [&str; 5] = ["lineArt", "color", "bw", "gradient", "lowPoly"];

/// The exact default parameter set for a preset family — mirrors the web
/// editor's `MODE_PRESETS` (keep in sync). Used as the baseline candidate so
/// smart tuning is always compared against the preset itself, and can never
/// be worse than it.
pub fn preset_params(family: &str) -> Option<SmartParams> {
    let mut p = SmartParams::default();
    Some(match family {
        // 线稿: bw + binary + lineArt, fixed threshold 128, adaptive off.
        "lineArt" => {
            p.preset = "bw".into();
            p.clustering = "binary".into();
            p.line_art = true;
            p.mode = "spline".into();
            p
        }
        // 彩色填充: already the SmartParams default (poster/color/spline/6/16).
        "color" => p,
        // 黑白: bw + binary + adaptive threshold on.
        "bw" => {
            p.preset = "bw".into();
            p.clustering = "binary".into();
            p.binary_adaptive = true;
            p.mode = "spline".into();
            p.filter_speckle = 4;
            p
        }
        // 渐变: photo + fine gradient (precision 8 / diff 4 / simplify 0.2).
        "gradient" => {
            p.preset = "photo".into();
            p.clustering = "color".into();
            p.color_precision = 8;
            p.layer_difference = 4;
            p.mode = "spline".into();
            p.simplify = 0.2;
            p
        }
        // 低多边形: poster + coarse (precision 4 / diff 32) + polygon fit.
        "lowPoly" => {
            p.preset = "poster".into();
            p.clustering = "color".into();
            p.color_precision = 4;
            p.layer_difference = 32;
            p.mode = "polygon".into();
            p
        }
        _ => return None,
    })
}

/// Trial candidates within one preset family.
///
/// The user has already chosen *what kind of output* they want (the family);
/// this varies the tunable parameters around the preset defaults — with
/// noise/lighting-adaptive tweaks from the image statistics — so the trial
/// scoring can find the best settings *for this image* without ever leaving
/// the chosen family.
pub fn family_candidates(stats: &ImageStats, family: &str) -> Vec<SmartCandidate> {
    let speckle = |base: u32| (base as f64 + stats.noise / 6.0).round().clamp(0.0, 20.0) as u32;
    let adaptive = stats.illumination_var > 20.0;
    let mut out = Vec::new();
    match family {
        "lineArt" => {
            // Otsu threshold, fixed vs adaptive (uneven lighting).
            out.push(line_art_candidate(stats, false, speckle(4)));
            out.push(line_art_candidate(stats, adaptive, speckle(4)));
            // Looser threshold to keep faint strokes (skip at the extremes).
            if (20..236).contains(&stats.otsu_threshold) {
                let mut c = line_art_candidate(stats, false, speckle(4));
                c.params.binary_threshold = (stats.otsu_threshold as i32 - 20).clamp(0, 255) as u8;
                out.push(c);
            }
            // Bradley–Roth adaptive pairs: local-mean thresholding keeps
            // faint strokes on unevenly lit scans regardless of the global
            // lighting statistic that gates `adaptive` above.
            for t in [15.0, 25.0] {
                out.push(bradley_line_art_candidate(stats, speckle(4), t));
            }
        }
        "bw" => {
            out.push(bw_candidate(stats, adaptive, speckle(4)));
            out.push(bw_candidate(stats, false, speckle(4)));
            if (20..236).contains(&stats.otsu_threshold) {
                let mut c = bw_candidate(stats, false, speckle(4));
                c.params.binary_threshold = (stats.otsu_threshold as i32 + 20).clamp(0, 255) as u8;
                out.push(c);
            }
            for t in [15.0, 25.0] {
                out.push(bradley_bw_candidate(stats, speckle(4), t));
            }
        }
        "color" => {
            // Preset default (precision 6 / diff 16), coarser, finer.
            out.push(poster_candidate(speckle(4)));
            let mut coarse = poster_candidate(speckle(4));
            coarse.params.color_precision = 4;
            coarse.params.layer_difference = 24;
            out.push(coarse);
            let mut fine = poster_candidate(speckle(4));
            fine.params.color_precision = 8;
            fine.params.layer_difference = 8;
            out.push(fine);
        }
        "gradient" => {
            // Preset default (precision 8 / diff 4 / simplify 0.2).
            out.push(gradient_candidate(speckle(8)));
            let mut coarse = gradient_candidate(speckle(8));
            coarse.params.layer_difference = 8;
            coarse.params.simplify = 0.3;
            out.push(coarse);
            let mut fine = gradient_candidate(speckle(8));
            fine.params.layer_difference = 2;
            fine.params.simplify = 0.1;
            out.push(fine);
        }
        "lowPoly" => {
            // Preset default (precision 4 / diff 32 / polygon fit).
            out.push(low_poly_candidate(speckle(4)));
            let mut coarse = low_poly_candidate(speckle(4));
            coarse.params.color_precision = 3;
            coarse.params.layer_difference = 48;
            out.push(coarse);
            let mut fine = low_poly_candidate(speckle(4));
            fine.params.color_precision = 5;
            fine.params.layer_difference = 16;
            out.push(fine);
        }
        _ => {}
    }
    out
}

fn line_art_candidate(stats: &ImageStats, adaptive: bool, speckle: u32) -> SmartCandidate {
    let p = SmartParams {
        preset: "bw".into(),
        clustering: "binary".into(),
        line_art: true,
        filter_speckle: speckle,
        binary_adaptive: adaptive,
        binary_threshold: stats.otsu_threshold,
        ..Default::default()
    };
    SmartCandidate {
        family: "lineArt",
        label: "lineArt",
        params: p,
        is_baseline: false,
    }
}

/// Line art with Bradley–Roth adaptive thresholding: `t` percent below the
/// local mean, auto window (`binary_adaptive_window = 0`).
fn bradley_line_art_candidate(stats: &ImageStats, speckle: u32, t: f64) -> SmartCandidate {
    let mut c = line_art_candidate(stats, true, speckle);
    c.params.binary_adaptive_window = 0;
    c.params.binary_adaptive_t = t;
    c
}

/// Black & white with Bradley–Roth adaptive thresholding (see above).
fn bradley_bw_candidate(stats: &ImageStats, speckle: u32, t: f64) -> SmartCandidate {
    let mut c = bw_candidate(stats, true, speckle);
    c.params.binary_adaptive_window = 0;
    c.params.binary_adaptive_t = t;
    c
}

fn bw_candidate(stats: &ImageStats, adaptive: bool, speckle: u32) -> SmartCandidate {
    let p = SmartParams {
        preset: "bw".into(),
        clustering: "binary".into(),
        filter_speckle: speckle,
        binary_adaptive: adaptive,
        binary_threshold: stats.otsu_threshold,
        ..Default::default()
    };
    SmartCandidate {
        family: "bw",
        label: "bw",
        params: p,
        is_baseline: false,
    }
}

fn poster_candidate(speckle: u32) -> SmartCandidate {
    let p = SmartParams {
        filter_speckle: speckle,
        ..Default::default()
    };
    SmartCandidate {
        family: "color",
        label: "color",
        params: p,
        is_baseline: false,
    }
}

fn photo_candidate(speckle: u32) -> SmartCandidate {
    let p = SmartParams {
        preset: "photo".into(),
        color_precision: 8,
        layer_difference: 48,
        filter_speckle: speckle.max(8),
        corner_threshold: 180,
        simplify: 0.2,
        ..Default::default()
    };
    SmartCandidate {
        family: "photo",
        label: "photo",
        params: p,
        is_baseline: false,
    }
}

fn gradient_candidate(speckle: u32) -> SmartCandidate {
    let p = SmartParams {
        preset: "photo".into(),
        color_precision: 8,
        layer_difference: 4,
        filter_speckle: speckle,
        simplify: 0.2,
        ..Default::default()
    };
    SmartCandidate {
        family: "gradient",
        label: "gradient",
        params: p,
        is_baseline: false,
    }
}

fn low_poly_candidate(speckle: u32) -> SmartCandidate {
    let p = SmartParams {
        color_precision: 4,
        layer_difference: 32,
        filter_speckle: speckle,
        mode: "polygon".into(),
        ..Default::default()
    };
    SmartCandidate {
        family: "lowPoly",
        label: "lowPoly",
        params: p,
        is_baseline: false,
    }
}

fn flat_candidate(speckle: u32) -> SmartCandidate {
    let p = SmartParams {
        clustering: "watershed".into(),
        filter_speckle: speckle,
        ..Default::default()
    };
    SmartCandidate {
        family: "flat",
        label: "flat",
        params: p,
        is_baseline: false,
    }
}

/// Scale the pixel-unit parameters from the original image scale to a test
/// resolution (`scale = test / original`, ≤ 1).
fn scale_params(p: &SmartParams, scale: f64) -> SmartParams {
    if scale >= 1.0 {
        return p.clone();
    }
    SmartParams {
        filter_speckle: ((p.filter_speckle as f64) * scale).round().clamp(0.0, 20.0) as u32,
        length_threshold: p.length_threshold * scale,
        binary_adaptive_window: if p.binary_adaptive_window == 0 {
            0 // 0 = auto (scales with the image itself)
        } else {
            ((p.binary_adaptive_window as f64) * scale).round().max(1.0) as u32
        },
        ..p.clone()
    }
}

// ── Trial scoring ───────────────────────────────────────────────────────────

/// Scored outcome of one trial candidate.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateReport {
    pub family: String,
    pub label: String,
    /// Full parameter set at the original image scale.
    pub params: SmartParams,
    /// 0..=1 — how closely the trace reproduces the source colors.
    pub fidelity: f64,
    /// 0..=1 — how heavy the vector output is (SVG size vs image area).
    pub complexity: f64,
    /// `fidelity − λ·complexity` (per-family λ); higher is better.
    pub score: f64,
    /// Stroke-recall F1 (see `stroke_fidelity`) for this candidate, present
    /// only when the source is stroke-dominant — the case where plain
    /// fidelity can hide a lost stroke. Surfaces *why* a candidate won.
    pub stroke_fidelity: Option<f64>,
    /// True for the preset-default baseline candidate.
    pub is_baseline: bool,
    pub shape_count: usize,
    pub command_count: usize,
    pub svg_bytes: usize,
    pub elapsed_ms: u64,
}

/// The smart recommendation: the winning params plus the full trial report.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartDecision {
    /// Winning parameter set at the original image scale — apply as-is.
    pub params: SmartParams,
    /// Family id of the winner.
    pub family: String,
    /// i18n key for the human-readable reason: "scoped" (search restricted to
    /// one preset family, incl. the preset baseline) or "auto" (full search).
    /// The frontend translates it from `stats`, `candidates` and `family`.
    pub reason_key: String,
    /// Source image statistics that drove the candidate selection.
    pub stats: ImageStats,
    /// All trial candidates, best score first.
    pub candidates: Vec<CandidateReport>,
    /// The preset-default baseline report (for comparison), when the search
    /// was restricted to a preset family.
    pub preset: Option<CandidateReport>,
    /// True when a noisy / unevenly-lit source was run through the enhance
    /// step before analysis (see `needs_preprocess`); the callers that can
    /// preprocess set it, the core never does on its own.
    pub preprocessed: bool,
    /// Total elapsed time (ms).
    pub elapsed_ms: u64,
}

/// Composite the segmentation layers into an RGB canvas (white background).
/// Layers are stored bottom-to-top; the topmost paints first and wins, so
/// lower layers only fill the gaps. Shared by the fidelity metrics.
fn composite_seg(seg: &Segmentation, w: usize, h: usize) -> Vec<u8> {
    let n = (w * h).max(1);
    let mut canvas = vec![255u8; n * 3];
    let mut filled = vec![false; n];
    for layer in seg.layers.iter().rev() {
        let [r, g, b] = layer.paint.color().rgb_u8();
        let m = &layer.mask;
        let (ox, oy) = (m.offset.x as isize, m.offset.y as isize);
        for y in 0..m.image.height {
            let py = (oy + y as isize) as usize;
            if py >= h {
                continue;
            }
            for x in 0..m.image.width {
                let px = (ox + x as isize) as usize;
                if px >= w || !m.image.get_pixel(x, y) {
                    continue;
                }
                let i = py * w + px;
                if filled[i] {
                    continue;
                }
                filled[i] = true;
                canvas[i * 3] = r;
                canvas[i * 3 + 1] = g;
                canvas[i * 3 + 2] = b;
            }
        }
    }
    canvas
}

/// Harmonic mean of precision and recall from the binary confusion counts.
fn f1(tp: usize, fp: usize, fn_: usize) -> f64 {
    if tp == 0 {
        // No ink captured. If the source has no ink either, that's a perfect
        // (empty) match; otherwise it's a total miss.
        return if fn_ == 0 { 1.0 } else { 0.0 };
    }
    let recall = tp as f64 / (tp + fn_) as f64;
    let precision = tp as f64 / (tp + fp) as f64;
    (2.0 * precision * recall / (precision + recall).max(1e-9)).clamp(0.0, 1.0)
}

/// Dilate a binary mask by a square `r`-pixel neighborhood (two separable
/// max passes: horizontal then vertical). Public so the CLI quality gate
/// dilates its F1 masks identically to the smart scoring.
pub fn dilate_mask(mask: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    if r == 0 {
        return mask.to_vec();
    }
    let mut tmp = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let x1 = (x + r + 1).min(w);
            if mask[y * w + x.saturating_sub(r)..y * w + x1]
                .iter()
                .any(|&v| v)
            {
                tmp[y * w + x] = true;
            }
        }
    }
    let mut out = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let y1 = (y + r + 1).min(h);
            for yy in y.saturating_sub(r)..y1 {
                if tmp[yy * w + x] {
                    out[y * w + x] = true;
                    break;
                }
            }
        }
    }
    out
}

/// Composite the segmentation (top-to-bottom, first paint wins) into an RGB
/// canvas and measure the RMSE color distance to the source image.
/// Returns 1.0 for a pixel-perfect reproduction.
pub fn color_fidelity(seg: &Segmentation, img: &ColorImage) -> f64 {
    let (w, h) = (seg.width as usize, seg.height as usize);
    let n = (w * h).max(1);
    let canvas = composite_seg(seg, w, h);
    let mut se = 0.0f64;
    for i in 0..n {
        let dr = canvas[i * 3] as f64 - img.pixels[i * 4] as f64;
        let dg = canvas[i * 3 + 1] as f64 - img.pixels[i * 4 + 1] as f64;
        let db = canvas[i * 3 + 2] as f64 - img.pixels[i * 4 + 2] as f64;
        se += dr * dr + dg * dg + db * db;
    }
    let rmse = (se / (n * 3) as f64).sqrt();
    (1.0 - rmse / 255.0).clamp(0.0, 1.0)
}

/// Ink F1 for binary styles (line art / black & white): how well the traced
/// mask captures the source's dark content. A pixel is "ink" when its
/// luminance is below 128. Returns the harmonic mean of recall (did we keep
/// every stroke?) and precision (did we avoid adding speckle?). This is far
/// more sensitive to missing/extra strokes than plain color RMSE, which is
/// dominated by the large white background.
pub fn ink_fidelity(seg: &Segmentation, img: &ColorImage) -> f64 {
    let (w, h) = (seg.width as usize, seg.height as usize);
    let n = (w * h).max(1);
    let mut canvas = vec![255u8; n * 3];
    let mut filled = vec![false; n];
    for layer in seg.layers.iter().rev() {
        let [r, g, b] = layer.paint.color().rgb_u8();
        let m = &layer.mask;
        let (ox, oy) = (m.offset.x as isize, m.offset.y as isize);
        for y in 0..m.image.height {
            let py = (oy + y as isize) as usize;
            if py >= h {
                continue;
            }
            for x in 0..m.image.width {
                let px = (ox + x as isize) as usize;
                if px >= w || !m.image.get_pixel(x, y) {
                    continue;
                }
                let i = py * w + px;
                if filled[i] {
                    continue;
                }
                filled[i] = true;
                canvas[i * 3] = r;
                canvas[i * 3 + 1] = g;
                canvas[i * 3 + 2] = b;
            }
        }
    }
    let luma = |i: usize| {
        0.2126 * canvas[i * 3] as f64
            + 0.7152 * canvas[i * 3 + 1] as f64
            + 0.0722 * canvas[i * 3 + 2] as f64
    };
    let src_luma = |i: usize| {
        0.2126 * img.pixels[i * 4] as f64
            + 0.7152 * img.pixels[i * 4 + 1] as f64
            + 0.0722 * img.pixels[i * 4 + 2] as f64
    };
    let (mut tp, mut fp, mut fn_) = (0usize, 0usize, 0usize);
    for i in 0..n {
        let s_ink = src_luma(i) < 128.0;
        let o_ink = luma(i) < 128.0;
        match (s_ink, o_ink) {
            (true, true) => tp += 1,
            (false, true) => fp += 1,
            (true, false) => fn_ += 1,
            (false, false) => {}
        }
    }
    f1(tp, fp, fn_)
}

/// Stroke-recall fidelity for line-art-like sources: binarize the source and
/// the composited result by luminance, dilate both ink masks by
/// [`STROKE_DILATE_PX`] (so a stroke that merely *thinned* during tracing
/// still matches), and report the F1 of the dilated masks. Unlike color
/// RMSE — which a dropped 1-px stroke barely moves because the white
/// background dominates — this collapses when strokes are *lost*, which is
/// exactly the failure the smart metric must not hide (see the 2026-11
/// quality evaluation, finding F1).
pub fn stroke_fidelity(seg: &Segmentation, img: &ColorImage) -> f64 {
    let (w, h) = (seg.width as usize, seg.height as usize);
    let n = (w * h).max(1);
    let canvas = composite_seg(seg, w, h);
    let mut src_ink = vec![false; n];
    let mut out_ink = vec![false; n];
    for i in 0..n {
        let sl = 0.2126 * img.pixels[i * 4] as f64
            + 0.7152 * img.pixels[i * 4 + 1] as f64
            + 0.0722 * img.pixels[i * 4 + 2] as f64;
        let ol = 0.2126 * canvas[i * 3] as f64
            + 0.7152 * canvas[i * 3 + 1] as f64
            + 0.0722 * canvas[i * 3 + 2] as f64;
        src_ink[i] = sl < 128.0;
        out_ink[i] = ol < 128.0;
    }
    let src_ink = dilate_mask(&src_ink, w, h, STROKE_DILATE_PX);
    let out_ink = dilate_mask(&out_ink, w, h, STROKE_DILATE_PX);
    let (mut tp, mut fp, mut fn_) = (0usize, 0usize, 0usize);
    for i in 0..n {
        match (src_ink[i], out_ink[i]) {
            (true, true) => tp += 1,
            (false, true) => fp += 1,
            (true, false) => fn_ += 1,
            (false, false) => {}
        }
    }
    f1(tp, fp, fn_)
}

/// Normalize SVG size to 0..=1: bytes-per-pixel on a log scale, saturating at
/// `COMPLEXITY_REF_BPP` bytes/pixel.
pub fn complexity_of(svg_bytes: usize, width: usize, height: usize) -> f64 {
    let pixels = (width * height).max(1) as f64;
    let bpp = svg_bytes as f64 / pixels;
    (bpp.ln_1p() / COMPLEXITY_REF_BPP.ln_1p()).clamp(0.0, 1.0)
}

fn doc_stats(doc: &VectorDoc) -> (usize, usize) {
    let mut commands = 0;
    for shape in &doc.shapes {
        for sub in &shape.path.subpaths {
            commands += sub.commands.len();
        }
    }
    (doc.shapes.len(), commands)
}

/// Trial-trace the candidates at reduced resolution and score each one.
/// Candidates sharing a segmentation key (e.g. 线稿 vs 黑白) reuse the
/// expensive clustering result.
fn trial_candidates(
    img: &ColorImage,
    stats: &ImageStats,
    candidates: &[SmartCandidate],
) -> Result<Vec<CandidateReport>, Error> {
    // Trial at reduced resolution; scale pixel-unit params accordingly.
    let test_scale = (TEST_MAX_SIDE as f64 / img.width.max(img.height) as f64).min(1.0);
    let test_img = downsample(img, TEST_MAX_SIDE);

    let mut seg_cache: Vec<(SegmentKey, Segmentation)> = Vec::new();
    let mut reports: Vec<CandidateReport> = Vec::new();
    for c in candidates {
        let t = Instant::now();
        let test_params = scale_params(&c.params, test_scale);
        let config = test_params.to_vtracer_config();
        let pipeline = config.build()?;
        let key = config.segment_key();
        let seg = match seg_cache.iter().find(|(k, _)| *k == key) {
            Some((_, s)) => s.clone(),
            None => {
                let s = pipeline.segment(&test_img)?;
                seg_cache.push((key, s.clone()));
                s
            }
        };
        let doc = pipeline.finish(&seg)?;
        // Score with this candidate's own family criteria (per-style λ and
        // fidelity metric), so each style is judged by what makes *it* good.
        let criteria = criteria_for(c.family);
        let fidelity = if criteria.ink_aware {
            ink_fidelity(&seg, &test_img)
        } else if criteria.stroke_aware && stroke_dominant(stats) {
            // Line-art-like source: RMSE would call a stroke-losing
            // candidate faithful; score the dilated stroke F1 instead.
            stroke_fidelity(&seg, &test_img)
        } else {
            color_fidelity(&seg, &test_img)
        };
        // On stroke-like sources (dominant or noise-routed), report the stroke
        // F1 for *every* candidate (even ones scored by other metrics) so the
        // report shows which candidates actually kept the strokes.
        let stroke = (stroke_dominant(stats) || stroke_like(stats))
            .then(|| stroke_fidelity(&seg, &test_img));
        let (shape_count, command_count) = doc_stats(&doc);
        let svg = pipeline.writer.write(&doc);
        let complexity = complexity_of(svg.len(), test_img.width, test_img.height);
        let score = fidelity - criteria.lambda * complexity;
        reports.push(CandidateReport {
            family: c.family.into(),
            label: c.label.into(),
            params: c.params.clone(),
            fidelity,
            complexity,
            score,
            stroke_fidelity: stroke,
            is_baseline: c.is_baseline,
            shape_count,
            command_count,
            svg_bytes: svg.len(),
            elapsed_ms: t.elapsed().as_millis() as u64,
        });
    }
    Ok(reports)
}

/// Build the final decision from scored reports. `scope` carries the family
/// id when the search was restricted to one preset family.
fn decision_from(
    stats: ImageStats,
    mut reports: Vec<CandidateReport>,
    scope: Option<&str>,
    t0: Instant,
) -> SmartDecision {
    reports.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // The preset baseline report (for the UI comparison), if present.
    let baseline_idx = reports.iter().position(|r| r.is_baseline);
    let preset = baseline_idx.map(|i| reports[i].clone());

    // Prefer the preset unless a variation clearly beats it by the margin —
    // this keeps smart tuning from ever landing on a result that is
    // indistinguishable-from (or worse than) the known-good preset.
    let winner_idx = match baseline_idx {
        Some(bi) => {
            let baseline_score = reports[bi].score;
            let best_other = reports
                .iter()
                .filter(|r| !r.is_baseline)
                .map(|r| r.score)
                .fold(f64::NEG_INFINITY, f64::max);
            if best_other - baseline_score >= BASELINE_MARGIN {
                0
            } else {
                bi
            }
        }
        None => 0,
    };
    let winner = &reports[winner_idx];

    let reason_key = if scope.is_some() { "scoped" } else { "auto" }.to_string();
    SmartDecision {
        params: winner.params.clone(),
        family: winner.family.clone(),
        reason_key,
        stats,
        candidates: reports,
        preset,
        preprocessed: false,
        elapsed_ms: t0.elapsed().as_millis() as u64,
    }
}

/// Auto mode: analyze the image, pick candidate families from the statistics,
/// trial-trace them, and return the winner with a full report.
pub fn recommend(img: &ColorImage) -> Result<SmartDecision, Error> {
    let t0 = Instant::now();
    let stats = analyze_image(img);
    let candidates = candidate_params(&stats);
    if candidates.is_empty() {
        return Err(Error::Config("no smart candidates".into()));
    }
    let reports = trial_candidates(img, &stats, &candidates)?;
    Ok(decision_from(stats, reports, None, t0))
}

/// Family-scoped mode: the user chose the preset family (what kind of output
/// they want); trial-trace parameter variations *within* that family and
/// return the best one. The winner always stays in the requested family.
pub fn recommend_for(img: &ColorImage, family: &str) -> Result<SmartDecision, Error> {
    let t0 = Instant::now();
    // Validate and get the 'static family id (SmartCandidate.family is 'static).
    let family_id: &'static str = match family {
        "lineArt" => "lineArt",
        "color" => "color",
        "bw" => "bw",
        "gradient" => "gradient",
        "lowPoly" => "lowPoly",
        other => return Err(Error::Config(format!("unknown smart family: {other}"))),
    };
    let stats = analyze_image(img);
    let mut candidates = family_candidates(&stats, family_id);
    // Always include the exact preset default as the baseline, so the smart
    // result is compared against the preset itself and can never be worse
    // than it (the preset wins unless a variation clearly beats it).
    if let Some(baseline) = preset_params(family_id) {
        candidates.insert(
            0,
            SmartCandidate {
                family: family_id,
                label: "presetDefault",
                params: baseline,
                is_baseline: true,
            },
        );
    }
    if candidates.is_empty() {
        return Err(Error::Config(format!(
            "no smart candidates for family {family_id}"
        )));
    }
    let reports = trial_candidates(img, &stats, &candidates)?;
    Ok(decision_from(stats, reports, Some(family_id), t0))
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_image(
        w: usize,
        h: usize,
        mut f: impl FnMut(usize, usize) -> (u8, u8, u8),
    ) -> ColorImage {
        let mut pixels = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let (r, g, b) = f(x, y);
                pixels.extend_from_slice(&[r, g, b, 255]);
            }
        }
        ColorImage {
            pixels,
            width: w,
            height: h,
        }
    }

    /// Left half black, right half white.
    fn two_tone(w: usize, h: usize) -> ColorImage {
        make_image(w, h, |x, _| {
            let v = if x < w / 2 { 0u8 } else { 255u8 };
            (v, v, v)
        })
    }

    /// A few flat color rectangles (poster/illustration).
    fn poster(w: usize, h: usize) -> ColorImage {
        make_image(w, h, |x, y| match (x < w / 2, y < h / 2) {
            (true, true) => (220, 40, 40),
            (false, true) => (40, 180, 60),
            (true, false) => (50, 80, 220),
            (false, false) => (240, 200, 40),
        })
    }

    /// Smooth 2D gradient with deterministic noise (photo-like).
    fn photo_like(w: usize, h: usize) -> ColorImage {
        let mut state: u64 = 0x9E3779B97F4A7C15;
        make_image(w, h, |x, y| {
            let r = (x as f64 / w.max(1) as f64 * 255.0) as u8;
            let b = (y as f64 / h.max(1) as f64 * 255.0) as u8;
            let g = ((x + y) as f64 / (w + h) as f64 * 255.0) as u8;
            // LCG noise, ±12.
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let n = ((state >> 33) % 25) as i16 - 12;
            let adj = |v: u8| (v as i16 + n).clamp(0, 255) as u8;
            (adj(r), adj(g), adj(b))
        })
    }

    /// White ground + thin dark strokes + JPEG-like degradation: 2×2-block
    /// chroma wobble (as in 4:2:0 chroma subsampling) and per-pixel
    /// luminance grain. Matches the measured profile of the real scans:
    /// tiny palette, low colorfulness, and a smeared grayscale histogram
    /// whose bimodality collapses to ~0.
    fn noisy_scan(w: usize, h: usize) -> ColorImage {
        let mut state: u64 = 0xBF58476D1CE4E5B9;
        let mut rnd = move |span: i16| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) % (2 * span as u64 + 1)) as i16 - span
        };
        // Chroma wobble: fresh value per 2×2 block, opposite in R and B.
        let mut cr = 0i16;
        make_image(w, h, |x, y| {
            if x % 2 == 0 && y % 2 == 0 {
                cr = rnd(8);
            }
            let base = if x % 48 < 2 || y % 48 < 2 { 20i16 } else { 242 };
            // Triangular pseudo-Gaussian grain, roughly ±12.
            let l = base + rnd(12) + rnd(12);
            let cl = |v: i16| v.clamp(0, 255) as u8;
            (cl(l + cr), cl(l), cl(l - cr))
        })
    }

    #[test]
    fn downsample_keeps_constant_and_shrinks() {
        let img = make_image(400, 200, |_, _| (10, 200, 30));
        let out = downsample(&img, 100);
        assert_eq!((out.width, out.height), (100, 50));
        for i in (0..out.pixels.len()).step_by(4) {
            assert_eq!(&out.pixels[i..i + 3], &[10, 200, 30]);
        }
        // Already small: unchanged.
        let small = make_image(50, 50, |_, _| (1, 2, 3));
        let same = downsample(&small, 100);
        assert_eq!((same.width, same.height), (50, 50));
        assert_eq!(same.pixels, small.pixels);
    }

    #[test]
    fn analyze_two_tone_detects_bimodality() {
        let stats = analyze_image(&two_tone(256, 256));
        assert!(stats.bimodality > 0.5, "bimodality = {}", stats.bimodality);
        assert!(
            stats.colorfulness < 10.0,
            "colorfulness = {}",
            stats.colorfulness
        );
        assert!(
            stats.effective_colors <= 4,
            "colors = {}",
            stats.effective_colors
        );
        assert!((stats.otsu_threshold as i32 - 128).abs() < 40);
    }

    /// Thin black strokes on white: the dark peak covers only a few percent
    /// of the image, but the histogram is still clearly two-tone.
    #[test]
    fn analyze_line_art_detects_asymmetric_bimodality() {
        let stats = analyze_image(&make_image(256, 256, |x, y| {
            let stroke = x % 32 < 3 || y % 32 < 3; // ~18% ink
            let v = if stroke { 0u8 } else { 255u8 };
            (v, v, v)
        }));
        assert!(stats.bimodality > 0.45, "bimodality = {}", stats.bimodality);
        assert!(
            stats.colorfulness < 10.0,
            "colorfulness = {}",
            stats.colorfulness
        );
        let c = candidate_params(&stats);
        assert_eq!(c[0].family, "lineArt");
    }

    #[test]
    fn analyze_noisy_has_more_noise_than_clean() {
        let clean = make_image(256, 256, |_, _| (128, 128, 128));
        let mut state: u64 = 42;
        let noisy = make_image(256, 256, |_, _| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let v = (128i16 + ((state >> 33) % 41) as i16 - 20).clamp(0, 255) as u8;
            (v, v, v)
        });
        let (clean_stats, noisy_stats) = (analyze_image(&clean), analyze_image(&noisy));
        assert!(
            noisy_stats.noise > clean_stats.noise + 5.0,
            "noisy {} vs clean {}",
            noisy_stats.noise,
            clean_stats.noise
        );
    }

    #[test]
    fn candidates_two_tone_prefers_line_art() {
        let stats = analyze_image(&two_tone(256, 256));
        let c = candidate_params(&stats);
        assert!(!c.is_empty());
        assert_eq!(c[0].family, "lineArt");
    }

    #[test]
    fn candidates_poster_prefers_color_families() {
        let stats = analyze_image(&poster(256, 256));
        let c = candidate_params(&stats);
        assert!(!c.is_empty());
        assert!(c.iter().all(|x| x.params.clustering != "binary"));
    }

    #[test]
    fn candidates_photo_like_includes_photo() {
        let stats = analyze_image(&photo_like(256, 256));
        let c = candidate_params(&stats);
        assert!(c
            .iter()
            .any(|x| x.family == "photo" || x.family == "gradient"));
    }

    #[test]
    fn scale_params_scales_pixel_units() {
        let p = SmartParams {
            filter_speckle: 8,
            length_threshold: 4.0,
            binary_adaptive_window: 64,
            ..Default::default()
        };
        let s = scale_params(&p, 0.5);
        assert_eq!(s.filter_speckle, 4);
        assert!((s.length_threshold - 2.0).abs() < 1e-9);
        assert_eq!(s.binary_adaptive_window, 32);
        // Non-pixel params untouched.
        assert_eq!(s.color_precision, p.color_precision);
        assert_eq!(s.layer_difference, p.layer_difference);
    }

    #[test]
    fn complexity_of_is_monotonic_and_saturates() {
        let (w, h) = (100, 100);
        let a = complexity_of(100, w, h);
        let b = complexity_of(10_000, w, h);
        let c = complexity_of(10_000_000, w, h);
        assert!(a < b && b < c);
        assert!(c <= 1.0);
    }

    #[test]
    fn recommend_two_tone_picks_binary_family() {
        let decision = recommend(&two_tone(256, 256)).expect("recommend");
        assert!(
            matches!(decision.family.as_str(), "lineArt" | "bw"),
            "winner = {}",
            decision.family
        );
        assert_eq!(decision.candidates[0].family, decision.family);
        assert_eq!(decision.reason_key, "auto");
    }

    #[test]
    fn recommend_poster_picks_color_clustering() {
        let decision = recommend(&poster(256, 256)).expect("recommend");
        assert_eq!(decision.params.clustering, "color");
        assert!(decision.candidates.len() >= 3);
        // Best score first.
        for pair in decision.candidates.windows(2) {
            assert!(pair[0].score >= pair[1].score);
        }
    }

    #[test]
    fn recommend_photo_like_runs_all_candidates() {
        let decision = recommend(&photo_like(256, 256)).expect("recommend");
        assert!(!decision.candidates.is_empty());
        assert!(decision.candidates[0].fidelity > 0.5);
        assert!(decision.params.simplify >= 0.0);
    }

    #[test]
    fn family_candidates_nonempty_for_all_families() {
        let stats = analyze_image(&poster(256, 256));
        for family in FAMILIES {
            let c = family_candidates(&stats, family);
            assert!(c.len() >= 2, "family {family} should yield ≥ 2 candidates");
            assert!(
                c.iter().all(|x| x.family == family),
                "family {family} leaked another family"
            );
        }
        assert!(family_candidates(&stats, "nope").is_empty());
    }

    #[test]
    fn family_candidates_keep_family_defining_settings() {
        let stats = analyze_image(&two_tone(256, 256));
        // lineArt: binary clustering + line-art flag in every candidate.
        let la = family_candidates(&stats, "lineArt");
        assert!(la
            .iter()
            .all(|c| c.params.clustering == "binary" && c.params.line_art));
        // lowPoly: polygon fit in every candidate.
        let lp = family_candidates(&stats, "lowPoly");
        assert!(lp.iter().all(|c| c.params.mode == "polygon"));
        // gradient: photo preset in every candidate.
        let gr = family_candidates(&stats, "gradient");
        assert!(gr.iter().all(|c| c.params.preset == "photo"));
    }

    #[test]
    fn recommend_for_line_art_stays_in_family() {
        let decision = recommend_for(&two_tone(256, 256), "lineArt").expect("recommend_for");
        assert_eq!(decision.family, "lineArt");
        assert!(decision.params.line_art);
        assert_eq!(decision.params.clustering, "binary");
        assert!(decision.candidates.len() >= 2);
        assert_eq!(decision.reason_key, "scoped");
    }

    #[test]
    fn recommend_for_low_poly_stays_in_family() {
        let decision = recommend_for(&poster(256, 256), "lowPoly").expect("recommend_for");
        assert_eq!(decision.family, "lowPoly");
        assert_eq!(decision.params.mode, "polygon");
        assert!(decision.candidates.len() >= 2);
    }

    #[test]
    fn recommend_for_color_stays_in_family() {
        let decision = recommend_for(&poster(256, 256), "color").expect("recommend_for");
        assert_eq!(decision.family, "color");
        assert_eq!(decision.params.clustering, "color");
        assert!(decision.candidates.len() >= 2);
    }

    #[test]
    fn recommend_for_unknown_family_errors() {
        let img = two_tone(64, 64);
        assert!(recommend_for(&img, "nope").is_err());
    }

    #[test]
    fn preset_params_match_web_presets() {
        // 线稿: bw + binary + lineArt, fixed threshold 128, adaptive off.
        let la = preset_params("lineArt").expect("lineArt");
        assert_eq!(la.preset, "bw");
        assert_eq!(la.clustering, "binary");
        assert!(la.line_art);
        assert_eq!(la.binary_threshold, 128);
        assert!(!la.binary_adaptive);
        // 彩色填充: the SmartParams default (poster/color, precision 6 / diff 16).
        let c = preset_params("color").expect("color");
        assert_eq!(c.clustering, "color");
        assert_eq!(c.color_precision, 6);
        assert_eq!(c.layer_difference, 16);
        // 黑白: adaptive threshold on.
        assert!(preset_params("bw").expect("bw").binary_adaptive);
        // 渐变: photo + fine gradient.
        let gr = preset_params("gradient").expect("gradient");
        assert_eq!(gr.preset, "photo");
        assert_eq!(gr.color_precision, 8);
        assert_eq!(gr.layer_difference, 4);
        // 低多边形: polygon fit + coarse.
        let lp = preset_params("lowPoly").expect("lowPoly");
        assert_eq!(lp.mode, "polygon");
        assert_eq!(lp.color_precision, 4);
        // Unknown family.
        assert!(preset_params("nope").is_none());
    }

    #[test]
    fn criteria_are_per_family() {
        // Line art judges by ink capture (strokes); the rest by color fidelity.
        assert!(criteria_for("lineArt").ink_aware);
        assert!(!criteria_for("bw").ink_aware);
        assert!(!criteria_for("color").ink_aware);
        assert!(!criteria_for("gradient").ink_aware);
        assert!(!criteria_for("lowPoly").ink_aware);
        // Gradient barely penalizes complexity (needs detail); lowPoly heavily
        // penalizes it (should stay simple).
        assert!(criteria_for("gradient").lambda < criteria_for("color").lambda);
        assert!(criteria_for("lowPoly").lambda > criteria_for("color").lambda);
    }

    #[test]
    fn recommend_for_includes_preset_baseline() {
        let decision = recommend_for(&two_tone(256, 256), "lineArt").expect("recommend_for");
        // The exact preset default is a candidate and is exposed for comparison.
        assert!(
            decision.candidates.iter().any(|c| c.is_baseline),
            "expected a baseline candidate"
        );
        let preset = decision.preset.as_ref().expect("preset report present");
        assert!(preset.is_baseline);
        // The applied winner is one of the trial candidates.
        assert!(decision
            .candidates
            .iter()
            .any(|c| c.params == decision.params));
    }

    #[test]
    fn recommend_for_winner_never_worse_than_preset() {
        // Invariant: the applied winner must score at least as high as the
        // preset baseline, so smart tuning can never land on a result that is
        // worse than the known-good preset.
        let img = poster(256, 256);
        for family in FAMILIES {
            let decision = recommend_for(&img, family).expect("recommend_for");
            let preset = decision.preset.as_ref().expect("preset present");
            let winner_score = decision
                .candidates
                .iter()
                .find(|c| c.params == decision.params)
                .expect("winner is a candidate")
                .score;
            assert!(
                winner_score >= preset.score - 1e-9,
                "family {family}: winner {winner_score} < preset {}",
                preset.score
            );
        }
    }

    // ── QWP1: stroke fidelity ──────────────────────────────────────────

    use visioncortex::BinaryImage;
    use vtracer::ir::{Layer as SegLayer, Paint as SegPaint, RegionMask};
    use vtracer::{Color, PointI32};

    /// White canvas with one 1-px vertical dark stroke at `x`.
    fn one_stroke(w: usize, h: usize, x: usize) -> ColorImage {
        make_image(w, h, move |px, _| {
            let v = if px == x { 0u8 } else { 255u8 };
            (v, v, v)
        })
    }

    /// Segmentation for `one_stroke`: white background below, dark stroke on
    /// top (`strokes = false` drops the stroke layer — the failure mode).
    fn seg_stroke(w: usize, h: usize, x: usize, strokes: bool) -> Segmentation {
        let mut bg = BinaryImage::new_w_h(w, h);
        for y in 0..h {
            for px in 0..w {
                bg.set_pixel(px, y, true);
            }
        }
        let mut stroke = BinaryImage::new_w_h(w, h);
        if strokes {
            for y in 0..h {
                stroke.set_pixel(x, y, true);
            }
        }
        let mut seg = Segmentation::new(w as u32, h as u32);
        seg.layers.push(SegLayer {
            paint: SegPaint::Solid(Color::new(255, 255, 255)),
            mask: RegionMask::new(bg, PointI32 { x: 0, y: 0 }),
        });
        seg.layers.push(SegLayer {
            paint: SegPaint::Solid(Color::new(0, 0, 0)),
            mask: RegionMask::new(stroke, PointI32 { x: 0, y: 0 }),
        });
        seg
    }

    /// Line art: sparse 1-px strokes on white (low ink, real edges).
    fn stroke_art(w: usize, h: usize, gap: usize) -> ColorImage {
        make_image(w, h, move |px, _| {
            let v = if px % gap == gap / 2 { 0u8 } else { 255u8 };
            (v, v, v)
        })
    }

    #[test]
    fn ink_coverage_separates_line_art_from_photo() {
        let line = analyze_image(&stroke_art(256, 256, 32));
        assert!(
            line.ink_coverage < 0.15,
            "line art ink coverage = {}",
            line.ink_coverage
        );
        assert!(
            stroke_dominant(&line),
            "line art should be stroke-dominant (coverage {}, texture {})",
            line.ink_coverage,
            line.texture
        );
        let photo = analyze_image(&photo_like(256, 256));
        assert!(
            !stroke_dominant(&photo),
            "photo should not be stroke-dominant (coverage {}, texture {})",
            photo.ink_coverage,
            photo.texture
        );
        let two_tone = analyze_image(&two_tone(256, 256));
        assert!(
            (two_tone.ink_coverage - 0.5).abs() < 0.02,
            "two-tone ink coverage = {}",
            two_tone.ink_coverage
        );
    }

    #[test]
    fn dilate_mask_grows_a_line_by_radius() {
        let (w, h, r) = (16usize, 8usize, STROKE_DILATE_PX);
        let mut m = vec![false; w * h];
        for y in 0..h {
            m[y * w + 8] = true;
        }
        let d = dilate_mask(&m, w, h, r);
        for y in 0..h {
            for x in 0..w {
                assert_eq!(
                    d[y * w + x],
                    (x as i32 - 8).abs() <= r as i32,
                    "pixel ({x},{y}) after dilation"
                );
            }
        }
    }

    /// QWP1 meta-test: dropping one stroke must collapse stroke_fidelity
    /// while the color-RMSE fidelity stays healthy (the blind spot F1 found
    /// in the evaluation: smart fid 0.94–1.00 while ink F1 was 0.37).
    #[test]
    fn stroke_fidelity_flags_a_lost_stroke_that_rmse_hides() {
        let src = one_stroke(256, 256, 128);
        let kept = seg_stroke(256, 256, 128, true);
        let lost = seg_stroke(256, 256, 128, false);

        let sf_kept = stroke_fidelity(&kept, &src);
        let sf_lost = stroke_fidelity(&lost, &src);
        assert!(sf_kept > 0.95, "kept strokes F1 = {sf_kept}");
        assert!(sf_lost < 0.05, "lost strokes F1 = {sf_lost}");

        let cf_kept = color_fidelity(&kept, &src);
        let cf_lost = color_fidelity(&lost, &src);
        assert!(cf_kept > 0.95, "kept color fidelity = {cf_kept}");
        assert!(
            cf_lost > 0.9,
            "color fidelity should stay high when a thin stroke is lost \
             (that is the RMSE blind spot stroke_fidelity closes): {cf_lost}"
        );
        // Stroke loss moves the stroke metric more than 5x more than RMSE.
        assert!(
            (sf_kept - sf_lost) > 5.0 * (cf_kept - cf_lost),
            "stroke metric should dominate: sfΔ {} vs cfΔ {}",
            sf_kept - sf_lost,
            cf_kept - cf_lost
        );
    }

    #[test]
    fn stroke_like_routes_noisy_scans_but_not_clean_images() {
        let scan = analyze_image(&noisy_scan(256, 256));
        assert!(
            stroke_like(&scan),
            "noisy scan should route as stroke-like (colors {}, cf {}, noise {}, texture {})",
            scan.effective_colors,
            scan.colorfulness,
            scan.noise,
            scan.texture
        );
        // Bimodality alone would route a scan to color families; stroke_like
        // is what puts binarization into the trial set.
        println!(
            "scan stats: bimodality {} cf {} noise {} texture {}",
            scan.bimodality, scan.colorfulness, scan.noise, scan.texture
        );
        // Clean flat-color and photo images must not hit it.
        assert!(
            !stroke_like(&analyze_image(&poster(256, 256))),
            "flat poster should not be stroke-like"
        );
        assert!(
            !stroke_like(&analyze_image(&photo_like(256, 256))),
            "photo should not be stroke-like"
        );
    }

    #[test]
    fn stroke_like_branch_adds_binary_and_bradley_candidates() {
        let scan = analyze_image(&noisy_scan(256, 256));
        let cands = candidate_params(&scan);
        assert!(
            cands.iter().any(|c| c.family == "lineArt"),
            "stroke-like scan must get lineArt trials"
        );
        assert!(
            cands.iter().any(|c| c.family == "bw"),
            "stroke-like scan must get bw trials"
        );
        assert!(
            cands
                .iter()
                .any(|c| c.params.binary_adaptive
                    && (c.params.binary_adaptive_t - 25.0).abs() < 1e-9),
            "stroke-like scan must get the Bradley–Roth t=25 pair"
        );
        // Clean color routes keep their candidates untouched.
        let poster_cands = candidate_params(&analyze_image(&poster(256, 256)));
        assert!(
            !poster_cands.iter().any(|c| c.family == "lineArt"),
            "clean poster route must not grow lineArt candidates"
        );
    }

    #[test]
    fn family_candidates_include_bradley_pairs_for_binary_families() {
        let stats = analyze_image(&poster(256, 256));
        for family in ["lineArt", "bw"] {
            let cands = family_candidates(&stats, family);
            let br = cands
                .iter()
                .filter(|c| c.params.binary_adaptive && c.params.binary_adaptive_window == 0)
                .count();
            assert!(
                br >= 2,
                "{family} should trial both Bradley–Roth t values, got {br}"
            );
            assert!(
                cands
                    .iter()
                    .any(|c| (c.params.binary_adaptive_t - 25.0).abs() < 1e-9),
                "{family} should trial the more tolerant t=25"
            );
        }
    }

    #[test]
    fn trial_reports_carry_stroke_fidelity_on_stroke_dominant_sources() {
        let decision = recommend_for(&stroke_art(256, 256, 32), "bw").expect("recommend_for");
        assert!(
            stroke_dominant(&decision.stats),
            "premise: stroke art is stroke-dominant"
        );
        assert!(
            decision
                .candidates
                .iter()
                .all(|c| c.stroke_fidelity.is_some()),
            "every candidate should report strokeFidelity on a stroke-dominant source"
        );
        // And omit it when the source is not stroke-dominant.
        let poster_decision = recommend(&poster(256, 256)).expect("recommend");
        assert!(
            poster_decision
                .candidates
                .iter()
                .all(|c| c.stroke_fidelity.is_none()),
            "non-stroke sources should not report strokeFidelity"
        );
    }

    #[test]
    fn needs_preprocess_fires_on_degraded_stroke_sources_only() {
        // A clean stroke source never preprocesses, however lit.
        let clean = analyze_image(&stroke_art(256, 256, 32));
        assert!(
            !needs_preprocess(&clean),
            "clean line art must not preprocess"
        );
        // The same source degrades either way and then must.
        let mut noisy = clean.clone();
        noisy.noise = 15.0;
        assert!(needs_preprocess(&noisy), "noisy line art should preprocess");
        let mut uneven = clean.clone();
        uneven.illumination_var = 25.0;
        assert!(
            needs_preprocess(&uneven),
            "unevenly-lit line art should preprocess"
        );
        // Color-field images have large illumination_var from their own
        // blocks, yet must not be taken for scans.
        let poster_stats = analyze_image(&poster(256, 256));
        assert!(
            !needs_preprocess(&poster_stats),
            "clean poster should not preprocess (illumination_var {})",
            poster_stats.illumination_var
        );
        assert!(
            !needs_preprocess(&analyze_image(&photo_like(256, 256))),
            "clean photo should not preprocess"
        );
    }
}
