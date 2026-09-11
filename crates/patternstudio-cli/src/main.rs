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
        /// Input image path
        input: PathBuf,
        /// Output SVG path
        #[arg(short, long, default_value = "output.svg")]
        output: PathBuf,
        /// Resolution preset
        #[arg(short = 'r', long, default_value = "medium")]
        resolution: String,
        /// Simplification level
        #[arg(short = 's', long, default_value = "low")]
        simplify: String,
        /// VTracer preset
        #[arg(short, long, default_value = "poster")]
        preset: String,
    },
    /// Generate a pattern SVG
    Pattern {
        /// Pattern type
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
        Command::Trace { input, output, resolution, simplify, preset } => {
            cmd_trace(&input, &output, &resolution, &simplify, &preset)
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
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("Loading: {}", input.display());
    let img = load_image(input)?;
    eprintln!("Size: {}x{}", img.width, img.height);

    let p = match preset {
        "bw" => Preset::Bw,
        "photo" => Preset::Photo,
        _ => Preset::Poster,
    };
    let config = PatternStudioConfig {
        resolution: parse_resolution(resolution),
        simplify: parse_simplify(simplify),
        vtracer: vtracer::Config::from_preset(p),
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
