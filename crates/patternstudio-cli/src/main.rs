//! PatternStudio CLI: trace images to layered SVG.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use patternstudio_core::{LayerAssignment, PatternStudioConfig, PatternStudioPipeline, Resolution, Simplify};
use patternstudio_svg::LayeredWriter;
use patternstudio_tracer::load_image;
use vtracer::config::Preset;

#[derive(Parser)]
#[command(name = "patternstudio", version, about = "PatternStudio: image-to-layered-SVG tracing")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Trace an image to layered SVG
    Trace {
        /// Input image path (or directory for batch)
        input: PathBuf,
        /// Output SVG path (or directory for batch)
        #[arg(short, long, default_value = "output.svg")]
        output: PathBuf,
        /// Resolution preset
        #[arg(short = 'r', long, default_value = "medium")]
        resolution: String,
        /// Simplification level
        #[arg(short = 's', long, default_value = "low")]
        simplify: String,
        /// VTracer preset: bw/poster/photo
        #[arg(short, long, default_value = "poster")]
        preset: String,
        /// Fit mode: pixel/polygon/spline
        #[arg(long, default_value = "spline")]
        mode: String,
        /// Max colors (0 = unlimited)
        #[arg(long, default_value_t = 0)]
        max_colors: usize,
        /// Color precision (1-8 bits)
        #[arg(long, default_value_t = 4)]
        color_precision: i32,
        /// Speckle filter size
        #[arg(long, default_value_t = 2)]
        filter_speckle: usize,
        /// Auto-separate layers by color (0 = single layer, N = max N layers)
        #[arg(short = 'l', long, default_value_t = 0)]
        layers: usize,
        /// Remove background (make transparent)
        #[arg(long)]
        no_background: bool,
        /// Open output in browser after tracing
        #[arg(long)]
        open: bool,
        /// Output JSON stats to stdout (for scripting)
        #[arg(long)]
        json: bool,
        /// Suppress progress output
        #[arg(short, long)]
        quiet: bool,
    },
    /// Generate a pattern SVG
    Pattern {
        /// Pattern type: rosette/meander/scroll/taotie/ice_crack/cloud/wave/greek_key
        #[arg(short, long)]
        kind: String,
        /// Output SVG path
        #[arg(short, long, default_value = "pattern.svg")]
        output: PathBuf,
        /// Number of repeating units
        #[arg(short = 'n', long, default_value_t = 8)]
        n_units: u32,
        /// Complexity (0-100)
        #[arg(short, long, default_value_t = 50)]
        complexity: u8,
        /// Output size in pixels
        #[arg(short = 'z', long, default_value_t = 400)]
        size: u32,
        /// Random seed for reproducible patterns
        #[arg(long)]
        seed: Option<u64>,
        /// Override stroke color (hex, e.g. #333333)
        #[arg(long)]
        color: Option<String>,
        /// Open output in browser after generating
        #[arg(long)]
        open: bool,
    },
    /// List available pattern types
    List,
    /// Start the API server
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value_t = 8000)]
        port: u16,
    },
    /// Convert/re-trace an existing SVG with different settings
    Convert {
        /// Input SVG file
        input: PathBuf,
        /// Output SVG path
        #[arg(short, long, default_value = "output.svg")]
        output: PathBuf,
        /// Simplification level
        #[arg(short = 's', long, default_value = "medium")]
        simplify: String,
        /// Fit mode: pixel/polygon/spline
        #[arg(long, default_value = "spline")]
        mode: String,
        /// Max colors (0 = unlimited)
        #[arg(long, default_value_t = 0)]
        max_colors: usize,
        /// Auto-separate layers by color
        #[arg(short = 'l', long, default_value_t = 0)]
        layers: usize,
        /// Open output in browser
        #[arg(long)]
        open: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Trace { input, output, resolution, simplify, preset, mode, max_colors, color_precision, filter_speckle, layers, no_background, open, json, quiet } => {
            cmd_trace(&input, &output, &resolution, &simplify, &preset, &mode, max_colors, color_precision, filter_speckle, layers, no_background, open, json, quiet)
        }
        Command::Pattern { kind, output, n_units, complexity, size, seed, color, open } => {
            cmd_pattern(&kind, &output, n_units, complexity, size, seed, color.as_deref(), open)
        }
        Command::List => {
            cmd_list()
        }
        Command::Serve { port } => {
            cmd_serve(port)
        }
        Command::Convert { input, output, simplify, mode, max_colors, layers, open } => {
            cmd_convert(&input, &output, &simplify, &mode, max_colors, layers, open)
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => { eprintln!("error: {e}"); ExitCode::FAILURE }
    }
}

fn cmd_trace(
    input: &PathBuf, output: &PathBuf,
    resolution: &str, simplify: &str, preset: &str,
    mode: &str, max_colors: usize, color_precision: i32, filter_speckle: usize,
    layers: usize, no_background: bool, open: bool, json: bool, quiet: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Batch mode: input is a directory
    if input.is_dir() {
        let out_dir = output.clone();
        std::fs::create_dir_all(&out_dir)?;
        let exts = ["png", "jpg", "jpeg", "webp", "bmp"];
        let mut files: Vec<PathBuf> = std::fs::read_dir(input)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|e| exts.contains(&e.to_string_lossy().to_lowercase().as_str())).unwrap_or(false))
            .collect();
        files.sort();

        eprintln!("Batch mode: {} images", files.len());
        let mut ok = 0;
        for (i, file) in files.iter().enumerate() {
            let stem = file.file_stem().unwrap_or_default().to_string_lossy();
            let out_svg = out_dir.join(format!("{stem}.svg"));
            match trace_single(file, &out_svg, resolution, simplify, preset, mode, max_colors, color_precision, filter_speckle, layers, no_background, false, false, true) {
                Ok(()) => { ok += 1; }
                Err(e) => eprintln!("  [{}] FAILED: {e}", file.display()),
            }
            if (i + 1) % 10 == 0 {
                eprintln!("  Progress: {}/{}", i + 1, files.len());
            }
        }
        eprintln!("Batch done: {ok}/{} succeeded", files.len());
        return Ok(());
    }

    // Single file mode
    trace_single(input, output, resolution, simplify, preset, mode, max_colors, color_precision, filter_speckle, layers, no_background, open, json, quiet)
}

fn trace_single(
    input: &PathBuf, output: &PathBuf,
    resolution: &str, simplify: &str, preset: &str,
    mode: &str, max_colors: usize, color_precision: i32, filter_speckle: usize,
    layers: usize, no_background: bool, open: bool, json: bool, quiet: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if !quiet {
        eprintln!("Loading: {}", input.display());
    }
    let img = load_image(input)?;
    if !quiet {
        eprintln!("Size: {}x{}", img.width, img.height);
    }

    let p = match preset {
        "bw" => Preset::Bw,
        "photo" => Preset::Photo,
        _ => Preset::Poster,
    };
    let mut vt_config = vtracer::Config::from_preset(p);
    vt_config.mode = match mode {
        "pixel" => vtracer::config::FitMode::Pixel,
        "polygon" => vtracer::config::FitMode::Polygon,
        _ => vtracer::config::FitMode::Spline,
    };
    if max_colors > 0 {
        vt_config.max_colors = Some(max_colors);
    }
    vt_config.color_precision = color_precision;
    vt_config.filter_speckle = filter_speckle;

    let config = PatternStudioConfig {
        resolution: parse_resolution(resolution),
        simplify: parse_simplify(simplify),
        vtracer: vt_config,
    };

    if !quiet {
        eprintln!("Tracing...");
    }
    let pipeline = PatternStudioPipeline::new(&config)?;

    let layered = if layers > 1 {
        pipeline.trace_layered_auto(&img, layers)?
    } else {
        let assignment = LayerAssignment::single_layer();
        pipeline.trace_layered(&img, &assignment)?
    };

    // Remove background layer if requested
    let layered = if no_background {
        let mut l = layered;
        l.layers.retain(|layer| layer.role != patternstudio_core::LayerRole::Background);
        l
    } else {
        layered
    };

    if !quiet {
        eprintln!("Done: {} shapes in {} layer(s)", layered.total_shapes(), layered.layers.len());
    }

    let svg = LayeredWriter::write(&layered);
    std::fs::write(output, &svg)?;
    if !quiet {
        eprintln!("Saved: {}", output.display());
    }

    if json {
        let stats = serde_json::json!({
            "input": input.display().to_string(),
            "output": output.display().to_string(),
            "width": layered.width,
            "height": layered.height,
            "shape_count": layered.total_shapes(),
            "layer_count": layered.layers.len(),
            "layers": layered.layers.iter().map(|l| serde_json::json!({
                "name": l.name,
                "role": l.role.to_string(),
                "shapes": l.shapes.len(),
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&stats)?);
    }

    if open {
        open_in_browser(output);
    }
    Ok(())
}

fn cmd_pattern(
    kind: &str, output: &PathBuf, n_units: u32, complexity: u8, size: u32,
    _seed: Option<u64>, color: Option<&str>, open: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("Generating pattern: {kind} (n={n_units}, complexity={complexity}, size={size})");
    let mut svg = patternstudio_patterns::generate_sized(kind, n_units, complexity, size)?;
    // Override stroke color if specified
    if let Some(c) = color {
        let c = c.trim_start_matches('#');
        if c.len() == 6 {
            svg = svg.replace("#333", &format!("#{c}")).replace("#666", &format!("#{c}"));
        }
    }
    std::fs::write(output, &svg)?;
    eprintln!("Saved: {}", output.display());
    if open {
        open_in_browser(output);
    }
    Ok(())
}

fn cmd_list() -> Result<(), Box<dyn std::error::Error>> {
    println!("Available pattern types:");
    for (kind, desc) in patternstudio_patterns::list_kinds() {
        println!("  {kind:<12} {desc}");
    }
    Ok(())
}

fn parse_resolution(s: &str) -> Resolution {
    match s { "coarse" => Resolution::Coarse, "fine" => Resolution::Fine, _ => Resolution::Medium }
}

fn parse_simplify(s: &str) -> Simplify {
    match s {
        "minimal" => Simplify::Minimal,
        "medium" => Simplify::Medium,
        "high" => Simplify::High,
        "maximum" => Simplify::Maximum,
        _ => Simplify::Low,
    }
}

fn cmd_convert(
    input: &PathBuf, output: &PathBuf,
    simplify: &str, mode: &str, max_colors: usize, layers: usize, open: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("Converting: {}", input.display());

    // Load SVG as image (render to bitmap then re-trace)
    // For now, we read the SVG and use the image crate to rasterize it
    // This is a simplified approach - in production you'd use a proper SVG renderer
    let svg_content = std::fs::read_to_string(input)?;
    let _ = svg_content; // SVG content available for future use

    // Use the tracer to re-process: load as image if possible, otherwise just copy
    // For SVG input, we need to rasterize first. Use a simple approach:
    // If the file is actually an image (misnamed), trace it normally.
    // Otherwise, report that SVG re-tracing requires a rasterizer.
    let ext = input.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ext == "svg" {
        eprintln!("Note: SVG re-tracing requires rasterization. Using passthrough mode.");
        eprintln!("For best results, convert SVG to PNG first, then use `trace`.");
        std::fs::copy(input, output)?;
        eprintln!("Copied: {}", output.display());
    } else {
        // It's actually an image file - trace it
        trace_single(input, output, "medium", simplify, "poster", mode, max_colors, 4, 2, layers, false, open, false, false)?;
    }
    Ok(())
}

fn cmd_serve(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("Starting PatternStudio API server on port {port}...");
    std::env::set_var("PORT", port.to_string());
    // Delegate to the server binary's main logic
    // For now, just print instructions since we can't easily call another binary's main
    eprintln!("Use: cargo run --bin patternstudio-server");
    eprintln!("Or: PORT={port} ./target/release/patternstudio-server");
    Ok(())
}

fn open_in_browser(path: &PathBuf) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd").args(["/C", "start"]).arg(path).spawn();
    }
}
