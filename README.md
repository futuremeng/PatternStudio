# PatternStudio

Image-to-layered-SVG tracing engine built on [VTracer 1.0](https://github.com/vtracer/vtracer).

## Project Background

PatternStudio evolved from **PenPaw**, an early prototype that explored vectorizing
hand-drawn patterns and traditional Chinese decorative motifs (团花, 回纹, 卷草, 饕餮,
冰裂纹) into editable layered SVG. The PenPaw codebase (v0.0.x) established the core
architecture — a `LayeredDoc` IR, VTracer-based tracing pipeline, and procedural
pattern generators — but was limited to a single CLI binary with 5 pattern types and
no API surface.

In **v0.1.0** the project was refactored and renamed to **PatternStudio** to better
reflect its broader scope:

| Aspect | PenPaw (v0.0.x) | PatternStudio (v0.1.0) |
|--------|-----------------|------------------------|
| Crates | 5 (penpaw-*) | 6 (patternstudio-*) |
| Pattern types | 5 | 8 (+ cloud, wave, greek_key) |
| CLI commands | 2 (trace, pattern) | 5 (trace, pattern, list, serve, convert) |
| API | None | axum REST (5 endpoints) |
| Layer separation | Manual | Auto color-based (`--layers N`) |
| Transforms | None | `--scale`, `--rotate`, `--flip` |
| Output options | SVG only | SVG + JSON stats + PNG preview + browser open |
| Batch | None | Directory input + `--quiet` |
| Tests | 9 | 19 |
| Binary size | ~3MB | 6.2MB (full features) |

The rename also clarified the naming convention: all types, crates, and binaries now
use the `patternstudio` prefix, eliminating the "pen/paw" metaphor that no longer
matched the product's focus on **pattern design and layered vector output**.

## Refactoring Summary

The v0.1.0 refactor (28 commits) delivered:

1. **Architecture** — Clean 6-crate workspace with clear separation of concerns
   (core IR → tracer → SVG writer → patterns → CLI → server)
2. **Feature expansion** — From 5 to 8 pattern types, from 2 to 5 CLI commands
3. **API layer** — Full axum REST server for web/desktop integration
4. **Usability** — 17 new CLI flags for transforms, output control, and scripting
5. **Quality** — 19 tests, CI workflow, Makefile, CHANGELOG, LICENSE
6. **Performance** — `--optimize` flag reduces SVG file size by ~53%

## Roadmap

| Phase | Status | Highlights |
|-------|--------|------------|
| **v0.1.0** — Core Engine | ✅ Done | VTracer pipeline, 8 patterns, CLI, REST API, 19 tests |
| **v0.2.0** — Distribution | 🔜 Next | crates.io publication, Homebrew tap, cross-platform release (macOS/Linux/Windows) |
| **v0.3.0** — Web UI | ✅ Done | React frontend + API integration, drag-and-drop tracing, layer editor |
| **v0.4.0** — Semantic Layers | 📋 Planned | VTracer segmentation for semantic layer separation (not just color-based) |
| **v0.5.0** — Plugin System | 💡 Idea | WASM plugin API for custom pattern generators and post-processors |
| **v1.0.0** — Stable | 💡 Idea | API stability guarantee, Python bindings (PyO3), WASM bindings |

### v0.2.0 — Distribution (Next)

- [ ] Publish `patternstudio` to crates.io
- [ ] Create Homebrew tap (`brew install patternstudio`)
- [ ] GitHub Actions release workflow (auto-tag → build → upload binaries)
- [ ] Cross-compilation: Linux (x86_64, aarch64), macOS (x86_64, aarch64), Windows (x86_64)
- [ ] `cargo install patternstudio` support

### v0.3.0 — Web UI

- [x] React + Vite frontend served by the axum server
- [x] Drag-and-drop image upload with live preview
- [x] Layer panel: show/hide, recolor, delete (rename/reorder pending)
- [x] Parameter controls mapped to trace options
- [x] Pattern gallery with live regeneration

### v0.4.0 — Semantic Layers

- [ ] Integrate VTracer segmentation (superpixel-based)
- [ ] Semantic layer naming (e.g., "sky", "ground", "foreground")
- [ ] `--semantic-layers` flag as alternative to color-based `--layers`
- [ ] Layer metadata: area, centroid, bounding box

## Features

- **VTracer 1.0 core** — full tracing pipeline (segment → color fit → compose → curve fit)
- **VT2-style parameters** — high-level `resolution` / `simplify` controls
- **Auto layer separation** — `--layers N` groups shapes by color into semantic layers
- **Layered SVG output** — semantic `<g>` layers with editability metadata
- **8 pattern generators** — rosette, meander, scroll, taotie, ice_crack, cloud, wave, greek_key
- **Transforms** — `--scale`, `--rotate`, `--flip` (h/v/both)
- **Batch processing** — trace entire directories
- **REST API** — axum server for web/desktop integration
- **CLI** — single binary for all operations
- **Scripting** — `--json` output, `--quiet` mode, `--no-background` transparent SVGs

## Quick Start

```bash
# Build
cargo build --release

# Trace a single image
cargo run --bin patternstudio -- trace input.png -o output.svg

# Trace with auto layer separation (5 layers by color)
cargo run --bin patternstudio -- trace input.png -o out.svg --layers 5

# Trace with full options
cargo run --bin patternstudio -- trace input.png -o out.svg \
    --preset poster --mode spline --simplify low --resolution medium \
    --max-colors 16 --color-precision 4 --filter-speckle 2

# Trace with transparent background + JSON stats
cargo run --bin patternstudio -- trace input.png -o out.svg \
    --layers 3 --no-background --json

# Batch trace a directory (quiet mode)
cargo run --bin patternstudio -- trace ./images/ -o ./svg_output/ --quiet

# Generate a pattern and open in browser
cargo run --bin patternstudio -- pattern -k rosette -n 8 -z 800 -o rosette.svg --open

# List available patterns
cargo run --bin patternstudio -- list

# Convert/re-trace with different settings
cargo run --bin patternstudio -- convert input.png -o out.svg --simplify high --layers 5

# Start API server
cargo run --bin patternstudio-server
# → http://localhost:8000/api/v1/health
```

## Web UI

```bash
# Build the frontend (served by the API server)
make web          # cd web && npm install && npm run build

# Start the server (serves UI + API on http://localhost:8000)
make web-serve    # cargo run --bin patternstudio-server

# Development with hot reload (Vite dev server proxies /api to :8000)
make web-dev      # then open http://localhost:1421
```

Features: drag-and-drop image upload, trace parameter panel (preset / resolution / simplify / auto layers), layer editor (show/hide, recolor, delete, download SVG), and a pattern generator tab with live preview.

## Desktop App (Tauri)

The same React frontend runs in a Tauri window; tracing happens in-process (no server needed).

```bash
make desktop      # build frontend + desktop binary
make desktop-run  # run it

# Hot-reload development (Vite + Tauri watcher)
make desktop-dev
```

Debug the Rust side from VS Code: pick **"Debug Desktop (Tauri)"** in the Run and Debug panel (breakpoints work in `desktop/src/lib.rs`).

## CLI Options

### `trace` command

| Flag | Description |
|------|-------------|
| `-o, --output` | Output SVG path (or directory for batch) |
| `-r, --resolution` | coarse / medium / fine |
| `-s, --simplify` | minimal / low / medium / high / maximum |
| `--preset` | bw / poster / photo |
| `--mode` | pixel / polygon / spline |
| `--max-colors` | Max colors (0 = unlimited) |
| `--color-precision` | Color quantization bits (1-8) |
| `--filter-speckle` | Speckle filter size |
| `-l, --layers` | Auto-separate into N color layers |
| `--no-background` | Remove background layer (transparent) |
| `--threshold` | Background removal threshold (0-255) |
| `--scale` | Scale factor (0.5 = half, 2.0 = double) |
| `--rotate` | Rotate degrees (0-360) |
| `--flip` | Flip: none / h / v / both |
| `--preview` | Save PNG preview alongside SVG |
| `--optimize` | Round coordinates to reduce file size |
| `--open` | Open output in browser |
| `--json` | Output JSON stats to stdout |
| `-q, --quiet` | Suppress progress output |

### `pattern` command

| Flag | Description |
|------|-------------|
| `-k, --kind` | Pattern type (see list) |
| `-o, --output` | Output SVG path |
| `-n, --n-units` | Number of repeating units |
| `-c, --complexity` | Complexity 0-100 |
| `-z, --size` | Output size in pixels |
| `--seed` | Random seed for reproducible patterns |
| `--color` | Override stroke color (hex, e.g. `#ff0000`) |
| `--open` | Open output in browser |

### `convert` command

| Flag | Description |
|------|-------------|
| `-o, --output` | Output SVG path |
| `-s, --simplify` | Simplification level |
| `--mode` | Fit mode |
| `--max-colors` | Max colors |
| `-l, --layers` | Auto-separate layers |
| `--open` | Open output in browser |

## API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/health` | Health check |
| GET | `/api/v1/patterns` | List available pattern types |
| POST | `/api/v1/uploads` | Upload image (multipart) |
| POST | `/api/v1/trace` | Trace image → SVG |
| POST | `/api/v1/pattern` | Generate pattern SVG |

### Trace Request

```json
{
  "file_path": "/path/to/image.png",
  "resolution": "medium",
  "simplify": "low",
  "preset": "poster",
  "layers": 5
}
```

### Trace Response

```json
{
  "svg": "<svg ...>...</svg>",
  "width": 477,
  "height": 477,
  "shape_count": 2985,
  "layer_count": 5
}
```

### Pattern Request

```json
{
  "kind": "cloud",
  "n_units": 4,
  "complexity": 70,
  "size": 800
}
```

## Pattern Types

| Kind | Description |
|------|-------------|
| `rosette` | 团花 - circular floral rosette |
| `meander` | 回纹 - Greek key / thunder pattern |
| `scroll` | 卷草 - scrolling vine |
| `taotie` | 饕餮 - taotie mask |
| `ice_crack` | 冰裂纹 - ice crack / crazed glaze |
| `cloud` | 云纹 - auspicious cloud |
| `wave` | 水纹 - water / wave pattern |
| `greek_key` | 希腊回纹 - continuous key border |

## Architecture

```
crates/
├── patternstudio-core/       # IR (LayeredDoc), Config, Pipeline
├── patternstudio-tracer/     # VTracer integration, MaskFrontend, image I/O
├── patternstudio-svg/        # Layered SVG writer
├── patternstudio-patterns/   # Procedural pattern generators (8 types)
├── patternstudio-cli/        # CLI binary (`patternstudio`)
├── patternstudio-server/     # REST API server (`patternstudio-server`)
├── patternstudio-py/         # Python bindings (PyO3)
└── patternstudio-wasm/       # WASM bindings (wasm-bindgen) + web demo
```

## VTracer Presets

| Preset | Use Case |
|--------|----------|
| `bw` | Black & white line art |
| `poster` | Flat color illustrations (default) |
| `photo` | Photographic input |

## Fit Modes

| Mode | Description |
|------|-------------|
| `pixel` | Exact pixel-lattice polyline |
| `polygon` | Douglas-Peucker polygon |
| `spline` | Cubic Bézier curves (default) |

## Layer Separation

Use `--layers N` (or `"layers": N` in API) to auto-separate shapes into color-based layers:

- Most common color → `background` layer
- Other colors → `color-XXXXXX` layers
- Overflow (beyond N) → merged into `other` layer

All shapes are preserved — no data loss.

## Development

```bash
make build      # cargo build --workspace
make test       # cargo test --workspace (19 tests)
make release    # cargo build --release
make clippy     # cargo clippy --workspace
make fmt        # cargo fmt --all
```

## Feature Summary

| Category | Features |
|----------|----------|
| **Tracing** | VTracer 1.0, 3 presets (bw/poster/photo), 3 fit modes, full config |
| **Layers** | Auto color-based separation, `--no-background`, editability metadata |
| **Transforms** | `--scale`, `--rotate`, `--flip` (h/v/both) |
| **Patterns** | 8 types: rosette, meander, scroll, taotie, ice_crack, cloud, wave, greek_key |
| **Output** | Layered SVG, `--json` stats, `--open` browser preview |
| **Batch** | Directory input, `--quiet` mode |
| **API** | axum REST: /trace, /pattern, /patterns, /uploads, /health |
| **CLI** | trace, pattern, list, serve, convert commands |
| **Binary** | 2.5MB release, single static binary |

## License

MIT OR Apache-2.0