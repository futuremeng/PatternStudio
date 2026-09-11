# PatternStudio

Image-to-layered-SVG tracing engine built on [VTracer 1.0](https://github.com/vtracer/vtracer).

## Features

- **VTracer 1.0 core** — full tracing pipeline (segment → color fit → compose → curve fit)
- **VT2-style parameters** — high-level `resolution` / `simplify` controls
- **Layered SVG output** — semantic `<g>` layers with editability metadata
- **Pattern generators** — rosette, meander, scroll, taotie, ice_crack
- **Batch processing** — trace entire directories
- **REST API** — axum server for web/desktop integration
- **CLI** — single binary for all operations

## Quick Start

```bash
# Build
cargo build --release

# Trace a single image
cargo run --bin patternstudio -- trace input.png -o output.svg

# Trace with options
cargo run --bin patternstudio -- trace input.png -o out.svg \
    --preset poster --mode spline --simplify low --resolution medium

# Batch trace a directory
cargo run --bin patternstudio -- trace ./images/ -o ./svg_output/

# Generate a pattern
cargo run --bin patternstudio -- pattern -k rosette -n 8 -o rosette.svg

# Start API server
cargo run --bin patternstudio-server
# → http://localhost:8000/api/v1/health
```

## API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/health` | Health check |
| POST | `/api/v1/uploads` | Upload image (multipart) |
| POST | `/api/v1/trace` | Trace image → SVG |
| POST | `/api/v1/pattern` | Generate pattern SVG |

### Trace Request

```json
{
  "file_path": "/path/to/image.png",
  "resolution": "medium",
  "simplify": "low",
  "preset": "poster"
}
```

### Trace Response

```json
{
  "svg": "<svg ...>...</svg>",
  "width": 477,
  "height": 477,
  "shape_count": 2985,
  "layer_count": 1
}
```

## Architecture

```
crates/
├── patternstudio-core/       # IR (LayeredDoc), Config, Pipeline
├── patternstudio-tracer/     # VTracer integration, MaskFrontend, image I/O
├── patternstudio-svg/        # Layered SVG writer
├── patternstudio-patterns/   # Procedural pattern generators
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

## Development

```bash
cargo test          # Run all tests
cargo clippy        # Lint
cargo fmt           # Format
```

## License

MIT OR Apache-2.0