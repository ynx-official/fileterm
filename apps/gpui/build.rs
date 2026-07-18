//! Build script — platform resource bundling skeleton.
//!
//! G5 phase of `docs/plans/active/gpui-refactor.md` section 6.6.
//!
//! Runs at `cargo build` time. Responsibilities (per platform):
//!
//! * **macOS**: writes `Info.plist` into `target/{profile}/FileTerm.app/Contents/`
//!   so the binary is recognizable as a macOS app bundle. Icons + code
//!   signing land in the release shell script (`scripts/build-macos.sh`)
//!   because they need `codesign` / `create-dmg` tooling that doesn't
//!   belong in `build.rs`.
//! * **Windows**: writes a `FileTerm.exe.manifest` so Windows applies
//!   DPI-awareness + theme preferences. NSIS installer generation lives
//!   in `scripts/build-windows.sh`.
//! * **Linux**: no-op for now — AppImage bundling is done entirely by
//!   `scripts/build-linux.sh` (uses `linuxdeploy`).
//!
//! ## Why a skeleton
//!
//! G5 ships the structure; the actual icon assets (`icon.icns` /
//! `icon.ico` / `icon.png`) are tracked in `apps/gpui/assets/icons/`
//! but don't exist yet (placeholder until the design token pass). When
//! the assets land, this script grows the `cp` / `embed-resource` calls
//! to copy them into the right place per platform.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    // Tell cargo to re-run us only if this script or the assets dir changes.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap_or_default());

    match target_os.as_str() {
        "macos" => write_macos_plist(&out_dir),
        "windows" => write_windows_manifest(&out_dir),
        "linux" | "" => {
            // No build-time work for Linux; bundling is done by
            // scripts/build-linux.sh via linuxdeploy at packaging time.
        }
        other => {
            // Unknown target — log to build output but don't fail the
            // build (cargo would otherwise silently swallow the panic).
            println!("cargo:warning=fileterm-gpui build.rs: unhandled target_os={other}, skipping platform resource bundling");
        }
    }
}

/// Write a minimal `Info.plist` for macOS so the binary is treated as
/// an app bundle when packaged into `FileTerm.app`.
///
/// The release shell script (`scripts/build-macos.sh`) copies this into
/// `FileTerm.app/Contents/Info.plist` after `cargo build --release`.
fn write_macos_plist(_out_dir: &std::path::Path) {
    let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>FileTerm</string>
    <key>CFBundleDisplayName</key>
    <string>FileTerm</string>
    <key>CFBundleIdentifier</key>
    <string>dev.fileterm.gpui</string>
    <key>CFBundleVersion</key>
    <string>0.1.0</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleExecutable</key>
    <string>fileterm-gpui</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSRequiresAquaSystemAppearance</key>
    <false/>
</dict>
</plist>
"#;
    // Write to OUT_DIR so cargo's cache holds it; the shell script copies
    // it out. We don't write directly to the .app bundle because that
    // path is layout-dependent and not known to build.rs.
    let out_path = _out_dir.join("Info.plist");
    let _ = fs::write(&out_path, plist);
    println!("cargo:warning=fileterm-gpui build.rs: wrote {}", out_path.display());
}

/// Write a Windows application manifest declaring DPI awareness so
/// GPUI's renderer gets the real physical pixel resolution instead of
/// the scaled virtual one.
fn write_windows_manifest(_out_dir: &std::path::Path) {
    let manifest = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity version="0.1.0.0" name="FileTerm.Gpui" type="win32"/>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
    </windowsSettings>
  </application>
</assembly>
"#;
    let out_path = _out_dir.join("FileTerm.exe.manifest");
    let _ = fs::write(&out_path, manifest);
    println!("cargo:warning=fileterm-gpui build.rs: wrote {}", out_path.display());
}
