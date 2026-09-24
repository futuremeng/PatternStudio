//! PatternStudio tracing: VTracer integration and custom frontends.
//!
//! Provides `MaskFrontend` (background removal), image loading utilities,
//! and palette parsing.

use visioncortex::Color;

pub mod frontend;
pub mod image_io;
pub mod preprocess;

pub use frontend::MaskFrontend;
pub use image_io::{load_image, load_image_from_memory};
pub use preprocess::{find_models_dir, find_realesrgan_bin, preprocess_image, PreprocessOut};

/// Parse a comma/whitespace/newline separated list of `#rrggbb` colors.
/// Empty or whitespace-only input yields an empty palette (= no fixed palette).
pub fn parse_palette(text: &str) -> Result<Vec<Color>, String> {
    let mut colors = Vec::new();
    for token in text.split(|c: char| c == ',' || c.is_whitespace()) {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        colors.push(parse_hex_color(token)?);
    }
    Ok(colors)
}

fn parse_hex_color(token: &str) -> Result<Color, String> {
    let hex = token.strip_prefix('#').unwrap_or(token);
    let bytes: Vec<u8> = hex.bytes().collect();
    if bytes.len() != 6 || !bytes.iter().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("`{token}` is not a #rrggbb color"));
    }
    let parse = |range: std::ops::Range<usize>| {
        u8::from_str_radix(std::str::from_utf8(&bytes[range]).unwrap(), 16)
            .map_err(|_| format!("`{token}` is not a #rrggbb color"))
    };
    Ok(Color::new(parse(0..2)?, parse(2..4)?, parse(4..6)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_palette_basic() {
        let colors = parse_palette("#ff0000,#00ff00 #0000ff").unwrap();
        assert_eq!(colors.len(), 3);
    }

    #[test]
    fn parse_palette_empty() {
        assert!(parse_palette("").unwrap().is_empty());
        assert!(parse_palette("  , ").unwrap().is_empty());
    }

    #[test]
    fn parse_palette_invalid() {
        assert!(parse_palette("#gggggg").is_err());
        assert!(parse_palette("#12345").is_err());
        // 6 UTF-8 bytes but not ASCII hex — must error, not panic
        assert!(parse_palette("红色").is_err());
    }
}
