//! Tauri commands: in-process access to the tracing engine.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use patternstudio_core::ir::LayeredDoc;
use patternstudio_core::profiles::{apply_profiles, AutoDecision, IdealParams};
use patternstudio_core::{key_for_source, LayerAssignment, SessionCache};
use patternstudio_patterns::{generate_sized, list_kinds};
use patternstudio_svg::LayeredWriter;
use patternstudio_tracer::{load_image, load_image_from_memory};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};
use vtracer::config::{Clustering, FitMode, Hierarchical, Preset};
use vtracer::progress::CancelToken;
use vtracer::Session;

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
    pub binary_adaptive: bool,
    #[serde(default)]
    pub binary_threshold: Option<u8>,
    #[serde(default)]
    pub watershed_detail: Option<u32>,
    #[serde(default)]
    pub palette: Option<String>,
    #[serde(default)]
    pub layers: usize,
    #[serde(default)]
    pub corner_threshold: Option<i32>,
    #[serde(default)]
    pub length_threshold: Option<f64>,
    #[serde(default)]
    pub optimize: Option<u8>,
    #[serde(default)]
    pub binary_adaptive_t: Option<f64>,
    #[serde(default)]
    pub binary_adaptive_window: Option<u32>,
    #[serde(default)]
    pub layer_strategy: Option<String>,
    #[serde(default)]
    pub complexity_threshold: Option<u8>,
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
    /// Multi-level parameter profile ("L0" coarse / "L1" standard / "L2" fine).
    pub profile: String,
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
    /// Present when `layerStrategy: "auto"`: the chosen strategy and why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strategy_info: Option<AutoDecision>,
    /// Elapsed time for the profile re-fitting step (ms), when auto layering is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_ms: Option<u64>,
}

// ── Core trace logic ──────────────────────────────────────────────────────

fn build_vtracer_config(params: &TraceParams) -> Result<vtracer::Config, String> {
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
    config.binary_adaptive = params.binary_adaptive;
    if let Some(v) = params.binary_threshold { config.binary_threshold = v; }
    if let Some(v) = params.watershed_detail { config.watershed_detail = v; }
    if let Some(ref p) = params.palette {
        config.palette = patternstudio_tracer::parse_palette(p)?;
    }
    if let Some(v) = params.corner_threshold { config.corner_threshold = v; }
    if let Some(v) = params.length_threshold { config.length_threshold = v; }
    if let Some(v) = params.optimize { config.optimize = v; }
    if let Some(v) = params.binary_adaptive_t { config.binary_adaptive_t = v; }
    if let Some(v) = params.binary_adaptive_window { config.binary_adaptive_window = v; }
    Ok(config)
}

/// One-shot trace from in-memory bytes: decode and run through a fresh session.
/// Used by batch tracing, where each file is traced exactly once (no reuse).
fn trace_bytes_internal(data: &[u8], params: &TraceParams) -> Result<TraceOut, String> {
    let img = load_image_from_memory(data).map_err(|e| format!("failed to decode image: {e}"))?;
    let config = build_vtracer_config(params)?;
    let mut session = Session::new(img);
    trace_with_session(&mut session, &config, params, &CancelToken::new())
}

/// Convert a smart-recommended parameter set into the command's `TraceParams`.
/// Both structs use camelCase with identical field names, so a JSON round-trip
/// is a safe, field-agnostic mapping.
fn smart_to_trace_params(p: &patternstudio_core::smart::SmartParams) -> Result<TraceParams, String> {
    let v = serde_json::to_value(p).map_err(|e| e.to_string())?;
    serde_json::from_value(v).map_err(|e| e.to_string())
}

/// Smart trace of in-memory image bytes: analyze, trial-trace candidate
/// parameter sets, then trace with the winner. `family` restricts the search
/// to one preset family (absent = auto). Returns the result plus the decision.
fn trace_bytes_smart(data: &[u8], family: Option<&str>) -> Result<(TraceOut, patternstudio_core::smart::SmartDecision), String> {
    let img = load_image_from_memory(data).map_err(|e| format!("failed to decode image: {e}"))?;
    let decision = match family {
        Some(f) => patternstudio_core::smart::recommend_for(&img, f),
        None => patternstudio_core::smart::recommend(&img),
    }
    .map_err(|e| format!("smart analysis failed: {e}"))?;
    let params = smart_to_trace_params(&decision.params)?;
    let config = build_vtracer_config(&params)?;
    let mut session = Session::new(img);
    let out = trace_with_session(&mut session, &config, &params, &CancelToken::new())?;
    Ok((out, decision))
}

/// Layering + SVG over a document produced by a [`Session`].
///
/// `session.render` reuses the cached segmentation when the clustering
/// parameters are unchanged and only re-runs the cheap downstream stages — the
/// core of the interactive tuning loop. Split out from the one-shot path so the
/// `trace_file` command can drive a cached session.
fn trace_with_session(session: &mut Session, config: &vtracer::Config, params: &TraceParams, cancel: &CancelToken) -> Result<TraceOut, String> {
    let mut on_progress = |_p: vtracer::progress::Progress| {};
    let doc = session.render_with_progress(config, cancel, &mut on_progress)
        .map_err(|e| if matches!(e, vtracer::Error::Cancelled) {
            "cancelled".into()
        } else {
            e.to_string()
        })?;
    if cancel.is_cancelled() { return Err("cancelled".into()); }

    // "auto": derive the strategy + multi-level profiles from the ideal params.
    let is_auto = params.layer_strategy.as_deref() == Some("auto");
    let ideal = IdealParams::from_options(
        params.clustering.as_deref(),
        params.line_art,
        params.color_precision,
        params.layer_difference,
        params.max_colors,
        params.simplify,
        params.path_precision,
        params.corner_threshold,
        params.length_threshold,
        params.filter_speckle,
        params.optimize,
    );
    let (assignment, decision) = if is_auto {
        let (a, d) = AutoDecision::assign(&doc, &ideal);
        (a, Some(d))
    } else {
        let assignment = match params.layer_strategy.as_deref().unwrap_or("none") {
            "color" => {
                let n = params.layers.max(2);
                LayerAssignment::by_color(&doc, n)
            }
            "complexity" => {
                let n = params.layers.max(2);
                let t = params.complexity_threshold.unwrap_or(70);
                LayerAssignment::by_complexity(&doc, t, n)
            }
            "hybrid" => {
                let n = params.layers.max(2);
                let t = params.complexity_threshold.unwrap_or(70);
                LayerAssignment::hybrid(&doc, n, t)
            }
            _ => LayerAssignment::single_layer(),
        };
        (assignment, None)
    };
    if cancel.is_cancelled() { return Err("cancelled".into()); }
    let mut layered = LayeredDoc::from_vector_doc(&doc, &assignment);
    let mut profile_ms: Option<u64> = None;
    if is_auto {
        let t = std::time::Instant::now();
        apply_profiles(&mut layered, &ideal);
        profile_ms = Some(t.elapsed().as_millis() as u64);
    }

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
            profile: layer.profile.as_str().into(),
        })
        .collect();

    Ok(TraceOut {
        svg,
        width: layered.width,
        height: layered.height,
        shape_count: layered.total_shapes(),
        layer_count: layered.layers.len(),
        layers,
        strategy_info: decision,
        profile_ms,
    })
}

/// Smart Preset: analyze the image and trial-trace candidate parameter sets,
/// returning the best one plus the full candidate report.
#[tauri::command]
pub async fn smart_file(path: String, family: Option<String>) -> Result<patternstudio_core::smart::SmartDecision, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = Path::new(&path);
        if !p.exists() {
            return Err(format!("image not found: {path}"));
        }
        let image = load_image(p)
            .map_err(|e| format!("failed to load image: {e}"))?;
        // Family-scoped when the client picked a preset, auto otherwise.
        let decision = match family.as_deref() {
            Some(f) => patternstudio_core::smart::recommend_for(&image, f),
            None => patternstudio_core::smart::recommend(&image),
        };
        decision.map_err(|e| format!("smart analysis failed: {e}"))
    })
    .await
    .map_err(|e| e.to_string())?
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

/// How many (source, segmentation) sessions to keep around for instant
/// re-tracing in the desktop app. Sized to hold the pre-warmed presets plus
/// headroom for interactive clustering tweaks.
const MAX_SESSIONS: usize = 10;

/// Process-wide LRU cache of VTracer sessions, keyed by source + segmentation
/// params. Caching the expensive segmentation here is what makes parameter
/// tweaks instant.
fn session_cache() -> &'static Mutex<SessionCache> {
    static CACHE: OnceLock<Mutex<SessionCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(SessionCache::new(MAX_SESSIONS)))
}

/// One in-flight trace per image: the request id that owns it and the token a
/// newer interactive request trips to abort it between pipeline batches.
struct ActiveTrace {
    request_id: String,
    cancel: CancelToken,
}

fn active_traces() -> &'static Mutex<HashMap<String, ActiveTrace>> {
    static ACTIVE: OnceLock<Mutex<HashMap<String, ActiveTrace>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Trace an image file by path, reusing a cached session (and thus a cached
/// segmentation) across calls. This is the fast path for the editor: the image
/// is read and decoded once, and only the cheap downstream stages re-run when a
/// non-clustering parameter changes.
#[tauri::command]
pub async fn trace_file(
    file_path: String,
    params: TraceParams,
    request_id: Option<String>,
    priority: Option<String>,
) -> Result<TraceOut, String> {
    let request_id = request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let is_background = priority.as_deref() == Some("background");
    let image_key = file_path.clone();

    // Preemption: an interactive request cancels whatever is in flight for
    // this image (it aborts between pipeline batches); a background request
    // (pre-warm) yields instead and is rejected as busy.
    let cancel = CancelToken::new();
    {
        let mut active = active_traces().lock().unwrap();
        match active.get(&image_key) {
            Some(old) if is_background => return Err("busy".into()),
            Some(old) => old.cancel.cancel(),
            None => {}
        }
        active.insert(image_key.clone(), ActiveTrace {
            request_id: request_id.clone(),
            cancel: cancel.clone(),
        });
    }

    let result = tauri::async_runtime::spawn_blocking(move || {
        let path = Path::new(&file_path);
        if !path.exists() {
            return Err(format!("image not found: {file_path}"));
        }
        // Build the config up front so the session cache can be keyed by the
        // segmentation parameters (not just the file): each preset keeps its own
        // cached segmentation instead of evicting the previous one.
        let config = build_vtracer_config(&params)?;
        let key = key_for_source(path, &config.segment_key());
        let session = {
            let mut cache = session_cache().lock().unwrap();
            let p: PathBuf = path.to_path_buf();
            cache.get_or_create(&key, || {
                load_image(&p).map_err(|e| format!("failed to decode image: {e}"))
            })?
        };
        // A cancelled predecessor may still be finishing its current batch;
        // poll until the session lock frees (cancellation is near-instant).
        let mut guard = loop {
            match session.try_lock() {
                Ok(g) => break g,
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
        };
        trace_with_session(&mut guard, &config, &params, &cancel)
    })
    .await
    .map_err(|e| e.to_string())?;

    // Drop our registration only if we are still the current trace for this image.
    {
        let mut active = active_traces().lock().unwrap();
        if active.get(&image_key).is_some_and(|a| a.request_id == request_id) {
            active.remove(&image_key);
        }
    }
    result
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
    smart: Option<bool>,
    family: Option<String>,
) -> Result<String, String> {
    let smart = smart.unwrap_or(false);
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
            let smart_clone = smart;
            let family_clone = family.clone();
            let result = tauri::async_runtime::spawn_blocking(move || {
                let data = std::fs::read(&file_path)
                    .map_err(|e| format!("failed to read {file_path}: {e}"))?;
                if smart_clone {
                    trace_bytes_smart(&data, family_clone.as_deref()).map(|(out, d)| (out, Some(d)))
                } else {
                    trace_bytes_internal(&data, &params_clone).map(|out| (out, None))
                }
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));

            let elapsed = start.elapsed().as_millis() as u64;
            match result {
                Ok((out, decision)) => {
                    success += 1;
                    let smart_info = decision.as_ref().map(|d| serde_json::json!({
                        "family": d.family,
                        "reason": d.reason,
                        "params": d.params,
                    }));
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
                        "strategyInfo": out.strategy_info,
                        "smart": smart_info,
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
