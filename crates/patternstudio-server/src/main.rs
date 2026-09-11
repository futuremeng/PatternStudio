//! PatternStudio API server: axum-based REST API.

use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::path::PathBuf;

use axum::{
    extract::{Multipart, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use patternstudio_core::{LayerAssignment, PatternStudioConfig, PatternStudioPipeline};
use patternstudio_svg::LayeredWriter;
use patternstudio_tracer::load_image;
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;
use vtracer::config::Preset;

#[derive(Clone)]
struct AppState {
    upload_dir: PathBuf,
    assets: Arc<Mutex<HashMap<String, PathBuf>>>,
}

#[derive(Deserialize)]
struct TraceRequest {
    #[serde(default)]
    asset_id: Option<String>,
    #[serde(default)]
    file_path: Option<String>,
    #[serde(default = "default_medium")]
    resolution: String,
    #[serde(default = "default_low")]
    simplify: String,
    #[serde(default = "default_poster")]
    preset: String,
}

fn default_medium() -> String { "medium".into() }
fn default_low() -> String { "low".into() }
fn default_poster() -> String { "poster".into() }

#[derive(Deserialize)]
struct PatternRequest {
    kind: String,
    #[serde(default = "default_n_units")]
    n_units: u32,
    #[serde(default = "default_complexity")]
    complexity: u8,
}

fn default_n_units() -> u32 { 8 }
fn default_complexity() -> u8 { 50 }

#[derive(Serialize)]
struct TraceResponse {
    svg: String,
    width: u32,
    height: u32,
    shape_count: usize,
    layer_count: usize,
}

#[derive(Serialize)]
struct PatternResponse {
    svg: String,
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

async fn trace(
    State(state): State<AppState>,
    Json(req): Json<TraceRequest>,
) -> Result<Json<TraceResponse>, (StatusCode, String)> {
    let img_path = if let Some(ref asset_id) = req.asset_id {
        state.assets.lock().unwrap().get(asset_id).cloned()
            .ok_or((StatusCode::NOT_FOUND, format!("asset {asset_id} not found")))?
    } else if let Some(ref path) = req.file_path {
        PathBuf::from(path)
    } else {
        return Err((StatusCode::BAD_REQUEST, "asset_id or file_path required".into()));
    };

    let img = load_image(&img_path)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("failed to load: {e}")))?;

    let preset = match req.preset.as_str() {
        "bw" => Preset::Bw,
        "photo" => Preset::Photo,
        _ => Preset::Poster,
    };
    let config = PatternStudioConfig {
        resolution: match req.resolution.as_str() {
            "coarse" => patternstudio_core::Resolution::Coarse,
            "fine" => patternstudio_core::Resolution::Fine,
            _ => patternstudio_core::Resolution::Medium,
        },
        simplify: match req.simplify.as_str() {
            "minimal" => patternstudio_core::Simplify::Minimal,
            "medium" => patternstudio_core::Simplify::Medium,
            "high" => patternstudio_core::Simplify::High,
            "maximum" => patternstudio_core::Simplify::Maximum,
            _ => patternstudio_core::Simplify::Low,
        },
        vtracer: vtracer::Config::from_preset(preset),
    };

    let pipeline = PatternStudioPipeline::new(&config)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let layered = pipeline.trace_layered(&img, &LayerAssignment::single_layer())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let svg = LayeredWriter::write(&layered);
    Ok(Json(TraceResponse {
        svg,
        width: layered.width,
        height: layered.height,
        shape_count: layered.total_shapes(),
        layer_count: layered.layers.len(),
    }))
}

async fn generate_pattern(
    Json(req): Json<PatternRequest>,
) -> Result<Json<PatternResponse>, (StatusCode, String)> {
    let svg = patternstudio_patterns::generate(&req.kind, req.n_units, req.complexity)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(PatternResponse { svg }))
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8000);
    let upload_dir = PathBuf::from("/tmp/patternstudio-uploads");
    std::fs::create_dir_all(&upload_dir).ok();

    let state = AppState { upload_dir, assets: Arc::new(Mutex::new(HashMap::new())) };

    let app = Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/uploads", post(upload))
        .route("/api/v1/trace", post(trace))
        .route("/api/v1/pattern", post(generate_pattern))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = format!("0.0.0.0:{port}");
    println!("PatternStudio API server listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}