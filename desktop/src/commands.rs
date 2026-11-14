//! Tauri commands: in-process access to the tracing engine.

use patternstudio_core::{key_for_source, LineageEntry, LineageFile, SessionCache};
use patternstudio_engine::{
    build_vtracer_config, layer_doc, lock_session, trace_bytes_once, trace_bytes_smart,
    trace_with_session, vectorize_preview, vectorize_signature, vectorize_with_session,
    ActiveTraces, DocCache, EngineError, TraceParams, TraceResult, VectorizeResult,
};
use patternstudio_patterns::{generate_sized, list_kinds};
use patternstudio_tracer::load_image;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{Emitter, Manager};

/// Smart Preset: analyze the image and trial-trace candidate parameter sets,
/// returning the best one plus the full candidate report.
#[tauri::command]
pub async fn smart_file(
    path: String,
    family: Option<String>,
) -> Result<patternstudio_core::smart::SmartDecision, EngineError> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = Path::new(&path);
        if !p.exists() {
            return Err(EngineError::Io(format!("image not found: {path}")));
        }
        let image =
            load_image(p).map_err(|e| EngineError::Io(format!("failed to load image: {e}")))?;
        // Family-scoped when the client picked a preset, auto otherwise.
        let decision = match family.as_deref() {
            Some(f) => patternstudio_core::smart::recommend_for(&image, f),
            None => patternstudio_core::smart::recommend(&image),
        };
        decision.map_err(|e| EngineError::Internal(format!("smart analysis failed: {e}")))
    })
    .await
    .map_err(|e| EngineError::Internal(e.to_string()))?
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatternInfo {
    pub kind: &'static str,
    pub description: &'static str,
}

#[tauri::command]
pub async fn trace_bytes(data: Vec<u8>, params: TraceParams) -> Result<TraceResult, EngineError> {
    tauri::async_runtime::spawn_blocking(move || trace_bytes_once(&data, &params))
        .await
        .map_err(|e| EngineError::Internal(e.to_string()))?
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

/// How many vectorized documents to keep around for instant layering re-runs.
const MAX_DOCS: usize = 8;

/// Process-wide LRU cache of vectorized documents, keyed by
/// `file_path::vectorize_signature`. The layering stage re-runs against a
/// cached doc without re-vectorizing.
fn doc_cache() -> &'static Mutex<DocCache> {
    static DOCS: OnceLock<Mutex<DocCache>> = OnceLock::new();
    DOCS.get_or_init(|| Mutex::new(DocCache::new(MAX_DOCS)))
}

/// Process-wide in-flight trace registry (preemption + busy detection).
fn active_traces() -> &'static ActiveTraces {
    static ACTIVE: OnceLock<ActiveTraces> = OnceLock::new();
    ACTIVE.get_or_init(ActiveTraces::new)
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
) -> Result<TraceResult, EngineError> {
    let request_id = request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let is_background = priority.as_deref() == Some("background");
    let image_key = file_path.clone();

    // Preemption: an interactive request cancels whatever is in flight for
    // this image (it aborts between pipeline batches); a background request
    // (pre-warm) yields instead and is rejected as busy.
    let cancel = active_traces().register(&image_key, &request_id, is_background)?;

    let result = tauri::async_runtime::spawn_blocking(move || {
        let path = Path::new(&file_path);
        if !path.exists() {
            return Err(EngineError::Io(format!("image not found: {file_path}")));
        }
        // Build the config up front so the session cache can be keyed by the
        // segmentation parameters (not just the file): each preset keeps its own
        // cached segmentation instead of evicting the previous one.
        let config = build_vtracer_config(&params)?;
        let key = key_for_source(path, &config.segment_key());
        let session = {
            let mut cache = session_cache().lock().unwrap();
            let p: PathBuf = path.to_path_buf();
            cache
                .get_or_create(&key, || {
                    load_image(&p).map_err(|e| format!("failed to decode image: {e}"))
                })
                .map_err(EngineError::InvalidInput)?
        };
        // A cancelled predecessor may still be finishing its current batch;
        // block until it drops the lock (cancellation is near-instant).
        let mut guard = lock_session(&session);
        trace_with_session(&mut guard, &config, &params, &cancel)
    })
    .await
    .map_err(|e| EngineError::Internal(e.to_string()))?;

    // Drop our registration only if we are still the current trace for this image.
    active_traces().finish(&image_key, &request_id);
    result
}

/// Vectorization stage only: image → VectorDoc, cached under
/// `file_path::vectorize_signature` so layering can re-run without
/// re-vectorizing. Returns the raw document as a single-layer SVG.
#[tauri::command]
pub async fn vectorize_file(
    file_path: String,
    params: TraceParams,
    request_id: Option<String>,
    priority: Option<String>,
) -> Result<VectorizeResult, EngineError> {
    let request_id = request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let is_background = priority.as_deref() == Some("background");
    let image_key = file_path.clone();

    let cancel = active_traces().register(&image_key, &request_id, is_background)?;

    let doc_image_key = image_key.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let path = Path::new(&file_path);
        if !path.exists() {
            return Err(EngineError::Io(format!("image not found: {file_path}")));
        }
        let config = build_vtracer_config(&params)?;
        let key = key_for_source(path, &config.segment_key());
        let session = {
            let mut cache = session_cache().lock().unwrap();
            let p: PathBuf = path.to_path_buf();
            cache
                .get_or_create(&key, || {
                    load_image(&p).map_err(|e| format!("failed to decode image: {e}"))
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
        doc_cache().lock().unwrap().insert(doc_key, doc.clone());
        Ok::<VectorizeResult, EngineError>(vectorize_preview(&doc, params.line_art))
    })
    .await
    .map_err(|e| EngineError::Internal(e.to_string()))?;

    active_traces().finish(&image_key, &request_id);
    result
}

/// Layering stage only: cached VectorDoc → layered SVG. Falls back to
/// vectorizing (and caching the doc) when the doc cache has no entry.
#[tauri::command]
pub async fn layer_file(
    file_path: String,
    params: TraceParams,
    request_id: Option<String>,
    priority: Option<String>,
) -> Result<TraceResult, EngineError> {
    let request_id = request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let is_background = priority.as_deref() == Some("background");
    let image_key = file_path.clone();

    let cancel = active_traces().register(&image_key, &request_id, is_background)?;

    let doc_image_key = image_key.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let (v, _l) = params.split();
        let doc_key = format!("{}::{}", doc_image_key, vectorize_signature(&v));
        let doc = match doc_cache().lock().unwrap().get(&doc_key) {
            Some(doc) => doc,
            None => {
                let path = Path::new(&file_path);
                if !path.exists() {
                    return Err(EngineError::Io(format!("image not found: {file_path}")));
                }
                let config = build_vtracer_config(&params)?;
                let key = key_for_source(path, &config.segment_key());
                let session = {
                    let mut cache = session_cache().lock().unwrap();
                    let p: PathBuf = path.to_path_buf();
                    cache
                        .get_or_create(&key, || {
                            load_image(&p).map_err(|e| format!("failed to decode image: {e}"))
                        })
                        .map_err(EngineError::InvalidInput)?
                };
                let mut guard = lock_session(&session);
                let doc = vectorize_with_session(&mut guard, &config, &cancel)?;
                doc_cache().lock().unwrap().insert(doc_key, doc.clone());
                Arc::new(doc)
            }
        };
        layer_doc(&doc, &params, &cancel)
    })
    .await
    .map_err(|e| EngineError::Internal(e.to_string()))?;

    active_traces().finish(&image_key, &request_id);
    result
}

/// Cancel any in-flight trace for `file_path` (e.g. the editor switched to a
/// different image and the old work is no longer wanted). The trace aborts at
/// its next batch boundary. Returns true if a trace was cancelled.
#[tauri::command]
pub fn cancel_trace(file_path: String) -> bool {
    active_traces().cancel(&file_path)
}

#[tauri::command]
pub fn open_folder(title: Option<String>) -> Result<String, String> {
    let folder = rfd::FileDialog::new()
        .set_title(title.unwrap_or_else(|| "Select project folder".into()))
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

/// One lineage entry as returned to the frontend: `file` is the map key
/// (the generated filename, or the top-level filename for legacy inferred
/// entries) plus the entry fields.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineageView {
    pub file: String,
    #[serde(flatten)]
    pub entry: LineageEntry,
}

/// List the project's lineage entries (auto-generated and/or confirmed
/// derived images). Reads the sidecar file, falling back to the legacy
/// naming-convention inference for pre-existing projects. Entries whose
/// files no longer exist on disk are dropped.
#[tauri::command]
pub async fn list_lineage(folder_path: String) -> Result<Vec<LineageView>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let folder = Path::new(&folder_path);
        if !folder.is_dir() {
            return Err(format!("not a directory: {folder_path}"));
        }
        // Top-level image names, for the inference fallback and existence checks.
        let mut top_level = Vec::new();
        if let Ok(entries) = std::fs::read_dir(folder) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
                if IMAGE_EXTS.contains(&ext.as_str()) {
                    top_level.push(name);
                }
            }
        }
        let file = LineageFile::load(folder, &top_level);
        let gen_dir = patternstudio_core::generated_dir(folder);
        let views: Vec<LineageView> = file
            .entries
            .into_iter()
            .filter(|(key, entry)| {
                // Keep the entry if either of its files still exists.
                gen_dir.join(key).exists()
                    || entry
                        .confirmed_file
                        .as_deref()
                        .map(|f| folder.join(f).exists())
                        .unwrap_or(false)
            })
            .map(|(file, entry)| LineageView { file, entry })
            .collect();
        Ok(views)
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

/// Preprocess (digital-art enhancement) a local image file with the
/// Real-ESRGAN anime model. Returns the path of the enhanced image
/// (same size as the input) plus timing info. Results are cached on disk.
#[tauri::command]
pub async fn preprocess_file(
    app: tauri::AppHandle,
    file_path: String,
) -> Result<serde_json::Value, EngineError> {
    let resource_dir = app.path().resource_dir().ok();
    let out = tauri::async_runtime::spawn_blocking(move || {
        patternstudio_tracer::preprocess_image(
            std::path::Path::new(&file_path),
            resource_dir.as_deref(),
        )
        .map_err(EngineError::from_preprocess_error)
    })
    .await
    .map_err(|e| EngineError::Internal(e.to_string()))??;
    Ok(serde_json::json!({
        "path": out.path,
        "width": out.width,
        "height": out.height,
        "elapsedMs": out.elapsed_ms,
        "cached": out.cached,
    }))
}

/// Redraw a local image file with the Qwen-Image edit model using a prompt.
/// Returns the path of the redrawn image plus timing info. Each call is a
/// fresh generation (no caching).
#[tauri::command]
pub async fn redraw_file(
    file_path: String,
    prompt: String,
    seed: Option<u64>,
) -> Result<serde_json::Value, String> {
    let out = tauri::async_runtime::spawn_blocking(move || {
        patternstudio_tracer::redraw_image(std::path::Path::new(&file_path), &prompt, seed)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(serde_json::json!({
        "path": out.path,
        "width": out.width,
        "height": out.height,
        "elapsedMs": out.elapsed_ms,
        "prompt": out.prompt,
        "seed": out.seed,
    }))
}

/// Current Qwen-Image (redraw) settings, with the API key masked.
#[tauri::command]
pub fn get_settings() -> serde_json::Value {
    let s = patternstudio_tracer::load_qwen_settings();
    serde_json::json!({
        "qwen": patternstudio_tracer::QwenSettingsView::from_settings(&s),
    })
}

/// Update the Qwen-Image (redraw) settings. Blank `provider`/`apiBase`/
/// `model` clear the field (per-provider default applies); a blank
/// `apiKey` keeps the current key.
#[tauri::command]
pub fn save_settings(
    qwen: patternstudio_tracer::QwenSettings,
) -> Result<serde_json::Value, String> {
    let mut s = patternstudio_tracer::load_qwen_settings();
    patternstudio_tracer::apply_qwen_edit(&mut s, &qwen);
    patternstudio_tracer::save_qwen_settings(&s)?;
    Ok(serde_json::json!({
        "qwen": patternstudio_tracer::QwenSettingsView::from_settings(&s),
    }))
}

/// Save a processed image into the project. Two destinations:
/// - `generated = false` (default): the project root — the user's "save as
///   new image" confirmation; the file joins the top-level image list.
/// - `generated = true`: `.patternstudio/generated/` — auto-persisted
///   iterations (enhance/redraw previews) kept out of the image list.
///
/// When `lineage` is provided it is recorded in the project's sidecar
/// lineage file under `lineage_key` (default: `filename`), so the derived
/// tree survives re-opening the folder. Overwrites an existing file with
/// the same name.
#[tauri::command]
pub async fn save_project_image(
    project_id: String,
    file_path: String,
    filename: String,
    lineage: Option<LineageEntry>,
    lineage_key: Option<String>,
    generated: Option<bool>,
) -> Result<String, String> {
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return Err("invalid filename".into());
    }
    let src = std::path::Path::new(&file_path);
    if !src.exists() {
        return Err("source file not found".into());
    }
    let folder = std::path::Path::new(&project_id);
    if !folder.is_dir() {
        return Err(format!("not a directory: {project_id}"));
    }
    let dst = if generated.unwrap_or(false) {
        let dir = patternstudio_core::generated_dir(folder);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        dir.join(&filename)
    } else {
        folder.join(&filename)
    };
    std::fs::copy(src, &dst).map_err(|e| e.to_string())?;
    if let Some(entry) = lineage {
        let key = lineage_key.unwrap_or_else(|| filename.clone());
        let mut top_level = Vec::new();
        if let Ok(entries) = std::fs::read_dir(folder) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
                if IMAGE_EXTS.contains(&ext.as_str()) {
                    top_level.push(name);
                }
            }
        }
        let mut file = LineageFile::load(folder, &top_level);
        file.upsert(&key, entry);
        file.save(folder).map_err(|e| e.to_string())?;
    }
    Ok(filename)
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

fn default_n_units() -> u32 {
    8
}
fn default_complexity() -> u8 {
    50
}
fn default_size() -> u32 {
    400
}

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
    batch_cancel_flags()
        .lock()
        .unwrap()
        .insert(job_id.clone(), cancelled.clone());
    let job_id_cleanup = job_id.clone();

    tauri::async_runtime::spawn(async move {
        let start = std::time::Instant::now();
        let total = image_files.len();

        let _ = app.emit(
            "batch-trace",
            serde_json::json!({
                "type": "start",
                "total": total,
                "files": image_files.iter().map(|f| f.0.clone()).collect::<Vec<_>>(),
            }),
        );

        let mut success = 0usize;
        let mut failed = 0usize;

        for (i, (name, file_path)) in image_files.iter().enumerate() {
            if cancelled.load(Ordering::SeqCst) {
                let _ = app.emit(
                    "batch-trace",
                    serde_json::json!({
                        "type": "cancelled",
                        "index": i,
                        "total": total,
                        "success": success,
                        "failed": failed,
                        "total_elapsed_ms": start.elapsed().as_millis() as u64,
                    }),
                );
                break;
            }
            let elapsed = start.elapsed().as_millis() as u64;
            let _ = app.emit(
                "batch-trace",
                serde_json::json!({
                    "type": "processing",
                    "index": i,
                    "total": total,
                    "file": name,
                    "elapsed_ms": elapsed,
                }),
            );

            let file_path = file_path.clone();
            let params_clone = params.clone();
            let smart_clone = smart;
            let family_clone = family.clone();
            let result = tauri::async_runtime::spawn_blocking(move || {
                let data = std::fs::read(&file_path)
                    .map_err(|e| EngineError::Io(format!("failed to read {file_path}: {e}")))?;
                if smart_clone {
                    trace_bytes_smart(&data, family_clone.as_deref()).map(|(out, d)| (out, Some(d)))
                } else {
                    trace_bytes_once(&data, &params_clone).map(|out| (out, None))
                }
            })
            .await
            .unwrap_or_else(|e| Err(EngineError::Internal(e.to_string())));

            let elapsed = start.elapsed().as_millis() as u64;
            match result {
                Ok((out, decision)) => {
                    success += 1;
                    let smart_info = decision.as_ref().map(|d| {
                        serde_json::json!({
                            "family": d.family,
                            "reasonKey": d.reason_key,
                            "params": d.params,
                        })
                    });
                    let _ = app.emit(
                        "batch-trace",
                        serde_json::json!({
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
                        }),
                    );
                }
                Err(e) => {
                    failed += 1;
                    let _ = app.emit(
                        "batch-trace",
                        serde_json::json!({
                            "type": "error",
                            "index": i,
                            "total": total,
                            "file": name,
                            "error": e.to_string(),
                            "code": e.code(),
                            "elapsed_ms": elapsed,
                        }),
                    );
                }
            }
        }

        if !cancelled.load(Ordering::SeqCst) {
            let _ = app.emit(
                "batch-trace",
                serde_json::json!({
                    "type": "complete",
                    "total": total,
                    "success": success,
                    "failed": failed,
                    "total_elapsed_ms": start.elapsed().as_millis() as u64,
                }),
            );
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

/// Save arbitrary binary bytes (e.g. a ZIP of exported layer SVGs) to a
/// user-chosen path. Returns the saved path, or an empty string if cancelled.
#[tauri::command]
pub fn save_bytes(data: Vec<u8>, suggested_name: String) -> Result<String, String> {
    let path = rfd::FileDialog::new()
        .set_file_name(&suggested_name)
        .save_file();
    if let Some(p) = path {
        std::fs::write(&p, data).map_err(|e| e.to_string())?;
        Ok(p.to_string_lossy().to_string())
    } else {
        Ok(String::new())
    }
}
