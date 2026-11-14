# Changelog

All notable changes to PatternStudio are documented in this file.

## [Unreleased]

### Added

**Context menus & drag import**
- Right-click menus on project and image cards (Open / Remove) with
  keyboard-dismissable, focus-trapped popover
- Dragging images from Finder onto the desktop window imports them into the
  open project and refreshes the grid in place

**Desktop integration & accessibility**
- Native macOS menu bar in the Tauri app (About / Settings… / Open Folder… /
  Undo / Redo / Toggle Appearance / Help); custom items bridge to the UI via a
  `menu-command` event, predefined items stay native
- Global shortcuts: ⌘, opens Settings anywhere, ⌘O opens a project; menu
  Undo/Redo reach the SVG editor's undo stack
- Settings dialog mounted app-wide (no longer owned by the header)
- Desktop window geometry is remembered and restored; the window title names
  the open project (Mac convention)
- Dialogs (settings, confirm) trap Tab focus and expose role=dialog /
  aria-modal; `prefers-reduced-motion` is honored globally; header and empty
  states swap emoji for the app's 16px line icons

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
- Exported layers: their own list at the bottom of the layer panel, each with
  its own export button; clicking a layer's picture asks whether it becomes the
  editing object of the editing area (preview, patch editing and the advanced
  editor then work on that layer alone), and clicking it again — or "Back to
  full result" — switches back to the full layered result
- Editing-object-consistent export: "Download SVG" now saves what the editing
  area shows — the selected snapshot when one is picked (named
  `<image>-<layer>.svg`) — instead of always the full layered result
- Patch editing is reversible: moved or deleted shapes are marked as unapplied
  in the preview and can be applied, discarded or stepped back one edit at a
  time (⌘/Ctrl+Z); switching the editing object, entering the advanced editor
  or exporting applies them first so nothing on screen is silently dropped
- Confirmation before an irreversible action: removing a project from the list
  and dropping an exported snapshot both ask first, through a shared
  `ConfirmModal` (also used by the layer panel's editing-object switch)
- Discoverability: the exported-layer cards name their click action on hover,
  the patch-select tool explains its gestures once, smart tuning shows its
  busy state in the preview, and the advanced toolbar notes that the parameter
  panel is hidden there
- Narrow screens: the editor's parameter and layer panels fold into drawers
  opened from the header, with a scrim that closes them again

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
- Layer naming unified across the UI: snapshots taken from the canvas (patch
  "save as layer" and the advanced editor alike) are all "snapshot N", the list
  is "Snapshots", and the layered-result export says "Export checked layers"
  to keep the two apart
- Loading is concurrent where it was serial: project thumbnails on the project
  list, cached-result lookups for the split previews and for saved-config
  thumbnails; the project list now shows a skeleton instead of an empty state
  while it loads, and reports a failed project removal
- Accessibility: viewer and lineage controls label themselves in the active
  language instead of hardcoded English, lineage rows and nodes are reachable
  by keyboard, and every focusable element gets a visible focus ring
- VTracer is vendored in-tree at `crates/vtracer` (fork commit `13435cf`) so a
  plain clone + build works; CI no longer fetches and patches a sibling
  checkout, and the server crate dropped its unused direct `vtracer` dep

### Fixed

- `save-image` writes atomically (stage tmp file in the project folder +
  rename): a crash mid-save can no longer leave a truncated PNG, and reads
  running concurrently never observe partial content; tmp left by a crash is
  swept at server startup

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