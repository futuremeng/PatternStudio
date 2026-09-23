#!/usr/bin/env bash
#
# Fetch the image-preprocessing runtime for PatternStudio:
#   1. The `realesrgan-ncnn-vulkan` binary (MIT, xinntao/Real-ESRGAN-ncnn-vulkan v0.2.0)
#   2. The `realesrgan-x4plus-anime` ncnn model ("digital art" model, the same
#      model family as Upscayl's digital-art-4x; shipped in the Real-ESRGAN v0.2.2.4
#      release package).
#
# Files are placed under resources/ (git-ignored):
#   resources/bin/<platform>/realesrgan-ncnn-vulkan
#   resources/models/realesrgan-x4plus-anime.param
#   resources/models/realesrgan-x4plus-anime.bin
#
# Usage: scripts/fetch-preprocess.sh   (from the repo root, or anywhere)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RES="$ROOT/resources"

# ── Platform detection ─────────────────────────────────────────────────────
case "$(uname -s)" in
  Darwin)
    PLATFORM="darwin"
    BIN_ZIP="realesrgan-ncnn-vulkan-v0.2.0-macos.zip"
    MODEL_ZIP="realesrgan-ncnn-vulkan-20210901-macos.zip"
    ;;
  Linux)
    PLATFORM="linux"
    BIN_ZIP="realesrgan-ncnn-vulkan-v0.2.0-ubuntu.zip"
    MODEL_ZIP="realesrgan-ncnn-vulkan-20210901-ubuntu.zip"
    ;;
  MINGW*|MSYS*|CYGWIN*)
    PLATFORM="windows"
    BIN_ZIP="realesrgan-ncnn-vulkan-v0.2.0-windows.zip"
    MODEL_ZIP="realesrgan-ncnn-vulkan-20210901-windows.zip"
    ;;
  *)
    echo "Unsupported platform: $(uname -s)" >&2
    exit 1
    ;;
esac

BIN_URL="https://github.com/xinntao/Real-ESRGAN-ncnn-vulkan/releases/download/v0.2.0/$BIN_ZIP"
MODEL_URL="https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.2.4/$MODEL_ZIP"

BIN_DIR="$RES/bin/$PLATFORM"
MODEL_DIR="$RES/models"
BIN_NAME="realesrgan-ncnn-vulkan"
[ "$PLATFORM" = "windows" ] && BIN_NAME="realesrgan-ncnn-vulkan.exe"

mkdir -p "$BIN_DIR" "$MODEL_DIR"

# Proxy support: honor PATTERNSTUDIO_PROXY, then the usual http(s)_proxy env.
PROXY_ARGS=()
if [ -n "${PATTERNSTUDIO_PROXY:-}" ]; then
  PROXY_ARGS=(--proxy "$PATTERNSTUDIO_PROXY")
elif [ -n "${https_proxy:-${HTTPS_PROXY:-}}" ]; then
  PROXY_ARGS=(--proxy "${https_proxy:-$HTTPS_PROXY}")
elif [ -n "${http_proxy:-${HTTP_PROXY:-}}" ]; then
  PROXY_ARGS=(--proxy "${http_proxy:-$HTTP_PROXY}")
fi

download() {
  local url="$1" dest="$2"
  if [ -s "$dest" ]; then
    echo "✓ already downloaded: $dest"
    return
  fi
  echo "↓ downloading $url"
  local ok=0
  for attempt in 1 2 3 4 5; do
    if curl -fL --http1.1 -C - "${PROXY_ARGS[@]}" -o "$dest" "$url"; then
      ok=1
      break
    fi
    echo "  attempt $attempt failed, retrying in 3s..."
    sleep 3
  done
  if [ "$ok" != "1" ]; then
    rm -f "$dest"
    echo "error: download failed: $url" >&2
    exit 1
  fi
}

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# ── 1. Binary ──────────────────────────────────────────────────────────────
if [ -x "$BIN_DIR/$BIN_NAME" ]; then
  echo "✓ binary present: $BIN_DIR/$BIN_NAME"
else
  download "$BIN_URL" "$TMP/$BIN_ZIP"
  # The binary sits in a top-level folder inside the zip; match by basename.
  unzip -o -j "$TMP/$BIN_ZIP" "*/$BIN_NAME" -d "$BIN_DIR" >/dev/null \
    || unzip -o -j "$TMP/$BIN_ZIP" "$BIN_NAME" -d "$BIN_DIR" >/dev/null
  chmod +x "$BIN_DIR/$BIN_NAME"
  echo "✓ binary: $BIN_DIR/$BIN_NAME"
fi

# ── 2. Model (digital art: realesrgan-x4plus-anime) ─────────────────────────
if [ -f "$MODEL_DIR/realesrgan-x4plus-anime.param" ] && \
   [ -f "$MODEL_DIR/realesrgan-x4plus-anime.bin" ]; then
  echo "✓ model present: $MODEL_DIR/realesrgan-x4plus-anime.*"
else
  download "$MODEL_URL" "$TMP/$MODEL_ZIP"
  unzip -o -j "$TMP/$MODEL_ZIP" \
    "models/realesrgan-x4plus-anime.param" \
    "models/realesrgan-x4plus-anime.bin" \
    -d "$MODEL_DIR" >/dev/null
  echo "✓ model: $MODEL_DIR/realesrgan-x4plus-anime.*"
fi

# ── Verify ─────────────────────────────────────────────────────────────────
echo
echo "Preprocess runtime ready:"
ls -lh "$BIN_DIR/$BIN_NAME" "$MODEL_DIR"/realesrgan-x4plus-anime.* | sed 's/^/  /'
if [ "$PLATFORM" = "darwin" ]; then
  echo "  arch: $(file "$BIN_DIR/$BIN_NAME" | sed 's/.*: //')"
fi
