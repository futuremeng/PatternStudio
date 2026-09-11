//! Tauri commands: in-process access to the tracing engine.

use patternstudio_core::{LayerAssignment, PatternStudioConfig, PatternStudioPipeline, Resolution, Simplify};
use patternstudio_patterns::{generate_sized, list_kinds};
use patternstudio_svg::LayeredWriter;
use patternstudio_tracer::load_image_from_memory;
use serde::Serialize;
use vtracer::config::Preset;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceParams {
    #[serde(default = "default_preset")]
    pub preset: String,
    #[serde(default = "default_resolution")]
    pub resolution: String,
    #[serde(default = "default_simplify")]
    pub simplify: String,
    #[serde(default)]
    pub layers: usize,
}

fn default_preset() -> String {
    "poster".into()
}
fn default_resolution() -> String {
    "medium".into()
}
fn default_simplify() -> String {
    "low".into()
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatternParams {
    pub kind: String,
    #[serde(default = "default_n_units")]
    pub n_units: u32,
    #[serde(default = "default_complexity")]
    pub complexity: u8,
    #[serde(default = "default_size")]
    pub size: u32,
}

fn default_n_units() -> u32 {
    8
}
fn default_complexity() -> u8 {
    50
}
fn default_size() -> u32 {
    400
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceOut {
    pub svg: String,
    pub width: u32,
    pub height: u32,
    pub shape_count: usize,
    pub layer_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatternInfo {
    pub kind: &'static str,
    pub description: &'static str,
}

#[tauri::command]
pub fn trace_bytes(data: Vec<u8>, params: TraceParams) -> Result<TraceOut, String> {
    let img = load_image_from_memory(&data).map_err(|e| format!("failed to decode image: {e}"))?;

    let preset = match params.preset.to_ascii_lowercase().as_str() {
        "bw" => Preset::Bw,
        "photo" => Preset::Photo,
        _ => Preset::Poster,
    };
    let config = PatternStudioConfig {
        resolution: match params.resolution.to_ascii_lowercase().as_str() {
            "coarse" => Resolution::Coarse,
            "fine" => Resolution::Fine,
            _ => Resolution::Medium,
        },
        simplify: match params.simplify.to_ascii_lowercase().as_str() {
            "minimal" => Simplify::Minimal,
            "medium" => Simplify::Medium,
            "high" => Simplify::High,
            "maximum" => Simplify::Maximum,
            _ => Simplify::Low,
        },
        vtracer: vtracer::Config::from_preset(preset),
    };

    let pipeline = PatternStudioPipeline::new(&config).map_err(|e| e.to_string())?;
    let layered = if params.layers > 1 {
        pipeline
            .trace_layered_auto(&img, params.layers)
            .map_err(|e| e.to_string())?
    } else {
        pipeline
            .trace_layered(&img, &LayerAssignment::single_layer())
            .map_err(|e| e.to_string())?
    };

    Ok(TraceOut {
        svg: LayeredWriter::write(&layered),
        width: layered.width,
        height: layered.height,
        shape_count: layered.total_shapes(),
        layer_count: layered.layers.len(),
    })
}

#[tauri::command]
pub fn generate_pattern(params: PatternParams) -> Result<String, String> {
    generate_sized(&params.kind, params.n_units, params.complexity, params.size)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_patterns() -> Vec<PatternInfo> {
    list_kinds()
        .into_iter()
        .map(|(kind, description)| PatternInfo { kind, description })
        .collect()
}

#[tauri::command]
pub fn save_svg(content: String, suggested_name: String) -> Result<String, String> {
    let path = rfd::FileDialog::new()
        .set_file_name(&suggested_name)
        .add_filter("SVG", &["svg"])
        .save_file();
    if let Some(p) = path {
        std::fs::write(&p, content).map_err(|e| e.to_string())?;
        Ok(p.to_string_lossy().to_string())
    } else {
        Ok(String::new())
    }
}
