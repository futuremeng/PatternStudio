//! Image redraw: Qwen-Image-based image-to-image restyling.
//!
//! A non-mandatory stage alongside the Real-ESRGAN enhancement
//! (`preprocess`): the *original* source image (never an enhanced result) is
//! sent to a Qwen-Image edit model with a prompt, and the model returns a
//! restyled rendering of the same subject.
//!
//! Two wire protocols are supported, selected by `PATTERNSTUDIO_QWEN_PROVIDER`:
//!
//! - `"dashscope"` (default) — the native DashScope (Alibaba Model Studio)
//!   multimodal-generation protocol. Works against the official API and any
//!   DashScope-compatible gateway. The image is passed as a base64 data URL;
//!   the response is either synchronous (`output.choices[0].message.content`)
//!   or an async task (`output.task_id`, polled via `GET /tasks/{id}`).
//! - `"openai"` — an OpenAI-compatible `POST /images/edits` endpoint
//!   (self-hosted gateways such as one-api / new-api / LiteLLM). The image is
//!   sent as a multipart file; the result comes back as `data[0].url` or
//!   `data[0].b64_json`.
//!
//! Configuration (environment variables):
//!   PATTERNSTUDIO_QWEN_PROVIDER   "dashscope" (default) | "openai"
//!   DASHSCOPE_API_KEY             API key (or PATTERNSTUDIO_QWEN_API_KEY)
//!   PATTERNSTUDIO_QWEN_API_BASE   endpoint base (defaults per provider)
//!   PATTERNSTUDIO_QWEN_MODEL      model name (defaults per provider)
//!
//! Results are written to `~/.patternstudio/redraw/`. They are NOT cached:
//! each call is a fresh, prompt-driven generation.
//!
//! The Qwen-Image API caps input images at 10MB. Files above the cap are
//! automatically downscaled (longest side capped at `MAX_INPUT_DIMENSION`)
//! and re-encoded as JPEG before upload, iterating until the payload fits.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;

/// Hard cap for one redraw run (model generation can be slow).
const RUN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// DashScope input image size cap.
const MAX_INPUT_BYTES: u64 = 10 * 1024 * 1024;
/// Longest-side cap applied when compressing oversized inputs. The model
/// resizes its input internally, so larger dimensions add no detail.
const MAX_INPUT_DIMENSION: u32 = 2048;
/// JPEG quality used when re-encoding oversized inputs.
const JPEG_QUALITY: u8 = 90;
/// Per-iteration scale factor if the encoded image is still over the cap.
const DOWNSCALE_STEP: f64 = 0.85;
/// Maximum downscale iterations before giving up.
const MAX_DOWNSCALE_ITERATIONS: u32 = 8;
/// Poll interval while waiting for an async DashScope task.
const TASK_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Result of a redraw run.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedrawOut {
    /// Path of the redrawn image (PNG).
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub elapsed_ms: u64,
    /// The prompt that was sent to the model.
    pub prompt: String,
    /// Seed used for the generation (None when the model chose one).
    pub seed: Option<u64>,
}

/// Wire protocol for the Qwen-Image API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// Native DashScope (Alibaba Model Studio) protocol.
    DashScope,
    /// OpenAI-compatible `/images/edits` protocol.
    OpenAICompat,
}

/// Resolved redraw configuration.
#[derive(Debug, Clone)]
pub struct RedrawConfig {
    pub provider: Provider,
    pub api_key: String,
    pub api_base: String,
    pub model: String,
}

impl RedrawConfig {
    /// Build a config from explicit values, applying per-provider defaults.
    pub fn resolve(
        provider: Provider,
        api_key: Option<String>,
        api_base: Option<String>,
        model: Option<String>,
    ) -> Result<Self, String> {
        let api_key = api_key
            .filter(|k| !k.trim().is_empty())
            .ok_or_else(|| {
                "Qwen API key not configured — set it in Settings (or the DASHSCOPE_API_KEY environment variable)"
                    .to_string()
            })?;
        let (default_base, default_model) = match provider {
            Provider::DashScope => ("https://dashscope.aliyuncs.com/api/v1", "qwen-image-2.0"),
            Provider::OpenAICompat => ("http://127.0.0.1:8080/v1", "qwen-image-edit"),
        };
        Ok(Self {
            provider,
            api_key,
            api_base: api_base
                .filter(|b| !b.trim().is_empty())
                .map(|b| b.trim_end_matches('/').to_string())
                .unwrap_or_else(|| default_base.into()),
            model: model
                .filter(|m| !m.trim().is_empty())
                .unwrap_or_else(|| default_model.into()),
        })
    }

    /// Resolve configuration from the environment (see the module docs).
    pub fn from_env() -> Result<Self, String> {
        let provider = match std::env::var("PATTERNSTUDIO_QWEN_PROVIDER")
            .ok()
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("dashscope")
            .to_ascii_lowercase()
            .as_str()
        {
            "dashscope" => Provider::DashScope,
            "openai" => Provider::OpenAICompat,
            other => {
                return Err(format!(
                    "unknown PATTERNSTUDIO_QWEN_PROVIDER `{other}` (expected \"dashscope\" or \"openai\")"
                ))
            }
        };
        let api_key = std::env::var("PATTERNSTUDIO_QWEN_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())
            .or_else(|| {
                std::env::var("DASHSCOPE_API_KEY")
                    .ok()
                    .filter(|k| !k.trim().is_empty())
            });
        let api_base = std::env::var("PATTERNSTUDIO_QWEN_API_BASE").ok();
        let model = std::env::var("PATTERNSTUDIO_QWEN_MODEL").ok();
        Self::resolve(provider, api_key, api_base, model)
    }

    /// Resolve configuration for the app: saved settings (Settings dialog,
    /// `~/.patternstudio/settings.json`) first, then environment variables,
    /// then per-provider defaults.
    pub fn load() -> Result<Self, String> {
        let saved = crate::settings::load_qwen_settings();
        let env = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let provider = match saved
            .provider
            .clone()
            .or_else(|| env("PATTERNSTUDIO_QWEN_PROVIDER"))
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("dashscope")
            .to_ascii_lowercase()
            .as_str()
        {
            "dashscope" => Provider::DashScope,
            "openai" => Provider::OpenAICompat,
            other => {
                return Err(format!(
                    "unknown Qwen provider `{other}` (expected \"dashscope\" or \"openai\")"
                ))
            }
        };
        let api_key = saved
            .api_key
            .clone()
            .filter(|k| !k.trim().is_empty())
            .or_else(|| env("PATTERNSTUDIO_QWEN_API_KEY"))
            .or_else(|| env("DASHSCOPE_API_KEY"));
        let api_base = saved
            .api_base
            .clone()
            .filter(|b| !b.trim().is_empty())
            .or_else(|| env("PATTERNSTUDIO_QWEN_API_BASE"));
        let model = saved
            .model
            .clone()
            .filter(|m| !m.trim().is_empty())
            .or_else(|| env("PATTERNSTUDIO_QWEN_MODEL"));
        Self::resolve(provider, api_key, api_base, model)
    }
}

/// MIME type for an input image, from its extension (default: PNG).
pub fn mime_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("tif" | "tiff") => "image/tiff",
        _ => "image/png",
    }
}

/// Encode image bytes as a `data:<mime>;base64,<...>` URL.
pub fn build_data_url(mime: &str, bytes: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{mime};base64,{b64}")
}

/// Read `input` and prepare it for upload.
///
/// Images within `MAX_INPUT_BYTES` are returned as-is with their original
/// MIME type. Larger images are decoded, downscaled (longest side capped at
/// `MAX_INPUT_DIMENSION`) and re-encoded as JPEG, iterating until the
/// payload fits under the API limit.
fn prepare_input(input: &Path) -> Result<(Vec<u8>, &'static str), String> {
    let bytes =
        std::fs::read(input).map_err(|e| format!("failed to read {}: {e}", input.display()))?;
    if bytes.len() as u64 <= MAX_INPUT_BYTES {
        return Ok((bytes, mime_for_path(input)));
    }
    compress_to_limit(&bytes, input)
}

/// Downscale + JPEG re-encode `bytes` until the result is under
/// `MAX_INPUT_BYTES`. Returns the encoded bytes and the new MIME type.
fn compress_to_limit(bytes: &[u8], source: &Path) -> Result<(Vec<u8>, &'static str), String> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| format!("failed to decode {}: {e}", source.display()))?;
    let mut scale = 1.0f64;
    for _ in 0..MAX_DOWNSCALE_ITERATIONS {
        let (w, h) = (img.width(), img.height());
        let longest = w.max(h) as f64;
        let factor = (scale * (MAX_INPUT_DIMENSION as f64 / longest)).min(1.0);
        let new_w = (w as f64 * factor).round().max(1.0) as u32;
        let new_h = (h as f64 * factor).round().max(1.0) as u32;
        let resized = if (new_w, new_h) == (w, h) {
            img.clone()
        } else {
            img.resize(new_w, new_h, image::imageops::FilterType::Lanczos3)
        };
        // JPEG has no alpha channel: composite onto white.
        let rgba = resized.to_rgba8();
        let rgb = image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
            let p = rgba.get_pixel(x, y);
            let a = p[3] as f32 / 255.0;
            let mix = |c: u8| (c as f32 * a + 255.0 * (1.0 - a)) as u8;
            image::Rgb([mix(p[0]), mix(p[1]), mix(p[2])])
        });
        let mut buf: Vec<u8> = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, JPEG_QUALITY);
        image::ImageEncoder::write_image(
            encoder,
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ColorType::Rgb8.into(),
        )
        .map_err(|e| format!("failed to encode compressed redraw input: {e}"))?;
        if buf.len() as u64 <= MAX_INPUT_BYTES {
            return Ok((buf, "image/jpeg"));
        }
        scale *= DOWNSCALE_STEP;
    }
    Err(format!(
        "image too large for redraw: could not compress {} below 10MB",
        source.display()
    ))
}

/// Build the DashScope multimodal-generation request body.
pub fn build_dashscope_body(
    model: &str,
    data_url: &str,
    prompt: &str,
    seed: Option<u64>,
) -> serde_json::Value {
    let mut parameters = serde_json::json!({ "n": 1 });
    if let Some(s) = seed {
        parameters["seed"] = serde_json::json!(s);
    }
    serde_json::json!({
        "model": model,
        "input": {
            "messages": [{
                "role": "user",
                "content": [
                    { "image": data_url },
                    { "text": prompt },
                ],
            }],
        },
        "parameters": parameters,
    })
}

/// Extract the result image URL from a synchronous DashScope response.
pub fn dashscope_sync_image_url(resp: &serde_json::Value) -> Option<String> {
    resp.pointer("/output/choices/0/message/content")?
        .as_array()?
        .iter()
        .find_map(|item| item.get("image").and_then(|i| i.as_str()))
        .map(str::to_string)
}

/// Extract an async task id from a DashScope response, if present.
pub fn dashscope_task_id(resp: &serde_json::Value) -> Option<String> {
    resp.pointer("/output/task_id")
        .and_then(|t| t.as_str())
        .map(str::to_string)
}

/// Extract the result image URL from a succeeded DashScope task response.
pub fn dashscope_task_image_url(resp: &serde_json::Value) -> Option<String> {
    let results = resp.pointer("/output/results")?.as_array()?;
    results
        .iter()
        .find_map(|r| {
            r.get("url")
                .and_then(|u| u.as_str())
                .or_else(|| r.get("image").and_then(|i| i.as_str()))
        })
        .map(str::to_string)
}

/// Extract `data[0].b64_json` (decoded) from an OpenAI-compatible response.
pub fn openai_image_b64(resp: &serde_json::Value) -> Option<Vec<u8>> {
    let b64 = resp.pointer("/data/0/b64_json")?.as_str()?;
    base64::engine::general_purpose::STANDARD.decode(b64).ok()
}

/// Extract `data[0].url` from an OpenAI-compatible response.
pub fn openai_image_url(resp: &serde_json::Value) -> Option<String> {
    resp.pointer("/data/0/url")
        .and_then(|u| u.as_str())
        .map(str::to_string)
}

/// `~/.patternstudio/redraw/` — where redraw results are written.
fn redraw_dir() -> Result<PathBuf, String> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "cannot resolve home directory".to_string())?;
    let dir = PathBuf::from(home).join(".patternstudio").join("redraw");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create redraw dir {dir:?}: {e}"))?;
    Ok(dir)
}

/// Unique file name for a redraw result (millis + per-process sequence).
fn redraw_file_name() -> String {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("redraw-{ms}-{seq}.png")
}

fn http_err(e: reqwest::Error) -> String {
    if e.is_timeout() {
        format!("redraw timed out after {}s", RUN_TIMEOUT.as_secs())
    } else {
        format!("HTTP request failed: {e}")
    }
}

fn download_image(client: &reqwest::blocking::Client, url: &str) -> Result<Vec<u8>, String> {
    client
        .get(url)
        .send()
        .map_err(http_err)?
        .error_for_status()
        .map_err(|e| format!("failed to download result image: {e}"))?
        .bytes()
        .map_err(|e| format!("failed to read result image: {e}"))
        .map(|b| b.to_vec())
}

/// DashScope: submit the edit request, then either read the synchronous
/// result or poll the async task until it finishes.
fn dashscope_redraw(
    client: &reqwest::blocking::Client,
    config: &RedrawConfig,
    data_url: &str,
    prompt: &str,
    seed: Option<u64>,
) -> Result<Vec<u8>, String> {
    let url = format!(
        "{}/services/aigc/multimodal-generation/generation",
        config.api_base
    );
    let body = build_dashscope_body(&config.model, data_url, prompt, seed);
    let resp: serde_json::Value = client
        .post(&url)
        .bearer_auth(&config.api_key)
        .json(&body)
        .send()
        .map_err(http_err)?
        .error_for_status()
        .map_err(|e| format!("dashscope request failed: {e}"))?
        .json()
        .map_err(|e| format!("failed to parse dashscope response: {e}"))?;

    // DashScope reports API-level errors with HTTP 200 + a `code` field.
    if let Some(code) = resp.get("code").and_then(|c| c.as_str()) {
        if !code.is_empty() {
            let msg = resp
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(format!("dashscope error {code}: {msg}"));
        }
    }

    if let Some(image_url) = dashscope_sync_image_url(&resp) {
        return download_image(client, &image_url);
    }
    if let Some(task_id) = dashscope_task_id(&resp) {
        return poll_dashscope_task(client, config, &task_id);
    }
    Err("unrecognized dashscope response (no image or task_id)".into())
}

/// Poll `GET /tasks/{id}` until the DashScope task succeeds or fails.
fn poll_dashscope_task(
    client: &reqwest::blocking::Client,
    config: &RedrawConfig,
    task_id: &str,
) -> Result<Vec<u8>, String> {
    let url = format!("{}/tasks/{task_id}", config.api_base);
    let deadline = Instant::now() + RUN_TIMEOUT;
    loop {
        let resp: serde_json::Value = client
            .get(&url)
            .bearer_auth(&config.api_key)
            .send()
            .map_err(http_err)?
            .error_for_status()
            .map_err(|e| format!("dashscope task query failed: {e}"))?
            .json()
            .map_err(|e| format!("failed to parse task response: {e}"))?;
        let status = resp
            .pointer("/output/task_status")
            .and_then(|s| s.as_str())
            .unwrap_or("UNKNOWN");
        match status {
            "SUCCEEDED" => {
                let image_url =
                    dashscope_task_image_url(&resp).ok_or("succeeded task has no result image")?;
                return download_image(client, &image_url);
            }
            "FAILED" | "CANCELED" | "UNKNOWN" => {
                let msg = resp
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or(status);
                return Err(format!("dashscope task {status}: {msg}"));
            }
            _ => {
                if Instant::now() >= deadline {
                    return Err(format!(
                        "redraw timed out after {}s (task still {status})",
                        RUN_TIMEOUT.as_secs()
                    ));
                }
                std::thread::sleep(TASK_POLL_INTERVAL);
            }
        }
    }
}

/// OpenAI-compatible: `POST /images/edits` with the image as a multipart file.
fn openai_redraw(
    client: &reqwest::blocking::Client,
    config: &RedrawConfig,
    image_bytes: &[u8],
    mime: &str,
    prompt: &str,
    seed: Option<u64>,
) -> Result<Vec<u8>, String> {
    let url = format!("{}/images/edits", config.api_base);
    let file_name = if mime == "image/jpeg" {
        "input.jpg"
    } else {
        "input.png"
    };
    let mut form = reqwest::blocking::multipart::Form::new()
        .text("model", config.model.clone())
        .text("prompt", prompt.to_string())
        .text("n", "1")
        .part(
            "image",
            reqwest::blocking::multipart::Part::bytes(image_bytes.to_vec())
                .file_name(file_name)
                .mime_str(mime)
                .map_err(|e| format!("invalid mime type: {e}"))?,
        );
    if let Some(s) = seed {
        form = form.text("seed", s.to_string());
    }
    let resp: serde_json::Value = client
        .post(&url)
        .bearer_auth(&config.api_key)
        .multipart(form)
        .send()
        .map_err(http_err)?
        .error_for_status()
        .map_err(|e| format!("images/edits request failed: {e}"))?
        .json()
        .map_err(|e| format!("failed to parse images/edits response: {e}"))?;

    if let Some(bytes) = openai_image_b64(&resp) {
        return Ok(bytes);
    }
    if let Some(image_url) = openai_image_url(&resp) {
        return download_image(client, &image_url);
    }
    Err("unrecognized images/edits response (no data[0].url or b64_json)".into())
}

/// Redraw `input` with the Qwen-Image model using `prompt`.
///
/// The result is written to `~/.patternstudio/redraw/` and returned. Each
/// call is a fresh generation (no caching).
pub fn redraw_image(input: &Path, prompt: &str, seed: Option<u64>) -> Result<RedrawOut, String> {
    let start = Instant::now();
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("redraw prompt is empty".into());
    }
    let config = RedrawConfig::load()?;
    if !input.exists() {
        return Err(format!("image not found: {}", input.display()));
    }
    // Oversized inputs are downscaled + re-encoded as JPEG (see prepare_input).
    let (bytes, mime) = prepare_input(input)?;

    let client = reqwest::blocking::Client::builder()
        .timeout(RUN_TIMEOUT)
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    let out_bytes = match config.provider {
        Provider::DashScope => {
            let data_url = build_data_url(mime, &bytes);
            dashscope_redraw(&client, &config, &data_url, prompt, seed)?
        }
        Provider::OpenAICompat => openai_redraw(&client, &config, &bytes, mime, prompt, seed)?,
    };

    // Decode to get dimensions and normalize to PNG.
    let img = image::load_from_memory(&out_bytes)
        .map_err(|e| format!("failed to decode redraw result: {e}"))?;
    let (width, height) = (img.width(), img.height());
    let out_path = redraw_dir()?.join(redraw_file_name());
    img.save(&out_path)
        .map_err(|e| format!("failed to save redraw result: {e}"))?;

    Ok(RedrawOut {
        path: out_path,
        width,
        height,
        elapsed_ms: start.elapsed().as_millis() as u64,
        prompt: prompt.to_string(),
        seed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_detection() {
        assert_eq!(mime_for_path(Path::new("a.png")), "image/png");
        assert_eq!(mime_for_path(Path::new("a.JPG")), "image/jpeg");
        assert_eq!(mime_for_path(Path::new("a.webp")), "image/webp");
        assert_eq!(mime_for_path(Path::new("a.tiff")), "image/tiff");
        assert_eq!(mime_for_path(Path::new("noext")), "image/png");
    }

    #[test]
    fn data_url_roundtrip() {
        let url = build_data_url("image/png", &[1, 2, 3]);
        assert!(url.starts_with("data:image/png;base64,"));
        let b64 = url.strip_prefix("data:image/png;base64,").unwrap();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(b64)
                .unwrap(),
            vec![1, 2, 3]
        );
    }

    /// A small image passes through `prepare_input` unchanged.
    #[test]
    fn prepare_input_small_passthrough() {
        let dir = std::env::temp_dir().join(format!("ps-redraw-small-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("small.png");
        let img = image::RgbImage::from_pixel(64, 48, image::Rgb([200, 10, 10]));
        img.save(&path).unwrap();
        let original = std::fs::read(&path).unwrap();
        let (bytes, mime) = prepare_input(&path).unwrap();
        assert_eq!(mime, "image/png");
        assert_eq!(bytes, original);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An oversized image is downscaled + re-encoded under the 10MB cap.
    #[test]
    fn prepare_input_compresses_oversized() {
        // ~25MB of pseudo-random noise: incompressible as PNG, so the file
        // is guaranteed to exceed MAX_INPUT_BYTES.
        let (w, h) = (2500u32, 2500u32);
        let img = image::RgbaImage::from_fn(w, h, |x, y| {
            let n = (x as u32).wrapping_mul(2654435761) ^ (y as u32).wrapping_mul(40503);
            image::Rgba([n as u8, (n >> 8) as u8, (n >> 16) as u8, 255])
        });
        let dir = std::env::temp_dir().join(format!("ps-redraw-large-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("large.png");
        img.save(&path).unwrap();
        let png = std::fs::read(&path).unwrap();
        assert!(png.len() as u64 > MAX_INPUT_BYTES);

        let (bytes, mime) = prepare_input(&path).unwrap();
        assert_eq!(mime, "image/jpeg");
        assert!(bytes.len() as u64 <= MAX_INPUT_BYTES);
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert!(decoded.width() <= MAX_INPUT_DIMENSION);
        assert!(decoded.height() <= MAX_INPUT_DIMENSION);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dashscope_body_shape() {
        let body = build_dashscope_body(
            "qwen-image-2.0",
            "data:image/png;base64,AAA",
            "clean line art",
            Some(42),
        );
        assert_eq!(body["model"], "qwen-image-2.0");
        assert_eq!(
            body.pointer("/input/messages/0/role"),
            Some(&serde_json::json!("user"))
        );
        assert_eq!(
            body.pointer("/input/messages/0/content/0/image"),
            Some(&serde_json::json!("data:image/png;base64,AAA"))
        );
        assert_eq!(
            body.pointer("/input/messages/0/content/1/text"),
            Some(&serde_json::json!("clean line art"))
        );
        assert_eq!(body["parameters"]["n"], 1);
        assert_eq!(body["parameters"]["seed"], 42);
        // No seed → no seed field.
        let body2 = build_dashscope_body("m", "u", "p", None);
        assert!(body2["parameters"].get("seed").is_none());
    }

    #[test]
    fn dashscope_sync_result_parsing() {
        let resp = serde_json::json!({
            "output": { "choices": [ { "message": { "content": [ { "image": "https://x/y.png" } ] } } ] }
        });
        assert_eq!(
            dashscope_sync_image_url(&resp).as_deref(),
            Some("https://x/y.png")
        );
        assert!(dashscope_task_id(&resp).is_none());
    }

    #[test]
    fn dashscope_async_result_parsing() {
        let submit =
            serde_json::json!({ "output": { "task_id": "t-1", "task_status": "PENDING" } });
        assert_eq!(dashscope_task_id(&submit).as_deref(), Some("t-1"));
        assert!(dashscope_sync_image_url(&submit).is_none());
        let done = serde_json::json!({
            "output": { "task_status": "SUCCEEDED", "results": [ { "url": "https://x/z.png" } ] }
        });
        assert_eq!(
            dashscope_task_image_url(&done).as_deref(),
            Some("https://x/z.png")
        );
    }

    #[test]
    fn openai_result_parsing() {
        let url_resp = serde_json::json!({ "data": [ { "url": "https://x/a.png" } ] });
        assert_eq!(
            openai_image_url(&url_resp).as_deref(),
            Some("https://x/a.png")
        );
        let b64 = base64::engine::general_purpose::STANDARD.encode([9u8, 8, 7]);
        let b64_resp = serde_json::json!({ "data": [ { "b64_json": b64 } ] });
        assert_eq!(
            openai_image_b64(&b64_resp).as_deref(),
            Some(&[9u8, 8, 7][..])
        );
    }

    #[test]
    fn config_defaults() {
        let c =
            RedrawConfig::resolve(Provider::DashScope, Some("sk-test".into()), None, None).unwrap();
        assert_eq!(c.api_base, "https://dashscope.aliyuncs.com/api/v1");
        assert_eq!(c.model, "qwen-image-2.0");
        let c = RedrawConfig::resolve(
            Provider::OpenAICompat,
            Some("k".into()),
            Some("http://h:1/v1/".into()),
            None,
        )
        .unwrap();
        assert_eq!(c.api_base, "http://h:1/v1");
        assert_eq!(c.model, "qwen-image-edit");
        // Missing / blank API key is an error.
        assert!(RedrawConfig::resolve(Provider::DashScope, None, None, None).is_err());
        assert!(RedrawConfig::resolve(Provider::DashScope, Some("  ".into()), None, None).is_err());
    }

    /// End-to-end redraw (skips unless a Qwen API key is configured).
    #[test]
    fn redraw_image_roundtrip() {
        let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/lineart.png");
        if !input.exists() {
            eprintln!("skipping: no fixture at {}", input.display());
            return;
        }
        if RedrawConfig::load().is_err() {
            eprintln!("skipping: Qwen API key not configured");
            return;
        }
        let out =
            redraw_image(&input, "clean black and white line art", None).expect("redraw failed");
        assert!(out.path.exists());
        assert!(out.width > 0 && out.height > 0);
        assert_eq!(out.prompt, "clean black and white line art");
    }
}
