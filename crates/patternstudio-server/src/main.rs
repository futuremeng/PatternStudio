//! PatternStudio API server: axum-based REST API.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::{Multipart, Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::stream::{Stream, StreamExt};
use patternstudio_core::smart::SmartDecision;
use patternstudio_core::{generated_dir, key_for_source, LineageEntry, LineageFile, SessionCache};
use patternstudio_engine::{
    build_vtracer_config, layer_doc, lock_session, trace_with_session, vectorize_preview,
    vectorize_signature, vectorize_with_session, ActiveTraces, DocCache, EngineError, TraceParams,
    TraceResult, VectorizeResult,
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

/// How many vectorized documents to keep around for instant layering re-runs.
/// Each entry holds the full shape geometry of one (image, vectorize-params)
/// combination, so layering can re-run without re-vectorizing.
const MAX_DOCS: usize = 8;

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
    /// Vectorized documents keyed by `image_key::vectorize_signature`. The
    /// layering stage re-runs against a cached doc without re-vectorizing.
    docs: Arc<Mutex<DocCache>>,
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
    /// Smart mode: also trial the opt-in extended candidate set (cutout /
    /// palette / fine-watershed trials). Default false. Only meaningful in
    /// auto-family mode; a family-scoped search varies parameters inside the
    /// chosen preset and ignores this.
    #[serde(default)]
    extended: Option<bool>,
    /// Client-generated id for this request; identifies the in-flight trace
    /// so a newer request can supersede it.
    #[serde(default)]
    request_id: Option<String>,
    /// "interactive" (default) preempts any in-flight trace of the same image;
    /// "background" (pre-warm) yields to it and is rejected with 409 busy.
    #[serde(default)]
    priority: Option<String>,
}

/// Cache key for the sector session of a symmetry plan: the source key plus
/// the fold's identifying geometry, so /trace and /layer share one entry.
fn sector_key(source_key: &str, plan: &patternstudio_core::assembly::AssemblyPlan) -> String {
    format!(
        "{source_key}::sector::{}:{}:{:.1},{:.1},{:.1}",
        plan.n, plan.mirror, plan.base_deg, plan.center.0, plan.center.1
    )
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
        // Fall back to the auto-generated iterations subfolder (tracing an
        // unconfirmed derived image opened from the lineage tree), mirroring
        // the serve_image handler.
        let p = if p.exists() {
            p
        } else {
            let gen = generated_dir(&project.path).join(name);
            if gen.exists() {
                gen
            } else {
                return Err((StatusCode::NOT_FOUND, "image not found".into()));
            }
        };
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
    let img_path = resolve_source_path(&state, &req).map_err(|(s, e)| {
        (
            s,
            Json(serde_json::json!({ "error": e, "code": "invalid_input" })),
        )
    })?;
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
                Json(serde_json::json!({ "busy": true, "code": "busy" })),
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
            cache
                .get_or_create(&key, || {
                    load_image(&path).map_err(|e| format!("failed to load image: {e}"))
                })
                .map_err(EngineError::InvalidInput)?
        };
        // A cancelled predecessor may still be finishing its current batch;
        // block until it drops the lock (cancellation is near-instant).
        // QWP7 follow-up: symmetry-assembly trace. When asked for "auto",
        // probe the source for an exact fold; on a hit trace only the
        // fundamental sector (cached as its own session) and assemble the
        // artwork from `<use>` copies. No symmetry found → plain path below.
        if params.symmetry.as_deref() == Some("auto") {
            let full = lock_session(&session);
            let plan = patternstudio_core::assembly::plan_for(full.image());
            if let Some(plan) = plan {
                let sector = patternstudio_core::assembly::sector_image(full.image(), &plan);
                drop(full);
                let skey = sector_key(&key, &plan);
                let sector_session = {
                    let mut cache = sessions.lock().unwrap();
                    cache
                        .get_or_create(&skey, move || Ok::<_, String>(sector))
                        .map_err(EngineError::InvalidInput)?
                };
                let mut sguard = lock_session(&sector_session);
                let doc = vectorize_with_session(&mut sguard, &config, &cancel)?;
                return patternstudio_engine::layer_doc_symmetric(&doc, &params, &plan, &cancel);
            }
        }
        let mut guard = lock_session(&session);
        trace_with_session(&mut guard, &config, &params, &cancel)
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string(), "code": "internal" })),
        )
    })?;

    // Drop our registration only if we are still the current trace for this image.
    state.active_traces.finish(&image_key, &request_id);

    match result {
        Ok(r) => Ok(Json(r)),
        Err(EngineError::Cancelled) => Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "cancelled": true, "code": "cancelled" })),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string(), "code": e.code() })),
        )),
    }
}

/// Vectorization stage only: image → VectorDoc. The document is cached under
/// `image_key::vectorize_signature` so the layering stage can re-run without
/// re-vectorizing. Returns the raw document as a single-layer SVG.
async fn vectorize(
    State(state): State<AppState>,
    Json(req): Json<TraceRequest>,
) -> Result<Json<VectorizeResult>, (StatusCode, Json<serde_json::Value>)> {
    let img_path = resolve_source_path(&state, &req).map_err(|(s, e)| {
        (
            s,
            Json(serde_json::json!({ "error": e, "code": "invalid_input" })),
        )
    })?;
    let params = req.params.unwrap_or_default();
    let request_id = req
        .request_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let is_background = req.priority.as_deref() == Some("background");
    let image_key = img_path.to_string_lossy().to_string();
    let doc_image_key = image_key.clone();
    let sessions = state.sessions.clone();
    let docs = state.docs.clone();

    let cancel = state
        .active_traces
        .register(&image_key, &request_id, is_background)
        .map_err(|_| {
            (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "busy": true, "code": "busy" })),
            )
        })?;

    let result = tokio::task::spawn_blocking(move || {
        let config = build_vtracer_config(&params)?;
        let key = key_for_source(&img_path, &config.segment_key());
        let session = {
            let mut cache = sessions.lock().unwrap();
            let path = img_path.clone();
            cache
                .get_or_create(&key, || {
                    load_image(&path).map_err(|e| format!("failed to load image: {e}"))
                })
                .map_err(EngineError::InvalidInput)?
        };
        let mut guard = lock_session(&session);
        let doc = vectorize_with_session(&mut guard, &config, &cancel)?;
        let doc_key = format!(
            "{}::{}",
            doc_image_key,
            vectorize_signature(&params.split().0)
        );
        docs.lock().unwrap().insert(doc_key, doc.clone());
        Ok::<VectorizeResult, EngineError>(vectorize_preview(&doc, params.line_art))
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string(), "code": "internal" })),
        )
    })?;

    state.active_traces.finish(&image_key, &request_id);

    match result {
        Ok(r) => Ok(Json(r)),
        Err(EngineError::Cancelled) => Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "cancelled": true, "code": "cancelled" })),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string(), "code": e.code() })),
        )),
    }
}

/// Layering stage only: cached VectorDoc → layered SVG. Falls back to
/// vectorizing (and caching the doc) when the doc cache has no entry, so the
/// endpoint is self-contained after a restart or cache eviction.
async fn layer(
    State(state): State<AppState>,
    Json(req): Json<TraceRequest>,
) -> Result<Json<TraceResult>, (StatusCode, Json<serde_json::Value>)> {
    let img_path = resolve_source_path(&state, &req).map_err(|(s, e)| {
        (
            s,
            Json(serde_json::json!({ "error": e, "code": "invalid_input" })),
        )
    })?;
    let params = req.params.unwrap_or_default();
    let request_id = req
        .request_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let is_background = req.priority.as_deref() == Some("background");
    let image_key = img_path.to_string_lossy().to_string();
    let doc_image_key = image_key.clone();
    let sessions = state.sessions.clone();
    let docs = state.docs.clone();

    let cancel = state
        .active_traces
        .register(&image_key, &request_id, is_background)
        .map_err(|_| {
            (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "busy": true, "code": "busy" })),
            )
        })?;

    let result = tokio::task::spawn_blocking(move || {
        let (v, _l) = params.split();
        // QWP7 follow-up: symmetry-assembly layering. The layer stage is the
        // editor's trace path, so "auto" is resolved here as well: probe the
        // source, and on a hit layer/assemble the SECTOR doc (cached under
        // the sector-session key) instead of the plain one.
        if params.symmetry.as_deref() == Some("auto") {
            let config = build_vtracer_config(&params)?;
            let key = key_for_source(&img_path, &config.segment_key());
            let full = {
                let mut cache = sessions.lock().unwrap();
                let path = img_path.clone();
                cache
                    .get_or_create(&key, || {
                        load_image(&path).map_err(|e| format!("failed to load image: {e}"))
                    })
                    .map_err(EngineError::InvalidInput)?
            };
            let full_guard = lock_session(&full);
            if let Some(plan) = patternstudio_core::assembly::plan_for(full_guard.image()) {
                let sector = patternstudio_core::assembly::sector_image(full_guard.image(), &plan);
                drop(full_guard);
                let skey = sector_key(&key, &plan);
                let sector_session = {
                    let mut cache = sessions.lock().unwrap();
                    cache
                        .get_or_create(&skey, move || Ok::<_, String>(sector))
                        .map_err(EngineError::InvalidInput)?
                };
                let sdoc_key = format!("{skey}::{}", vectorize_signature(&v));
                let sdoc = {
                    let mut cache = docs.lock().unwrap();
                    cache.get(&sdoc_key)
                };
                let sdoc = match sdoc {
                    Some(doc) => doc,
                    None => {
                        let mut sguard = lock_session(&sector_session);
                        let doc = vectorize_with_session(&mut sguard, &config, &cancel)?;
                        docs.lock().unwrap().insert(sdoc_key, doc.clone());
                        Arc::new(doc)
                    }
                };
                return patternstudio_engine::layer_doc_symmetric(&sdoc, &params, &plan, &cancel);
            }
        }
        let doc_key = format!("{}::{}", doc_image_key, vectorize_signature(&v));
        let cached = {
            let mut cache = docs.lock().unwrap();
            cache.get(&doc_key)
        };
        let doc = match cached {
            Some(doc) => doc,
            None => {
                // Cache miss: vectorize first (the session cache keeps this
                // cheap when the segmentation parameters are unchanged).
                let config = build_vtracer_config(&params)?;
                let key = key_for_source(&img_path, &config.segment_key());
                let session = {
                    let mut cache = sessions.lock().unwrap();
                    let path = img_path.clone();
                    cache
                        .get_or_create(&key, || {
                            load_image(&path).map_err(|e| format!("failed to load image: {e}"))
                        })
                        .map_err(EngineError::InvalidInput)?
                };
                let mut guard = lock_session(&session);
                let doc = vectorize_with_session(&mut guard, &config, &cancel)?;
                docs.lock().unwrap().insert(doc_key, doc.clone());
                Arc::new(doc)
            }
        };
        layer_doc(&doc, &params, &cancel)
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string(), "code": "internal" })),
        )
    })?;

    state.active_traces.finish(&image_key, &request_id);

    match result {
        Ok(r) => Ok(Json(r)),
        Err(EngineError::Cancelled) => Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "cancelled": true, "code": "cancelled" })),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string(), "code": e.code() })),
        )),
    }
}

/// Cancel any in-flight trace for the identified image (e.g. the editor
/// switched to a different image and the old work is no longer wanted). The
/// trace aborts at its next batch boundary.
async fn cancel_trace(
    State(state): State<AppState>,
    Json(req): Json<TraceRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let img_path = resolve_source_path(&state, &req)?;
    let image_key = img_path.to_string_lossy().to_string();
    let cancelled = state.active_traces.cancel(&image_key);
    Ok(Json(serde_json::json!({ "cancelled": cancelled })))
}

/// Smart Preset: analyze the image and trial-trace candidate parameter sets,
/// returning the best one plus the full candidate report.
async fn smart(
    State(state): State<AppState>,
    Json(req): Json<TraceRequest>,
) -> Result<Json<SmartDecision>, (StatusCode, Json<serde_json::Value>)> {
    let img_path = resolve_source_path(&state, &req).map_err(|(s, e)| {
        (
            s,
            Json(serde_json::json!({ "error": e, "code": "invalid_input" })),
        )
    })?;
    let family = req.family;
    let extended = req.extended.unwrap_or(false);
    let result = tokio::task::spawn_blocking(move || {
        let total_t = Instant::now();
        let mut image = load_image(&img_path)
            .map_err(|e| EngineError::InvalidInput(format!("failed to load image: {e}")))?;
        // Analyze once up front: the same stats drive both the preprocess gate
        // and (when no preprocess runs) the recommendation itself, so the
        // common clean-source path never pays for a second analysis pass.
        let ta = Instant::now();
        let stats = patternstudio_core::smart::analyze_image(&image);
        let analyze_ms = ta.elapsed().as_millis() as u64;
        // Automatic enhancement: noisy / unevenly-lit stroke sources (paper
        // scans) analyze and trace better after the Real-ESRGAN pass. Any
        // failure (missing model, decode) silently keeps the original image
        // so smart never regresses because of this step.
        let mut preprocessed = false;
        let mut preprocess_ms = None;
        if patternstudio_core::smart::needs_preprocess(&stats) {
            let tp = Instant::now();
            if let Ok(out) = patternstudio_tracer::preprocess_image(&img_path, None) {
                if let Ok(enhanced) = load_image(&out.path) {
                    image = enhanced;
                    preprocessed = true;
                    preprocess_ms = Some(tp.elapsed().as_millis() as u64);
                }
            }
        }
        // Family-scoped when the client picked a preset, auto otherwise.
        // After preprocessing the image changed, so the recommendation
        // re-analyzes; otherwise it reuses the stats measured above.
        // `extended` adds the opt-in QWP3 color-graph trials (auto mode only).
        let mut decision = match (family.as_deref(), preprocessed) {
            (Some(f), true) => patternstudio_core::smart::recommend_with_opts(
                &image,
                &patternstudio_core::smart::SmartOpts {
                    family: Some(f),
                    extended,
                    stats: None,
                },
            ),
            (None, true) => patternstudio_core::smart::recommend_with_opts(
                &image,
                &patternstudio_core::smart::SmartOpts {
                    family: None,
                    extended,
                    stats: None,
                },
            ),
            (Some(f), false) => patternstudio_core::smart::recommend_with_opts(
                &image,
                &patternstudio_core::smart::SmartOpts {
                    family: Some(f),
                    extended,
                    stats: Some(&stats),
                },
            ),
            (None, false) => patternstudio_core::smart::recommend_with_opts(
                &image,
                &patternstudio_core::smart::SmartOpts {
                    family: None,
                    extended,
                    stats: Some(&stats),
                },
            ),
        }
        .map_err(|e| EngineError::Internal(format!("smart analysis failed: {e}")))?;
        decision.preprocessed = preprocessed;
        // The server sees the whole call (load + gate analyze + trials), so
        // report the true totals from here.
        decision.elapsed_ms = total_t.elapsed().as_millis() as u64;
        decision.elapsed_breakdown.preprocess_ms = preprocess_ms;
        if !preprocessed {
            decision.elapsed_breakdown.analyze_ms = analyze_ms;
        }
        Ok::<SmartDecision, EngineError>(decision)
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string(), "code": "internal" })),
        )
    })?
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string(), "code": e.code() })),
        )
    })?;
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
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let img_path = resolve_source_path(&state, &req).map_err(|(s, e)| {
        (
            s,
            Json(serde_json::json!({ "error": e, "code": "invalid_input" })),
        )
    })?;
    let out = tokio::task::spawn_blocking(move || {
        patternstudio_tracer::preprocess_image(&img_path, None)
            .map_err(EngineError::from_preprocess_error)
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string(), "code": "internal" })),
        )
    })?
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string(), "code": e.code() })),
        )
    })?;
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

/// Redraw request: reuses the trace source-identifier fields (asset_id /
/// project_id+filename / file_path) plus the Qwen-Image prompt.
#[derive(Deserialize)]
struct RedrawRequest {
    #[serde(default)]
    asset_id: Option<String>,
    #[serde(default)]
    file_path: Option<String>,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    filename: Option<String>,
    prompt: String,
    #[serde(default)]
    seed: Option<u64>,
}

/// Redraw: restyle the source image with the Qwen-Image edit model using a
/// prompt. The result is registered as an asset so the client can display it
/// and save it via /projects/{id}/save-image. Each call is a fresh
/// generation (no caching).
async fn redraw(
    State(state): State<AppState>,
    Json(req): Json<RedrawRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let source = TraceRequest {
        asset_id: req.asset_id,
        file_path: req.file_path,
        project_id: req.project_id,
        filename: req.filename,
        params: None,
        family: None,
        extended: None,
        request_id: None,
        priority: None,
    };
    let img_path = resolve_source_path(&state, &source)?;
    let prompt = req.prompt;
    let seed = req.seed;
    let out = tokio::task::spawn_blocking(move || {
        patternstudio_tracer::redraw_image(&img_path, &prompt, seed)
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
        "prompt": out.prompt,
        "seed": out.seed,
    })))
}

/// Current Qwen-Image (redraw) settings, with the API key masked.
async fn get_settings() -> Json<serde_json::Value> {
    let s = patternstudio_tracer::load_qwen_settings();
    Json(serde_json::json!({
        "qwen": patternstudio_tracer::QwenSettingsView::from_settings(&s),
    }))
}

/// Update the Qwen-Image (redraw) settings. Blank `provider`/`apiBase`/
/// `model` clear the field (per-provider default applies); a blank
/// `apiKey` keeps the current key.
#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct SettingsRequest {
    qwen: patternstudio_tracer::QwenSettings,
}

async fn save_settings(
    Json(req): Json<SettingsRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut s = patternstudio_tracer::load_qwen_settings();
    patternstudio_tracer::apply_qwen_edit(&mut s, &req.qwen);
    patternstudio_tracer::save_qwen_settings(&s)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(serde_json::json!({
        "qwen": patternstudio_tracer::QwenSettingsView::from_settings(&s),
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
    /// Lineage entries (derived images) for the project's list/tree view.
    lineage: Vec<LineageView>,
}

/// One lineage entry as returned to the frontend: `file` is the map key
/// (the generated filename, or the top-level filename for legacy inferred
/// entries) plus the entry fields.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LineageView {
    file: String,
    #[serde(flatten)]
    entry: LineageEntry,
}

/// Load the project's lineage entries (sidecar file, with legacy naming
/// inference fallback), dropping entries whose files no longer exist.
fn project_lineage(p: &Project) -> Vec<LineageView> {
    let top_level = list_image_files(&p.path);
    let file = LineageFile::load(&p.path, &top_level);
    let gen_dir = generated_dir(&p.path);
    file.entries
        .into_iter()
        .filter(|(key, entry)| {
            gen_dir.join(key).exists()
                || entry
                    .confirmed_file
                    .as_deref()
                    .map(|f| p.path.join(f).exists())
                    .unwrap_or(false)
        })
        .map(|(file, entry)| LineageView { file, entry })
        .collect()
}

fn project_to_info(p: &Project) -> ProjectInfo {
    ProjectInfo {
        id: p.id.clone(),
        name: p.name.clone(),
        path: p.path.to_string_lossy().to_string(),
        image_count: p.images.len(),
        images: p.images.clone(),
        lineage: project_lineage(p),
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
    // Fall back to the auto-generated iterations subfolder (list-view
    // thumbnails of unconfirmed derived images).
    let img_path = if img_path.exists() {
        img_path
    } else {
        let gen = generated_dir(&project.path).join(&filename);
        if gen.exists() {
            gen
        } else {
            return Err((StatusCode::NOT_FOUND, "image not found".into()));
        }
    };
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
    /// Lineage record to persist (provenance of the derived image).
    #[serde(default)]
    lineage: Option<LineageEntry>,
    /// Map key for the lineage entry (the generated filename when confirming
    /// a previously auto-persisted iteration). Defaults to `filename`.
    #[serde(default)]
    lineage_key: Option<String>,
    /// True: store under `.patternstudio/generated/` (auto-persisted
    /// iteration, kept out of the project image list).
    #[serde(default)]
    generated: bool,
}

/// Copy `src` onto `dst` atomically: content lands in a hidden tmp file in
/// the destination directory first, then a same-volume rename swaps it in.
/// A crash between the two leaves the old `dst` intact (plus a `.save-*.tmp`
/// the server sweeps at startup); concurrent readers never observe a
/// truncated file. An existing `dst` is overwritten.
fn atomic_copy(src: &Path, dst: &Path) -> Result<(), String> {
    let dir = dst
        .parent()
        .ok_or_else(|| "save target has no parent directory".to_string())?;
    let tmp = dir.join(format!(".save-{}.tmp", uuid::Uuid::new_v4()));
    if let Err(e) = std::fs::copy(src, &tmp) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("failed to stage copy: {e}"));
    }
    if let Err(e) = std::fs::rename(&tmp, dst) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("failed to activate copy: {e}"));
    }
    Ok(())
}

/// Remove tmp files a crashed `atomic_copy` left behind in known project
/// folders. Only the exact `.save-<uuid>.tmp` shape is touched.
fn sweep_tmp_files(projects: &HashMap<String, Project>) {
    for p in projects.values() {
        let Ok(entries) = std::fs::read_dir(&p.path) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(".save-") && name.ends_with(".tmp") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// Save a processed asset into the project folder. `generated = false`
/// (default) copies into the project root and adds the file to the project's
/// image list (the user's "save as new image" confirmation); `generated =
/// true` copies into `.patternstudio/generated/` (auto-persisted iteration).
/// A provided `lineage` entry is recorded in the project's sidecar lineage
/// file. Overwrites an existing file with the same name.
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
    let dst = if req.generated {
        let dir = generated_dir(&project.path);
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        dir.join(&req.filename)
    } else {
        project.path.join(&req.filename)
    };
    let src_path = src.clone();
    tokio::task::spawn_blocking(move || atomic_copy(&src_path, &dst))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if let Some(entry) = req.lineage {
        let key = req.lineage_key.unwrap_or_else(|| req.filename.clone());
        let folder = project.path.clone();
        tokio::task::spawn_blocking(move || {
            let top_level = list_image_files(&folder);
            let mut file = LineageFile::load(&folder, &top_level);
            file.upsert(&key, entry);
            file.save(&folder)
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    if !req.generated {
        // Add to the project's image list (deduped, sorted like list_image_files).
        let mut projects = state.projects.lock().unwrap();
        if let Some(p) = projects.get_mut(&id) {
            if !p.images.contains(&req.filename) {
                p.images.push(req.filename.clone());
                p.images.sort();
            }
        }
        save_projects(&state.projects_file, &projects);
    }
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
            .unwrap_or_else(|e| Err(EngineError::Internal(e.to_string())));
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
                            "preprocessed": d.preprocessed,
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
                                    "status": "error", "error": e.to_string(),
                                    "code": e.code(), "elapsed_ms": elapsed
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
    sweep_tmp_files(&projects);
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
        docs: Arc::new(Mutex::new(DocCache::new(MAX_DOCS))),
    };
    let app = Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/uploads", post(upload))
        .route("/api/v1/trace", post(trace))
        .route("/api/v1/vectorize", post(vectorize))
        .route("/api/v1/layer", post(layer))
        .route("/api/v1/trace/cancel", post(cancel_trace))
        .route("/api/v1/smart", post(smart))
        .route("/api/v1/preprocess", post(preprocess))
        .route("/api/v1/redraw", post(redraw))
        .route("/api/v1/settings", get(get_settings).put(save_settings))
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

#[cfg(test)]
mod tests {
    use super::atomic_copy;
    use std::fs;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ps-atomic-{}-{}", tag, std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn leftovers(dir: &std::path::Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".save-") && n.ends_with(".tmp"))
            .collect()
    }

    #[test]
    fn copy_writes_content_and_leaves_no_tmp() {
        let dir = tmp_dir("ok");
        let src = dir.join("src.bin");
        let dst = dir.join("dst.bin");
        fs::write(&src, b"new content").unwrap();
        atomic_copy(&src, &dst).unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"new content");
        assert!(
            leftovers(&dir).is_empty(),
            "tmp leftovers: {:?}",
            leftovers(&dir)
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_overwrites_existing_atomically() {
        let dir = tmp_dir("overwrite");
        let src = dir.join("src.bin");
        let dst = dir.join("dst.bin");
        fs::write(&src, b"v2").unwrap();
        fs::write(&dst, b"v1").unwrap();
        atomic_copy(&src, &dst).unwrap();
        // A rename swap means every reader sees exactly one whole value.
        for _ in 0..50 {
            assert_eq!(fs::read(&dst).unwrap(), b"v2");
        }
        assert!(leftovers(&dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_copy_preserves_destination_and_cleans_tmp() {
        let dir = tmp_dir("fail");
        let src = dir.join("missing.bin"); // does not exist -> copy fails
        let dst = dir.join("dst.bin");
        fs::write(&dst, b"original").unwrap();
        assert!(atomic_copy(&src, &dst).is_err());
        assert_eq!(fs::read(&dst).unwrap(), b"original");
        assert!(
            leftovers(&dir).is_empty(),
            "tmp leftovers: {:?}",
            leftovers(&dir)
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
