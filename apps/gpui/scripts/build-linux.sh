#!/usr/bin/env bash
# FileTerm GPUI — Linux release packaging skeleton.
#
# G5 phase of `docs/plans/active/gpui-refactor.md` section 6.6.
#
# Produces `dist/FileTerm-GPUI-{version}-linux-{arch}.AppImage` from a
# clean `cargo build --release`. Uses `linuxdeploy` + `appimagetool` to
# bundle the binary + desktop integration (.desktop file, icon, MIME
# associations) into a single self-mounting AppImage.
#
# ## Toolchain requirements (host: Linux x86_64)
#
# * Rust stable (rustup toolchain)
# * `linuxdeploy` (https://github.com/linuxdeploy/linuxdeploy) on PATH
#   or at `$LINUXDEPLOY_BIN`
# * `appimagetool` (https://github.com/AppImage/AppImageKit) on PATH
#   or at `$APPIMAGETOOL_BIN`
# * Optional: `patchelf` for rpath fixup of bundled libs
#
# ## What's a skeleton
#
# G5 ships the structure; the .desktop file + icon are generated inline
# because they're trivial. Real release packaging will need to handle
# glibc version skew (build on the oldest supported distro, e.g.
# ubuntu:20.04), bundle required shared libs (fontconfig, freetype),
# and possibly produce separate X11 / Wayland variants.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

VERSION="${FILETERM_VERSION:-0.1.0}"
APP_NAME="FileTerm"
APP_BINARY="fileterm-gpui"
APP_NAME_LOWER="fileterm-gpui"
DIST_DIR="$REPO_ROOT/dist"
STAGE_DIR="$DIST_DIR/stage-linux"
APPDIR="$STAGE_DIR/FileTerm.AppDir"
ARCH="${FILETERM_ARCH:-x86_64}"
APPRUN_OUTPUT="$DIST_DIR/${APP_NAME}-GPUI-${VERSION}-linux-${ARCH}.AppImage"

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
Name=FileTerm (GPUI)
GenericName=Remote Workspace
Comment=SSH/SFTP/FTP desktop workstation (GPUI runtime)
Exec=$APP_BINARY %U
Icon=$APP_NAME_LOWER
Categories=Development;System;TerminalEmulator;
Terminal=false
StartupWMClass=$APP_NAME
DESKTOP

# Copy icon if present (G5 skeleton: assets may not ship yet).
PNG_SRC="$REPO_ROOT/apps/gpui/assets/icons/icon-256.png"
if [[ -f "$PNG_SRC" ]]; then
  cp "$PNG_SRC" "$APPDIR/usr/share/icons/hicolor/256x256/apps/${APP_NAME_LOWER}.png"
else
  echo "[build-linux] WARN: icon-256.png missing; AppImage will have no icon" >&2
fi

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
  --output appimage || {
    # If icon-file is missing linuxdeploy errors out; retry without it
    # so the skeleton still produces an AppImage (just icon-less).
    echo "[build-linux] retry linuxdeploy without icon-file (likely missing icon)" >&2
    "$LINUXDEPLOY" \
      --appdir "$APPDIR" \
      --desktop-file "$APPDIR/usr/share/applications/${APP_NAME_LOWER}.desktop" \
      --output appimage
  }

if [[ ! -f "$APPRUN_OUTPUT" ]]; then
  echo "[build-linux] FATAL: $APPRUN_OUTPUT not produced" >&2
  exit 1
fi

echo "[build-linux] OK: $APPRUN_OUTPUT"
