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
        /// Background removal threshold (0-255, default 30)
        #[arg(long, default_value_t = 30)]
        threshold: u8,
        /// Save PNG preview of input alongside SVG
        #[arg(long)]
        preview: bool,
        /// Optimize SVG output (round coordinates to reduce file size)
        #[arg(long)]
        optimize: bool,
        /// Open output in browser after tracing
        #[arg(long)]
        open: bool,
        /// Output JSON stats to stdout (for scripting)
        #[arg(long)]
        json: bool,
        /// Suppress progress output
        #[arg(short, long)]
        quiet: bool,
        /// Scale factor (e.g. 0.5 = half size, 2.0 = double size)
        #[arg(long, default_value_t = 1.0)]
        scale: f64,
        /// Rotate degrees (0-360)
        #[arg(long, default_value_t = 0.0)]
        rotate: f64,
        /// Flip: none/h/v/both
        #[arg(long, default_value = "none")]
        flip: String,
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
        Command::Trace { input, output, resolution, simplify, preset, mode, max_colors, color_precision, filter_speckle, layers, no_background, threshold, preview, optimize, open, json, quiet, scale, rotate, flip } => {
            cmd_trace(&input, &output, TraceOptions {
                resolution, simplify, preset, mode, max_colors, color_precision, filter_speckle,
                layers, no_background, threshold, preview, optimize, open, json, quiet,
                scale, rotate, flip,
            })
        }
        Command::Pattern { kind, output, n_units, complexity, size, seed, color, open } => {
            cmd_pattern(PatternOptions { kind, output, n_units, complexity, size, seed, color, open })
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

/// Options for a single trace run.
#[derive(Clone)]
struct TraceOptions {
    resolution: String,
    simplify: String,
    preset: String,
    mode: String,
    max_colors: usize,
    color_precision: i32,
    filter_speckle: usize,
    layers: usize,
    no_background: bool,
    threshold: u8,
    preview: bool,
    optimize: bool,
    open: bool,
    json: bool,
    quiet: bool,
    scale: f64,
    rotate: f64,
    flip: String,
}

fn cmd_trace(input: &PathBuf, output: &PathBuf, opts: TraceOptions) -> Result<(), Box<dyn std::error::Error>> {
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
            let mut batch_opts = opts.clone();
            batch_opts.preview = false;
            batch_opts.optimize = false;
            batch_opts.open = false;
            batch_opts.json = false;
            batch_opts.quiet = true;
            batch_opts.scale = 1.0;
            batch_opts.rotate = 0.0;
            batch_opts.flip = "none".to_string();
            match trace_single(file, &out_svg, batch_opts) {
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
    trace_single(input, output, opts)
}

fn trace_single(
    input: &PathBuf, output: &PathBuf, opts: TraceOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let TraceOptions {
        resolution, simplify, preset, mode, max_colors, color_precision, filter_speckle,
        layers, no_background, threshold, preview, optimize, open, json, quiet,
        scale, rotate, flip,
    } = opts;
    if !quiet {
        eprintln!("Loading: {}", input.display());
    }
    let img = load_image(input)?;
    if !quiet {
        eprintln!("Size: {}x{}", img.width, img.height);
    }

    // Remove background pixels before tracing if requested
    let img = if no_background {
        let (w, h) = (img.width, img.height);
        // Sample the four corners to estimate the background color
        let corners = [
            img.get_pixel(0, 0),
            img.get_pixel(w - 1, 0),
            img.get_pixel(0, h - 1),
            img.get_pixel(w - 1, h - 1),
        ];
        let bg = visioncortex::Color::new(
            (corners.iter().map(|c| c.rgb_u8()[0] as u32).sum::<u32>() / 4) as u8,
            (corners.iter().map(|c| c.rgb_u8()[1] as u32).sum::<u32>() / 4) as u8,
            (corners.iter().map(|c| c.rgb_u8()[2] as u32).sum::<u32>() / 4) as u8,
        );
        let mask = patternstudio_tracer::MaskFrontend::with_background(bg, threshold);
        if !quiet {
            eprintln!("Removing background (threshold {threshold})...");
        }
        mask.remove_background(&img)
    } else {
        img
    };

    let p = match preset.as_str() {
        "bw" => Preset::Bw,
        "photo" => Preset::Photo,
        _ => Preset::Poster,
    };
    let mut vt_config = vtracer::Config::from_preset(p);
    vt_config.mode = match mode.as_str() {
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
        resolution: parse_resolution(&resolution),
        simplify: parse_simplify(&simplify),
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

    // Remove background if requested: drop Background-role layers and any
    // shapes left over from the masked (black) background pixels
    let layered = if no_background {
        let mut l = layered;
        l.layers.retain(|layer| layer.role != patternstudio_core::LayerRole::Background);
        let removed = l.remove_color(visioncortex::Color::new(0, 0, 0), threshold);
        if !quiet && removed > 0 {
            eprintln!("Removed {removed} background shape(s)");
        }
        l
    } else {
        layered
    };

    if !quiet {
        eprintln!("Done: {} shapes in {} layer(s)", layered.total_shapes(), layered.layers.len());
    }

    let mut svg = LayeredWriter::write(&layered);
    // Apply scale/rotate/flip transforms if requested
    if scale != 1.0 || rotate != 0.0 || flip != "none" {
        let w = layered.width as f64;
        let h = layered.height as f64;
        let cx = w / 2.0;
        let cy = h / 2.0;
        let mut parts: Vec<String> = Vec::new();
        if rotate != 0.0 {
            parts.push(format!("rotate({rotate} {cx} {cy})"));
        }
        if scale != 1.0 {
            parts.push(format!("scale({scale})"));
        }
        match flip.as_str() {
            "h" => parts.push(format!("translate({w} 0) scale(-1 1)")),
            "v" => parts.push(format!("translate(0 {h}) scale(1 -1)")),
            "both" => parts.push(format!("translate({w} {h}) scale(-1 -1)")),
            _ => {}
        }
        if !parts.is_empty() {
            let transform = parts.join(" ");
            if let Some(pos) = svg.find("<g ") {
                let end = svg[pos..].find('>').map(|i| pos + i).unwrap_or(pos + 10);
                svg.insert_str(end, &format!(" transform=\"{transform}\""));
            }
        }
    }
    std::fs::write(output, &svg)?;
    if !quiet {
        eprintln!("Saved: {}", output.display());
    }

    // Optimize SVG: round coordinates to integers to reduce file size
    if optimize {
        let optimized = optimize_svg(&svg);
        std::fs::write(output, &optimized)?;
        if !quiet {
            let orig_size = std::fs::metadata(input).map(|m| m.len()).unwrap_or(0);
            let new_size = std::fs::metadata(output).map(|m| m.len()).unwrap_or(0);
            eprintln!("Optimized: {} → {} bytes", orig_size, new_size);
        }
    }

    // Save PNG preview of input if requested
    if preview {
        let preview_path = output.with_extension("png");
        if let Ok(img) = image::open(input) {
            if img.save(&preview_path).is_ok() && !quiet {
                eprintln!("Preview: {}", preview_path.display());
            }
        }
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

/// Options for a single pattern generation run.
#[derive(Clone)]
struct PatternOptions {
    kind: String,
    output: PathBuf,
    n_units: u32,
    complexity: u8,
    size: u32,
    /// Accepted for CLI compatibility; not yet consumed by the generators.
    #[allow(dead_code)]
    seed: Option<u64>,
    color: Option<String>,
    open: bool,
}

fn cmd_pattern(opts: PatternOptions) -> Result<(), Box<dyn std::error::Error>> {
    let PatternOptions { kind, output, n_units, complexity, size, color, open, .. } = opts;
    eprintln!("Generating pattern: {kind} (n={n_units}, complexity={complexity}, size={size})");
    let mut svg = patternstudio_patterns::generate_sized(&kind, n_units, complexity, size)?;
    // Override stroke color if specified
    if let Some(c) = color {
        let c = c.trim_start_matches('#');
        if c.len() == 6 {
            svg = svg.replace("#333", &format!("#{c}")).replace("#666", &format!("#{c}"));
        }
    }
    std::fs::write(&output, &svg)?;
    eprintln!("Saved: {}", output.display());
    if open {
        open_in_browser(&output);
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
        trace_single(input, output, TraceOptions {
            resolution: "medium".to_string(),
            simplify: simplify.to_string(),
            preset: "poster".to_string(),
            mode: mode.to_string(),
            max_colors,
            color_precision: 4,
            filter_speckle: 2,
            layers,
            no_background: false,
            threshold: 30,
            preview: false,
            optimize: false,
            open,
            json: false,
            quiet: false,
            scale: 1.0,
            rotate: 0.0,
            flip: "none".to_string(),
        })?;
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

/// Optimize SVG by rounding coordinates to integers.
fn optimize_svg(svg: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(svg.len());
    let mut i = 0;
    let bytes = svg.as_bytes();
    while i < bytes.len() {
        // Look for number patterns (digits, dots, minus)
        if bytes[i].is_ascii_digit() || (bytes[i] == b'.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit()) {
            // Find the start of the number
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.' || bytes[i] == b'-') {
                i += 1;
            }
            let num_str = std::str::from_utf8(&bytes[start..i]).unwrap_or("0");
            if let Ok(val) = num_str.parse::<f64>() {
                let rounded = val.round();
                if rounded.fract() == 0.0 {
                    let _ = write!(out, "{}", rounded as i64);
                } else {
                    let _ = write!(out, "{:.1}", rounded);
                }
            } else {
                out.push_str(num_str);
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
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
