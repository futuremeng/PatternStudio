//! Image I/O: load images into ColorImage.

use std::path::Path;
use vtracer::ColorImage;

/// Longest-side cap for the vectorization pipeline. Images larger than this
/// are downscaled proportionally before tracing.
///
/// Two integer-overflow guards in the clustering backend (visioncortex):
/// 1. Pixel indices are computed as `y * width + x` in `i32`, which overflows
///    once `width * height` exceeds `i32::MAX` (~46340²).
/// 2. The adaptive-threshold (black/white) path builds a `SummedAreaTable` of
///    `u32` whose entries accumulate `up + left + curr ≈ 2·x·y·intensity`. For
///    a bright image this exceeds `u32::MAX` once the longest side is above
///    ~2900 px.
///
/// 2048 keeps both safe (2048² bright SAT ≈ 2.14e9 < u32::MAX) while staying
/// fast for interactive tracing. Raise it only if you accept the overflow risk.
pub const MAX_TRACE_DIMENSION: u32 = 2048;

pub fn load_image(path: &Path) -> Result<ColorImage, Box<dyn std::error::Error>> {
    let img = image::open(path)?;
    image_to_color_image(&img)
}

/// Decode an image from in-memory bytes (PNG, JPEG, etc.) into a ColorImage.
pub fn load_image_from_memory(data: &[u8]) -> Result<ColorImage, Box<dyn std::error::Error>> {
    let img = image::load_from_memory(data)?;
    image_to_color_image(&img)
}

fn image_to_color_image(
    img: &image::DynamicImage,
) -> Result<ColorImage, Box<dyn std::error::Error>> {
    // Downscale oversized images so the vectorization pipeline (and the i32
    // pixel-index math in the clustering backend) stays safe and fast.
    let rgba = if img.width().max(img.height()) > MAX_TRACE_DIMENSION {
        let (w, h) = (img.width() as f64, img.height() as f64);
        let scale = MAX_TRACE_DIMENSION as f64 / w.max(h);
        let new_w = (w * scale).round().max(1.0) as u32;
        let new_h = (h * scale).round().max(1.0) as u32;
        img.resize(new_w, new_h, image::imageops::FilterType::Lanczos3)
            .to_rgba8()
    } else {
        img.to_rgba8()
    };
    // `to_rgba8` yields an `RgbaImage` whose backing buffer is already laid out
    // as row-major RGBA — exactly the memory layout `ColorImage` expects. Move
    // the buffer instead of copying pixel-by-pixel (the old per-pixel loop was
    // a significant cost on large images).
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    Ok(ColorImage {
        pixels: rgba.into_raw(),
        width: w,
        height: h,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba(w: u32, h: u32) -> image::DynamicImage {
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(w, h))
    }

    #[test]
    fn small_image_is_untouched() {
        let img = rgba(100, 50);
        let out = image_to_color_image(&img).unwrap();
        assert_eq!((out.width, out.height), (100, 50));
    }

    #[test]
    fn oversized_image_is_downscaled_to_cap() {
        // Longest side 8192 -> capped to 2048, aspect ratio preserved.
        let img = rgba(8192, 4096);
        let out = image_to_color_image(&img).unwrap();
        assert_eq!((out.width, out.height), (2048, 1024));
    }

    #[test]
    fn portrait_oversized_image_is_downscaled() {
        let img = rgba(2000, 10000);
        let out = image_to_color_image(&img).unwrap();
        // Longest side capped to ~2048 (±1 px for resize rounding), aspect
        // ratio preserved.
        assert!(out.height <= MAX_TRACE_DIMENSION as usize);
        assert!((out.height as i64 - 2048).abs() <= 1);
        assert!((out.width as i64 - 410).abs() <= 1);
    }
}
