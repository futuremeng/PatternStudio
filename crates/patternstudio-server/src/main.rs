//! PatternStudio API server: axum-based REST API.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Multipart, Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::stream::{Stream, StreamExt};
use patternstudio_core::ir::LayeredDoc;
use patternstudio_core::profiles::{AutoDecision, IdealParams, apply_profiles};
use patternstudio_core::smart::SmartDecision;
use patternstudio_core::{key_for_source, LayerAssignment, SessionCache};
use patternstudio_svg::LayeredWriter;
use patternstudio_tracer::load_image;
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use vtracer::config::{Clustering, FitMode, Hierarchical, Preset};
use vtracer::progress::CancelToken;
use vtracer::Session;

/// How many (source, segmentation) sessions to keep around for instant
/// re-tracing. Each entry holds one RGBA image plus its segmentation, so this
/// bounds the extra memory used by the cache. Sized to hold the pre-warmed
/// presets plus headroom for interactive clustering tweaks.
const MAX_SESSIONS: usize = 10;

#[derive(Clone)]
struct AppState {
    upload_dir: PathBuf,
    assets: Arc<Mutex<HashMap<String, PathBuf>>>,
    projects: Arc<Mutex<HashMap<String, Project>>>,
    jobs: Arc<Mutex<HashMap<String, Option<tokio::sync::mpsc::Receiver<Event>>>>>,
    cancels: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    projects_file: PathBuf,
    /// Reusable VTracer sessions keyed by source (path:size:mtime). Caching the
    /// expensive segmentation here is what makes parameter tweaks instant.
    sessions: Arc<Mutex<SessionCache>>,
    /// In-flight traces keyed by image path, for preemption: a newer
    /// interactive request trips the older one's token so it aborts between
    /// pipeline batches.
    active_traces: Arc<Mutex<HashMap<String, ActiveTrace>>>,
}

/// One in-flight trace: the request id that owns it and the token a newer
/// interactive request trips to abort it.
struct ActiveTrace {
    request_id: String,
    cancel: CancelToken,
}

#[derive(Clone, Serialize, Deserialize)]
struct Project {
    id: String,
    path: PathBuf,
    name: String,
    images: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct ProjectsFile {
    projects: Vec<Project>,
}

fn projects_file_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let dir = PathBuf::from(&home).join(".patternstudio");
    std::fs::create_dir_all(&dir).ok();
    dir.join("projects.json")
}

fn load_projects(file: &Path) -> HashMap<String, Project> {
    let mut map = HashMap::new();
    if let Ok(data) = std::fs::read_to_string(file) {
        if let Ok(pf) = serde_json::from_str::<ProjectsFile>(&data) {
            for p in pf.projects {
                // Only keep projects whose folder still exists
                if p.path.is_dir() {
                    map.insert(p.id.clone(), p);
                }
            }
        }
    }
    map
}

fn save_projects(file: &Path, projects: &HashMap<String, Project>) {
    let pf = ProjectsFile { projects: projects.values().cloned().collect() };
    if let Ok(json) = serde_json::to_string_pretty(&pf) {
        let _ = std::fs::write(file, json);
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct TraceParams {
    #[serde(default)]
    clustering: Option<String>,
    #[serde(default)]
    hierarchical: Option<String>,
    #[serde(default)]
    color_precision: Option<u8>,
    #[serde(default)]
    layer_difference: Option<u8>,
    #[serde(default)]
    filter_speckle: Option<u32>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    simplify: Option<f64>,
    #[serde(default)]
    path_precision: Option<u8>,
    #[serde(default)]
    max_colors: Option<usize>,
    #[serde(default = "default_preset")]
    preset: String,
    #[serde(default)]
    line_art: bool,
    #[serde(default)]
    binary_adaptive: bool,
    #[serde(default)]
    binary_threshold: Option<u8>,
    #[serde(default)]
    watershed_detail: Option<u32>,
    #[serde(default)]
    palette: Option<String>,
    #[serde(default)]
    layers: usize,
    #[serde(default)]
    corner_threshold: Option<i32>,
    #[serde(default)]
    length_threshold: Option<f64>,
    #[serde(default)]
    optimize: Option<u8>,
    #[serde(default)]
    binary_adaptive_t: Option<f64>,
    #[serde(default)]
    binary_adaptive_window: Option<u32>,
    #[serde(default)]
    layer_strategy: Option<String>,
    #[serde(default)]
    complexity_threshold: Option<u8>,
}

fn default_preset() -> String { "poster".into() }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LayerInfo {
    name: String,
    role: String,
    shape_count: usize,
    dominant_color: String,
    preview_svg: String,
    /// Multi-level parameter profile ("L0" coarse / "L1" standard / "L2" fine).
    profile: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TraceResult {
    svg: String,
    width: u32,
    height: u32,
    shape_count: usize,
    layer_count: usize,
    layers: Vec<LayerInfo>,
    /// Present when `layerStrategy: "auto"`: the chosen strategy and why.
    #[serde(skip_serializing_if = "Option::is_none")]
    strategy_info: Option<AutoDecision>,
    /// Elapsed time for the profile re-fitting step (ms), when auto layering is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    profile_ms: Option<u64>,
}

fn build_vtracer_config(params: &TraceParams) -> Result<vtracer::Config, String> {
    let preset = match params.preset.as_str() {
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
    if let Some(v) = params.max_colors { config.max_colors = Some(v); }
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

/// One-shot trace: decode the image and run it through a fresh session. Used by
/// batch tracing, where each file is traced exactly once (no reuse benefit).
fn do_trace(img_path: &Path, params: &TraceParams) -> Result<TraceResult, String> {
    let img = load_image(img_path).map_err(|e| format!("failed to load image: {e}"))?;
    let config = build_vtracer_config(params)?;
    let mut session = Session::new(img);
    do_trace_with_session(&mut session, &config, params, &CancelToken::new())
}

/// Convert a smart-recommended parameter set into the API's `TraceParams`.
/// Both structs use camelCase with identical field names, so a JSON round-trip
/// is a safe, field-agnostic mapping.
fn smart_to_trace_params(p: &patternstudio_core::smart::SmartParams) -> Result<TraceParams, String> {
    let v = serde_json::to_value(p).map_err(|e| e.to_string())?;
    serde_json::from_value(v).map_err(|e| e.to_string())
}

/// Smart one-shot trace: decode the image once, analyze it, trial-trace
/// candidate parameter sets, then trace at full resolution with the winner.
/// `family` restricts the search to one preset family (absent = auto).
/// Returns the trace result plus the decision (family, reason, params).
fn do_trace_smart(img_path: &Path, family: Option<&str>) -> Result<(TraceResult, patternstudio_core::smart::SmartDecision), String> {
    let img = load_image(img_path).map_err(|e| format!("failed to load image: {e}"))?;
    let decision = match family {
        Some(f) => patternstudio_core::smart::recommend_for(&img, f),
        None => patternstudio_core::smart::recommend(&img),
    }
    .map_err(|e| format!("smart analysis failed: {e}"))?;
    let params = smart_to_trace_params(&decision.params)?;
    let config = build_vtracer_config(&params)?;
    let mut session = Session::new(img);
    let tr = do_trace_with_session(&mut session, &config, &params, &CancelToken::new())?;
    Ok((tr, decision))
}

/// Layering + SVG over a document produced by a [`Session`].
///
/// `session.render` reuses the cached segmentation when the clustering
/// parameters are unchanged and only re-runs the cheap downstream stages — the
/// core of the interactive tuning loop. Split out from the one-shot path so the
/// `/trace` handler can drive a cached session.
fn do_trace_with_session(session: &mut Session, config: &vtracer::Config, params: &TraceParams, cancel: &CancelToken) -> Result<TraceResult, String> {
    let mut on_progress = |_p: vtracer::progress::Progress| {};
    let doc = session.render_with_progress(config, cancel, &mut on_progress)
        .map_err(|e| if matches!(e, vtracer::Error::Cancelled) {
            "cancelled".into()
        } else {
            format!("trace failed: {e}")
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
    let layers: Vec<LayerInfo> = layered.layers.iter().enumerate()
        .map(|(i, layer)| LayerInfo {
            name: layer.name.clone(),
            role: layer.role.to_string(),
            shape_count: layer.shapes.len(),
            dominant_color: LayeredWriter::layer_dominant_color(layer),
            preview_svg: LayeredWriter::layer_preview_svg(&layered, i),
            profile: layer.profile.as_str().into(),
        }).collect();
    Ok(TraceResult {
        svg, width: layered.width, height: layered.height,
        shape_count: layered.total_shapes(), layer_count: layered.layers.len(), layers,
        strategy_info: decision,
        profile_ms,
    })
}

#[derive(Serialize)]
struct HealthResponse { status: String, version: String }

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok".into(), version: env!("CARGO_PKG_VERSION").into() })
}

async fn upload(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut asset_id = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("file").to_string();
        if name == "file" {
            let content = field.bytes().await.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
            let id = uuid::Uuid::new_v4().to_string();
            let path = state.upload_dir.join(format!("{id}.png"));
            let write_path = path.clone();
            tokio::task::spawn_blocking(move || std::fs::write(&write_path, &content))
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            state.assets.lock().unwrap().insert(id.clone(), path);
            asset_id = Some(id);
        }
    }
    match asset_id {
        Some(id) => Ok(Json(serde_json::json!({ "asset_id": id }))),
        None => Err((StatusCode::BAD_REQUEST, "no file field found".into())),
    }
}

#[derive(Deserialize)]
struct TraceRequest {
    #[serde(default)]
    asset_id: Option<String>,
    #[serde(default)]
    file_path: Option<String>,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    params: Option<TraceParams>,
    /// Smart mode: restrict the search to this preset family
    /// ("lineArt" | "color" | "bw" | "gradient" | "lowPoly").
    /// Absent = auto family selection.
    #[serde(default)]
    family: Option<String>,
    /// Client-generated id for this request; identifies the in-flight trace
    /// so a newer request can supersede it.
    #[serde(default)]
    request_id: Option<String>,
    /// "interactive" (default) preempts any in-flight trace of the same image;
    /// "background" (pre-warm) yields to it and is rejected with 409 busy.
    #[serde(default)]
    priority: Option<String>,
}

/// Resolve the source image path from whichever identifier the client sent.
/// `project_id` + `filename` lets the web UI trace straight from the project
/// folder without re-uploading the image on every parameter change.
fn resolve_source_path(
    state: &AppState,
    req: &TraceRequest,
) -> Result<PathBuf, (StatusCode, String)> {
    if let Some(ref asset_id) = req.asset_id {
        state.assets.lock().unwrap().get(asset_id).cloned()
            .ok_or((StatusCode::NOT_FOUND, format!("asset {asset_id} not found")))
    } else if let (Some(ref pid), Some(ref name)) = (&req.project_id, &req.filename) {
        let project = state.projects.lock().unwrap().get(pid).cloned()
            .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;
        if name.contains("..") || name.contains('/') || name.contains('\\') {
            return Err((StatusCode::BAD_REQUEST, "invalid filename".into()));
        }
        let p = project.path.join(name);
        if !p.exists() {
            return Err((StatusCode::NOT_FOUND, "image not found".into()));
        }
        Ok(p)
    } else if let Some(ref fp) = req.file_path {
        Ok(PathBuf::from(fp))
    } else {
        Err((StatusCode::BAD_REQUEST,
            "asset_id, project_id+filename, or file_path required".into()))
    }
}

async fn trace(
    State(state): State<AppState>,
    Json(req): Json<TraceRequest>,
) -> Result<Json<TraceResult>, (StatusCode, Json<serde_json::Value>)> {
    let img_path = resolve_source_path(&state, &req)
        .map_err(|(s, e)| (s, Json(serde_json::json!({ "error": e }))))?;
    let params = req.params.unwrap_or_default();
    let request_id = req.request_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let is_background = req.priority.as_deref() == Some("background");
    let image_key = img_path.to_string_lossy().to_string();
    let sessions = state.sessions.clone();

    // Preemption: an interactive request cancels whatever is in flight for
    // this image (it aborts between pipeline batches); a background request
    // (pre-warm) yields instead and is rejected with 409 busy.
    let cancel = CancelToken::new();
    {
        let mut active = state.active_traces.lock().unwrap();
        match active.get(&image_key) {
            Some(old) if is_background => {
                return Err((StatusCode::CONFLICT, Json(serde_json::json!({ "busy": true }))));
            }
            Some(old) => old.cancel.cancel(),
            None => {}
        }
        active.insert(image_key.clone(), ActiveTrace {
            request_id: request_id.clone(),
            cancel: cancel.clone(),
        });
    }

    let result = tokio::task::spawn_blocking(move || {
        // Build the config up front so the session cache can be keyed by the
        // segmentation parameters (not just the file): each preset keeps its own
        // cached segmentation instead of evicting the previous one.
        let config = build_vtracer_config(&params)?;
        let key = key_for_source(&img_path, &config.segment_key());
        let session = {
            let mut cache = sessions.lock().unwrap();
            let path = img_path.clone();
            cache.get_or_create(&key, || {
                load_image(&path).map_err(|e| format!("failed to load image: {e}"))
            })?
        };
        // A cancelled predecessor may still be finishing its current batch;
        // poll until the session lock frees (cancellation is near-instant).
        let mut guard = loop {
            match session.try_lock() {
                Ok(g) => break g,
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        };
        do_trace_with_session(&mut guard, &config, &params, &cancel)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))))?;

    // Drop our registration only if we are still the current trace for this image.
    {
        let mut active = state.active_traces.lock().unwrap();
        if active.get(&image_key).is_some_and(|a| a.request_id == request_id) {
            active.remove(&image_key);
        }
    }

    match result {
        Ok(r) => Ok(Json(r)),
        Err(e) if e == "cancelled" => {
            Err((StatusCode::CONFLICT, Json(serde_json::json!({ "cancelled": true }))))
        }
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e })))),
    }
}

/// Smart Preset: analyze the image and trial-trace candidate parameter sets,
/// returning the best one plus the full candidate report.
async fn smart(
    State(state): State<AppState>,
    Json(req): Json<TraceRequest>,
) -> Result<Json<SmartDecision>, (StatusCode, String)> {
    let img_path = resolve_source_path(&state, &req)?;
    let family = req.family;
    let result = tokio::task::spawn_blocking(move || {
        let image = load_image(&img_path)
            .map_err(|e| format!("failed to load image: {e}"))?;
        // Family-scoped when the client picked a preset, auto otherwise.
        let decision = match family.as_deref() {
            Some(f) => patternstudio_core::smart::recommend_for(&image, f),
            None => patternstudio_core::smart::recommend(&image),
        }
        .map_err(|e| format!("smart analysis failed: {e}"))?;
        Ok::<SmartDecision, String>(decision)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(result))
}

#[derive(Deserialize)]
struct PatternRequest {
    kind: String,
    #[serde(default = "default_n_units")]
    n_units: u32,
    #[serde(default = "default_complexity")]
    complexity: u8,
    #[serde(default = "default_size")]
    size: u32,
}

fn default_n_units() -> u32 { 8 }
fn default_complexity() -> u8 { 50 }
fn default_size() -> u32 { 400 }

#[derive(Serialize)]
struct PatternResponse { svg: String }

async fn generate_pattern(
    Json(req): Json<PatternRequest>,
) -> Result<Json<PatternResponse>, (StatusCode, String)> {
    let svg = tokio::task::spawn_blocking(move || {
        patternstudio_patterns::generate_sized(&req.kind, req.n_units, req.complexity, req.size)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(PatternResponse { svg }))
}

async fn list_patterns() -> Json<serde_json::Value> {
    let kinds: Vec<serde_json::Value> = patternstudio_patterns::list_kinds()
        .iter()
        .map(|(kind, desc)| serde_json::json!({ "kind": kind, "description": desc }))
        .collect();
    Json(serde_json::json!({ "patterns": kinds }))
}

const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "tiff", "gif"];

fn list_image_files(folder: &Path) -> Vec<String> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
            if IMAGE_EXTS.contains(&ext.as_str()) { files.push(name); }
        }
    }
    files.sort();
    files
}

#[derive(Deserialize)]
struct CreateProjectRequest { path: String }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectInfo {
    id: String,
    name: String,
    path: String,
    image_count: usize,
    images: Vec<String>,
}

fn project_to_info(p: &Project) -> ProjectInfo {
    ProjectInfo {
        id: p.id.clone(),
        name: p.name.clone(),
        path: p.path.to_string_lossy().to_string(),
        image_count: p.images.len(),
        images: p.images.clone(),
    }
}

async fn create_project(
    State(state): State<AppState>,
    Json(req): Json<CreateProjectRequest>,
) -> Result<Json<ProjectInfo>, (StatusCode, String)> {
    let path = PathBuf::from(&req.path);
    if !path.is_dir() {
        return Err((StatusCode::BAD_REQUEST, format!("not a directory: {}", req.path)));
    }
    let images = list_image_files(&path);
    if images.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "no image files found in directory".into()));
    }
    // Check if project already exists (same path)
    {
        let projects = state.projects.lock().unwrap();
        if let Some(existing) = projects.values().find(|p| p.path == path) {
            return Ok(Json(project_to_info(existing)));
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let name = path.file_name().map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".into());
    let project = Project { id: id.clone(), path: path.clone(), name, images };
    let info = project_to_info(&project);
    state.projects.lock().unwrap().insert(id, project);
    save_projects(&state.projects_file, &state.projects.lock().unwrap());
    Ok(Json(info))
}

async fn delete_project(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut projects = state.projects.lock().unwrap();
    if projects.remove(&id).is_none() {
        return Err((StatusCode::NOT_FOUND, "project not found".into()));
    }
    save_projects(&state.projects_file, &projects);
    Ok(StatusCode::NO_CONTENT)
}

async fn list_projects(State(state): State<AppState>) -> Json<serde_json::Value> {
    let projects: Vec<ProjectInfo> = state.projects.lock().unwrap().values()
        .map(project_to_info).collect();
    Json(serde_json::json!({ "projects": projects }))
}

async fn get_project(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<ProjectInfo>, (StatusCode, String)> {
    let project = state.projects.lock().unwrap().get(&id).cloned()
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;
    Ok(Json(project_to_info(&project)))
}

async fn serve_image(
    State(state): State<AppState>,
    UrlPath((project_id, filename)): UrlPath<(String, String)>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let project = state.projects.lock().unwrap().get(&project_id).cloned()
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return Err((StatusCode::BAD_REQUEST, "invalid filename".into()));
    }
    let img_path = project.path.join(&filename);
    if !img_path.exists() {
        return Err((StatusCode::NOT_FOUND, "image not found".into()));
    }
    let data = tokio::task::spawn_blocking(move || std::fs::read(&img_path))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let content_type = match filename.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "tiff" => "image/tiff",
        _ => "application/octet-stream",
    };
    Ok(axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, content_type)
        .body(axum::body::Body::from(data))
        .unwrap())
}

#[derive(Deserialize)]
struct BatchTraceRequest {
    #[serde(default)]
    params: Option<TraceParams>,
    #[serde(default)]
    files: Option<Vec<String>>,
    /// Smart mode: ignore `params`, analyze each image and trial-trace
    /// candidate parameter sets, using the winner per image.
    #[serde(default)]
    smart: bool,
    /// Smart mode: restrict the search to this preset family.
    /// Absent = auto family selection per image.
    #[serde(default)]
    family: Option<String>,
}

async fn start_batch_trace(
    State(state): State<AppState>,
    UrlPath(project_id): UrlPath<String>,
    Json(req): Json<BatchTraceRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let project = state.projects.lock().unwrap().get(&project_id).cloned()
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;
    let job_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    state.jobs.lock().unwrap().insert(job_id.clone(), Some(rx));
    let files = match req.files {
        Some(wanted) => project
            .images
            .iter()
            .filter(|f| wanted.contains(*f))
            .cloned()
            .collect::<Vec<_>>(),
        None => project.images.clone(),
    };
    let folder = project.path.clone();
    let params = req.params.unwrap_or_default();
    let smart = req.smart;
    let family = req.family;
    let cancelled = Arc::new(AtomicBool::new(false));
    state.cancels.lock().unwrap().insert(job_id.clone(), cancelled.clone());
    let job_id_cleanup = job_id.clone();

    tokio::spawn(async move {
        let start = Instant::now();
        let total = files.len();
        let _ = tx.send(Event::default().event("start")
            .data(serde_json::json!({ "total": total }).to_string())).await;
        let mut success = 0usize;
        let mut failed = 0usize;
        for (i, file) in files.iter().enumerate() {
            if cancelled.load(Ordering::SeqCst) {
                let _ = tx.send(Event::default().event("progress")
                    .data(serde_json::json!({
                        "index": i, "total": total,
                        "status": "cancelled", "success": success, "failed": failed,
                        "total_elapsed_ms": start.elapsed().as_millis() as u64
                    }).to_string())).await;
                break;
            }
            let elapsed = start.elapsed().as_millis() as u64;
            let _ = tx.send(Event::default().event("progress")
                .data(serde_json::json!({
                    "index": i, "total": total, "file": file,
                    "status": "processing", "elapsed_ms": elapsed
                }).to_string())).await;
            let img_path = folder.join(file);
            let params_clone = params.clone();
            let family_clone = family.clone();
            let result = tokio::task::spawn_blocking(move || {
                if smart {
                    do_trace_smart(&img_path, family_clone.as_deref()).map(|(tr, d)| (tr, Some(d)))
                } else {
                    do_trace(&img_path, &params_clone).map(|tr| (tr, None))
                }
            })
            .await.unwrap_or_else(|e| Err(e.to_string()));
            let elapsed = start.elapsed().as_millis() as u64;
            match result {
                Ok((tr, decision)) => {
                    success += 1;
                    // Smart mode: report which family won and with which
                    // params, so the client can cache the result under those
                    // params and show the recommendation.
                    let smart_info = decision.as_ref().map(|d| serde_json::json!({
                        "family": d.family,
                        "reasonKey": d.reason_key,
                        "params": d.params,
                    }));
                    let _ = tx.send(Event::default().event("progress")
                        .data(serde_json::json!({
                            "index": i, "total": total, "file": file,
                            "status": "done", "svg": tr.svg,
                            "width": tr.width, "height": tr.height,
                            "shape_count": tr.shape_count, "layers": tr.layers,
                            "strategy_info": tr.strategy_info,
                            "smart": smart_info,
                            "elapsed_ms": elapsed
                        }).to_string())).await;
                }
                Err(e) => {
                    failed += 1;
                    let _ = tx.send(Event::default().event("progress")
                        .data(serde_json::json!({
                            "index": i, "total": total, "file": file,
                            "status": "error", "error": e, "elapsed_ms": elapsed
                        }).to_string())).await;
                }
            }
            if tx.is_closed() { break; }
        }
        if !cancelled.load(Ordering::SeqCst) {
            let _ = tx.send(Event::default().event("complete")
                .data(serde_json::json!({
                    "total": total, "success": success, "failed": failed,
                    "total_elapsed_ms": start.elapsed().as_millis() as u64
                }).to_string())).await;
        }
        state.cancels.lock().unwrap().remove(&job_id_cleanup);
    });

    Ok(Json(serde_json::json!({ "job_id": job_id })))
}

async fn cancel_batch_trace(
    State(state): State<AppState>,
    UrlPath((_project_id, job_id)): UrlPath<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let flags = state.cancels.lock().unwrap();
    if let Some(flag) = flags.get(&job_id) {
        flag.store(true, Ordering::SeqCst);
        Ok(Json(serde_json::json!({ "cancelled": true })))
    } else {
        Err((StatusCode::NOT_FOUND, "job not found".into()))
    }
}

async fn batch_trace_events(
    State(state): State<AppState>,
    UrlPath((_project_id, job_id)): UrlPath<(String, String)>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, (StatusCode, String)> {
    let rx = state.jobs.lock().unwrap().remove(&job_id)
        .and_then(|opt| opt)
        .ok_or((StatusCode::NOT_FOUND, "job not found or already consumed".into()))?;
    let stream = ReceiverStream::new(rx).map(Ok);
    Ok(Sse::new(stream))
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8000);
    let upload_dir = PathBuf::from("/tmp/patternstudio-uploads");
    std::fs::create_dir_all(&upload_dir).ok();
    let projects_file = projects_file_path();
    let projects = load_projects(&projects_file);
    let state = AppState {
        upload_dir,
        assets: Arc::new(Mutex::new(HashMap::new())),
        projects: Arc::new(Mutex::new(projects)),
        jobs: Arc::new(Mutex::new(HashMap::new())),
        cancels: Arc::new(Mutex::new(HashMap::new())),
        active_traces: Arc::new(Mutex::new(HashMap::new())),
        projects_file,
        sessions: Arc::new(Mutex::new(SessionCache::new(MAX_SESSIONS))),
    };
    let app = Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/uploads", post(upload))
        .route("/api/v1/trace", post(trace))
        .route("/api/v1/smart", post(smart))
        .route("/api/v1/pattern", post(generate_pattern))
        .route("/api/v1/patterns", get(list_patterns))
        .route("/api/v1/projects", post(create_project).get(list_projects))
        .route("/api/v1/projects/{id}", get(get_project).delete(delete_project))
        .route("/api/v1/projects/{id}/images/{filename}", get(serve_image))
        .route("/api/v1/projects/{id}/batch-trace", post(start_batch_trace))
        .route("/api/v1/projects/{id}/batch-trace/{job_id}/events", get(batch_trace_events))
        .route("/api/v1/projects/{id}/batch-trace/{job_id}/cancel", post(cancel_batch_trace))
        .fallback_service(ServeDir::new("web/dist").append_index_html_on_directories(true))
        .layer(CorsLayer::permissive())
        .with_state(state);
    let addr = format!("0.0.0.0:{port}");
    println!("PatternStudio API server listening on {addr}");
    if !PathBuf::from("web/dist/index.html").exists() {
        println!("Note: web/dist not found - build the UI first: cd web && npm install && npm run build");
    }
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}