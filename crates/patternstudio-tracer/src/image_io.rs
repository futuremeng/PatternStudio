//! Image I/O: load images into ColorImage.

use std::path::Path;
use vtracer::ColorImage;

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
    // `to_rgba8` yields an `RgbaImage` whose backing buffer is already laid out
    // as row-major RGBA — exactly the memory layout `ColorImage` expects. Move
    // the buffer instead of copying pixel-by-pixel (the old per-pixel loop was
    // a significant cost on large images).
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    Ok(ColorImage {
        pixels: rgba.into_raw(),
        width: w,
        height: h,
    })
}
