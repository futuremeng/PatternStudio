//! Image I/O: load images into ColorImage.

use std::path::Path;
use vtracer::ColorImage;
use visioncortex::Color;

pub fn load_image(path: &Path) -> Result<ColorImage, Box<dyn std::error::Error>> {
    let img = image::open(path)?;
    image_to_color_image(&img)
}

/// Decode an image from in-memory bytes (PNG, JPEG, etc.) into a ColorImage.
pub fn load_image_from_memory(data: &[u8]) -> Result<ColorImage, Box<dyn std::error::Error>> {
    let img = image::load_from_memory(data)?;
    image_to_color_image(&img)
}

fn image_to_color_image(img: &image::DynamicImage) -> Result<ColorImage, Box<dyn std::error::Error>> {
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    let mut ci = ColorImage::new_w_h(w, h);
    for y in 0..h {
        for x in 0..w {
            let p = rgba.get_pixel(x as u32, y as u32);
            ci.set_pixel(x, y, &Color::new(p[0], p[1], p[2]));
        }
    }
    Ok(ci)
}
