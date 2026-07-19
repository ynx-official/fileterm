#!/usr/bin/env bash
# Build the FileTerm Windows installer on a Windows release host.
#
# Required: Rust stable with the MSVC target, NSIS 3.x, and the checked-in
# ICO asset. Authenticode signing is enabled through FILETERM_CERT_SUBJECT.

set -euo pipefail

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) ;;
  *)
    echo "[build-windows] FATAL: this package must be built on Windows" >&2
    exit 1
    ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$REPO_ROOT"

VERSION="${FILETERM_VERSION:-$(node -p "require('./package.json').version")}"
APP_NAME="FileTerm"
APP_BINARY="fileterm-gpui.exe"
DIST_DIR="$REPO_ROOT/dist"
STAGE_DIR="$DIST_DIR/stage-windows"
TARGET_ARCH="${FILETERM_ARCH:-x86_64-pc-windows-msvc}"
INSTALLER_OUTPUT="$DIST_DIR/${APP_NAME}-${VERSION}-windows-${TARGET_ARCH}-setup.exe"

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

ICO_SRC="$REPO_ROOT/apps/gpui/assets/icons/icon.ico"
if [[ ! -f "$ICO_SRC" ]]; then
  echo "[build-windows] FATAL: $ICO_SRC is required" >&2
  exit 1
fi
cp "$ICO_SRC" "$STAGE_DIR/icon.ico"

# Step 3: generate the NSIS installer script.
NSI_FILE="$STAGE_DIR/installer.nsi"
cat > "$NSI_FILE" <<NSI
!define APP_NAME "$APP_NAME"
!define APP_VERSION "$VERSION"
!define APP_EXE "$APP_BINARY"
!define APP_PUBLISHER "FileTerm"

Name "\${APP_NAME} \${APP_VERSION}"
OutFile "$INSTALLER_OUTPUT"
Unicode True
Icon "icon.ico"
UninstallIcon "icon.ico"
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
  File "icon.ico"
  CreateShortcut "\$DESKTOP\\\${APP_NAME}.lnk" "\$INSTDIR\\\${APP_EXE}" "" "\$INSTDIR\\icon.ico"
  CreateDirectory "\$SMPROGRAMS\\\${APP_NAME}"
  CreateShortcut "\$SMPROGRAMS\\\${APP_NAME}\\\${APP_NAME}.lnk" "\$INSTDIR\\\${APP_EXE}" "" "\$INSTDIR\\icon.ico"
  WriteUninstaller "\$INSTDIR\\uninstall.exe"
  CreateShortcut "\$SMPROGRAMS\\\${APP_NAME}\\Uninstall.lnk" "\$INSTDIR\\uninstall.exe"
SectionEnd

Section "Uninstall"
  Delete "\$INSTDIR\\\${APP_EXE}"
  Delete "\$INSTDIR\\icon.ico"
  Delete "\$INSTDIR\\uninstall.exe"
  Delete "\$DESKTOP\\\${APP_NAME}.lnk"
  Delete "\$SMPROGRAMS\\\${APP_NAME}\\\${APP_NAME}.lnk"
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
    /tr http://timestamp.digicert.com /td sha256 \
    "$INSTALLER_OUTPUT"
else
  echo "[build-windows] WARN: signing skipped (FILETERM_CERT_SUBJECT unset or signtool missing)"
fi

echo "[build-windows] OK: $INSTALLER_OUTPUT"
