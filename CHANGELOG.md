# Changelog

All notable changes to PatternStudio are documented in this file.

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