//! Persistent app settings: `~/.patternstudio/settings.json`.
//!
//! Currently holds the Qwen-Image (redraw) API configuration, editable from
//! the Settings dialog. Precedence when resolving the redraw config:
//! saved settings → environment variables → per-provider defaults.
//!
//! The API key is stored in plain text next to `projects.json` (a local,
//! single-user tool) but is never returned to the client unmasked — the
//! settings endpoints only expose a masked form (`sk-…abcd`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Qwen-Image (redraw) API settings. `None` = fall back to the environment
/// variable, then to the per-provider default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QwenSettings {
    /// "dashscope" | "openai"
    pub provider: Option<String>,
    pub api_key: Option<String>,
    pub api_base: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct SettingsFile {
    qwen: QwenSettings,
}

/// JSON view of the current Qwen settings, safe to send to the client
/// (the API key is masked).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QwenSettingsView {
    /// Effective provider ("dashscope" when unset).
    pub provider: String,
    pub api_base: Option<String>,
    pub model: Option<String>,
    pub has_api_key: bool,
    /// Masked key for display, e.g. `sk-…abcd` (None when no key is set).
    pub api_key_masked: Option<String>,
}

impl QwenSettingsView {
    pub fn from_settings(s: &QwenSettings) -> Self {
        let key = s
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty());
        Self {
            provider: s
                .provider
                .clone()
                .filter(|p| !p.trim().is_empty())
                .unwrap_or_else(|| "dashscope".into()),
            api_base: s.api_base.clone(),
            model: s.model.clone(),
            has_api_key: key.is_some(),
            api_key_masked: key.map(mask_api_key),
        }
    }
}

/// Mask an API key for display: keep a short prefix + the last 4 chars.
pub fn mask_api_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 8 {
        return "••••".to_string();
    }
    let prefix: String = chars.iter().take(3).collect();
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    format!("{prefix}…{suffix}")
}

/// `~/.patternstudio/settings.json` (the directory is created on save).
fn settings_path() -> Result<PathBuf, String> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "cannot resolve home directory".to_string())?;
    Ok(PathBuf::from(home)
        .join(".patternstudio")
        .join("settings.json"))
}

/// Load the Qwen settings from an explicit path (missing/corrupt → defaults).
fn load_from(path: &std::path::Path) -> QwenSettings {
    match std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<SettingsFile>(&s).ok())
    {
        Some(file) => file.qwen,
        None => QwenSettings::default(),
    }
}

/// Load the saved Qwen settings (missing/corrupt file → defaults).
pub fn load_qwen_settings() -> QwenSettings {
    match settings_path() {
        Ok(p) => load_from(&p),
        Err(_) => QwenSettings::default(),
    }
}

/// Save the Qwen settings to an explicit path (atomic write: tmp + rename).
fn save_to(path: &std::path::Path, settings: &QwenSettings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create settings dir {:?}: {e}", parent))?;
    }
    let json = serde_json::to_string_pretty(&SettingsFile {
        qwen: settings.clone(),
    })
    .map_err(|e| format!("cannot serialize settings: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("cannot write settings: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("cannot save settings: {e}"))
}

/// Save the Qwen settings (atomic write: tmp file + rename).
pub fn save_qwen_settings(settings: &QwenSettings) -> Result<(), String> {
    let path = settings_path()?;
    save_to(&path, settings)
}

/// Apply a user edit to the stored settings.
///
/// - `provider` / `api_base` / `model`: `Some("")` (or blank) clears the
///   field so the per-provider default applies; `Some(value)` sets it.
/// - `api_key`: blank/empty keeps the current key; a non-blank value
///   replaces it. (The UI pre-fills the masked key as a placeholder, so an
///   untouched field round-trips as "keep".)
pub fn apply_qwen_edit(current: &mut QwenSettings, edit: &QwenSettings) {
    let clean = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    if edit.provider.is_some() {
        current.provider = clean(&edit.provider);
    }
    if edit.api_base.is_some() {
        current.api_base = clean(&edit.api_base);
    }
    if edit.model.is_some() {
        current.model = clean(&edit.model);
    }
    if let Some(key) = clean(&edit.api_key) {
        current.api_key = Some(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_api_key_shapes() {
        assert_eq!(mask_api_key("short"), "••••");
        assert_eq!(mask_api_key("12345678"), "••••");
        let masked = mask_api_key("sk-1234567890abcdef");
        assert!(masked.starts_with("sk-"));
        assert!(masked.ends_with("cdef"));
        assert!(!masked.contains("1234567890"));
    }

    #[test]
    fn apply_edit_semantics() {
        let mut cur = QwenSettings {
            provider: Some("dashscope".into()),
            api_key: Some("sk-old".into()),
            api_base: Some("http://old".into()),
            model: Some("m-old".into()),
        };
        // Blank key keeps the current one; blank base/model clear them.
        apply_qwen_edit(
            &mut cur,
            &QwenSettings {
                provider: Some("openai".into()),
                api_key: Some("   ".into()),
                api_base: Some("".into()),
                model: Some("".into()),
            },
        );
        assert_eq!(cur.provider.as_deref(), Some("openai"));
        assert_eq!(cur.api_key.as_deref(), Some("sk-old"));
        assert!(cur.api_base.is_none());
        assert!(cur.model.is_none());
        // Non-blank key replaces; None leaves fields untouched.
        apply_qwen_edit(
            &mut cur,
            &QwenSettings {
                provider: None,
                api_key: Some("sk-new".into()),
                api_base: None,
                model: None,
            },
        );
        assert_eq!(cur.api_key.as_deref(), Some("sk-new"));
        assert_eq!(cur.provider.as_deref(), Some("openai"));
    }

    #[test]
    fn view_masks_key() {
        let s = QwenSettings {
            provider: None,
            api_key: Some("sk-1234567890abcdef".into()),
            api_base: None,
            model: None,
        };
        let view = QwenSettingsView::from_settings(&s);
        assert_eq!(view.provider, "dashscope");
        assert!(view.has_api_key);
        assert!(view.api_key_masked.unwrap().ends_with("cdef"));
        let empty = QwenSettingsView::from_settings(&QwenSettings::default());
        assert!(!empty.has_api_key);
        assert!(empty.api_key_masked.is_none());
    }

    #[test]
    fn save_load_roundtrip() {
        // Explicit temp path (no HOME mutation — other tests read settings
        // from the real home dir in parallel).
        let dir = std::env::temp_dir().join(format!("ps-settings-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("settings.json");
        let s = QwenSettings {
            provider: Some("openai".into()),
            api_key: Some("sk-test-key-123456".into()),
            api_base: Some("http://127.0.0.1:9999/v1".into()),
            model: Some("qwen-image-edit".into()),
        };
        save_to(&path, &s).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded, s);
        // Corrupt file falls back to defaults.
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(load_from(&path), QwenSettings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
