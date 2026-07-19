#!/usr/bin/env bash
# Build a native FileTerm macOS application bundle and DMG.
#
# Required: Rust stable, Xcode command-line tools, and the checked-in icon.
# Developer ID signing and notarization are enabled through
# FILETERM_SIGN_IDENTITY and FILETERM_NOTARY_PROFILE respectively.

set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "[build-macos] FATAL: this package must be built on macOS" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$REPO_ROOT"

VERSION="${FILETERM_VERSION:-$(node -p "require('./package.json').version")}"
APP_NAME="FileTerm"
APP_BUNDLE="${APP_NAME}.app"
APP_BINARY="fileterm-gpui"
DIST_DIR="$REPO_ROOT/dist"
STAGE_DIR="$DIST_DIR/stage-macos"
APP_DIR="$STAGE_DIR/$APP_BUNDLE"
DMG_OUTPUT="$DIST_DIR/${APP_NAME}-${VERSION}-macos.dmg"

echo "[build-macos] version=$VERSION  binary=$APP_BINARY"

# Step 1: cargo release build.
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

# Copy the metadata generated for this exact release build.
PLIST_SRC=""
for candidate in "$REPO_ROOT"/target/release/build/fileterm-gpui-*/out/Info.plist; do
  if [[ -f "$candidate" && ( -z "$PLIST_SRC" || "$candidate" -nt "$PLIST_SRC" ) ]]; then
    PLIST_SRC="$candidate"
  fi
done
if [[ -z "$PLIST_SRC" ]]; then
  echo "[build-macos] FATAL: generated Info.plist not found" >&2
  exit 1
fi
cp "$PLIST_SRC" "$APP_DIR/Contents/Info.plist"

ICNS_SRC="$REPO_ROOT/apps/gpui/assets/icons/icon.icns"
if [[ ! -f "$ICNS_SRC" ]]; then
  echo "[build-macos] FATAL: $ICNS_SRC is required" >&2
  exit 1
fi
cp "$ICNS_SRC" "$APP_DIR/Contents/Resources/icon.icns"
plutil -lint "$APP_DIR/Contents/Info.plist"
[[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$APP_DIR/Contents/Info.plist")" == "com.fileterm.desktop" ]]
[[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP_DIR/Contents/Info.plist")" == "$VERSION" ]]

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
    --volname "$APP_NAME $VERSION" \
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
  hdiutil create -volname "$APP_NAME $VERSION" \
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
