//! PatternStudio API server: axum-based REST API.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::{Multipart, Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::stream::{Stream, StreamExt};
use patternstudio_core::ir::LayeredDoc;
use patternstudio_core::LayerAssignment;
use patternstudio_svg::LayeredWriter;
use patternstudio_tracer::load_image;
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use vtracer::config::{Clustering, FitMode, Hierarchical, Preset};

#[derive(Clone)]
struct AppState {
    upload_dir: PathBuf,
    assets: Arc<Mutex<HashMap<String, PathBuf>>>,
    projects: Arc<Mutex<HashMap<String, Project>>>,
    jobs: Arc<Mutex<HashMap<String, Option<tokio::sync::mpsc::Receiver<Event>>>>>,
    projects_file: PathBuf,
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
    layers: usize,
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
}

fn build_vtracer_config(params: &TraceParams) -> vtracer::Config {
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
    config
}

fn do_trace(img_path: &Path, params: &TraceParams) -> Result<TraceResult, String> {
    let img = load_image(img_path).map_err(|e| format!("failed to load image: {e}"))?;
    let vtracer_config = build_vtracer_config(params);
    let pipeline = vtracer_config.build().map_err(|e| format!("failed to build pipeline: {e}"))?;
    let doc = pipeline.run(&img).map_err(|e| format!("trace failed: {e}"))?;
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
    let layers: Vec<LayerInfo> = layered.layers.iter().enumerate()
        .map(|(i, layer)| LayerInfo {
            name: layer.name.clone(),
            role: layer.role.to_string(),
            shape_count: layer.shapes.len(),
            dominant_color: LayeredWriter::layer_dominant_color(layer),
            preview_svg: LayeredWriter::layer_preview_svg(&layered, i),
        }).collect();
    Ok(TraceResult {
        svg, width: layered.width, height: layered.height,
        shape_count: layered.total_shapes(), layer_count: layered.layers.len(), layers,
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
            std::fs::write(&path, &content).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
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
    params: Option<TraceParams>,
}

async fn trace(
    State(state): State<AppState>,
    Json(req): Json<TraceRequest>,
) -> Result<Json<TraceResult>, (StatusCode, String)> {
    let img_path = if let Some(ref asset_id) = req.asset_id {
        state.assets.lock().unwrap().get(asset_id).cloned()
            .ok_or((StatusCode::NOT_FOUND, format!("asset {asset_id} not found")))?
    } else if let Some(ref fp) = req.file_path {
        PathBuf::from(fp)
    } else {
        return Err((StatusCode::BAD_REQUEST, "asset_id or file_path required".into()));
    };
    let params = req.params.unwrap_or_default();
    let result = do_trace(&img_path, &params).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
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
    let svg = patternstudio_patterns::generate_sized(&req.kind, req.n_units, req.complexity, req.size)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
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
    let data = std::fs::read(&img_path)
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
    let files = project.images.clone();
    let folder = project.path.clone();
    let params = req.params.unwrap_or_default();

    tokio::spawn(async move {
        let start = Instant::now();
        let total = files.len();
        let _ = tx.send(Event::default().event("start")
            .data(serde_json::json!({ "total": total }).to_string())).await;
        let mut success = 0usize;
        let mut failed = 0usize;
        for (i, file) in files.iter().enumerate() {
            let elapsed = start.elapsed().as_millis() as u64;
            let _ = tx.send(Event::default().event("progress")
                .data(serde_json::json!({
                    "index": i, "total": total, "file": file,
                    "status": "processing", "elapsed_ms": elapsed
                }).to_string())).await;
            let img_path = folder.join(file);
            let params_clone = params.clone();
            let result = tokio::task::spawn_blocking(move || do_trace(&img_path, &params_clone))
                .await.unwrap_or_else(|e| Err(e.to_string()));
            let elapsed = start.elapsed().as_millis() as u64;
            match result {
                Ok(tr) => {
                    success += 1;
                    let _ = tx.send(Event::default().event("progress")
                        .data(serde_json::json!({
                            "index": i, "total": total, "file": file,
                            "status": "done", "svg": tr.svg,
                            "width": tr.width, "height": tr.height,
                            "shape_count": tr.shape_count, "layers": tr.layers,
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
        let _ = tx.send(Event::default().event("complete")
            .data(serde_json::json!({
                "total": total, "success": success, "failed": failed,
                "total_elapsed_ms": start.elapsed().as_millis() as u64
            }).to_string())).await;
    });

    Ok(Json(serde_json::json!({ "job_id": job_id })))
}

async fn batch_trace_events(
    State(state): State<AppState>,
    UrlPath(job_id): UrlPath<String>,
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
        projects_file,
    };
    let app = Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/uploads", post(upload))
        .route("/api/v1/trace", post(trace))
        .route("/api/v1/pattern", post(generate_pattern))
        .route("/api/v1/patterns", get(list_patterns))
        .route("/api/v1/projects", post(create_project).get(list_projects))
        .route("/api/v1/projects/{id}", get(get_project).delete(delete_project))
        .route("/api/v1/projects/{id}/images/{filename}", get(serve_image))
        .route("/api/v1/projects/{id}/batch-trace", post(start_batch_trace))
        .route("/api/v1/projects/{id}/batch-trace/{job_id}/events", get(batch_trace_events))
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