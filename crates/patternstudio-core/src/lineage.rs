//! Lineage (provenance) tracking for folder-based projects.
//!
//! When a folder is opened as a project, every image in it is an *original*.
//! Images produced by later stages (enhance / redraw / ...) are persisted and
//! recorded in a sidecar file (`.patternstudio/lineage.json`) inside the
//! project folder, so the derivation tree survives re-opening the folder:
//!
//! - Auto-generated iterations are stored under `.patternstudio/generated/`
//!   (a hidden subfolder, so they never pollute the top-level image list)
//!   with `confirmed = false`.
//! - "Save as new image" copies the iteration into the project root and
//!   marks the entry `confirmed = true` with its `confirmed_file`.
//!
//! Projects created before this feature had no sidecar file; their derived
//! images follow the legacy `{base}_{enhanced|redrawn}_{n}.ext` naming
//! convention, which {@link infer_entries} reconstructs as confirmed entries.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Hidden per-project metadata directory (kept out of the image list).
pub const LINEAGE_DIR: &str = ".patternstudio";
/// Sidecar file name inside [`LINEAGE_DIR`].
pub const LINEAGE_FILE: &str = "lineage.json";
/// Subdirectory (inside [`LINEAGE_DIR`]) holding auto-generated iterations.
pub const GENERATED_DIR: &str = "generated";

/// Stage that produced an image. Extensible; the UI maps unknown kinds to a
/// generic label.
pub const KIND_ENHANCE: &str = "enhance";
pub const KIND_REDRAW: &str = "redraw";

/// One derived image and where it came from.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LineageEntry {
    /// The root original image (top-level filename in the project folder).
    pub root: String,
    /// Direct source image: an original/confirmed filename, or a generated
    /// filename (for iterations chained from an earlier unsaved iteration).
    pub parent: String,
    /// Producing stage: [`KIND_ENHANCE`] / [`KIND_REDRAW`] / future stages.
    pub kind: String,
    /// True once the user confirmed the image into the project folder.
    pub confirmed: bool,
    /// Project-root filename once confirmed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_file: Option<String>,
    /// ISO 8601 creation time (empty for legacy inferred entries).
    #[serde(default)]
    pub created_at: String,
    /// Stage-specific metadata (e.g. redraw prompt/seed).
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub meta: serde_json::Value,
}

/// The sidecar file: entries keyed by the generated filename (or, for legacy
/// inferred entries, by the top-level filename itself).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LineageFile {
    pub version: u32,
    pub entries: BTreeMap<String, LineageEntry>,
}

/// Full path of the sidecar file for a project folder.
pub fn lineage_path(folder: &Path) -> PathBuf {
    folder.join(LINEAGE_DIR).join(LINEAGE_FILE)
}

/// Directory holding auto-generated iterations for a project folder.
pub fn generated_dir(folder: &Path) -> PathBuf {
    folder.join(LINEAGE_DIR).join(GENERATED_DIR)
}

impl LineageFile {
    /// Load the sidecar file. When it is absent (or unreadable), fall back to
    /// inferring entries from the legacy naming convention over
    /// `top_level_images`, so pre-existing projects still show a tree.
    pub fn load(folder: &Path, top_level_images: &[String]) -> Self {
        let path = lineage_path(folder);
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(file) = serde_json::from_str::<LineageFile>(&data) {
                return file;
            }
        }
        Self {
            version: 1,
            entries: infer_entries(top_level_images),
        }
    }

    /// Insert or replace the entry under `key`.
    pub fn upsert(&mut self, key: &str, entry: LineageEntry) {
        self.version = 1;
        self.entries.insert(key.to_string(), entry);
    }

    /// Persist atomically (tmp file + rename) so a crash mid-write cannot
    /// leave a truncated lineage file behind.
    pub fn save(&self, folder: &Path) -> std::io::Result<()> {
        let dir = folder.join(LINEAGE_DIR);
        std::fs::create_dir_all(&dir)?;
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let tmp = dir.join(format!("{}.tmp", LINEAGE_FILE));
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, lineage_path(folder))
    }
}

/// Legacy naming convention: `{base}_{enhanced|redrawn}_{n}.ext`. The kind is
/// the second-to-last stem segment, so `my_enhanced_1.png` parses as base
/// `my` (disambiguated by the `{base}.ext` existence check in
/// {@link infer_entries}).
fn parse_legacy_name(name: &str) -> Option<(&str, &str)> {
    let (stem, _ext) = name.rsplit_once('.')?;
    let (before_n, n) = stem.rsplit_once('_')?;
    if n.is_empty() || !n.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (base, kind) = before_n.rsplit_once('_')?;
    if kind == "enhanced" {
        Some((base, KIND_ENHANCE))
    } else if kind == "redrawn" {
        Some((base, KIND_REDRAW))
    } else {
        None
    }
}

/// Reconstruct lineage entries for a legacy project (no sidecar file) from
/// the naming convention. An entry is created when `{base}.{ext}` exists as a
/// top-level image; roots are then resolved transitively so chained files
/// (`cat_enhanced_1_enhanced_1.png`) point at the true root (`cat.png`).
pub fn infer_entries(top_level_images: &[String]) -> BTreeMap<String, LineageEntry> {
    let set: std::collections::HashSet<&str> =
        top_level_images.iter().map(|s| s.as_str()).collect();
    let mut entries: BTreeMap<String, LineageEntry> = BTreeMap::new();
    for name in top_level_images {
        let Some((base, kind)) = parse_legacy_name(name) else {
            continue;
        };
        let ext = name.rsplit_once('.').map(|e| e.1).unwrap_or("");
        let base_name = format!("{base}.{ext}");
        if !set.contains(base_name.as_str()) {
            continue;
        }
        entries.insert(
            name.clone(),
            LineageEntry {
                root: base_name.clone(),
                parent: base_name,
                kind: kind.to_string(),
                confirmed: true,
                confirmed_file: Some(name.clone()),
                created_at: String::new(),
                meta: serde_json::Value::Null,
            },
        );
    }
    // Resolve roots transitively (parent may itself be a derived file).
    let keys: Vec<String> = entries.keys().cloned().collect();
    for key in keys {
        let parent = entries.get(&key).unwrap().parent.clone();
        let mut root = parent;
        let mut guard = 0;
        while let Some(parent_entry) = entries.get(&root) {
            root = parent_entry.root.clone();
            guard += 1;
            if guard > 32 {
                break; // cycle guard
            }
        }
        entries.get_mut(&key).unwrap().root = root;
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_legacy_names() {
        assert_eq!(
            parse_legacy_name("cat_enhanced_1.png").map(|(b, k)| (b, k)),
            Some(("cat", KIND_ENHANCE))
        );
        assert_eq!(
            parse_legacy_name("cat_redrawn_12.png").map(|(b, k)| (b, k)),
            Some(("cat", KIND_REDRAW))
        );
        // Not a legacy name.
        assert_eq!(parse_legacy_name("cat.png"), None);
        assert_eq!(parse_legacy_name("cat_enhanced_x.png"), None);
        assert_eq!(parse_legacy_name("cat_foo_1.png"), None);
        // Base may itself contain underscores.
        assert_eq!(
            parse_legacy_name("my_enhanced_1.png").map(|(b, k)| (b, k)),
            Some(("my", KIND_ENHANCE))
        );
    }

    #[test]
    fn infers_flat_entries() {
        let images = names(&["cat.png", "cat_enhanced_1.png", "cat_redrawn_2.png", "dog.jpg"]);
        let entries = infer_entries(&images);
        assert_eq!(entries.len(), 2);
        let e = &entries["cat_enhanced_1.png"];
        assert_eq!(e.root, "cat.png");
        assert_eq!(e.parent, "cat.png");
        assert_eq!(e.kind, KIND_ENHANCE);
        assert!(e.confirmed);
        assert_eq!(e.confirmed_file.as_deref(), Some("cat_enhanced_1.png"));
    }

    #[test]
    fn infers_chained_roots() {
        let images = names(&[
            "cat.png",
            "cat_enhanced_1.png",
            "cat_enhanced_1_enhanced_1.png",
        ]);
        let entries = infer_entries(&images);
        assert_eq!(entries["cat_enhanced_1_enhanced_1.png"].root, "cat.png");
        assert_eq!(
            entries["cat_enhanced_1_enhanced_1.png"].parent,
            "cat_enhanced_1.png"
        );
    }

    #[test]
    fn roundtrip_save_load() {
        let dir = std::env::temp_dir().join(format!("ps-lineage-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut file = LineageFile::default();
        file.upsert(
            "gen_1.png",
            LineageEntry {
                root: "cat.png".into(),
                parent: "cat.png".into(),
                kind: KIND_REDRAW.into(),
                confirmed: false,
                confirmed_file: None,
                created_at: "2026-01-01T00:00:00Z".into(),
                meta: serde_json::json!({ "prompt": "lineart", "seed": 42 }),
            },
        );
        file.save(&dir).unwrap();
        let loaded = LineageFile::load(&dir, &[]);
        assert_eq!(loaded.entries.len(), 1);
        let e = &loaded.entries["gen_1.png"];
        assert_eq!(e.kind, KIND_REDRAW);
        assert!(!e.confirmed);
        assert_eq!(e.meta["seed"], 42);
        let _ = std::fs::remove_dir_all(&dir);
    }
}