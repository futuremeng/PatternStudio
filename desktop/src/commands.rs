//! Tauri commands: in-process access to the tracing engine.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use patternstudio_core::ir::LayeredDoc;
use patternstudio_core::LayerAssignment;
use patternstudio_patterns::{generate_sized, list_kinds};
use patternstudio_svg::LayeredWriter;
use patternstudio_tracer::load_image_from_memory;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};
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
pub async fn trace_bytes(data: Vec<u8>, params: TraceParams) -> Result<TraceOut, String> {
    tauri::async_runtime::spawn_blocking(move || trace_bytes_internal(&data, &params))
        .await
        .map_err(|e| e.to_string())?
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

// ── Recent projects (persisted in the app data dir) ───────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub path: String,
    pub name: String,
    pub opened_at: u64,
}

fn recent_projects_file(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("projects.json"))
}

fn load_recent(app: &tauri::AppHandle) -> Vec<RecentProject> {
    let file = match recent_projects_file(app) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    match std::fs::read_to_string(&file) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn save_recent(app: &tauri::AppHandle, projects: &[RecentProject]) -> Result<(), String> {
    let file = recent_projects_file(app)?;
    let s = serde_json::to_string_pretty(projects).map_err(|e| e.to_string())?;
    std::fs::write(&file, s).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn recent_projects(app: tauri::AppHandle) -> Vec<RecentProject> {
    load_recent(&app)
}

#[tauri::command]
pub fn add_recent_project(app: tauri::AppHandle, path: String) -> Result<RecentProject, String> {
    let name = Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "project".into());
    let opened_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut projects = load_recent(&app);
    projects.retain(|p| p.path != path);
    let project = RecentProject {
        path,
        name,
        opened_at,
    };
    projects.insert(0, project.clone());
    save_recent(&app, &projects)?;
    Ok(project)
}

#[tauri::command]
pub fn remove_recent_project(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let mut projects = load_recent(&app);
    projects.retain(|p| p.path != path);
    save_recent(&app, &projects)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageFile {
    pub name: String,
    pub path: String,
}

const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "tiff", "gif"];

#[tauri::command]
pub async fn list_images(folder_path: String) -> Result<Vec<ImageFile>, String> {
    tauri::async_runtime::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn read_image(file_path: String) -> Result<Vec<u8>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::read(&file_path).map_err(|e| format!("failed to read {file_path}: {e}"))
    })
    .await
    .map_err(|e| e.to_string())?
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

fn batch_cancel_flags() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static FLAGS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    FLAGS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[tauri::command]
pub fn cancel_batch_trace(job_id: String) -> Result<(), String> {
    let flags = batch_cancel_flags().lock().unwrap();
    if let Some(flag) = flags.get(&job_id) {
        flag.store(true, Ordering::SeqCst);
        Ok(())
    } else {
        Err("batch job not found".into())
    }
}

#[tauri::command]
pub async fn batch_trace(
    app: tauri::AppHandle,
    folder_path: String,
    params: TraceParams,
    files: Option<Vec<String>>,
) -> Result<String, String> {
    let job_id = uuid::Uuid::new_v4().to_string();

    let path = Path::new(&folder_path);
    if !path.is_dir() {
        return Err(format!("not a directory: {folder_path}"));
    }
    let mut image_files: Vec<(String, String)> = Vec::new();
    for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
        if IMAGE_EXTS.contains(&ext.as_str()) {
            if let Some(ref wanted) = files {
                if !wanted.contains(&name) {
                    continue;
                }
            }
            image_files.push((name, entry.path().to_string_lossy().to_string()));
        }
    }
    image_files.sort_by(|a, b| a.0.cmp(&b.0));

    let cancelled = Arc::new(AtomicBool::new(false));
    batch_cancel_flags().lock().unwrap().insert(job_id.clone(), cancelled.clone());
    let job_id_cleanup = job_id.clone();

    tauri::async_runtime::spawn(async move {
        let start = std::time::Instant::now();
        let total = image_files.len();

        let _ = app.emit("batch-trace", serde_json::json!({
            "type": "start",
            "total": total,
            "files": image_files.iter().map(|f| f.0.clone()).collect::<Vec<_>>(),
        }));

        let mut success = 0usize;
        let mut failed = 0usize;

        for (i, (name, file_path)) in image_files.iter().enumerate() {
            if cancelled.load(Ordering::SeqCst) {
                let _ = app.emit("batch-trace", serde_json::json!({
                    "type": "cancelled",
                    "index": i,
                    "total": total,
                    "success": success,
                    "failed": failed,
                    "total_elapsed_ms": start.elapsed().as_millis() as u64,
                }));
                break;
            }
            let elapsed = start.elapsed().as_millis() as u64;
            let _ = app.emit("batch-trace", serde_json::json!({
                "type": "processing",
                "index": i,
                "total": total,
                "file": name,
                "elapsed_ms": elapsed,
            }));

            let file_path = file_path.clone();
            let params_clone = params.clone();
            let result = tauri::async_runtime::spawn_blocking(move || {
                let data = std::fs::read(&file_path)
                    .map_err(|e| format!("failed to read {file_path}: {e}"))?;
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

        if !cancelled.load(Ordering::SeqCst) {
            let _ = app.emit("batch-trace", serde_json::json!({
                "type": "complete",
                "total": total,
                "success": success,
                "failed": failed,
                "total_elapsed_ms": start.elapsed().as_millis() as u64,
            }));
        }
        batch_cancel_flags().lock().unwrap().remove(&job_id_cleanup);
    });

    Ok(job_id)
}

#[tauri::command]
pub async fn generate_pattern(params: PatternParams) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        generate_sized(&params.kind, params.n_units, params.complexity, params.size)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
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
