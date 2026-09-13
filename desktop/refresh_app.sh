#!/usr/bin/env bash
# 创建/刷新 debug 用的 PatternStudio.app，让 lldb 跑 .app 里的二进制时
# Dock 显示校徽图标（裸二进制只能显示 macOS 默认图标）。
# 幂等：可重复运行；cargo clean 后也能自动重建。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/target/debug/PatternStudio.app"
BIN_NAME="patternstudio-desktop"
ICNS_SRC="$ROOT/desktop/icons/icon.icns"
PNG_SRC="$ROOT/desktop/icons/icon.png"
BIN_SRC="$ROOT/target/debug/$BIN_NAME"
M="/opt/homebrew/bin/magick"

mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

# Info.plist（CFBundleIconFile=icon → Resources/icon.icns）
cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>patternstudio-desktop</string>
    <key>CFBundleIconFile</key><string>icon</string>
    <key>CFBundleIdentifier</key><string>com.futuremeng.patternstudio</string>
    <key>CFBundleName</key><string>PatternStudio</string>
    <key>CFBundleDisplayName</key><string>PatternStudio</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>0.1.0</string>
    <key>CFBundleVersion</key><string>0.1.0</string>
    <key>LSMinimumSystemVersion</key><string>10.13</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# icon.icns：缺失则从 icon.png 生成（16–1024 全套尺寸）
if [ ! -f "$ICNS_SRC" ]; then
  echo "icon.icns missing -> generating from icon.png"
  ICONSET="$(mktemp -d)/ps.iconset"; mkdir -p "$ICONSET"
  "$M" "$PNG_SRC" -resize 16x16     "$ICONSET/icon_16x16.png"
  "$M" "$PNG_SRC" -resize 32x32     "$ICONSET/icon_16x16@2x.png"
  "$M" "$PNG_SRC" -resize 32x32     "$ICONSET/icon_32x32.png"
  "$M" "$PNG_SRC" -resize 64x64     "$ICONSET/icon_32x32@2x.png"
  "$M" "$PNG_SRC" -resize 128x128   "$ICONSET/icon_128x128.png"
  "$M" "$PNG_SRC" -resize 256x256   "$ICONSET/icon_128x128@2x.png"
  "$M" "$PNG_SRC" -resize 256x256   "$ICONSET/icon_256x256.png"
  "$M" "$PNG_SRC" -resize 512x512   "$ICONSET/icon_256x256@2x.png"
  "$M" "$PNG_SRC" -resize 512x512   "$ICONSET/icon_512x512.png"
  "$M" "$PNG_SRC" -resize 1024x1024 "$ICONSET/icon_512x512@2x.png"
  iconutil -c icns "$ICONSET" -o "$ICNS_SRC"
fi
cp "$ICNS_SRC" "$APP/Contents/Resources/icon.icns"

# 二进制（必须先 cargo build）
if [ ! -f "$BIN_SRC" ]; then
  echo "ERROR: $BIN_SRC not found. Run cargo build first." >&2
  exit 1
fi
cp "$BIN_SRC" "$APP/Contents/MacOS/$BIN_NAME"

# 注册到 LaunchServices，让 macOS 立即识别 .app 及其图标
LSREG="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
[ -x "$LSREG" ] && "$LSREG" -f "$APP" >/dev/null 2>&1 || true

echo "PatternStudio.app ready: $APP"
