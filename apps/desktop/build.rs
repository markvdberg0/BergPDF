//! Embeds the program icon and version information into `bergpdf.exe` on Windows, so Explorer,
//! the taskbar and shortcuts show the BergPDF icon. Does nothing on other platforms.

fn main() {
    #[cfg(windows)]
    windows_resources();
}

#[cfg(windows)]
fn windows_resources() {
    use std::path::PathBuf;
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let icon = manifest.join("../../assets/icons/bergpdf.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rerun-if-changed=build.rs");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let mut parts = version
        .split(|c: char| !c.is_ascii_digit())
        .map(|p| p.parse::<u16>().unwrap_or(0));
    let mut next = || parts.next().unwrap_or(0);
    let (a, b, c) = (next(), next(), next());
    // The resource compiler wants forward slashes (or doubled backslashes) in paths.
    let icon_path = icon.to_string_lossy().replace('\\', "/");
    let rc = format!(
        r#"1 ICON "{icon_path}"
1 VERSIONINFO
FILEVERSION {a},{b},{c},0
PRODUCTVERSION {a},{b},{c},0
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "ProductName", "BergPDF"
      VALUE "FileDescription", "BergPDF"
      VALUE "OriginalFilename", "bergpdf.exe"
      VALUE "FileVersion", "{version}"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default()).join("bergpdf.rc");
    if std::fs::write(&out, rc).is_err() {
        println!("cargo:warning=could not write the resource script; the exe will have no icon");
        return;
    }
    // A missing resource compiler must not break the build, but it is worth a warning.
    if let Err(e) = embed_resource::compile(&out, embed_resource::NONE).manifest_optional() {
        println!("cargo:warning=embedding the icon failed: {e}");
    }
}
