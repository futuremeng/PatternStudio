# Changelog

All notable changes to PatternStudio are documented in this file.

## [Unreleased]

### Added

**Smart Tracing**
- Smart Preset: image analysis (color count, bimodality, texture, lighting) →
  trial-trace candidate families (line art / bw / color / gradient / low-poly)
  → best parameter set, with per-family scoring criteria
- Real-ESRGAN (anime 4x) preprocess with iteration control, on-disk cache, and
  input downscaling for large photos
- 5 layering strategies: color, complexity, hybrid, spatial, and `auto`
  (intent-driven decision with human-readable reason)
- L0/L1/L2 multi-level profiles: background coarsened, details refined,
  middle layers kept verbatim (topology preserved)

**Performance**
- VTracer `Session` caching: segmentation runs once per (source, segmentation
  params); interactive parameter tweaks re-run only the cheap downstream stages
- In-flight trace preemption: a new interactive request cancels the older one
  between pipeline batches; background (pre-warm) requests yield with 409 busy
- Cancellable layering: the `auto` strategy's per-shape curve re-fit polls the
  request's cancel token, so a long layering run aborts at the next shape
  instead of running to completion (server + desktop)
- Project batch tracing with SSE progress stream and cancellation (web + desktop)
- Spatial layering (`by_spatial`) uses an x-axis sweep line instead of an
  O(n²) pair scan: same clusters, but sparse motif layouts only compare local
  x-neighbors

**UI**
- Tauri desktop app: the same React frontend with in-process tracing
  (no server needed), recent projects, native file dialogs
- Web UI: smart preset panel, preprocess modal with iteration comparison,
  project browser with batch trace, layer profiles in the layer panel
- Layer panel: an abort button while layering is running — auto layering
  re-fits every shape and can take a long time; cancelling keeps the previous
  result on screen

**Architecture**
- New `patternstudio-engine` crate: the trace pipeline (params → vtracer
  config → session → layering → profiles → SVG, plus preemption) is now shared
  by the web server and the desktop app instead of duplicated

### Changed

- Server binds to `127.0.0.1` by default; `HOST=0.0.0.0` opts into network
  access, which disables the `file_path` trace source and restricts project
  folders to the home directory
- `IdealParams::default()` is now derived from `SmartParams::default()`
  (single source of truth for the web editor defaults)
- `--optimize` only rounds numbers inside path `d="..."` attributes (other
  attributes keep their precision) and is UTF-8 safe
- Desktop byte transfer uses `Uint8Array` directly over Tauri IPC instead of
  number arrays
- Waiting for a preempted trace to release its session now blocks on the
  mutex (`lock_session`) instead of polling `try_lock` every 20 ms; a
  poisoned lock (panicked predecessor) is recovered instead of spinning
- `ImageEditorPage.jsx` (~1000 lines) split into focused modules:
  `editorUtils.js` (pure helpers), `hooks/usePatchEditing.js` (patch
  selection & dragging), `hooks/useSmartPreset.js` (smart tuning), and
  `components/SmartBanner.jsx` (decision banner)

### Fixed

- `refine_shape` no longer panics if an optimizer pass drops a shape (falls
  back to the pre-optimization geometry)
- `projects.json` is written atomically (tmp file + rename)
- `projects.json` location is cross-platform (`HOME` / `USERPROFILE`)
- Removed debug `eprintln!` from library code (`apply_profiles`)
- Separated layers keep the original canvas: a layer holds a subset of the
  shapes, not a crop, so layer thumbnails, layer exports (`导出选中图层` /
  ZIP), patch-edit and advanced-editor snapshots and the layered preview are
  all emitted on the traced canvas size instead of shrinking to the shapes'
  bounding box. The advanced editor's shrink-to-content button is now a
  view-only crop that no longer leaks into any export

### Removed

- CLI `serve` subcommand (it was a stub; use the `patternstudio-server` binary)
- `convert` no longer silently copies SVG input; it now fails with a clear
  message (re-tracing requires a raster source)

## [0.1.0] - 2026-09-11

### Added

**Core Engine**
- VTracer 1.0 integration with full tracing pipeline
- VT2-style high-level parameters (resolution, simplify)
- Layered SVG output with semantic `<g>` layers and editability metadata
- Auto color-based layer separation (`--layers N`)
- Background removal (`--no-background`, `--threshold`)

**CLI Commands**
- `trace` — image to layered SVG (single file or batch directory)
- `pattern` — generate procedural pattern SVGs
- `list` — list available pattern types
- `serve` — start API server
- `convert` — re-trace with different settings

**Trace Options**
- `--preset` (bw/poster/photo), `--mode` (pixel/polygon/spline)
- `--max-colors`, `--color-precision`, `--filter-speckle`
- `--layers N` — auto color-based layer separation
- `--no-background`, `--threshold` — transparent background
- `--scale`, `--rotate`, `--flip` — SVG transforms
- `--json` — structured stats output for scripting
- `--quiet` — suppress progress output
- `--open` — open in browser after tracing

**Pattern Generators (8 types)**
- rosette (团花), meander (回纹), scroll (卷草), taotie (饕餮)
- ice_crack (冰裂纹), cloud (云纹), wave (水纹), greek_key (希腊回纹)
- Options: `--size`, `--color`, `--seed`, `--n-units`, `--complexity`

**REST API (axum)**
- `GET /api/v1/health` — health check
- `GET /api/v1/patterns` — list pattern types
- `POST /api/v1/uploads` — upload image
- `POST /api/v1/trace` — trace image (supports layers)
- `POST /api/v1/pattern` — generate pattern (supports size)

**Infrastructure**
- Rust workspace with 6 crates
- 19 unit + integration tests
- GitHub Actions CI (build, test, clippy, fmt)
- Makefile with convenience targets
- 2.5MB release binary (LTO + strip)