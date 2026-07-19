#!/usr/bin/env bash
# Build the FileTerm Linux AppImage on a Linux release host.
#
# Required: Rust stable, linuxdeploy, and the checked-in PNG icon. Build on
# the oldest supported distribution to keep the resulting glibc baseline.

set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "[build-linux] FATAL: this package must be built on Linux" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$REPO_ROOT"

VERSION="${FILETERM_VERSION:-$(node -p "require('./package.json').version")}"
APP_NAME="FileTerm"
APP_BINARY="fileterm-gpui"
APP_NAME_LOWER="fileterm-gpui"
DIST_DIR="$REPO_ROOT/dist"
STAGE_DIR="$DIST_DIR/stage-linux"
APPDIR="$STAGE_DIR/FileTerm.AppDir"
ARCH="${FILETERM_ARCH:-x86_64}"
APPRUN_OUTPUT="$DIST_DIR/${APP_NAME}-${VERSION}-linux-${ARCH}.AppImage"

echo "[build-linux] version=$VERSION  arch=$ARCH"

# Step 1: cargo release build.
echo "[build-linux] cargo build --release -p fileterm-gpui"
cargo build --release -p fileterm-gpui

BINARY_PATH="$REPO_ROOT/target/release/$APP_BINARY"
if [[ ! -f "$BINARY_PATH" ]]; then
  echo "[build-linux] FATAL: $BINARY_PATH not found after cargo build" >&2
  exit 1
fi

# Step 2: stage AppDir layout.
echo "[build-linux] staging AppDir at $APPDIR"
rm -rf "$STAGE_DIR"
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/share/applications"
mkdir -p "$APPDIR/usr/share/icons/hicolor/256x256/apps"

cp "$BINARY_PATH" "$APPDIR/usr/bin/$APP_BINARY"

# Generate .desktop file (required by linuxdeploy for AppImage menu integration).
cat > "$APPDIR/usr/share/applications/${APP_NAME_LOWER}.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=FileTerm
GenericName=Remote Workspace
Comment=SSH, SFTP, and FTP desktop workstation
Exec=$APP_BINARY %U
Icon=$APP_NAME_LOWER
Categories=Development;System;TerminalEmulator;
Terminal=false
StartupWMClass=$APP_NAME
DESKTOP

PNG_SRC="$REPO_ROOT/apps/gpui/assets/icons/icon-256.png"
if [[ ! -f "$PNG_SRC" ]]; then
  echo "[build-linux] FATAL: $PNG_SRC is required" >&2
  exit 1
fi
cp "$PNG_SRC" "$APPDIR/usr/share/icons/hicolor/256x256/apps/${APP_NAME_LOWER}.png"

# Step 3: run linuxdeploy to bundle deps + generate AppRun entry point.
LINUXDEPLOY="${LINUXDEPLOY_BIN:-linuxdeploy}"
if ! command -v "$LINUXDEPLOY" >/dev/null 2>&1 && [[ ! -x "$LINUXDEPLOY" ]]; then
  echo "[build-linux] FATAL: linuxdeploy not found (set LINUXDEPLOY_BIN or install it)" >&2
  exit 1
fi

echo "[build-linux] linuxdeploy --appdir $APPDIR"
export OUTPUT="$APPRUN_OUTPUT"
export ARCH
"$LINUXDEPLOY" \
  --appdir "$APPDIR" \
  --desktop-file "$APPDIR/usr/share/applications/${APP_NAME_LOWER}.desktop" \
  --icon-file "$APPDIR/usr/share/icons/hicolor/256x256/apps/${APP_NAME_LOWER}.png" \
  --output appimage

if [[ ! -f "$APPRUN_OUTPUT" ]]; then
  echo "[build-linux] FATAL: $APPRUN_OUTPUT not produced" >&2
  exit 1
fi

echo "[build-linux] OK: $APPRUN_OUTPUT"
