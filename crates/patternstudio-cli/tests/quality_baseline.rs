//! Quality gate: the fixture samples' quality snapshots must match the
//! committed baseline, and a tampered baseline must fail the run. A failing
//! test means engine behaviour drifted (vtracer bump, param default change,
//! ...) — review and, if intended, re-bless with
//! `patternstudio trace <sample> --baseline <file> --bless` (or rerun
//! `tests/fixtures/quality/record.sh` for the generated fixtures) and note
//! it in the PR.
//!
//! Baseline v2 (QWP1) adds round-trip metrics to every sample: `ink_f1`
//! (F1 of the ink masks dilated by 2px, computed against the rasterized
//! traced document) and `psnr`. They compare with fixed absolute
//! tolerances (0.05 / 1 dB) instead of the `--tolerance` relative one,
//! because a stroke that disappears is a quality change no count drift
//! would reveal.
//!
//! NOTE: the baseline was blessed on an arm64 macOS debug build. Counts are
//! integer-stable for these flat-color samples, and the round-trip metrics
//! are computed by the in-tree scanline rasterizer (pure Rust, no external
//! renderer), so they are stable across platforms; if a platform ever
//! drifts past tolerance, re-bless from it in a PR.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const SAMPLES: [&str; 10] = [
    "bands",
    "colors",
    "duo",
    "gradient",
    "lineart",
    "rosette",
    "meander",
    "ice_crack",
    "cloud",
    "rosette-scan",
];

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/quality")
}

fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "patternstudio-quality-{}-{tag}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(sample: &str, baseline: &Path) -> (bool, String) {
    let dir = tmp_dir(sample);
    let out = dir.join(format!("{sample}.svg"));
    let status = Command::new(env!("CARGO_BIN_EXE_patternstudio"))
        .arg("trace")
        .arg(fixtures_dir().join(format!("{sample}.png")))
        .arg("-o")
        .arg(&out)
        .arg("--baseline")
        .arg(baseline)
        .arg("-q")
        .output()
        .expect("spawn patternstudio");
    fs::remove_dir_all(&dir).ok();
    (
        status.status.success(),
        String::from_utf8_lossy(&status.stderr).to_string(),
    )
}

#[test]
fn quality_gate_passes_against_committed_baseline() {
    let baseline = fixtures_dir().join("baseline.json");
    for sample in SAMPLES {
        let (ok, stderr) = run(sample, &baseline);
        assert!(ok, "{sample} drifted from baseline: {stderr}");
    }
}

#[test]
fn quality_gate_detects_drift() {
    // Baseline whose "duo" entry is actually the "colors" snapshot: the duo
    // trace must fail with a drift report (this is the CI-red acceptance path
    // for an engine change that moves the metrics; "colors" was picked because
    // it differs from duo on the shape/color counts).
    let committed: serde_json::Value =
        serde_json::from_slice(&fs::read(fixtures_dir().join("baseline.json")).unwrap()).unwrap();
    let mut tampered = committed.clone();
    tampered["samples"]["duo"] = committed["samples"]["colors"].clone();

    let dir = tmp_dir("tampered");
    let baseline = dir.join("baseline.json");
    fs::write(&baseline, serde_json::to_vec_pretty(&tampered).unwrap()).unwrap();
    let (ok, stderr) = run("duo", &baseline);
    fs::remove_dir_all(&dir).ok();

    assert!(!ok, "tampered baseline must fail the run");
    assert!(stderr.contains("quality drift"), "stderr: {stderr}");
}
