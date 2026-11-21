//! Shared trace engine: parameter mapping, vtracer config construction,
//! session-driven tracing with layering/profiling/SVG output, and preemption
//! of in-flight traces.
//!
//! Used by both the web server (`patternstudio-server`) and the desktop app
//! (`patternstudio-desktop`) so the trace pipeline has a single
//! implementation.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use patternstudio_core::ir::{LayerAssignment, LayeredDoc};
use patternstudio_core::profiles::{apply_profiles, AutoDecision, IdealParams};
use patternstudio_core::smart::{SmartDecision, SmartParams};
use patternstudio_svg::LayeredWriter;
use serde::Serialize;
use vtracer::config::{Clustering, FitMode, Hierarchical, Preset};
use vtracer::ir::VectorDoc;
use vtracer::progress::CancelToken;
use vtracer::{ColorImage, Config, Error, Session};

/// Engine error with a stable machine-readable [`code`](EngineError::code).
///
/// The serialized shape — `{ "code", "message" }` — is the wire contract for
/// both the HTTP server's JSON error bodies and the Tauri IPC rejections, so
/// the UI can classify failures (retry / show detail / stay silent) without
/// string-matching messages.
///
/// The `Display` strings are part of the compatibility contract: legacy
/// callers still match `"cancelled"` / `"busy"` exactly (server 409 mapping,
/// Tauri bridge in `api.js`), so variant messages must stay byte-identical to
/// the historical error strings.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    /// A newer request for the same image aborted this trace.
    #[error("cancelled")]
    Cancelled,
    /// A background request was rejected while an interactive trace runs.
    #[error("busy")]
    Busy,
    /// An optional external dependency is missing or unusable
    /// (e.g. the Real-ESRGAN binary or model files for preprocessing).
    #[error("{0}")]
    Dependency(String),
    /// The request source is wrong or the bytes cannot be decoded as an image.
    #[error("{0}")]
    InvalidInput(String),
    /// The source file could not be read from disk.
    #[error("{0}")]
    Io(String),
    /// Everything else: pipeline internals, analysis and config failures.
    #[error("{0}")]
    Internal(String),
}

impl EngineError {
    /// Stable machine-readable code, mirrored by the web/desktop error UI
    /// (`web/src/errorText.js`). Must never change once shipped.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Busy => "busy",
            Self::Dependency(_) => "dependency",
            Self::InvalidInput(_) => "invalid_input",
            Self::Io(_) => "io",
            Self::Internal(_) => "internal",
        }
    }

    /// Classify a `patternstudio_tracer::preprocess_image` error string: the
    /// two missing-dependency probes (`binary not found`,
    /// `models directory not found`, see tracer's `preprocess.rs`) mean the
    /// optional Real-ESRGAN install is absent; anything else is an opaque
    /// run failure.
    pub fn from_preprocess_error(msg: String) -> Self {
        if msg.contains("binary not found") || msg.contains("models directory not found") {
            Self::Dependency(msg)
        } else {
            Self::Internal(msg)
        }
    }
}

/// Compat for bodies predating the code contract that still carry plain
/// `String` errors: the two control-flow literals keep their exact meaning so
/// existing call sites compile unchanged after the signature migration.
impl From<&str> for EngineError {
    fn from(s: &str) -> Self {
        match s {
            "cancelled" => Self::Cancelled,
            "busy" => Self::Busy,
            other => Self::Internal(other.to_owned()),
        }
    }
}

impl From<String> for EngineError {
    fn from(s: String) -> Self {
        match s.as_str() {
            "cancelled" => Self::Cancelled,
            "busy" => Self::Busy,
            other => Self::Internal(other.to_owned()),
        }
    }
}

/// Wire contract: Tauri rejects the IPC promise with this JSON object; the
/// server embeds the same two fields in its JSON error bodies.
impl Serialize for EngineError {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = ser.serialize_map(Some(2))?;
        map.serialize_entry("code", self.code())?;
        map.serialize_entry("message", &self.to_string())?;
        map.end()
    }
}

/// Trace parameters accepted by the API / desktop commands.
/// All fields are optional so partial updates (e.g. a single slider change)
/// can be sent without resending the full parameter set.
#[derive(Debug, Clone, Default, serde::Deserialize, Serialize)]
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

/// Parameters that affect the vectorized document (segmentation → compose →
/// curve fit → path optimization). Changing any of these requires
/// re-vectorizing the image.
#[derive(Debug, Clone, Default, serde::Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorizeParams {
    #[serde(default = "default_preset")]
    pub preset: String,
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
    #[serde(default)]
    pub binary_adaptive: bool,
    #[serde(default)]
    pub binary_threshold: Option<u8>,
    #[serde(default)]
    pub watershed_detail: Option<u32>,
    #[serde(default)]
    pub palette: Option<String>,
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
}

/// Parameters that affect only the layering/output stage (VectorDoc → layer
/// assignment → SVG). This stage is cheap and pure: it can be re-run any
/// number of times against a cached vectorized document. New layering
/// strategies extend [`layer_doc`], never the vectorization path.
#[derive(Debug, Clone, Default, serde::Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerParams {
    #[serde(default)]
    pub line_art: bool,
    #[serde(default)]
    pub layers: usize,
    #[serde(default)]
    pub layer_strategy: Option<String>,
    #[serde(default)]
    pub complexity_threshold: Option<u8>,
}

impl TraceParams {
    /// Split into the vectorization part (affects the VectorDoc) and the
    /// layering part (affects only the layering/output stage).
    pub fn split(&self) -> (VectorizeParams, LayerParams) {
        (
            VectorizeParams {
                preset: self.preset.clone(),
                clustering: self.clustering.clone(),
                hierarchical: self.hierarchical.clone(),
                color_precision: self.color_precision,
                layer_difference: self.layer_difference,
                filter_speckle: self.filter_speckle,
                mode: self.mode.clone(),
                simplify: self.simplify,
                path_precision: self.path_precision,
                max_colors: self.max_colors,
                binary_adaptive: self.binary_adaptive,
                binary_threshold: self.binary_threshold,
                watershed_detail: self.watershed_detail,
                palette: self.palette.clone(),
                corner_threshold: self.corner_threshold,
                length_threshold: self.length_threshold,
                optimize: self.optimize,
                binary_adaptive_t: self.binary_adaptive_t,
                binary_adaptive_window: self.binary_adaptive_window,
            },
            LayerParams {
                line_art: self.line_art,
                layers: self.layers,
                layer_strategy: self.layer_strategy.clone(),
                complexity_threshold: self.complexity_threshold,
            },
        )
    }

    /// Reassemble a full param set from its two parts.
    pub fn from_parts(v: &VectorizeParams, l: &LayerParams) -> Self {
        Self {
            preset: v.preset.clone(),
            clustering: v.clustering.clone(),
            hierarchical: v.hierarchical.clone(),
            color_precision: v.color_precision,
            layer_difference: v.layer_difference,
            filter_speckle: v.filter_speckle,
            mode: v.mode.clone(),
            simplify: v.simplify,
            path_precision: v.path_precision,
            max_colors: v.max_colors,
            line_art: l.line_art,
            binary_adaptive: v.binary_adaptive,
            binary_threshold: v.binary_threshold,
            watershed_detail: v.watershed_detail,
            palette: v.palette.clone(),
            layers: l.layers,
            corner_threshold: v.corner_threshold,
            length_threshold: v.length_threshold,
            optimize: v.optimize,
            binary_adaptive_t: v.binary_adaptive_t,
            binary_adaptive_window: v.binary_adaptive_window,
            layer_strategy: l.layer_strategy.clone(),
            complexity_threshold: l.complexity_threshold,
        }
    }
}

/// Canonical signature of the vectorization parameters: the key under which
/// the vectorized document is cached, so layering re-runs can find it.
pub fn vectorize_signature(v: &VectorizeParams) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

/// Build a vtracer config from the (possibly partial) trace params.
pub fn build_vtracer_config(params: &TraceParams) -> Result<Config, EngineError> {
    let preset = match params.preset.to_ascii_lowercase().as_str() {
        "bw" => Preset::Bw,
        "photo" => Preset::Photo,
        _ => Preset::Poster,
    };
    let mut config = Config::from_preset(preset);

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
    if let Some(v) = params.color_precision {
        config.color_precision = v as i32;
    }
    if let Some(v) = params.layer_difference {
        config.layer_difference = v as i32;
    }
    if let Some(v) = params.filter_speckle {
        config.filter_speckle = v as usize;
    }
    if let Some(ref m) = params.mode {
        config.mode = match m.as_str() {
            "pixel" => FitMode::Pixel,
            "polygon" => FitMode::Polygon,
            _ => FitMode::Spline,
        };
    }
    if let Some(v) = params.simplify {
        config.simplify = Some(v);
    }
    if let Some(v) = params.path_precision {
        config.path_precision = Some(v as u32);
    }
    if let Some(v) = params.max_colors {
        config.max_colors = Some(v);
    }
    config.binary_adaptive = params.binary_adaptive;
    if let Some(v) = params.binary_threshold {
        config.binary_threshold = v;
    }
    if let Some(v) = params.watershed_detail {
        config.watershed_detail = v;
    }
    if let Some(ref p) = params.palette {
        config.palette =
            patternstudio_tracer::parse_palette(p).map_err(EngineError::InvalidInput)?;
    }
    if let Some(v) = params.corner_threshold {
        config.corner_threshold = v;
    }
    if let Some(v) = params.length_threshold {
        config.length_threshold = v;
    }
    if let Some(v) = params.optimize {
        config.optimize = v;
    }
    if let Some(v) = params.binary_adaptive_t {
        config.binary_adaptive_t = v;
    }
    if let Some(v) = params.binary_adaptive_window {
        config.binary_adaptive_window = v;
    }
    Ok(config)
}

/// One layer's metadata returned alongside the SVG.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerInfo {
    pub name: String,
    pub role: String,
    pub shape_count: usize,
    pub dominant_color: String,
    pub preview_svg: String,
    pub profile: String,
}

/// Result of a trace operation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceResult {
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

/// Result of the vectorization stage: the raw vectorized document rendered as
/// a single-layer SVG. Layering is a separate, re-runnable stage on top of
/// the same document.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorizeResult {
    pub svg: String,
    pub width: u32,
    pub height: u32,
    pub shape_count: usize,
}

/// Convert a smart-recommended parameter set into `TraceParams`.
/// Both structs use camelCase with identical field names, so a JSON round-trip
/// is a safe, field-agnostic mapping.
pub fn smart_to_trace_params(p: &SmartParams) -> Result<TraceParams, EngineError> {
    let v = serde_json::to_value(p).map_err(|e| e.to_string())?;
    serde_json::from_value(v).map_err(|e| EngineError::Internal(e.to_string()))
}

/// Acquire a cached session's lock, blocking until a predecessor releases it.
///
/// A plain [`Mutex::lock`] is the right wait here: a preempted trace was just
/// cancelled by [`ActiveTraces::register`] and drops the lock at its next
/// batch boundary, so the wait is near-instant — and blocking avoids both the
/// 20 ms polling latency and the infinite spin a `try_lock` loop would cause
/// on a poisoned mutex. A poisoned lock (predecessor panicked) is recovered:
/// the session's cache is keyed by segment parameters, so a stale entry is
/// simply re-segmented on the next render.
pub fn lock_session(session: &Mutex<Session>) -> MutexGuard<'_, Session> {
    session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Vectorization stage: image → [`VectorDoc`] via a [`Session`].
///
/// `session.render` reuses the cached segmentation when the clustering
/// parameters are unchanged and only re-runs the cheap downstream stages — the
/// core of the interactive tuning loop.
pub fn vectorize_with_session(
    session: &mut Session,
    config: &Config,
    cancel: &CancelToken,
) -> Result<VectorDoc, EngineError> {
    let mut on_progress = |_p: vtracer::progress::Progress| {};
    let doc = session
        .render_with_progress(config, cancel, &mut on_progress)
        .map_err(|e| {
            if matches!(e, Error::Cancelled) {
                EngineError::Cancelled
            } else {
                EngineError::Internal(format!("trace failed: {e}"))
            }
        })?;
    if cancel.is_cancelled() {
        return Err(EngineError::Cancelled);
    }
    Ok(doc)
}

/// Layering stage: [`VectorDoc`] → layered SVG.
///
/// Pure and usually cheap, so it can be re-run any number of times against a
/// cached vectorized document, without touching the vectorization pipeline. New
/// layering strategies extend the match below.
///
/// The `auto` strategy is the exception: it re-fits every shape's curves, which
/// on large artwork can take tens of seconds. `cancel` is therefore polled at
/// every stage boundary and inside the re-fit loop; on abort the result is
/// `Err("cancelled")`, which callers map to the same 409 as a cancelled
/// vectorization.
pub fn layer_doc(
    doc: &VectorDoc,
    params: &TraceParams,
    cancel: &CancelToken,
) -> Result<TraceResult, EngineError> {
    if cancel.is_cancelled() {
        return Err(EngineError::Cancelled);
    }
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
        let (a, d) = AutoDecision::assign(doc, &ideal);
        if cancel.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        (a, Some(d))
    } else {
        let assignment = match params.layer_strategy.as_deref().unwrap_or("none") {
            "color" => {
                let n = params.layers.max(2);
                LayerAssignment::by_color(doc, n)
            }
            "complexity" => {
                let n = params.layers.max(2);
                let t = params.complexity_threshold.unwrap_or(70);
                LayerAssignment::by_complexity(doc, t, n)
            }
            "hybrid" => {
                let n = params.layers.max(2);
                let t = params.complexity_threshold.unwrap_or(70);
                LayerAssignment::hybrid(doc, n, t)
            }
            "spatial" => {
                let n = params.layers.max(2);
                let gap = (doc.width.min(doc.height) as f64 * 0.02).max(5.0);
                LayerAssignment::by_spatial(doc, n, gap)
            }
            _ => LayerAssignment::single_layer(),
        };
        (assignment, None)
    };
    let mut layered = LayeredDoc::from_vector_doc(doc, &assignment);
    let mut profile_ms: Option<u64> = None;
    if is_auto {
        let t = std::time::Instant::now();
        // The slow step: polls the token between shapes so a long auto layering
        // can be aborted instead of running to completion.
        apply_profiles(&mut layered, &ideal, &|| cancel.is_cancelled())
            .map_err(|_| EngineError::Cancelled)?;
        profile_ms = Some(t.elapsed().as_millis() as u64);
    }
    if cancel.is_cancelled() {
        return Err(EngineError::Cancelled);
    }

    let svg = if params.line_art {
        LayeredWriter::write_line_art(&layered)
    } else {
        LayeredWriter::write(&layered)
    };

    // Per-layer previews re-serialize each layer: worth one more abort check
    // before the loop on large documents.
    if cancel.is_cancelled() {
        return Err(EngineError::Cancelled);
    }
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

    Ok(TraceResult {
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

/// Full pipeline over a [`Session`]: vectorization + layering in one call.
///
/// Kept for one-shot consumers (batch trace, smart tuning, CLI, Python,
/// WASM) that want the complete result without managing the two stages
/// separately. The interactive editor drives [`vectorize_with_session`] and
/// [`layer_doc`] independently.
pub fn trace_with_session(
    session: &mut Session,
    config: &Config,
    params: &TraceParams,
    cancel: &CancelToken,
) -> Result<TraceResult, EngineError> {
    let doc = vectorize_with_session(session, config, cancel)?;
    layer_doc(&doc, params, cancel)
}

/// Render a vectorized document as a single-layer SVG: the raw vectorization
/// preview shown before layering is applied. `line_art` selects the output
/// style (stroke vs fill) so the preview matches what layering will produce;
/// it is an output-style param and does not affect the cached document.
pub fn vectorize_preview(doc: &VectorDoc, line_art: bool) -> VectorizeResult {
    let layered = LayeredDoc::from_vector_doc(doc, &LayerAssignment::single_layer());
    let svg = if line_art {
        LayeredWriter::write_line_art(&layered)
    } else {
        LayeredWriter::write(&layered)
    };
    VectorizeResult {
        svg,
        width: doc.width,
        height: doc.height,
        shape_count: doc.shapes.len(),
    }
}

/// Bounded LRU cache of vectorized documents, keyed by
/// `image_key::vectorize_signature`. Layering re-runs against a cached doc
/// without touching the (expensive) vectorization stage. Wrap in a `Mutex`
/// like [`patternstudio_core::SessionCache`].
pub struct DocCache {
    map: HashMap<String, Arc<VectorDoc>>,
    order: VecDeque<String>,
    cap: usize,
}

impl DocCache {
    /// Create a cache holding at most `cap` documents (evicts least-recently
    /// used entries beyond that).
    pub fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            cap: cap.max(1),
        }
    }

    fn touch(&mut self, key: &str) {
        self.order.retain(|k| k != key);
        self.order.push_back(key.to_string());
    }

    /// The cached document for `key`, if present (refreshes its LRU position).
    pub fn get(&mut self, key: &str) -> Option<Arc<VectorDoc>> {
        if self.map.contains_key(key) {
            self.touch(key);
            self.map.get(key).cloned()
        } else {
            None
        }
    }

    /// Insert (or replace) the document for `key`, evicting LRU entries
    /// beyond the cap.
    pub fn insert(&mut self, key: String, doc: VectorDoc) {
        if !self.map.contains_key(&key) {
            while self.map.len() >= self.cap {
                match self.order.pop_front() {
                    Some(old) => {
                        self.map.remove(&old);
                    }
                    None => break,
                }
            }
        }
        self.touch(&key);
        self.map.insert(key, Arc::new(doc));
    }

    /// Number of documents currently cached.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// One-shot trace from an image file: decode and run through a fresh session.
pub fn trace_file_once(img_path: &Path, params: &TraceParams) -> Result<TraceResult, EngineError> {
    let img = patternstudio_tracer::load_image(img_path)
        .map_err(|e| EngineError::Io(format!("failed to load image: {e}")))?;
    let config = build_vtracer_config(params)?;
    let mut session = Session::new(img);
    trace_with_session(&mut session, &config, params, &CancelToken::new())
}

/// One-shot trace from in-memory image bytes. Used by batch tracing, where
/// each file is traced exactly once (no session reuse).
pub fn trace_bytes_once(data: &[u8], params: &TraceParams) -> Result<TraceResult, EngineError> {
    let img = patternstudio_tracer::load_image_from_memory(data)
        .map_err(|e| EngineError::InvalidInput(format!("failed to decode image: {e}")))?;
    let config = build_vtracer_config(params)?;
    let mut session = Session::new(img);
    trace_with_session(&mut session, &config, params, &CancelToken::new())
}

/// Smart one-shot trace of an image file: decode, analyze, trial-trace
/// candidate parameter sets, then trace at full resolution with the winner.
/// `family` restricts the search to one preset family (absent = auto).
pub fn trace_file_smart(
    img_path: &Path,
    family: Option<&str>,
) -> Result<(TraceResult, SmartDecision), EngineError> {
    let mut img = patternstudio_tracer::load_image(img_path)
        .map_err(|e| EngineError::Io(format!("failed to load image: {e}")))?;
    // Same automatic-enhancement hook as the server's /smart endpoint (see
    // `needs_preprocess`); any failure keeps the original image.
    let mut preprocessed = false;
    if patternstudio_core::smart::needs_preprocess(&patternstudio_core::smart::analyze_image(&img))
    {
        if let Ok(out) = patternstudio_tracer::preprocess_image(img_path, None) {
            if let Ok(enhanced) = patternstudio_tracer::load_image(&out.path) {
                img = enhanced;
                preprocessed = true;
            }
        }
    }
    let (tr, mut decision) = smart_trace_image(img, family)?;
    decision.preprocessed = preprocessed;
    Ok((tr, decision))
}

/// Smart one-shot trace of in-memory image bytes (see [`trace_file_smart`]).
pub fn trace_bytes_smart(
    data: &[u8],
    family: Option<&str>,
) -> Result<(TraceResult, SmartDecision), EngineError> {
    let img = patternstudio_tracer::load_image_from_memory(data)
        .map_err(|e| EngineError::InvalidInput(format!("failed to decode image: {e}")))?;
    smart_trace_image(img, family)
}

fn smart_trace_image(
    img: ColorImage,
    family: Option<&str>,
) -> Result<(TraceResult, SmartDecision), EngineError> {
    let decision = match family {
        Some(f) => patternstudio_core::smart::recommend_for(&img, f),
        None => patternstudio_core::smart::recommend(&img),
    }
    .map_err(|e| EngineError::Internal(format!("smart analysis failed: {e}")))?;
    let params = smart_to_trace_params(&decision.params)?;
    let config = build_vtracer_config(&params)?;
    let mut session = Session::new(img);
    let tr = trace_with_session(&mut session, &config, &params, &CancelToken::new())?;
    Ok((tr, decision))
}

/// An in-flight trace that can be preempted by a newer request for the same
/// image.
pub struct ActiveTrace {
    pub request_id: String,
    pub cancel: CancelToken,
}

/// Tracks in-flight traces per image key and handles preemption: a new
/// foreground request cancels the in-flight trace for the same image, while a
/// background request is rejected when one is already running.
#[derive(Default)]
pub struct ActiveTraces {
    map: Mutex<HashMap<String, ActiveTrace>>,
}

impl ActiveTraces {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a trace is currently in flight for `image_key`.
    pub fn is_busy(&self, image_key: &str) -> bool {
        self.map.lock().unwrap().contains_key(image_key)
    }

    /// Register a trace for `image_key` and return its cancel token.
    ///
    /// - Foreground (`is_background = false`): cancels any in-flight trace for
    ///   the same image, then registers the new one.
    /// - Background (`is_background = true`): returns [`EngineError::Busy`] if
    ///   a trace is already in flight, otherwise registers.
    pub fn register(
        &self,
        image_key: &str,
        request_id: &str,
        is_background: bool,
    ) -> Result<CancelToken, EngineError> {
        let mut map = self.map.lock().unwrap();
        if is_background {
            if map.contains_key(image_key) {
                return Err(EngineError::Busy);
            }
        } else if let Some(old) = map.remove(image_key) {
            old.cancel.cancel();
        }
        let token = CancelToken::new();
        map.insert(
            image_key.to_string(),
            ActiveTrace {
                request_id: request_id.to_string(),
                cancel: token.clone(),
            },
        );
        Ok(token)
    }

    /// Remove the registration if we still own it (i.e. we were not preempted
    /// by a newer request).
    pub fn finish(&self, image_key: &str, request_id: &str) {
        let mut map = self.map.lock().unwrap();
        if let Some(entry) = map.get(image_key) {
            if entry.request_id == request_id {
                map.remove(image_key);
            }
        }
    }

    /// Cancel the in-flight trace for `image_key`, if any (e.g. the editor
    /// switched to a different image and this work is no longer wanted).
    /// The trace aborts at its next batch boundary. Returns true if a trace
    /// was cancelled.
    pub fn cancel(&self, image_key: &str) -> bool {
        let mut map = self.map.lock().unwrap();
        match map.remove(image_key) {
            Some(old) => {
                old.cancel.cancel();
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Auto layering re-fits every shape and can run for a long time, so the
    /// layering stage must honour the cancel token its request registered.
    #[test]
    fn layer_doc_aborts_when_cancelled() {
        let doc = VectorDoc::new(100, 100);
        let params = TraceParams {
            layer_strategy: Some("auto".into()),
            ..Default::default()
        };
        let cancel = CancelToken::new();
        cancel.cancel();
        assert_eq!(
            layer_doc(&doc, &params, &cancel).unwrap_err().code(),
            "cancelled"
        );
    }

    #[test]
    fn default_params_build_config() {
        let params = TraceParams::default();
        let config = build_vtracer_config(&params).unwrap();
        assert_eq!(config.clustering, Clustering::ColorCluster);
        assert!(!config.binary_adaptive);
    }

    #[test]
    fn smart_params_map_to_trace_params() {
        let smart = SmartParams::default();
        let params = smart_to_trace_params(&smart).unwrap();
        assert_eq!(params.clustering.as_deref(), Some("color"));
        assert_eq!(params.color_precision, Some(6));
        assert!(!params.line_art);
    }

    #[test]
    fn params_split_roundtrip() {
        let params = TraceParams {
            preset: "photo".into(),
            clustering: Some("color".into()),
            color_precision: Some(8),
            simplify: Some(0.2),
            optimize: Some(2),
            line_art: true,
            layers: 6,
            layer_strategy: Some("hybrid".into()),
            complexity_threshold: Some(55),
            ..Default::default()
        };
        let (v, l) = params.split();
        let back = TraceParams::from_parts(&v, &l);
        assert_eq!(
            serde_json::to_string(&params).unwrap(),
            serde_json::to_string(&back).unwrap(),
            "split/from_parts must be lossless"
        );
        // The two parts must be disjoint: the layer part carries no
        // vectorization fields and vice versa.
        assert_eq!(v.optimize, Some(2));
        assert!(l.line_art);
        assert_eq!(l.layers, 6);
        assert_eq!(l.complexity_threshold, Some(55));
    }

    #[test]
    fn vectorize_signature_stable_and_sensitive() {
        let a = TraceParams {
            color_precision: Some(6),
            line_art: true,
            ..Default::default()
        }
        .split()
        .0;
        let b = TraceParams {
            color_precision: Some(6),
            line_art: false,
            ..Default::default()
        }
        .split()
        .0;
        let c = TraceParams {
            color_precision: Some(7),
            ..Default::default()
        }
        .split()
        .0;
        // Same vectorization params -> same signature, regardless of the
        // (disjoint) layering params.
        assert_eq!(vectorize_signature(&a), vectorize_signature(&b));
        // A vectorization change -> different signature.
        assert_ne!(vectorize_signature(&a), vectorize_signature(&c));
    }

    #[test]
    fn doc_cache_evicts_least_recently_used() {
        let doc = || VectorDoc::new(10, 10);
        let mut cache = DocCache::new(2);
        cache.insert("a".into(), doc());
        cache.insert("b".into(), doc());
        // Touch "a" so "b" becomes the least-recently-used.
        assert!(cache.get("a").is_some());
        cache.insert("c".into(), doc());
        assert_eq!(cache.len(), 2);
        assert!(cache.get("a").is_some());
        assert!(cache.get("c").is_some());
        assert!(cache.get("b").is_none(), "b must have been evicted");
    }

    #[test]
    fn active_traces_preemption() {
        let traces = ActiveTraces::new();
        let t1 = traces.register("img1", "req-1", false).unwrap();
        // A second foreground request preempts the first.
        let t2 = traces.register("img1", "req-2", false).unwrap();
        assert!(t1.is_cancelled());
        assert!(!t2.is_cancelled());
        // A background request is rejected while one is in flight.
        assert_eq!(
            traces.register("img1", "req-3", true).err().unwrap().code(),
            "busy"
        );
        // Finishing with a stale request id keeps the newer registration.
        traces.finish("img1", "req-1");
        assert!(traces.is_busy("img1"));
        traces.finish("img1", "req-2");
        assert!(!traces.is_busy("img1"));
    }

    /// The error-code contract: stable codes, byte-identical Display strings
    /// (legacy callers string-match `"cancelled"`/`"busy"`), the wire shape
    /// `{ code, message }`, and the string/preprocess classifiers.
    #[test]
    fn engine_error_code_contract() {
        for (err, code, msg) in [
            (EngineError::Cancelled, "cancelled", "cancelled"),
            (EngineError::Busy, "busy", "busy"),
            (
                EngineError::Dependency("binary not found".into()),
                "dependency",
                "binary not found",
            ),
            (
                EngineError::InvalidInput("bad bytes".into()),
                "invalid_input",
                "bad bytes",
            ),
            (EngineError::Io("disk gone".into()), "io", "disk gone"),
            (EngineError::Internal("boom".into()), "internal", "boom"),
        ] {
            assert_eq!(err.code(), code);
            assert_eq!(err.to_string(), msg, "Display must stay byte-identical");
            assert_eq!(
                serde_json::to_value(&err).unwrap(),
                serde_json::json!({ "code": code, "message": msg }),
            );
        }
        // String compat: the two control-flow literals keep classifying;
        // anything else is an opaque internal failure with its message kept.
        assert_eq!(EngineError::from("cancelled"), EngineError::Cancelled);
        assert_eq!(EngineError::from("busy".to_string()), EngineError::Busy);
        assert_eq!(EngineError::from("anything else").code(), "internal");
        assert_eq!(
            EngineError::from("anything else").to_string(),
            "anything else"
        );
        // Preprocess classification mirrors tracer's preprocess.rs probes.
        assert_eq!(
            EngineError::from_preprocess_error(
                "realesrgan-ncnn-vulkan binary not found (looked in resources/bin/).".into()
            )
            .code(),
            "dependency"
        );
        assert_eq!(
            EngineError::from_preprocess_error("models directory not found (need x.param).".into())
                .code(),
            "dependency"
        );
        assert_eq!(
            EngineError::from_preprocess_error("enhance step exited with signal 11".into()).code(),
            "internal"
        );
    }
}
