#!/usr/bin/env bash
# FileTerm GPUI — macOS release packaging skeleton.
#
# G5 phase of `docs/plans/active/gpui-refactor.md` section 6.6.
#
# Produces `dist/FileTerm-GPUI-{version}-macos.dmg` from a clean
# `cargo build --release`. Mirrors the layout Tauri's macOS bundler
# would produce (FileTerm.app bundle with Contents/{MacOS,Resources,
# Info.plist}) so users can drag-drop install identically.
#
# ## Toolchain requirements (host: macOS)
#
# * Rust stable (rustup toolchain)
# * Xcode command-line tools (`xcode-select --install`) for `codesign`
#   and `hdiutil`
# * `create-dmg` from Homebrew for the DMG layout (`brew install create-dmg`)
#
# ## What's a skeleton
#
# G5 ships the structure; code signing with a Developer ID certificate
# + notarization (`xcrun notarytool submit`) are gated behind env vars
# (`FILETERM_SIGN_IDENTITY`, `FILETERM_NOTARY_PROFILE`) and skip
# silently when unset. When the release pipeline lands for real, the
# CI job will set those vars and the script will produce a signed +
# notarized DMG.

set -euo pipefail

# Resolve repo root from script location.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

# Configuration.
VERSION="${FILETERM_VERSION:-0.1.0}"
APP_NAME="FileTerm"
APP_BUNDLE="${APP_NAME}.app"
APP_BINARY="fileterm-gpui"
DIST_DIR="$REPO_ROOT/dist"
STAGE_DIR="$DIST_DIR/stage-macos"
APP_DIR="$STAGE_DIR/$APP_BUNDLE"
DMG_OUTPUT="$DIST_DIR/${APP_NAME}-GPUI-${VERSION}-macos.dmg"

echo "[build-macos] version=$VERSION  binary=$APP_BINARY"

# Step 1: clean cargo release build.
echo "[build-macos] cargo build --release -p fileterm-gpui"
cargo build --release -p fileterm-gpui

BINARY_PATH="$REPO_ROOT/target/release/$APP_BINARY"
if [[ ! -f "$BINARY_PATH" ]]; then
  echo "[build-macos] FATAL: $BINARY_PATH not found after cargo build" >&2
  exit 1
fi

# Step 2: stage .app bundle layout.
echo "[build-macos] staging $APP_BUNDLE"
rm -rf "$STAGE_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

cp "$BINARY_PATH" "$APP_DIR/Contents/MacOS/$APP_BINARY"

# Copy Info.plist from cargo's OUT_DIR. build.rs writes it there; we
# find it by walking target/release/build/fileterm-gpui-*/out/Info.plist.
PLIST_SRC="$(find "$REPO_ROOT/target/release/build" -path '*/out/Info.plist' | head -n1 || true)"
if [[ -z "$PLIST_SRC" ]]; then
  echo "[build-macos] WARN: Info.plist not found in OUT_DIR; writing minimal fallback" >&2
  cat > "$APP_DIR/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>FileTerm</string>
  <key>CFBundleIdentifier</key><string>dev.fileterm.gpui</string>
  <key>CFBundleExecutable</key><string>fileterm-gpui</string>
  <key>CFBundleVersion</key><string>0.1.0</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
PLIST
else
  cp "$PLIST_SRC" "$APP_DIR/Contents/Info.plist"
fi

# Step 3: copy icon assets if present (G5 skeleton: assets may not ship yet).
ICNS_SRC="$REPO_ROOT/apps/gpui/assets/icons/icon.icns"
if [[ -f "$ICNS_SRC" ]]; then
  cp "$ICNS_SRC" "$APP_DIR/Contents/Resources/icon.icns"
  # Reference the icon in Info.plist (CFBundleIconFile key).
  /usr/libexec/PlistBuddy -c "Add :CFBundleIconFile string icon.icns" \
    "$APP_DIR/Contents/Info.plist" 2>/dev/null || true
else
  echo "[build-macos] WARN: icon.icns missing; bundle will have no app icon" >&2
fi

# Step 4: code signing (optional — gated by FILETERM_SIGN_IDENTITY).
if [[ -n "${FILETERM_SIGN_IDENTITY:-}" ]]; then
  echo "[build-macos] codesign with identity=$FILETERM_SIGN_IDENTITY"
  codesign --force --deep --options runtime \
    --sign "$FILETERM_SIGN_IDENTITY" \
    "$APP_DIR"
else
  echo "[build-macos] WARN: FILETERM_SIGN_IDENTITY unset; ad-hoc signing only"
  codesign --force --deep --sign - "$APP_DIR"
fi

# Step 5: build DMG.
echo "[build-macos] creating $DMG_OUTPUT"
rm -f "$DMG_OUTPUT"
if command -v create-dmg >/dev/null 2>&1; then
  create-dmg \
    --volname "$APP_NAME GPUI $VERSION" \
    --window-pos 200 120 \
    --window-size 600 400 \
    --icon-size 100 \
    --icon "$APP_BUNDLE" 175 190 \
    --hide-extension "$APP_BUNDLE" \
    --app-drop-link 425 190 \
    "$DMG_OUTPUT" \
    "$STAGE_DIR"
else
  # Fallback: bare hdiutil DMG (no pretty layout).
  hdiutil create -volname "$APP_NAME GPUI $VERSION" \
    -srcfolder "$STAGE_DIR" -ov -format UDZO \
    "$DMG_OUTPUT"
fi

# Step 6: notarization (optional — gated by FILETERM_NOTARY_PROFILE).
if [[ -n "${FILETERM_NOTARY_PROFILE:-}" ]]; then
  echo "[build-macos] notarizing with notarytool profile=$FILETERM_NOTARY_PROFILE"
  xcrun notarytool submit "$DMG_OUTPUT" \
    --keychain-profile "$FILETERM_NOTARY_PROFILE" \
    --wait
  xcrun stapler staple "$DMG_OUTPUT"
else
  echo "[build-macos] WARN: FILETERM_NOTARY_PROFILE unset; DMG is not notarized"
fi

echo "[build-macos] OK: $DMG_OUTPUT"
