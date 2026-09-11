//! PatternStudio WASM bindings (wasm-bindgen).
//!
//! Exposes the image-to-layered-SVG tracing pipeline and the procedural
//! pattern generators to the browser / JS runtimes.
//!
//! ```js
//! import init, { traceBytes, generatePattern } from "./patternstudio_wasm.js";
//! await init();
//!
//! const svg = traceBytes(imageBytes, "poster", "medium", "low", 3);
//! const pattern = generatePattern("rosette", 8, 50, 400);
//! ```

use wasm_bindgen::prelude::*;
use vtracer::config::Preset;
use vtracer::ColorImage;

use patternstudio_core::{LayerAssignment, PatternStudioConfig, PatternStudioPipeline, Resolution, Simplify};
use patternstudio_patterns::{generate_sized, list_kinds};
use patternstudio_svg::LayeredWriter;
use patternstudio_tracer::load_image_from_memory;

fn parse_preset(s: &str) -> Result<Preset, JsValue> {
    match s.to_ascii_lowercase().as_str() {
        "bw" => Ok(Preset::Bw),
        "photo" => Ok(Preset::Photo),
        "poster" => Ok(Preset::Poster),
        other => Err(JsValue::from_str(&format!(
            "unknown preset: {other} (expected bw/poster/photo)"
        ))),
    }
}

fn parse_resolution(s: &str) -> Result<Resolution, JsValue> {
    match s.to_ascii_lowercase().as_str() {
        "coarse" => Ok(Resolution::Coarse),
        "medium" => Ok(Resolution::Medium),
        "fine" => Ok(Resolution::Fine),
        other => Err(JsValue::from_str(&format!(
            "unknown resolution: {other} (expected coarse/medium/fine)"
        ))),
    }
}

fn parse_simplify(s: &str) -> Result<Simplify, JsValue> {
    match s.to_ascii_lowercase().as_str() {
        "minimal" => Ok(Simplify::Minimal),
        "low" => Ok(Simplify::Low),
        "medium" => Ok(Simplify::Medium),
        "high" => Ok(Simplify::High),
        "maximum" => Ok(Simplify::Maximum),
        other => Err(JsValue::from_str(&format!(
            "unknown simplify level: {other} (expected minimal/low/medium/high/maximum)"
        ))),
    }
}

/// Run the trace pipeline on a decoded image and return layered SVG.
fn run_trace(
    img: &ColorImage,
    preset: &str,
    resolution: &str,
    simplify: &str,
    layers: usize,
) -> Result<String, JsValue> {
    let mut config = PatternStudioConfig::from_preset(parse_preset(preset)?);
    config.resolution = parse_resolution(resolution)?;
    config.simplify = parse_simplify(simplify)?;

    let pipeline = PatternStudioPipeline::new(&config)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    let doc = if layers > 0 {
        pipeline
            .trace_layered_auto(img, layers)
            .map_err(|e| JsValue::from_str(&e.to_string()))?
    } else {
        pipeline
            .trace_layered(img, &LayerAssignment::single_layer())
            .map_err(|e| JsValue::from_str(&e.to_string()))?
    };
    Ok(LayeredWriter::write(&doc))
}
/// Trace image bytes (PNG, JPEG, ...) to layered SVG.
///
/// Args (wasm-bindgen has no default arguments, all are required):
/// - `data`: raw image bytes
/// - `preset`: "bw" / "poster" / "photo"
/// - `resolution`: "coarse" / "medium" / "fine"
/// - `simplify`: "minimal" / "low" / "medium" / "high" / "maximum"
/// - `layers`: auto-separate up to N color layers (0 = single layer)
///
/// Returns the layered SVG document as a string.
#[wasm_bindgen]
pub fn trace_bytes(
    data: &[u8],
    preset: &str,
    resolution: &str,
    simplify: &str,
    layers: u32,
) -> Result<String, JsValue> {
    let img = load_image_from_memory(data)
        .map_err(|e| JsValue::from_str(&format!("failed to decode image: {e}")))?;
    run_trace(&img, preset, resolution, simplify, layers as usize)
}

/// Generate a procedural pattern SVG.
///
/// - `kind`: one of `list_pattern_kinds()` (e.g. "rosette", "meander")
/// - `n_units`: number of repeating units
/// - `complexity`: 0-100
/// - `size`: output size in pixels
#[wasm_bindgen]
pub fn generate_pattern(kind: &str, n_units: u32, complexity: u32, size: u32) -> Result<String, JsValue> {
    generate_sized(kind, n_units, complexity as u8, size)
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

/// List available pattern kinds as "kind - description" strings.
#[wasm_bindgen]
pub fn list_pattern_kinds() -> Vec<String> {
    list_kinds()
        .into_iter()
        .map(|(kind, desc)| format!("{kind} - {desc}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_generation_works() {
        let svg = generate_pattern("rosette", 4, 50, 200).unwrap();
        assert!(svg.contains("<svg"));
    }

    #[test]
    fn unknown_kind_errors() {
        assert!(generate_pattern("nope", 4, 50, 200).is_err());
    }

    #[test]
    fn trace_bytes_roundtrip() {
        // 2x2 red/blue PNG
        let mut img = image::RgbaImage::new(2, 2);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        img.put_pixel(1, 0, image::Rgba([0, 0, 255, 255]));
        img.put_pixel(0, 1, image::Rgba([0, 0, 255, 255]));
        img.put_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
        let mut bytes = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();

        let svg = trace_bytes(&bytes, "poster", "medium", "low", 2).unwrap();
        assert!(svg.contains("<svg"));
    }
}

