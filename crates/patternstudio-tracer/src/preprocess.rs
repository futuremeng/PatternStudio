//! Image preprocessing: "digital art" enhancement before tracing.
//!
//! Runs the `realesrgan-ncnn-vulkan` binary (MIT, xinntao/Real-ESRGAN-ncnn-vulkan)
//! with the `realesrgan-x4plus-anime` model — the same model family as Upscayl's
//! "digital art" model. The model is a 4x super-resolver, so we run it at 4x and
//! downscale the result back to the original size: the net effect is a
//! denoised, edge-sharpened image at the input's dimensions, which traces much
//! cleaner (especially for line art).
//!
//! The binary and model are NOT committed; fetch them with
//! `make preprocess-setup` (scripts/fetch-preprocess.sh) into:
//!   resources/bin/<platform>/realesrgan-ncnn-vulkan
//!   resources/models/realesrgan-x4plus-anime.{param,bin}

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

/// The "digital art" model (Real-ESRGAN anime), a 4x super-resolver.
const MODEL_NAME: &str = "realesrgan-x4plus-anime";
const MODEL_SCALE: u32 = 4;
/// Hard cap for one enhancement run (large images on a CPU-only machine).
const RUN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Long-side cap for the model's 4x output. The `image` crate refuses to
/// decode buffers above 512 MiB by default, and the 4x output is 16x the
/// input's pixel count — so any input above ~11 MP would otherwise fail
/// with "Memory limit exceeded". Inputs whose 4x output would exceed this
/// cap are fed to the model downscaled; the result is resized back to the
/// original size, so the output contract is unchanged.
const MAX_MODEL_SIDE: u32 = 8192;

/// Result of a preprocess run.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreprocessOut {
    /// Path of the enhanced image (same dimensions as the input).
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub elapsed_ms: u64,
    /// True when the result was served from the on-disk cache.
    pub cached: bool,
}

fn bin_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "realesrgan-ncnn-vulkan.exe"
    } else {
        "realesrgan-ncnn-vulkan"
    }
}

fn platform_dir() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

/// Walk up from `start` (at most `max_levels` parents) looking for `rel(base)`.
fn find_up(start: &Path, max_levels: usize, rel: impl Fn(&Path) -> PathBuf) -> Option<PathBuf> {
    let mut dir = Some(start);
    for _ in 0..=max_levels {
        let d = dir?;
        let candidate = rel(d);
        if candidate.exists() {
            return Some(candidate);
        }
        dir = d.parent();
    }
    None
}

/// Locate the `realesrgan-ncnn-vulkan` binary.
///
/// Resolution order:
/// 1. `PATTERNSTUDIO_REALESRGAN_BIN` env var (full path)
/// 2. Tauri bundle resource dir (passed in by the desktop app)
/// 3. `resources/bin/<platform>/` found by walking up from the cwd
/// 4. `resources/bin/<platform>/` found by walking up from the executable
pub fn find_realesrgan_bin(resource_dir: Option<&Path>) -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("PATTERNSTUDIO_REALESRGAN_BIN") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Ok(p);
        }
        return Err(format!(
            "PATTERNSTUDIO_REALESRGAN_BIN is set but {p:?} does not exist"
        ));
    }
    if let Some(rd) = resource_dir {
        let p = rd.join("bin").join(bin_name());
        if p.exists() {
            return Ok(p);
        }
    }
    let rel = |base: &Path| {
        base.join("resources")
            .join("bin")
            .join(platform_dir())
            .join(bin_name())
    };
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(p) = find_up(&cwd, 5, rel) {
            return Ok(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            if let Some(p) = find_up(exe_dir, 6, rel) {
                return Ok(p);
            }
        }
    }
    Err(format!(
        "realesrgan-ncnn-vulkan binary not found (looked in resources/bin/{}/). \
         Run `make preprocess-setup` to download it, or set PATTERNSTUDIO_REALESRGAN_BIN.",
        platform_dir()
    ))
}

/// Locate the models directory (must contain `<MODEL_NAME>-x4.param`/`.bin`).
///
/// Resolution order:
/// 1. `PATTERNSTUDIO_MODELS_DIR` env var
/// 2. Tauri bundle resource dir
/// 3. `resources/models/` found by walking up from the cwd / executable
pub fn find_models_dir(resource_dir: Option<&Path>) -> Result<PathBuf, String> {
    let need = |d: &Path| d.join(format!("{MODEL_NAME}.param")).exists();
    if let Ok(p) = std::env::var("PATTERNSTUDIO_MODELS_DIR") {
        let p = PathBuf::from(p);
        if need(&p) {
            return Ok(p);
        }
        return Err(format!(
            "PATTERNSTUDIO_MODELS_DIR is set but {p:?} lacks {MODEL_NAME}.param"
        ));
    }
    if let Some(rd) = resource_dir {
        let p = rd.join("models");
        if need(&p) {
            return Ok(p);
        }
    }
    let rel = |base: &Path| base.join("resources").join("models");
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(p) = find_up(&cwd, 5, rel) {
            if need(&p) {
                return Ok(p);
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            if let Some(p) = find_up(exe_dir, 6, rel) {
                if need(&p) {
                    return Ok(p);
                }
            }
        }
    }
    Err(format!(
        "models directory not found (need {MODEL_NAME}.param). \
         Run `make preprocess-setup` to download it, or set PATTERNSTUDIO_MODELS_DIR."
    ))
}

/// On-disk cache for enhanced images: `~/.patternstudio/preprocess/`.
fn cache_dir() -> Result<PathBuf, String> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "cannot resolve home directory".to_string())?;
    let dir = PathBuf::from(home)
        .join(".patternstudio")
        .join("preprocess");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create cache dir {dir:?}: {e}"))?;
    Ok(dir)
}

/// Stable cache key for a source file: sha256 of `path:size:mtime`.
fn cache_key_for(path: &Path) -> Result<String, String> {
    let meta = path
        .metadata()
        .map_err(|e| format!("cannot stat {}: {e}", path.display()))?;
    let size = meta.len();
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update(b":");
    hasher.update(size.to_string().as_bytes());
    hasher.update(b":");
    hasher.update(mtime.to_string().as_bytes());
    let digest = hasher.finalize();
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

/// Run a child process, capturing output, with a hard timeout.
///
/// The pipes are drained by background threads so a chatty child (the binary
/// prints per-tile progress) can never fill a pipe buffer and deadlock us.
fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Result<std::process::Output, String> {
    use std::io::Read;
    use std::sync::{Arc, Mutex};

    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to start {:?}: {e}", cmd.get_program()))?;
    let stdout_buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let stderr_buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let (so, se) = (Arc::clone(&stdout_buf), Arc::clone(&stderr_buf));
    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();
    let t_out = std::thread::spawn(move || {
        if let Some(mut s) = stdout_handle {
            let mut buf = Vec::new();
            if s.read_to_end(&mut buf).is_ok() {
                so.lock().unwrap().extend_from_slice(&buf);
            }
        }
    });
    let t_err = std::thread::spawn(move || {
        if let Some(mut s) = stderr_handle {
            let mut buf = Vec::new();
            if s.read_to_end(&mut buf).is_ok() {
                se.lock().unwrap().extend_from_slice(&buf);
            }
        }
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child
            .try_wait()
            .map_err(|e| format!("preprocess wait failed: {e}"))?
        {
            Some(s) => break s,
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = (t_out.join(), t_err.join());
                    return Err(format!("preprocess timed out after {}s", timeout.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    };
    let _ = (t_out.join(), t_err.join());
    let stdout = stdout_buf.lock().unwrap().clone();
    let stderr = stderr_buf.lock().unwrap().clone();
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// Scale factor to apply to the input before the model so the 4x output's
/// long side stays at or below [`MAX_MODEL_SIDE`]. Returns 1.0 when no
/// downscale is needed.
fn input_scale_for(width: u32, height: u32) -> f64 {
    let four_x_long = width.max(height).saturating_mul(MODEL_SCALE) as f64;
    if four_x_long > MAX_MODEL_SIDE as f64 {
        MAX_MODEL_SIDE as f64 / four_x_long
    } else {
        1.0
    }
}

/// Enhance `input` with the digital-art model, returning a same-size image.
///
/// Results are cached on disk keyed by the source file's path:size:mtime, so
/// repeat calls (and re-opening the same image) are instant.
///
/// `resource_dir` is the Tauri bundle resource directory on desktop (None on
/// the server); see [`find_realesrgan_bin`] / [`find_models_dir`].
pub fn preprocess_image(
    input: &Path,
    resource_dir: Option<&Path>,
) -> Result<PreprocessOut, String> {
    let start = Instant::now();
    if !input.exists() {
        return Err(format!("image not found: {}", input.display()));
    }

    // 1. Cache check.
    let key = cache_key_for(input)?;
    let cached_path = cache_dir()?.join(format!("{key}.png"));
    if cached_path.exists() {
        let (width, height) = image::image_dimensions(&cached_path)
            .map_err(|e| format!("cached preprocess result is unreadable: {e}"))?;
        return Ok(PreprocessOut {
            path: cached_path,
            width,
            height,
            elapsed_ms: start.elapsed().as_millis() as u64,
            cached: true,
        });
    }

    // 2. Locate the runtime.
    let bin = find_realesrgan_bin(resource_dir)?;
    let models = find_models_dir(resource_dir)?;

    // 3. Run the model at 4x into a temp file.
    let (width, height) = image::image_dimensions(input)
        .map_err(|e| format!("failed to read image dimensions: {e}"))?;
    let tmp_dir = std::env::temp_dir().join("patternstudio-preprocess");
    std::fs::create_dir_all(&tmp_dir).map_err(|e| format!("cannot create temp dir: {e}"))?;
    let tmp4x = tmp_dir.join(format!("{key}-4x.png"));

    // Keep the 4x output within MAX_MODEL_SIDE (see the constant). Larger
    // inputs are fed to the model downscaled; step 4 resizes the result
    // back to the original size.
    let scale = input_scale_for(width, height);
    let model_in_path = tmp_dir.join(format!("{key}-in.png"));
    let model_input: &Path = if scale < 1.0 {
        let new_w = ((width as f64) * scale).round().max(1.0) as u32;
        let new_h = ((height as f64) * scale).round().max(1.0) as u32;
        let img = image::open(input)
            .map_err(|e| format!("failed to read image: {e}"))?
            .to_rgb8();
        let small =
            image::imageops::resize(&img, new_w, new_h, image::imageops::FilterType::Lanczos3);
        small
            .save(&model_in_path)
            .map_err(|e| format!("failed to save downscaled input: {e}"))?;
        &model_in_path
    } else {
        input
    };

    let mut cmd = Command::new(&bin);
    cmd.arg("-i")
        .arg(model_input)
        .arg("-o")
        .arg(&tmp4x)
        .arg("-m")
        .arg(&models)
        .arg("-n")
        .arg(MODEL_NAME)
        .arg("-s")
        .arg(MODEL_SCALE.to_string())
        .arg("-f")
        .arg("png");
    let output = run_with_timeout(cmd, RUN_TIMEOUT).map_err(|e| {
        std::fs::remove_file(&model_in_path).ok();
        format!("preprocess failed: {e}")
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let lines: Vec<&str> = stderr.lines().collect();
        let tail: String = lines
            .iter()
            .rev()
            .take(5)
            .copied()
            .collect::<Vec<_>>()
            .join(" | ");
        std::fs::remove_file(&tmp4x).ok();
        std::fs::remove_file(&model_in_path).ok();
        return Err(format!(
            "realesrgan-ncnn-vulkan exited with {:?}{}",
            output.status,
            if tail.is_empty() {
                String::new()
            } else {
                format!(": {tail}")
            }
        ));
    }
    if !tmp4x.exists() {
        std::fs::remove_file(&model_in_path).ok();
        return Err("preprocess produced no output image".into());
    }

    // 4. Downscale the 4x result back to the original size (Lanczos3).
    let enhanced = image::open(&tmp4x)
        .map_err(|e| format!("failed to read model output: {e}"))?
        .to_rgb8();
    let same_size = image::imageops::resize(
        &enhanced,
        width,
        height,
        image::imageops::FilterType::Lanczos3,
    );
    // Write atomically so a crashed run never leaves a truncated cache entry.
    let tmp_out = cached_path.with_file_name(format!("{key}.tmp.png"));
    same_size
        .save(&tmp_out)
        .map_err(|e| format!("failed to save preprocessed image: {e}"))?;
    std::fs::rename(&tmp_out, &cached_path)
        .map_err(|e| format!("failed to finalize preprocessed image: {e}"))?;
    std::fs::remove_file(&tmp4x).ok();
    std::fs::remove_file(&model_in_path).ok();

    Ok(PreprocessOut {
        path: cached_path,
        width,
        height,
        elapsed_ms: start.elapsed().as_millis() as u64,
        cached: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_scale_caps_model_output() {
        // Small image: no downscale.
        assert_eq!(input_scale_for(256, 256), 1.0);
        // 4x long side exactly at the cap: no downscale.
        assert_eq!(input_scale_for(2048, 1000), 1.0);
        // A common 4000×3000 photo: 4x long side 16000 > 8192 → downscale,
        // and the resulting 4x long side lands within the cap.
        let s = input_scale_for(4000, 3000);
        assert!(s < 1.0);
        assert!(4000f64 * s * MODEL_SCALE as f64 <= MAX_MODEL_SIDE as f64 + 1e-6);
        // Very wide image: the long side (width) drives the cap.
        let s = input_scale_for(10000, 700);
        assert!(s < 1.0);
        assert!(10000f64 * s * MODEL_SCALE as f64 <= MAX_MODEL_SIDE as f64 + 1e-6);
        assert!(700f64 * s * MODEL_SCALE as f64 <= MAX_MODEL_SIDE as f64 + 1e-6);
    }

    /// End-to-end: enhance a real image (skips when the runtime isn't set up
    /// via `make preprocess-setup`). Verifies the 4x→original downscale and
    /// the on-disk cache.
    #[test]
    fn preprocess_image_roundtrip() {
        let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/lineart.png");
        if !input.exists() {
            eprintln!("skipping: no fixture at {}", input.display());
            return;
        }
        if find_realesrgan_bin(None).is_err() || find_models_dir(None).is_err() {
            eprintln!("skipping: preprocess runtime not set up (make preprocess-setup)");
            return;
        }
        // Drop any cache entry from a previous run so the first call below is
        // guaranteed to be a cold (non-cached) run.
        let key = cache_key_for(&input).unwrap();
        let _ = std::fs::remove_file(cache_dir().unwrap().join(format!("{key}.png")));
        let out = preprocess_image(&input, None).expect("preprocess failed");
        assert!(out.path.exists());
        assert!(!out.cached);
        let (w, h) = image::image_dimensions(&input).unwrap();
        assert_eq!((out.width, out.height), (w, h));
        // Second call hits the cache.
        let out2 = preprocess_image(&input, None).expect("preprocess failed");
        assert!(out2.cached);
        assert_eq!(out2.path, out.path);
    }
}
