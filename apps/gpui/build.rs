//! Generates platform metadata consumed by the release packaging scripts.
//!
//! Binary embedding and installer assembly remain platform-specific, while
//! application identity and version always come from this crate's package
//! metadata so all produced bundles describe the same FileTerm application.

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
fn write_macos_plist(out_dir: &std::path::Path) {
    let version = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION is set by Cargo");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>FileTerm</string>
    <key>CFBundleDisplayName</key>
    <string>FileTerm</string>
    <key>CFBundleIdentifier</key>
    <string>com.fileterm.desktop</string>
    <key>CFBundleVersion</key>
    <string>{version}</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleExecutable</key>
    <string>fileterm-gpui</string>
    <key>CFBundleIconFile</key>
    <string>icon.icns</string>
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
"#
    );
    let out_path = out_dir.join("Info.plist");
    fs::write(&out_path, plist).expect("write generated macOS Info.plist");
    println!("cargo:warning=generated {}", out_path.display());
}

/// Write a Windows application manifest declaring DPI awareness so
/// GPUI's renderer gets the real physical pixel resolution instead of
/// the scaled virtual one.
fn write_windows_manifest(out_dir: &std::path::Path) {
    let package_version = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION is set by Cargo");
    let mut parts = package_version
        .split('.')
        .map(|part| part.split('-').next().unwrap_or("0"))
        .take(4)
        .collect::<Vec<_>>();
    parts.resize(4, "0");
    let assembly_version = parts.join(".");
    let manifest = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity version="{assembly_version}" name="FileTerm.Desktop" type="win32"/>
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
"#
    );
    let out_path = out_dir.join("FileTerm.exe.manifest");
    fs::write(&out_path, &manifest).expect("write generated Windows manifest");

    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("assets/icons/icon.ico")
        .set_manifest(&manifest)
        .set("ProductName", "FileTerm")
        .set("InternalName", "fileterm-gpui.exe")
        .set("OriginalFilename", "fileterm-gpui.exe")
        .set("FileDescription", "FileTerm remote workspace");
    resource
        .compile()
        .expect("compile Windows executable resources");

    println!("cargo:warning=generated {}", out_path.display());
}
