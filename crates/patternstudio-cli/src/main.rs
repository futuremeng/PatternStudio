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
    },
    /// Generate a pattern SVG
    Pattern {
        /// Pattern type: rosette/meander/scroll/taotie/ice_crack
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
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Trace { input, output, resolution, simplify, preset, mode, max_colors, color_precision, filter_speckle } => {
            cmd_trace(&input, &output, &resolution, &simplify, &preset, &mode, max_colors, color_precision, filter_speckle)
        }
        Command::Pattern { kind, output, n_units, complexity } => {
            cmd_pattern(&kind, &output, n_units, complexity)
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
            match trace_single(file, &out_svg, resolution, simplify, preset, mode, max_colors, color_precision, filter_speckle) {
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
    trace_single(input, output, resolution, simplify, preset, mode, max_colors, color_precision, filter_speckle)
}

fn trace_single(
    input: &PathBuf, output: &PathBuf,
    resolution: &str, simplify: &str, preset: &str,
    mode: &str, max_colors: usize, color_precision: i32, filter_speckle: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("Loading: {}", input.display());
    let img = load_image(input)?;
    eprintln!("Size: {}x{}", img.width, img.height);

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

    eprintln!("Tracing...");
    let pipeline = PatternStudioPipeline::new(&config)?;
    let assignment = LayerAssignment::single_layer();
    let layered = pipeline.trace_layered(&img, &assignment)?;
    eprintln!("Done: {} shapes in {} layer(s)", layered.total_shapes(), layered.layers.len());

    let svg = LayeredWriter::write(&layered);
    std::fs::write(output, &svg)?;
    eprintln!("Saved: {}", output.display());
    Ok(())
}

fn cmd_pattern(
    kind: &str, output: &PathBuf, n_units: u32, complexity: u8,
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("Generating pattern: {kind} (n={n_units}, complexity={complexity})");
    let svg = patternstudio_patterns::generate(kind, n_units, complexity)?;
    std::fs::write(output, &svg)?;
    eprintln!("Saved: {}", output.display());
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
