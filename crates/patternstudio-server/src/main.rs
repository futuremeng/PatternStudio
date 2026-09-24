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
use patternstudio_core::smart::SmartDecision;
use patternstudio_core::{key_for_source, SessionCache};
use patternstudio_engine::{
    build_vtracer_config, trace_with_session, ActiveTraces, TraceParams, TraceResult,
};
use patternstudio_tracer::load_image;
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

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
    active_traces: Arc<ActiveTraces>,
    /// True when bound to a loopback address. Non-loopback binds disable the
    /// `file_path` source (arbitrary file read) and restrict project folders
    /// to the user's home directory.
    local_only: bool,
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

/// The user's home directory (HOME on Unix, USERPROFILE on Windows).
fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn projects_file_path() -> PathBuf {
    let dir = home_dir().join(".patternstudio");
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
    let pf = ProjectsFile {
        projects: projects.values().cloned().collect(),
    };
    if let Ok(json) = serde_json::to_string_pretty(&pf) {
        // Atomic write: tmp file + rename, so a crash mid-write cannot leave
        // a truncated projects.json behind.
        let tmp = file.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, file);
        }
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    version: String,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    })
}

async fn upload(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut asset_id = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("file").to_string();
        if name == "file" {
            let content = field
                .bytes()
                .await
                .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
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
        state
            .assets
            .lock()
            .unwrap()
            .get(asset_id)
            .cloned()
            .ok_or((StatusCode::NOT_FOUND, format!("asset {asset_id} not found")))
    } else if let (Some(ref pid), Some(ref name)) = (&req.project_id, &req.filename) {
        let project = state
            .projects
            .lock()
            .unwrap()
            .get(pid)
            .cloned()
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
        // Arbitrary file read: only allowed when the server is bound to
        // loopback (the default). Remote clients must use uploads or projects.
        if state.local_only {
            Ok(PathBuf::from(fp))
        } else {
            Err((
                StatusCode::FORBIDDEN,
                "file_path is only allowed when the server is bound to loopback".into(),
            ))
        }
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            "asset_id, project_id+filename, or file_path required".into(),
        ))
    }
}

async fn trace(
    State(state): State<AppState>,
    Json(req): Json<TraceRequest>,
) -> Result<Json<TraceResult>, (StatusCode, Json<serde_json::Value>)> {
    let img_path = resolve_source_path(&state, &req)
        .map_err(|(s, e)| (s, Json(serde_json::json!({ "error": e }))))?;
    let params = req.params.unwrap_or_default();
    let request_id = req
        .request_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let is_background = req.priority.as_deref() == Some("background");
    let image_key = img_path.to_string_lossy().to_string();
    let sessions = state.sessions.clone();

    // Preemption: an interactive request cancels whatever is in flight for
    // this image (it aborts between pipeline batches); a background request
    // (pre-warm) yields instead and is rejected with 409 busy.
    let cancel = state
        .active_traces
        .register(&image_key, &request_id, is_background)
        .map_err(|_| {
            (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "busy": true })),
            )
        })?;

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
        trace_with_session(&mut guard, &config, &params, &cancel)
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    // Drop our registration only if we are still the current trace for this image.
    state.active_traces.finish(&image_key, &request_id);

    match result {
        Ok(r) => Ok(Json(r)),
        Err(e) if e == "cancelled" => Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "cancelled": true })),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )),
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
        let image = load_image(&img_path).map_err(|e| format!("failed to load image: {e}"))?;
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

/// Preprocess: enhance the source image with the "digital art" model
/// (Real-ESRGAN anime 4x, downscaled back to the original size) before
/// tracing. The result is registered as an asset so the client can display
/// it and pass it to /trace as `asset_id`. Reuses TraceRequest for the
/// source-identifier fields (asset_id / project_id+filename / file_path).
async fn preprocess(
    State(state): State<AppState>,
    Json(req): Json<TraceRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let img_path = resolve_source_path(&state, &req)?;
    let out = tokio::task::spawn_blocking(move || {
        patternstudio_tracer::preprocess_image(&img_path, None)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let id = uuid::Uuid::new_v4().to_string();
    state
        .assets
        .lock()
        .unwrap()
        .insert(id.clone(), out.path.clone());
    Ok(Json(serde_json::json!({
        "assetId": id,
        "url": format!("/api/v1/assets/{id}"),
        "width": out.width,
        "height": out.height,
        "elapsedMs": out.elapsed_ms,
        "cached": out.cached,
    })))
}

/// Serve a registered asset (preprocessed images).
async fn serve_asset(
    State(state): State<AppState>,
    UrlPath(asset_id): UrlPath<String>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let path = state
        .assets
        .lock()
        .unwrap()
        .get(&asset_id)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "asset not found".into()))?;
    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, "asset file missing".into()));
    }
    let data = tokio::task::spawn_blocking(move || std::fs::read(&path))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "image/png")
        .body(axum::body::Body::from(data))
        .unwrap())
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

fn default_n_units() -> u32 {
    8
}
fn default_complexity() -> u8 {
    50
}
fn default_size() -> u32 {
    400
}

#[derive(Serialize)]
struct PatternResponse {
    svg: String,
}

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
            if IMAGE_EXTS.contains(&ext.as_str()) {
                files.push(name);
            }
        }
    }
    files.sort();
    files
}

#[derive(Deserialize)]
struct CreateProjectRequest {
    path: String,
}

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
        return Err((
            StatusCode::BAD_REQUEST,
            format!("not a directory: {}", req.path),
        ));
    }
    // Non-loopback binds restrict project folders to the home directory so a
    // remote client cannot register (and read images from) arbitrary paths.
    if !state.local_only {
        let home = home_dir();
        if !path.is_absolute() || !path.starts_with(&home) {
            return Err((
                StatusCode::FORBIDDEN,
                "project paths outside the home directory are only allowed in loopback mode".into(),
            ));
        }
    }
    let images = list_image_files(&path);
    if images.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "no image files found in directory".into(),
        ));
    }
    // Check if project already exists (same path)
    {
        let projects = state.projects.lock().unwrap();
        if let Some(existing) = projects.values().find(|p| p.path == path) {
            return Ok(Json(project_to_info(existing)));
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".into());
    let project = Project {
        id: id.clone(),
        path: path.clone(),
        name,
        images,
    };
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
    let projects: Vec<ProjectInfo> = state
        .projects
        .lock()
        .unwrap()
        .values()
        .map(project_to_info)
        .collect();
    Json(serde_json::json!({ "projects": projects }))
}

async fn get_project(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<ProjectInfo>, (StatusCode, String)> {
    let project = state
        .projects
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;
    Ok(Json(project_to_info(&project)))
}

async fn serve_image(
    State(state): State<AppState>,
    UrlPath((project_id, filename)): UrlPath<(String, String)>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let project = state
        .projects
        .lock()
        .unwrap()
        .get(&project_id)
        .cloned()
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
    let content_type = match filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
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
#[serde(rename_all = "camelCase")]
struct SaveImageRequest {
    /// Source asset to copy (the selected enhanced iteration).
    asset_id: String,
    /// Destination filename inside the project folder.
    filename: String,
}

/// Save a processed asset (e.g. an enhanced iteration from the preprocess
/// modal) into the project folder as a new image and add it to the project's
/// image list. Overwrites an existing file with the same name.
async fn save_image(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    Json(req): Json<SaveImageRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if req.filename.contains("..") || req.filename.contains('/') || req.filename.contains('\\') {
        return Err((StatusCode::BAD_REQUEST, "invalid filename".into()));
    }
    let src = state
        .assets
        .lock()
        .unwrap()
        .get(&req.asset_id)
        .cloned()
        .ok_or((
            StatusCode::NOT_FOUND,
            format!("asset {} not found", req.asset_id),
        ))?;
    if !src.exists() {
        return Err((StatusCode::NOT_FOUND, "asset file missing".into()));
    }
    let project = state
        .projects
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;
    let dst = project.path.join(&req.filename);
    let src_path = src.clone();
    tokio::task::spawn_blocking(move || std::fs::copy(&src_path, &dst))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    // Add to the project's image list (deduped, sorted like list_image_files).
    let mut projects = state.projects.lock().unwrap();
    if let Some(p) = projects.get_mut(&id) {
        if !p.images.contains(&req.filename) {
            p.images.push(req.filename.clone());
            p.images.sort();
        }
    }
    save_projects(&state.projects_file, &projects);
    Ok(Json(serde_json::json!({ "filename": req.filename })))
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
    let project = state
        .projects
        .lock()
        .unwrap()
        .get(&project_id)
        .cloned()
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
    state
        .cancels
        .lock()
        .unwrap()
        .insert(job_id.clone(), cancelled.clone());
    let job_id_cleanup = job_id.clone();

    tokio::spawn(async move {
        let start = Instant::now();
        let total = files.len();
        let _ = tx
            .send(
                Event::default()
                    .event("start")
                    .data(serde_json::json!({ "total": total }).to_string()),
            )
            .await;
        let mut success = 0usize;
        let mut failed = 0usize;
        for (i, file) in files.iter().enumerate() {
            if cancelled.load(Ordering::SeqCst) {
                let _ = tx
                    .send(
                        Event::default().event("progress").data(
                            serde_json::json!({
                                "index": i, "total": total,
                                "status": "cancelled", "success": success, "failed": failed,
                                "total_elapsed_ms": start.elapsed().as_millis() as u64
                            })
                            .to_string(),
                        ),
                    )
                    .await;
                break;
            }
            let elapsed = start.elapsed().as_millis() as u64;
            let _ = tx
                .send(
                    Event::default().event("progress").data(
                        serde_json::json!({
                            "index": i, "total": total, "file": file,
                            "status": "processing", "elapsed_ms": elapsed
                        })
                        .to_string(),
                    ),
                )
                .await;
            let img_path = folder.join(file);
            let params_clone = params.clone();
            let family_clone = family.clone();
            let result = tokio::task::spawn_blocking(move || {
                if smart {
                    patternstudio_engine::trace_file_smart(&img_path, family_clone.as_deref())
                        .map(|(tr, d)| (tr, Some(d)))
                } else {
                    patternstudio_engine::trace_file_once(&img_path, &params_clone)
                        .map(|tr| (tr, None))
                }
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
            let elapsed = start.elapsed().as_millis() as u64;
            match result {
                Ok((tr, decision)) => {
                    success += 1;
                    // Smart mode: report which family won and with which
                    // params, so the client can cache the result under those
                    // params and show the recommendation.
                    let smart_info = decision.as_ref().map(|d| {
                        serde_json::json!({
                            "family": d.family,
                            "reasonKey": d.reason_key,
                            "params": d.params,
                        })
                    });
                    let _ = tx
                        .send(
                            Event::default().event("progress").data(
                                serde_json::json!({
                                    "index": i, "total": total, "file": file,
                                    "status": "done", "svg": tr.svg,
                                    "width": tr.width, "height": tr.height,
                                    "shape_count": tr.shape_count, "layers": tr.layers,
                                    "strategy_info": tr.strategy_info,
                                    "smart": smart_info,
                                    "elapsed_ms": elapsed
                                })
                                .to_string(),
                            ),
                        )
                        .await;
                }
                Err(e) => {
                    failed += 1;
                    let _ = tx
                        .send(
                            Event::default().event("progress").data(
                                serde_json::json!({
                                    "index": i, "total": total, "file": file,
                                    "status": "error", "error": e, "elapsed_ms": elapsed
                                })
                                .to_string(),
                            ),
                        )
                        .await;
                }
            }
            if tx.is_closed() {
                break;
            }
        }
        if !cancelled.load(Ordering::SeqCst) {
            let _ = tx
                .send(
                    Event::default().event("complete").data(
                        serde_json::json!({
                            "total": total, "success": success, "failed": failed,
                            "total_elapsed_ms": start.elapsed().as_millis() as u64
                        })
                        .to_string(),
                    ),
                )
                .await;
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
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, (StatusCode, String)>
{
    let rx = state
        .jobs
        .lock()
        .unwrap()
        .remove(&job_id)
        .and_then(|opt| opt)
        .ok_or((
            StatusCode::NOT_FOUND,
            "job not found or already consumed".into(),
        ))?;
    let stream = ReceiverStream::new(rx).map(Ok);
    Ok(Sse::new(stream))
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8000);
    // Loopback by default: the API can read local files, so it must not be
    // reachable from the network unless explicitly asked for (HOST=0.0.0.0).
    let host = std::env::var("HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let local_only = matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1");
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
        active_traces: Arc::new(ActiveTraces::new()),
        local_only,
        projects_file,
        sessions: Arc::new(Mutex::new(SessionCache::new(MAX_SESSIONS))),
    };
    let app = Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/uploads", post(upload))
        .route("/api/v1/trace", post(trace))
        .route("/api/v1/smart", post(smart))
        .route("/api/v1/preprocess", post(preprocess))
        .route("/api/v1/assets/{asset_id}", get(serve_asset))
        .route("/api/v1/pattern", post(generate_pattern))
        .route("/api/v1/patterns", get(list_patterns))
        .route("/api/v1/projects", post(create_project).get(list_projects))
        .route(
            "/api/v1/projects/{id}",
            get(get_project).delete(delete_project),
        )
        .route("/api/v1/projects/{id}/images/{filename}", get(serve_image))
        .route("/api/v1/projects/{id}/save-image", post(save_image))
        .route("/api/v1/projects/{id}/batch-trace", post(start_batch_trace))
        .route(
            "/api/v1/projects/{id}/batch-trace/{job_id}/events",
            get(batch_trace_events),
        )
        .route(
            "/api/v1/projects/{id}/batch-trace/{job_id}/cancel",
            post(cancel_batch_trace),
        )
        .fallback_service(ServeDir::new("web/dist").append_index_html_on_directories(true))
        .layer(CorsLayer::permissive())
        .with_state(state);
    let addr = format!("{host}:{port}");
    println!("PatternStudio API server listening on http://{addr}");
    if !local_only {
        println!(
            "WARNING: bound to a non-loopback address — file_path sources are disabled \
             and project folders are restricted to the home directory"
        );
    }
    if !PathBuf::from("web/dist/index.html").exists() {
        println!(
            "Note: web/dist not found - build the UI first: cd web && npm install && npm run build"
        );
    }
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
