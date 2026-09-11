//! Image I/O: load images into ColorImage.

use std::path::Path;
use vtracer::ColorImage;
use visioncortex::Color;

pub fn load_image(path: &Path) -> Result<ColorImage, Box<dyn std::error::Error>> {
    let img = image::open(path)?;
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
