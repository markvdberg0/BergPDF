//! The Microsoft Store package: an MSIX built from the same release binary as the other installers.
//!
//! `cargo xtask msix` stages a package layout (program, manifest, logos, licences, a marker file that tells
//! the program it is the Store copy), indexes the logos with `makepri` and packs the layout with `makeappx`
//! (both are part of the Windows SDK). The package is **not signed**: the Store signs what is uploaded to
//! Partner Center. To try a package on a machine see docs/MICROSOFT_STORE.md.
//!
//! The manifest and the asset list are plain functions so they can be unit-tested on any host; only the
//! final steps (the SDK tools) need Windows.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Name shown in the Start menu and the Store. Must be the name reserved in Partner Center.
const DISPLAY_NAME: &str = "BergPDF";
/// File name of the program inside the package.
const EXE: &str = "bergpdf.exe";
/// Oldest Windows 10 the package installs on (1809). The Store lists it as the minimum version.
const MIN_WINDOWS: &str = "10.0.17763.0";
/// Newest Windows version the package has been checked against.
const TESTED_WINDOWS: &str = "10.0.26100.0";

/// Who the package belongs to. Partner Center assigns all three when the app name is reserved
/// (*Product management ▸ Product identity*); the Store refuses a package whose identity differs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// `Package/Identity/Name`, e.g. `12345Publisher.BergPDF`.
    pub name: String,
    /// `Package/Identity/Publisher`, e.g. `CN=0123ABCD-…`.
    pub publisher: String,
    /// `Package/Properties/PublisherDisplayName`.
    pub publisher_display_name: String,
    /// A stand-in for local testing; the Store would reject a package built with it.
    pub placeholder: bool,
}

impl Identity {
    fn placeholder() -> Self {
        Identity {
            name: "BergPDF.Development".into(),
            publisher: "CN=BergPDF Development".into(),
            publisher_display_name: "BergPDF (development build)".into(),
            placeholder: true,
        }
    }

    /// From `--identity-name`, `--publisher` and `--publisher-display-name` (or the matching `BERG_STORE_*`
    /// environment variables). All three or none; `store` (`--store`) demands them.
    pub fn from_inputs(
        args: &[String],
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Identity, String> {
        let mut store = false;
        let mut name = None;
        let mut publisher = None;
        let mut display = None;
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let mut value = |what: &str| {
                it.next()
                    .cloned()
                    .ok_or_else(|| format!("{what} needs a value"))
            };
            match a.as_str() {
                "--store" => store = true,
                "--identity-name" => name = Some(value("--identity-name")?),
                "--publisher" => publisher = Some(value("--publisher")?),
                "--publisher-display-name" => display = Some(value("--publisher-display-name")?),
                other => return Err(format!("unknown option {other}")),
            }
        }
        let from_env = |key: &str| env(key).filter(|v| !v.trim().is_empty());
        let name = name.or_else(|| from_env("BERG_STORE_IDENTITY_NAME"));
        let publisher = publisher.or_else(|| from_env("BERG_STORE_PUBLISHER"));
        let display = display.or_else(|| from_env("BERG_STORE_PUBLISHER_DISPLAY_NAME"));
        match (name, publisher, display) {
            (Some(name), Some(publisher), Some(publisher_display_name)) => {
                let id = Identity {
                    name,
                    publisher,
                    publisher_display_name,
                    placeholder: false,
                };
                id.validate()?;
                Ok(id)
            }
            (None, None, None) if !store => Ok(Identity::placeholder()),
            (None, None, None) => Err(missing_identity()),
            _ => Err(format!(
                "the package identity is incomplete. {}",
                missing_identity()
            )),
        }
    }

    fn validate(&self) -> Result<(), String> {
        let ok_name = (3..=50).contains(&self.name.len())
            && self
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
        if !ok_name {
            return Err(format!(
                "identity name {:?} must be 3-50 letters, digits, '.' or '-'",
                self.name
            ));
        }
        if !self.publisher.starts_with("CN=") {
            return Err(format!(
                "publisher {:?} must be the distinguished name from Partner Center (it starts with CN=)",
                self.publisher
            ));
        }
        if self.publisher_display_name.trim().is_empty() {
            return Err("the publisher display name is empty".into());
        }
        Ok(())
    }
}

fn missing_identity() -> String {
    "Give the identity from Partner Center (Product management > Product identity) as \
     --identity-name, --publisher and --publisher-display-name, or as the environment variables \
         BERG_STORE_IDENTITY_NAME, BERG_STORE_PUBLISHER and BERG_STORE_PUBLISHER_DISPLAY_NAME."
        .into()
}

/// `0.2.2` becomes `0.2.2.0`: MSIX versions have four parts and the Store reserves the last one (it must be 0).
pub fn msix_version(version: &str) -> Result<String, String> {
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|p| p.is_empty() || p.parse::<u16>().is_err())
    {
        return Err(format!(
            "version {version:?} is not major.minor.patch with numbers up to 65535"
        ));
    }
    Ok(format!("{core}.0"))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `AppxManifest.xml`. BergPDF is a desktop program that runs with the user's own rights, which a package
/// declares with the `runFullTrust` capability (the Store asks for a reason; see docs/MICROSOFT_STORE.md).
pub fn manifest(id: &Identity, version: &str) -> String {
    let name = xml_escape(&id.name);
    let publisher = xml_escape(&id.publisher);
    let publisher_display = xml_escape(&id.publisher_display_name);
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Package
  xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
  xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
  xmlns:uap3="http://schemas.microsoft.com/appx/manifest/uap/windows10/3"
  xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
  IgnorableNamespaces="uap uap3 rescap">
  <Identity Name="{name}" Publisher="{publisher}" Version="{version}" ProcessorArchitecture="x64" />
  <Properties>
    <DisplayName>{DISPLAY_NAME}</DisplayName>
    <PublisherDisplayName>{publisher_display}</PublisherDisplayName>
    <Logo>Assets\StoreLogo.png</Logo>
  </Properties>
  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="{MIN_WINDOWS}" MaxVersionTested="{TESTED_WINDOWS}" />
  </Dependencies>
  <Resources>
    <Resource Language="en-us" />
    <Resource Language="nl-nl" />
    <Resource Language="de-de" />
  </Resources>
  <Applications>
    <Application Id="BergPDF" Executable="{EXE}" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements
        DisplayName="{DISPLAY_NAME}"
        Description="Offline PDF editor"
        BackgroundColor="transparent"
        Square150x150Logo="Assets\Square150x150Logo.png"
        Square44x44Logo="Assets\Square44x44Logo.png" />
      <Extensions>
        <uap3:Extension Category="windows.fileTypeAssociation">
          <uap3:FileTypeAssociation Name="pdf" Parameters="&quot;%1&quot;">
            <uap:SupportedFileTypes>
              <uap:FileType>.pdf</uap:FileType>
            </uap:SupportedFileTypes>
            <uap:DisplayName>PDF document</uap:DisplayName>
            <uap:Logo>Assets\PdfFile.png</uap:Logo>
          </uap3:FileTypeAssociation>
        </uap3:Extension>
      </Extensions>
    </Application>
  </Applications>
  <Capabilities>
    <rescap:Capability Name="runFullTrust" />
  </Capabilities>
</Package>
"#
    )
}

/// Logo files of the package: `(file name, pixels)`, all square. Scale variants for the tile and the Store
/// logo, target-size variants (plain and "unplated", so the taskbar and the Alt+Tab list show the logo itself
/// and not a coloured plate behind it) for the small icon.
pub fn logo_assets() -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for (base, px) in [
        ("Square44x44Logo", 44u32),
        ("Square150x150Logo", 150),
        ("StoreLogo", 50),
    ] {
        for scale in [100u32, 125, 150, 200, 400] {
            out.push((
                format!("{base}.scale-{scale}.png"),
                (px * scale).div_ceil(100),
            ));
        }
    }
    for size in [16u32, 24, 32, 48, 256] {
        out.push((format!("Square44x44Logo.targetsize-{size}.png"), size));
        out.push((
            format!("Square44x44Logo.targetsize-{size}_altform-unplated.png"),
            size,
        ));
    }
    out
}

/// The largest picture in an `.ico` file, when it is stored as PNG (as the ones generated here are).
pub fn largest_png_in_ico(ico: &[u8]) -> Option<Vec<u8>> {
    let count = usize::from(u16::from_le_bytes([*ico.get(4)?, *ico.get(5)?]));
    let mut best: Option<(u32, &[u8])> = None;
    for i in 0..count {
        let e = ico.get(6 + 16 * i..6 + 16 * (i + 1))?;
        let width = if e[0] == 0 { 256 } else { u32::from(e[0]) };
        let len = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
        let offset = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
        let data = ico.get(offset..offset.checked_add(len)?)?;
        if data.starts_with(b"\x89PNG\r\n\x1a\n") && best.is_none_or(|(w, _)| width > w) {
            best = Some((width, data));
        }
    }
    best.map(|(_, d)| d.to_vec())
}

/// Lay the package out in `layout`: program, marker, manifest, logos, licence files.
pub fn stage(
    layout: &Path,
    exe: &Path,
    root: &Path,
    id: &Identity,
    version: &str,
) -> Result<(), String> {
    let io = |e: io::Error| e.to_string();
    let _ = std::fs::remove_dir_all(layout);
    let assets = layout.join("Assets");
    std::fs::create_dir_all(&assets).map_err(io)?;
    std::fs::copy(exe, layout.join(EXE)).map_err(io)?;
    // Tells the program it is the Store copy (no update check of its own): see platform::distribution.
    std::fs::write(
        layout.join(platform::distribution::STORE_MARKER),
        "BergPDF is installed from the Microsoft Store, which updates it.\n",
    )
    .map_err(io)?;
    std::fs::write(layout.join("AppxManifest.xml"), manifest(id, version)).map_err(io)?;
    for (file, px) in logo_assets() {
        std::fs::write(assets.join(file), crate::icons::png_bytes(px).map_err(io)?).map_err(io)?;
    }
    // The picture Explorer shows for PDF files that open with BergPDF.
    let ico = std::fs::read(root.join("assets/icons/pdf-file.ico")).map_err(io)?;
    let file_icon = match largest_png_in_ico(&ico) {
        Some(png) => png,
        None => crate::icons::png_bytes(256).map_err(io)?,
    };
    std::fs::write(assets.join("PdfFile.png"), file_icon).map_err(io)?;
    for f in [
        "LICENSE",
        "README.md",
        "docs/THIRD_PARTY_LICENSES.md",
        "docs/DEPENDENCIES.md",
    ] {
        let src = root.join(f);
        let name = Path::new(f).file_name().unwrap_or_default();
        std::fs::copy(&src, layout.join(name)).map_err(|e| format!("{}: {e}", src.display()))?;
    }
    Ok(())
}

/// A Windows SDK tool: on `PATH`, else in the newest `Windows Kits\10\bin\<version>\x64`.
fn sdk_tool(name: &str) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    let pf = std::env::var_os("ProgramFiles(x86)")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));
    let bin = pf.join(r"Windows Kits\10\bin");
    let key = |p: &PathBuf| -> Vec<u32> {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.split('.').map(|x| x.parse().unwrap_or(0)).collect())
            .unwrap_or_default()
    };
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&bin)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    versions.sort_by_key(key);
    versions
        .iter()
        .rev()
        .map(|v| v.join("x64").join(name))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            format!("{name} not found: install the Windows 11 SDK (winget install Microsoft.WindowsSDK.10.0.26100)")
        })
}

fn run(tool: &Path, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let status = Command::new(tool)
        .args(args)
        .status()
        .map_err(|e| format!("could not run {}: {e}", tool.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} failed ({status})", tool.display()))
    }
}

/// `cargo xtask msix [--store] [--identity-name N --publisher CN=… --publisher-display-name D]`.
pub fn build(args: &[String]) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("MSIX packages are built on Windows (they need the Windows SDK tools)".into());
    }
    let id = Identity::from_inputs(args, |k| std::env::var(k).ok())?;
    let version = msix_version(env!("CARGO_PKG_VERSION"))?;
    // Fail before the long build when the tools are missing.
    let makepri = sdk_tool("makepri.exe")?;
    let makeappx = sdk_tool("makeappx.exe")?;
    let exe = crate::build_dist_exe()?;
    let root = crate::root();
    let work = root.join("dist/msix");
    let layout = work.join("layout");
    stage(&layout, &exe, &root, &id, &version)?;

    // Index the logos (scale and target-size variants) into resources.pri so Windows picks the right one.
    let config = work.join("priconfig.xml");
    run(
        &makepri,
        &[
            "createconfig".as_ref(),
            "/cf".as_ref(),
            config.as_os_str(),
            "/dq".as_ref(),
            "en-US".as_ref(),
            "/pv".as_ref(),
            "10.0.0".as_ref(),
            "/o".as_ref(),
        ],
    )?;
    run(
        &makepri,
        &[
            "new".as_ref(),
            "/pr".as_ref(),
            layout.as_os_str(),
            "/cf".as_ref(),
            config.as_os_str(),
            "/mn".as_ref(),
            layout.join("AppxManifest.xml").as_os_str(),
            "/of".as_ref(),
            layout.join("resources.pri").as_os_str(),
            "/o".as_ref(),
        ],
    )?;

    let out_dir = root.join("dist/packages");
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let stem = if id.placeholder {
        "BergPDF-DEVELOPMENT-NOT-FOR-THE-STORE"
    } else {
        "BergPDF"
    };
    let package = out_dir.join(format!("{stem}_{version}_x64.msix"));
    run(
        &makeappx,
        &[
            "pack".as_ref(),
            "/d".as_ref(),
            layout.as_os_str(),
            "/p".as_ref(),
            package.as_os_str(),
            "/o".as_ref(),
        ],
    )?;
    println!("MSIX package: {}", package.display());
    println!(
        "layout (for `Add-AppxPackage -Register`): {}",
        layout.display()
    );
    if id.placeholder {
        println!(
            "WARNING: built with a placeholder identity; Partner Center will refuse it. For the real one pass \
             --store with the identity from Partner Center (docs/MICROSOFT_STORE.md)."
        );
    } else {
        println!(
            "identity {} / {}: upload this file to Partner Center (unsigned is right: the Store signs it).",
            id.name, id.publisher
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn identity() -> Identity {
        Identity {
            name: "12345Publisher.BergPDF".into(),
            publisher: "CN=0123ABCD-0000-1111-2222-333344445555".into(),
            publisher_display_name: "A & B <Berg>".into(),
            placeholder: false,
        }
    }

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn versions_get_a_fourth_part_that_is_zero() {
        assert_eq!(msix_version("0.2.2").unwrap(), "0.2.2.0");
        assert_eq!(msix_version("1.0.0-rc1").unwrap(), "1.0.0.0");
        assert!(msix_version("1.2").is_err());
        assert!(msix_version("1.2.x").is_err());
        assert!(msix_version("1.2.70000").is_err());
    }

    #[test]
    fn the_manifest_carries_identity_version_and_the_pdf_association() {
        let m = manifest(&identity(), "0.2.2.0");
        assert!(m.contains(r#"Name="12345Publisher.BergPDF""#));
        assert!(m.contains(r#"Publisher="CN=0123ABCD-0000-1111-2222-333344445555""#));
        assert!(m.contains(r#"Version="0.2.2.0""#));
        assert!(m.contains(r#"Executable="bergpdf.exe""#));
        assert!(m.contains("<uap:FileType>.pdf</uap:FileType>"));
        assert!(m.contains(r#"Parameters="&quot;%1&quot;""#));
        assert!(m.contains(r#"<rescap:Capability Name="runFullTrust" />"#));
        // Text from outside is escaped, so a stray `&` cannot break the file.
        assert!(m.contains("<PublisherDisplayName>A &amp; B &lt;Berg&gt;</PublisherDisplayName>"));
        // Every logo the manifest names exists in the asset list (as a plain or a qualified file).
        let files: Vec<String> = logo_assets().into_iter().map(|(f, _)| f).collect();
        for logo in ["Square44x44Logo", "Square150x150Logo", "StoreLogo"] {
            assert!(m.contains(&format!(r"Assets\{logo}.png")));
            assert!(files.iter().any(|f| f.starts_with(&format!("{logo}."))));
        }
    }

    #[test]
    fn logo_sizes_follow_the_scale() {
        let sizes: std::collections::HashMap<String, u32> = logo_assets().into_iter().collect();
        assert_eq!(sizes["Square44x44Logo.scale-100.png"], 44);
        assert_eq!(sizes["Square44x44Logo.scale-200.png"], 88);
        assert_eq!(sizes["Square150x150Logo.scale-125.png"], 188);
        assert_eq!(sizes["StoreLogo.scale-400.png"], 200);
        assert_eq!(
            sizes["Square44x44Logo.targetsize-48_altform-unplated.png"],
            48
        );
    }

    #[test]
    fn identity_comes_from_flags_or_environment_all_or_nothing() {
        let none = |_: &str| None;
        assert!(Identity::from_inputs(&[], none).unwrap().placeholder);
        assert!(Identity::from_inputs(&args(&["--store"]), none).is_err());
        let full = args(&[
            "--identity-name",
            "12345Publisher.BergPDF",
            "--publisher",
            "CN=ABC",
            "--publisher-display-name",
            "Mark",
        ]);
        let id = Identity::from_inputs(&full, none).unwrap();
        assert_eq!(id.publisher, "CN=ABC");
        assert!(!id.placeholder);
        // Half an identity is an error, not a silent placeholder.
        assert!(Identity::from_inputs(&args(&["--publisher", "CN=ABC"]), none).is_err());
        // The environment fills in what the flags leave out; empty values count as unset.
        let env = |k: &str| match k {
            "BERG_STORE_IDENTITY_NAME" => Some("12345Publisher.BergPDF".to_string()),
            "BERG_STORE_PUBLISHER" => Some("CN=ABC".to_string()),
            "BERG_STORE_PUBLISHER_DISPLAY_NAME" => Some("Mark".to_string()),
            _ => None,
        };
        assert!(Identity::from_inputs(&args(&["--store"]), env).is_ok());
        let blank = |_: &str| Some("  ".to_string());
        assert!(Identity::from_inputs(&[], blank).unwrap().placeholder);
        // Values Partner Center would not accept are refused here, before the long build.
        let bad = args(&[
            "--identity-name",
            "x",
            "--publisher",
            "CN=ABC",
            "--publisher-display-name",
            "Mark",
        ]);
        assert!(Identity::from_inputs(&bad, none).is_err());
        let bad = args(&[
            "--identity-name",
            "12345Publisher.BergPDF",
            "--publisher",
            "Mark",
            "--publisher-display-name",
            "Mark",
        ]);
        assert!(Identity::from_inputs(&bad, none).is_err());
        assert!(Identity::from_inputs(&args(&["--nonsense"]), none).is_err());
    }

    #[test]
    fn the_file_icon_is_taken_from_the_pdf_ico() {
        let ico = std::fs::read(crate::root().join("assets/icons/pdf-file.ico")).unwrap();
        let png = largest_png_in_ico(&ico).expect("pdf-file.ico holds PNG pictures");
        assert!(png.starts_with(b"\x89PNG"));
        assert!(largest_png_in_ico(b"nope").is_none());
    }

    #[test]
    fn staging_writes_everything_the_package_needs() {
        let dir = std::env::temp_dir().join(format!("berg-msix-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("fake.exe");
        std::fs::write(&exe, b"MZ").unwrap();
        let layout = dir.join("layout");
        stage(&layout, &exe, &crate::root(), &identity(), "0.2.2.0").unwrap();
        for f in [
            "bergpdf.exe",
            "AppxManifest.xml",
            "LICENSE",
            "THIRD_PARTY_LICENSES.md",
            "Assets/PdfFile.png",
            "Assets/Square44x44Logo.targetsize-256.png",
        ] {
            assert!(layout.join(f).is_file(), "{f} missing");
        }
        // The program recognises the Store copy by this file.
        assert_eq!(
            platform::distribution::detect(Some(&layout), None),
            platform::distribution::Channel::Store
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
