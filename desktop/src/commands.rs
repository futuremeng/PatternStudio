//! Tauri commands: in-process access to the tracing engine.

use std::path::Path;
use patternstudio_core::ir::LayeredDoc;
use patternstudio_core::LayerAssignment;
use patternstudio_patterns::{generate_sized, list_kinds};
use patternstudio_svg::LayeredWriter;
use patternstudio_tracer::load_image_from_memory;
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use vtracer::config::{Clustering, FitMode, Hierarchical, Preset};

// ── Trace params ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TraceParams {
    #[serde(default)]
    pub clustering: Option<String>,
    #[serde(default)]
    pub hierarchical: Option<String>,
    #[serde(default)]
    pub color_precision: Option<u8>,
    #[serde(default)]
    pub layer_difference: Option<u8>,
    #[serde(default)]
    pub filter_speckle: Option<u32>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub simplify: Option<f64>,
    #[serde(default)]
    pub path_precision: Option<u8>,
    #[serde(default)]
    pub max_colors: Option<usize>,
    #[serde(default = "default_preset")]
    pub preset: String,
    #[serde(default)]
    pub line_art: bool,
    #[serde(default)]
    pub layers: usize,
}

fn default_preset() -> String {
    "poster".into()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerInfo {
    pub name: String,
    pub role: String,
    pub shape_count: usize,
    pub dominant_color: String,
    pub preview_svg: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceOut {
    pub svg: String,
    pub width: u32,
    pub height: u32,
    pub shape_count: usize,
    pub layer_count: usize,
    pub layers: Vec<LayerInfo>,
}

// ── Core trace logic ──────────────────────────────────────────────────────

fn build_vtracer_config(params: &TraceParams) -> vtracer::Config {
    let preset = match params.preset.to_ascii_lowercase().as_str() {
        "bw" => Preset::Bw,
        "photo" => Preset::Photo,
        _ => Preset::Poster,
    };
    let mut config = vtracer::Config::from_preset(preset);

    if let Some(ref c) = params.clustering {
        config.clustering = match c.as_str() {
            "binary" => Clustering::Binary,
            "watershed" => Clustering::Watershed,
            _ => Clustering::ColorCluster,
        };
    }
    if let Some(ref h) = params.hierarchical {
        config.hierarchical = match h.as_str() {
            "cutout" => Hierarchical::Cutout,
            _ => Hierarchical::Stacked,
        };
    }
    if let Some(v) = params.color_precision { config.color_precision = v as i32; }
    if let Some(v) = params.layer_difference { config.layer_difference = v as i32; }
    if let Some(v) = params.filter_speckle { config.filter_speckle = v as usize; }
    if let Some(ref m) = params.mode {
        config.mode = match m.as_str() {
            "pixel" => FitMode::Pixel,
            "polygon" => FitMode::Polygon,
            _ => FitMode::Spline,
        };
    }
    if let Some(v) = params.simplify { config.simplify = Some(v); }
    if let Some(v) = params.path_precision { config.path_precision = Some(v as u32); }
    if let Some(v) = params.max_colors {
        config.max_colors = Some(v);
    }
    config
}

fn trace_bytes_internal(data: &[u8], params: &TraceParams) -> Result<TraceOut, String> {
    let img = load_image_from_memory(data).map_err(|e| format!("failed to decode image: {e}"))?;
    let vtracer_config = build_vtracer_config(params);
    let pipeline = vtracer_config
        .build()
        .map_err(|e| e.to_string())?;
    let doc = pipeline.run(&img).map_err(|e| e.to_string())?;

    let assignment = if params.layers > 1 {
        LayerAssignment::by_color(&doc, params.layers)
    } else {
        LayerAssignment::single_layer()
    };
    let layered = LayeredDoc::from_vector_doc(&doc, &assignment);

    let svg = if params.line_art {
        LayeredWriter::write_line_art(&layered)
    } else {
        LayeredWriter::write(&layered)
    };

    let layers: Vec<LayerInfo> = layered
        .layers
        .iter()
        .enumerate()
        .map(|(i, layer)| LayerInfo {
            name: layer.name.clone(),
            role: layer.role.to_string(),
            shape_count: layer.shapes.len(),
            dominant_color: LayeredWriter::layer_dominant_color(layer),
            preview_svg: LayeredWriter::layer_preview_svg(&layered, i),
        })
        .collect();

    Ok(TraceOut {
        svg,
        width: layered.width,
        height: layered.height,
        shape_count: layered.total_shapes(),
        layer_count: layered.layers.len(),
        layers,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatternInfo {
    pub kind: &'static str,
    pub description: &'static str,
}

#[tauri::command]
pub fn trace_bytes(data: Vec<u8>, params: TraceParams) -> Result<TraceOut, String> {
    trace_bytes_internal(&data, &params)
}

#[tauri::command]
pub fn open_folder() -> Result<String, String> {
    let folder = rfd::FileDialog::new()
        .set_title("选择项目文件夹")
        .pick_folder();
    match folder {
        Some(p) => Ok(p.to_string_lossy().to_string()),
        None => Err("cancelled".into()),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageFile {
    pub name: String,
    pub path: String,
}

const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "tiff", "gif"];

#[tauri::command]
pub fn list_images(folder_path: String) -> Result<Vec<ImageFile>, String> {
    let path = Path::new(&folder_path);
    if !path.is_dir() {
        return Err(format!("not a directory: {folder_path}"));
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
        if IMAGE_EXTS.contains(&ext.as_str()) {
            files.push(ImageFile {
                name: name.clone(),
                path: entry.path().to_string_lossy().to_string(),
            });
        }
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(files)
}

#[tauri::command]
pub fn read_image(file_path: String) -> Result<Vec<u8>, String> {
    std::fs::read(&file_path).map_err(|e| format!("failed to read {file_path}: {e}"))
}

#[derive(Debug, Deserialize)]
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

fn default_n_units() -> u32 { 8 }
fn default_complexity() -> u8 { 50 }
fn default_size() -> u32 { 400 }

#[tauri::command]
pub async fn batch_trace(
    app: tauri::AppHandle,
    folder_path: String,
    params: TraceParams,
) -> Result<String, String> {
    let job_id = uuid::Uuid::new_v4().to_string();

    let path = Path::new(&folder_path);
    if !path.is_dir() {
        return Err(format!("not a directory: {folder_path}"));
    }
    let mut files: Vec<(String, String)> = Vec::new();
    for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
        if IMAGE_EXTS.contains(&ext.as_str()) {
            files.push((name, entry.path().to_string_lossy().to_string()));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));

    tauri::async_runtime::spawn(async move {
        let start = std::time::Instant::now();
        let total = files.len();

        let _ = app.emit("batch-trace", serde_json::json!({
            "type": "start",
            "total": total,
            "files": files.iter().map(|f| f.0.clone()).collect::<Vec<_>>(),
        }));

        let mut success = 0usize;
        let mut failed = 0usize;

        for (i, (name, file_path)) in files.iter().enumerate() {
            let elapsed = start.elapsed().as_millis() as u64;
            let _ = app.emit("batch-trace", serde_json::json!({
                "type": "processing",
                "index": i,
                "total": total,
                "file": name,
                "elapsed_ms": elapsed,
            }));

            let data = std::fs::read(file_path).unwrap_or_default();
            let params_clone = params.clone();
            let result = tauri::async_runtime::spawn_blocking(move || {
                trace_bytes_internal(&data, &params_clone)
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));

            let elapsed = start.elapsed().as_millis() as u64;
            match result {
                Ok(out) => {
                    success += 1;
                    let _ = app.emit("batch-trace", serde_json::json!({
                        "type": "done",
                        "index": i,
                        "total": total,
                        "file": name,
                        "svg": out.svg,
                        "width": out.width,
                        "height": out.height,
                        "shapeCount": out.shape_count,
                        "layers": out.layers,
                        "elapsed_ms": elapsed,
                    }));
                }
                Err(e) => {
                    failed += 1;
                    let _ = app.emit("batch-trace", serde_json::json!({
                        "type": "error",
                        "index": i,
                        "total": total,
                        "file": name,
                        "error": e,
                        "elapsed_ms": elapsed,
                    }));
                }
            }
        }

        let _ = app.emit("batch-trace", serde_json::json!({
            "type": "complete",
            "total": total,
            "success": success,
            "failed": failed,
            "total_elapsed_ms": start.elapsed().as_millis() as u64,
        }));
    });

    Ok(job_id)
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
