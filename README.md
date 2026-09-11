# PatternStudio

Image-to-layered-SVG tracing engine built on [VTracer 1.0](https://github.com/vtracer/vtracer).

## Features

- **VTracer 1.0 core** — full tracing pipeline (segment → color fit → compose → curve fit)
- **VT2-style parameters** — high-level `resolution` / `simplify` controls
- **Auto layer separation** — `--layers N` groups shapes by color into semantic layers
- **Layered SVG output** — semantic `<g>` layers with editability metadata
- **8 pattern generators** — rosette, meander, scroll, taotie, ice_crack, cloud, wave, greek_key
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
└── patternstudio-server/     # REST API server (`patternstudio-server`)
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

## License

MIT OR Apache-2.0