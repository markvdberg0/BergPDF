//! macOS `.app` bundle layout. Pure file generation so it can be unit-tested on any host;
//! launching the bundle, code signing and notarisation are separate steps that require a Mac
//! and the owner's Apple developer identity (not performed here).

use std::io;
use std::path::Path;

/// `Info.plist` for the application bundle.
pub fn info_plist(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>BergPDF</string>
  <key>CFBundleDisplayName</key><string>BergPDF</string>
  <key>CFBundleIdentifier</key><string>org.example.bergpdf.dev</string>
  <key>CFBundleExecutable</key><string>bergpdf</string>
  <key>CFBundleIconFile</key><string>bergpdf</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>PDF Document</string>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>LSItemContentTypes</key><array><string>com.adobe.pdf</string></array>
      <key>LSHandlerRank</key><string>Alternate</string>
    </dict>
  </array>
</dict>
</plist>
"#
    )
}

/// Create `<dir>/BergPDF.app` containing `bin`.
pub fn write_app_bundle(
    dir: &Path,
    bin: &Path,
    icns: Option<&Path>,
    version: &str,
) -> io::Result<()> {
    let app = dir.join("BergPDF.app/Contents");
    std::fs::create_dir_all(app.join("MacOS"))?;
    std::fs::create_dir_all(app.join("Resources"))?;
    std::fs::copy(bin, app.join("MacOS/bergpdf"))?;
    std::fs::write(app.join("Info.plist"), info_plist(version))?;
    if let Some(icns) = icns {
        std::fs::copy(icns, app.join("Resources/bergpdf.icns"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn bundle_layout_and_plist() {
        let d = std::env::temp_dir().join(format!("berg-bundle-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let fake = d.join("bergpdf");
        std::fs::write(&fake, b"x").unwrap();
        let icns = d.join("bergpdf.icns");
        std::fs::write(&icns, b"icns").unwrap();
        write_app_bundle(&d, &fake, Some(&icns), "1.2.3").unwrap();
        assert!(
            d.join("BergPDF.app/Contents/Resources/bergpdf.icns")
                .exists()
        );
        assert!(d.join("BergPDF.app/Contents/MacOS/bergpdf").exists());
        let p = std::fs::read_to_string(d.join("BergPDF.app/Contents/Info.plist")).unwrap();
        assert!(p.contains("<string>1.2.3</string>") && p.contains("com.adobe.pdf"));
        assert!(p.contains("CFBundleExecutable"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
