#!/usr/bin/env bash
# FileTerm GPUI — Windows release packaging skeleton.
#
# G5 phase of `docs/plans/active/gpui-refactor.md` section 6.6.
#
# Produces `dist/FileTerm-GPUI-{version}-windows-{arch}-setup.exe` from
# a clean `cargo build --release`. The installer is built with NSIS
# (Nullsoft Scriptable Install System) so it produces a single
# self-contained .exe that users double-click to install.
#
# ## Toolchain requirements
#
# This script is intended to run in a CI environment with the Windows
# target installed (`rustup target add x86_64-pc-windows-msvc`). It can
# also run on a Windows dev machine with NSIS installed.
#
# * Rust stable with `x86_64-pc-windows-msvc` target
# * NSIS 3.x (https://nsis.sourceforge.io/) — `makensis` on PATH
# * (Optional) `signtool` from Windows SDK + code-signing certificate
#   referenced by `FILETERM_CERT_SUBJECT` for signing the installer
#
# ## What's a skeleton
#
# G5 ships the structure; the NSIS template writes a basic
# install/uninstall flow (Program Files shortcut + Start Menu entry +
# uninstaller). Real release packaging will add file-type associations,
# auto-update stub, custom install wizard pages.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

VERSION="${FILETERM_VERSION:-0.1.0}"
APP_NAME="FileTerm"
APP_BINARY="fileterm-gpui.exe"
DIST_DIR="$REPO_ROOT/dist"
STAGE_DIR="$DIST_DIR/stage-windows"
TARGET_ARCH="${FILETERM_ARCH:-x86_64-pc-windows-msvc}"
INSTALLER_OUTPUT="$DIST_DIR/${APP_NAME}-GPUI-${VERSION}-windows-setup.exe"

echo "[build-windows] version=$VERSION  arch=$TARGET_ARCH"

# Step 1: cargo release build for the Windows target.
echo "[build-windows] cargo build --release --target $TARGET_ARCH -p fileterm-gpui"
cargo build --release --target "$TARGET_ARCH" -p fileterm-gpui

BINARY_PATH="$REPO_ROOT/target/$TARGET_ARCH/release/$APP_BINARY"
if [[ ! -f "$BINARY_PATH" ]]; then
  echo "[build-windows] FATAL: $BINARY_PATH not found after cargo build" >&2
  exit 1
fi

# Step 2: stage installer payload.
echo "[build-windows] staging payload at $STAGE_DIR"
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR"
cp "$BINARY_PATH" "$STAGE_DIR/$APP_BINARY"

# Copy icon if present (G5 skeleton: assets may not ship yet).
ICO_SRC="$REPO_ROOT/apps/gpui/assets/icons/icon.ico"
if [[ -f "$ICO_SRC" ]]; then
  cp "$ICO_SRC" "$STAGE_DIR/icon.ico"
else
  echo "[build-windows] WARN: icon.ico missing; installer will use default NSIS icon" >&2
fi

# Step 3: generate NSIS installer script (inlined here for skeleton simplicity).
NSI_FILE="$STAGE_DIR/installer.nsi"
cat > "$NSI_FILE" <<NSI
!define APP_NAME "$APP_NAME"
!define APP_VERSION "$VERSION"
!define APP_EXE "$APP_BINARY"
!define APP_PUBLISHER "FileTerm"

Name "\${APP_NAME} GPUI \${APP_VERSION}"
OutFile "$INSTALLER_OUTPUT"
Unicode True
InstallDir "\$LOCALAPPDATA\\\${APP_NAME}"
RequestExecutionLevel user
ShowInstDetails show

Page directory
Page instfiles

UninstPage uninstConfirm
UninstPage instfiles

Section "Install"
  SetOutPath "\$INSTDIR"
  File "\${APP_EXE}"
  File /nonfatal "icon.ico"
  CreateShortcut "\$DESKTOP\\\${APP_NAME} GPUI.lnk" "\$INSTDIR\\\${APP_EXE}"
  CreateDirectory "\$SMPROGRAMS\\\${APP_NAME}"
  CreateShortcut "\$SMPROGRAMS\\\${APP_NAME}\\\${APP_NAME} GPUI.lnk" "\$INSTDIR\\\${APP_EXE}"
  WriteUninstaller "\$INSTDIR\\uninstall.exe"
  CreateShortcut "\$SMPROGRAMS\\\${APP_NAME}\\Uninstall.lnk" "\$INSTDIR\\uninstall.exe"
SectionEnd

Section "Uninstall"
  Delete "\$INSTDIR\\\${APP_EXE}"
  Delete "\$INSTDIR\\icon.ico"
  Delete "\$INSTDIR\\uninstall.exe"
  Delete "\$DESKTOP\\\${APP_NAME} GPUI.lnk"
  Delete "\$SMPROGRAMS\\\${APP_NAME}\\\${APP_NAME} GPUI.lnk"
  Delete "\$SMPROGRAMS\\\${APP_NAME}\\Uninstall.lnk"
  RMDir "\$SMPROGRAMS\\\${APP_NAME}"
  RMDir "\$INSTDIR"
SectionEnd
NSI

# Step 4: build installer with NSIS.
if ! command -v makensis >/dev/null 2>&1; then
  echo "[build-windows] FATAL: makensis not found; install NSIS 3.x" >&2
  exit 1
fi
echo "[build-windows] makensis $NSI_FILE"
( cd "$STAGE_DIR" && makensis "$(basename "$NSI_FILE")" )

# Step 5: code signing (optional — gated by FILETERM_CERT_SUBJECT).
if [[ -n "${FILETERM_CERT_SUBJECT:-}" ]] && command -v signtool >/dev/null 2>&1; then
  echo "[build-windows] signtool sign with subject=$FILETERM_CERT_SUBJECT"
  signtool sign /fd sha256 /n "$FILETERM_CERT_SUBJECT" \
    /t http://timestamp.digicert.com \
    "$INSTALLER_OUTPUT"
else
  echo "[build-windows] WARN: signing skipped (FILETERM_CERT_SUBJECT unset or signtool missing)"
fi

echo "[build-windows] OK: $INSTALLER_OUTPUT"
