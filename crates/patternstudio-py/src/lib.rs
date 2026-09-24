//! PatternStudio Python bindings (PyO3).
//!
//! Exposes the image-to-layered-SVG tracing pipeline and the procedural
//! pattern generators to Python.
//!
//! ```python
//! import patternstudio
//!
//! svg = patternstudio.trace_file("photo.png", "out.svg", layers=3)
//! svg = patternstudio.generate_pattern("rosette", n_units=8, complexity=50)
//! ```

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use vtracer::config::Preset;
use vtracer::ColorImage;

use patternstudio_core::{
    LayerAssignment, PatternStudioConfig, PatternStudioPipeline, Resolution, Simplify,
};
use patternstudio_patterns::{generate_sized, list_kinds};
use patternstudio_svg::LayeredWriter;
use patternstudio_tracer::{load_image, load_image_from_memory};

fn parse_preset(s: &str) -> Result<Preset, PyErr> {
    match s.to_ascii_lowercase().as_str() {
        "bw" => Ok(Preset::Bw),
        "photo" => Ok(Preset::Photo),
        "poster" => Ok(Preset::Poster),
        other => Err(PyValueError::new_err(format!(
            "unknown preset: {other} (expected bw/poster/photo)"
        ))),
    }
}

fn parse_resolution(s: &str) -> Result<Resolution, PyErr> {
    match s.to_ascii_lowercase().as_str() {
        "coarse" => Ok(Resolution::Coarse),
        "medium" => Ok(Resolution::Medium),
        "fine" => Ok(Resolution::Fine),
        other => Err(PyValueError::new_err(format!(
            "unknown resolution: {other} (expected coarse/medium/fine)"
        ))),
    }
}

fn parse_simplify(s: &str) -> Result<Simplify, PyErr> {
    match s.to_ascii_lowercase().as_str() {
        "minimal" => Ok(Simplify::Minimal),
        "low" => Ok(Simplify::Low),
        "medium" => Ok(Simplify::Medium),
        "high" => Ok(Simplify::High),
        "maximum" => Ok(Simplify::Maximum),
        other => Err(PyValueError::new_err(format!(
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
) -> PyResult<String> {
    let mut config = PatternStudioConfig::from_preset(parse_preset(preset)?);
    config.resolution = parse_resolution(resolution)?;
    config.simplify = parse_simplify(simplify)?;

    let pipeline =
        PatternStudioPipeline::new(&config).map_err(|e| PyValueError::new_err(e.to_string()))?;

    let doc = if layers > 0 {
        pipeline
            .trace_layered_auto(img, layers)
            .map_err(|e| PyValueError::new_err(e.to_string()))?
    } else {
        pipeline
            .trace_layered(img, &LayerAssignment::single_layer())
            .map_err(|e| PyValueError::new_err(e.to_string()))?
    };
    Ok(LayeredWriter::write(&doc))
}

/// Trace an image file to layered SVG.
///
/// Args:
///     path: input image path (PNG, JPEG, ...)
///     output: optional output path; when given the SVG is also written there
///     preset: VTracer preset: "bw" / "poster" / "photo"
///     resolution: "coarse" / "medium" / "fine"
///     simplify: "minimal" / "low" / "medium" / "high" / "maximum"
///     layers: auto-separate up to N color layers (0 = single layer)
///
/// Returns:
///     The layered SVG document as a string.
#[pyfunction]
#[pyo3(signature = (path, output=None, preset="poster", resolution="medium", simplify="low", layers=0))]
fn trace_file(
    path: &str,
    output: Option<&str>,
    preset: &str,
    resolution: &str,
    simplify: &str,
    layers: usize,
) -> PyResult<String> {
    let img = load_image(std::path::Path::new(path))
        .map_err(|e| PyValueError::new_err(format!("failed to load image {path}: {e}")))?;
    let svg = run_trace(&img, preset, resolution, simplify, layers)?;
    if let Some(out) = output {
        std::fs::write(out, &svg)
            .map_err(|e| PyValueError::new_err(format!("failed to write {out}: {e}")))?;
    }
    Ok(svg)
}

/// Trace image bytes (PNG, JPEG, ...) to layered SVG.
///
/// Same options as [`trace_file`], but takes raw image bytes and always
/// returns the SVG string.
#[pyfunction]
#[pyo3(signature = (data, preset="poster", resolution="medium", simplify="low", layers=0))]
fn trace_bytes(
    data: &[u8],
    preset: &str,
    resolution: &str,
    simplify: &str,
    layers: usize,
) -> PyResult<String> {
    let img = load_image_from_memory(data)
        .map_err(|e| PyValueError::new_err(format!("failed to decode image: {e}")))?;
    run_trace(&img, preset, resolution, simplify, layers)
}

/// Generate a procedural pattern SVG.
///
/// Args:
///     kind: one of `list_pattern_kinds()` (e.g. "rosette", "meander")
///     n_units: number of repeating units
///     complexity: 0-100
///     size: output size in pixels
///
/// Returns:
///     The pattern SVG document as a string.
#[pyfunction]
#[pyo3(signature = (kind, n_units=8, complexity=50, size=400))]
fn generate_pattern(kind: &str, n_units: u32, complexity: u8, size: u32) -> PyResult<String> {
    generate_sized(kind, n_units, complexity, size)
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

/// List available pattern kinds as "kind - description" strings.
#[pyfunction]
fn list_pattern_kinds() -> Vec<String> {
    list_kinds()
        .into_iter()
        .map(|(kind, desc)| format!("{kind} - {desc}"))
        .collect()
}

/// PatternStudio: image-to-layered-SVG tracing and procedural patterns.
#[pymodule]
fn patternstudio(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(trace_file, m)?)?;
    m.add_function(wrap_pyfunction!(trace_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(generate_pattern, m)?)?;
    m.add_function(wrap_pyfunction!(list_pattern_kinds, m)?)?;
    Ok(())
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
    fn parse_helpers_reject_garbage() {
        assert!(parse_preset("nope").is_err());
        assert!(parse_resolution("nope").is_err());
        assert!(parse_simplify("nope").is_err());
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
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();

        let svg = trace_bytes(&bytes, "poster", "medium", "low", 2).unwrap();
        assert!(svg.contains("<svg"));
    }
}
