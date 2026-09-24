//! Integration tests: end-to-end tracing and pattern generation.

use patternstudio_core::{LayerAssignment, PatternStudioConfig, PatternStudioPipeline};
use patternstudio_svg::LayeredWriter;
use vtracer::config::Preset;

/// Test image: a small 2-color image (left half red, right half blue).
fn test_image() -> vtracer::ColorImage {
    let width = 64;
    let height = 64;
    let mut ci = vtracer::ColorImage::new_w_h(width, height);
    for y in 0..height {
        for x in 0..width {
            let color = if x < width / 2 {
                vtracer::Color::new(255, 0, 0)
            } else {
                vtracer::Color::new(0, 0, 255)
            };
            ci.set_pixel(x, y, &color);
        }
    }
    ci
}

#[test]
fn trace_produces_valid_svg() {
    let img = test_image();
    let config = PatternStudioConfig::from_preset(Preset::Poster);
    let pipeline = PatternStudioPipeline::new(&config).unwrap();
    let assignment = LayerAssignment::single_layer();
    let layered = pipeline.trace_layered(&img, &assignment).unwrap();

    assert!(layered.total_shapes() > 0);
    assert_eq!(layered.layers.len(), 1);

    let svg = LayeredWriter::write(&layered);
    assert!(svg.starts_with("<svg"));
    assert!(svg.contains("viewBox=\"0 0 64 64\""));
    assert!(svg.contains("<g id=\"artwork\""));
    assert!(svg.ends_with("</svg>\n"));
}

#[test]
fn trace_auto_layers_separates_colors() {
    let img = test_image();
    let config = PatternStudioConfig::from_preset(Preset::Poster);
    let pipeline = PatternStudioPipeline::new(&config).unwrap();
    let layered = pipeline.trace_layered_auto(&img, 5).unwrap();

    assert!(layered.layers.len() >= 2);
    assert_eq!(layered.layers[0].name, "background");
    assert!(layered.total_shapes() > 0);
}

#[test]
fn pattern_generation_produces_svg() {
    for kind in ["rosette", "meander", "scroll", "taotie", "ice_crack"] {
        let svg = patternstudio_patterns::generate(kind, 4, 50).unwrap();
        assert!(svg.starts_with("<svg"), "pattern {kind} should produce SVG");
        assert!(svg.contains("</svg>"), "pattern {kind} should be complete");
        assert!(svg.len() > 100, "pattern {kind} should have content");
    }
}

#[test]
fn layered_doc_roundtrip_preserves_shapes() {
    let img = test_image();
    let config = PatternStudioConfig::from_preset(Preset::Poster);
    let pipeline = PatternStudioPipeline::new(&config).unwrap();
    let assignment = LayerAssignment::single_layer();
    let layered = pipeline.trace_layered(&img, &assignment).unwrap();

    let total = layered.total_shapes();
    let flat = layered.to_vector_doc();
    assert_eq!(flat.shapes.len(), total);
    assert_eq!(flat.width, 64);
    assert_eq!(flat.height, 64);
}

#[test]
fn by_color_preserves_all_shapes() {
    let img = test_image();
    let config = PatternStudioConfig::from_preset(Preset::Poster);
    let pipeline = PatternStudioPipeline::new(&config).unwrap();

    let single = pipeline
        .trace_layered(&img, &LayerAssignment::single_layer())
        .unwrap();
    let total_single = single.total_shapes();

    let auto = pipeline.trace_layered_auto(&img, 3).unwrap();
    assert_eq!(auto.total_shapes(), total_single);
}
