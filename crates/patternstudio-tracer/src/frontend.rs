//! Custom Frontend: MaskFrontend with background removal.

use vtracer::error::Error;
use vtracer::frontend::ColorClusterFrontend;
use vtracer::ir::Segmentation;
use vtracer::ColorImage;
use visioncortex::Color;

/// Frontend that removes background before color-cluster segmentation.
pub struct MaskFrontend {
    inner: ColorClusterFrontend,
    pub background_color: Color,
    pub tolerance: u8,
}

impl Default for MaskFrontend {
    fn default() -> Self {
        Self {
            inner: ColorClusterFrontend::default(),
            background_color: Color::new(255, 255, 255),
            tolerance: 30,
        }
    }
}

impl MaskFrontend {
    pub fn with_background(background: Color, tolerance: u8) -> Self {
        Self {
            inner: ColorClusterFrontend::default(),
            background_color: background,
            tolerance,
        }
    }

    /// Replace pixels within `tolerance` of `background_color` with black.
    pub fn remove_background(&self, img: &ColorImage) -> ColorImage {
        let mut out = img.clone();
        let bg = self.background_color;
        let tol = self.tolerance as i32;
        let (w, h) = (out.width, out.height);
        for y in 0..h {
            for x in 0..w {
                let p = out.get_pixel(x, y);
                let [r, g, b] = p.rgb_u8();
                let dr = (r as i32 - bg.rgb_u8()[0] as i32).abs();
                let dg = (g as i32 - bg.rgb_u8()[1] as i32).abs();
                let db = (b as i32 - bg.rgb_u8()[2] as i32).abs();
                if dr <= tol && dg <= tol && db <= tol {
                    out.set_pixel(x, y, &Color::new(0, 0, 0));
                }
            }
        }
        out
    }
}

impl vtracer::frontend::Frontend for MaskFrontend {
    fn segment(&self, img: &ColorImage) -> Result<Segmentation, Error> {
        let masked = self.remove_background(img);
        self.inner.segment(&masked)
    }
}
