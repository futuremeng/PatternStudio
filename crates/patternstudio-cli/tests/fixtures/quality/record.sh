#!/usr/bin/env bash
# Record the quality gate: regenerate the 512px fixture PNGs and bless the
# baseline snapshot for every sample. Run from anywhere; requires a debug
# build, rsvg-convert (librsvg2-bin) and ImageMagick (magick) for the scan
# variant. Commit the regenerated PNGs and baseline.json together, with the
# current metric numbers noted in the PR.
set -euo pipefail

cd "$(dirname "$0")"
REPO="../../../../.."
BIN="${BIN:-$REPO/target/debug/patternstudio}"
[[ -x "$BIN" ]] || (cd "$REPO" && cargo build -p patternstudio-cli)

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# 512px line-art fixtures: deterministic generated patterns rasterized to PNG.
for kind in rosette meander ice_crack cloud; do
    "$BIN" pattern -k "$kind" --size 512 --seed 7 -o "$TMP/$kind.svg"
    rsvg-convert -w 512 -h 512 -b white "$TMP/$kind.svg" -o "$kind.png"
done

# Scan variant: downscale to 435px + JPEG q60, the 2026-11 evaluation's
# degradation recipe (deterministic: no random noise, committed as a PNG).
magick rosette.png -resize 435x435 -quality 60 "$TMP/rosette-scan.jpg"
magick "$TMP/rosette-scan.jpg" -background white -flatten rosette-scan.png

# Bless every sample into the baseline.
for png in *.png; do
    "$BIN" trace "$png" -o "$TMP/out.svg" --baseline baseline.json --bless -q
done

echo "recorded $(ls -1 *.png | wc -l | tr -d ' ') samples into baseline.json"
