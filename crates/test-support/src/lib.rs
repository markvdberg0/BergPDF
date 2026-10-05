//! Shared helpers for tests: fixture lookup, bitmap comparison and wrappers around
//! *independent* PDF tools (poppler) used purely as test oracles. These tools are optional;
//! tests skip the oracle check (and say so) when they are not installed.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod fixtures;

use std::path::{Path, PathBuf};
use std::process::Command;

/// Repository root (two levels above this crate).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// Absolute path of a file in `tests/fixtures`.
pub fn fixture(name: &str) -> PathBuf {
    repo_root().join("tests/fixtures").join(name)
}

/// Read a fixture's bytes.
pub fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(fixture(name)).unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

/// True if an external tool is on PATH.
pub fn have_tool(tool: &str) -> bool {
    Command::new(tool)
        .arg("-v")
        .output()
        .map(|_| true)
        .unwrap_or(false)
}

/// Extract text with `pdftotext` (poppler) from PDF bytes; `None` if unavailable.
pub fn poppler_text(bytes: &[u8]) -> Option<String> {
    if !have_tool("pdftotext") {
        return None;
    }
    let dir = tempfile::tempdir().ok()?;
    let p = dir.path().join("in.pdf");
    std::fs::write(&p, bytes).ok()?;
    let out = Command::new("pdftotext")
        .arg("-layout")
        .arg(&p)
        .arg("-")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Page count according to `pdfinfo`; also returns `Some(Err(stderr))` when poppler
/// reports syntax errors on stderr (a strictness signal for our writer).
pub fn poppler_info(bytes: &[u8]) -> Option<(Option<usize>, String)> {
    if !have_tool("pdfinfo") {
        return None;
    }
    let dir = tempfile::tempdir().ok()?;
    let p = dir.path().join("in.pdf");
    std::fs::write(&p, bytes).ok()?;
    let out = Command::new("pdfinfo").arg(&p).output().ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let pages = stdout
        .lines()
        .find_map(|l| l.strip_prefix("Pages:"))
        .and_then(|v| v.trim().parse().ok());
    Some((pages, String::from_utf8_lossy(&out.stderr).into_owned()))
}

/// Render page `n` (1-based) with `pdftoppm` at `dpi`, returning RGB bytes and size.
pub fn poppler_render(bytes: &[u8], page: usize, dpi: u32) -> Option<(u32, u32, Vec<u8>)> {
    if !have_tool("pdftoppm") {
        return None;
    }
    let dir = tempfile::tempdir().ok()?;
    let p = dir.path().join("in.pdf");
    std::fs::write(&p, bytes).ok()?;
    let out = dir.path().join("out");
    let st = Command::new("pdftoppm")
        .args([
            "-r",
            &dpi.to_string(),
            "-f",
            &page.to_string(),
            "-l",
            &page.to_string(),
            "-png",
        ])
        .arg(&p)
        .arg(&out)
        .status()
        .ok()?;
    if !st.success() {
        return None;
    }
    let file = std::fs::read_dir(dir.path())
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "png"))?;
    decode_png_rgb(&file)
}

/// Decode a PNG file to RGB8.
pub fn decode_png_rgb(path: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let rgb = match info.color_type {
        png::ColorType::Rgb => buf,
        png::ColorType::Rgba => buf
            .chunks_exact(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|g| [*g, *g, *g]).collect(),
        _ => return None,
    };
    Some((info.width, info.height, rgb))
}

/// Mean absolute per-channel difference (0..255) between two same-size RGB or RGBA buffers.
pub fn mean_abs_diff(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len(), "buffers differ in size");
    let s: u64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| u64::from(x.abs_diff(*y)))
        .sum();
    s as f64 / a.len().max(1) as f64
}

/// RGBA → RGB.
pub fn rgba_to_rgb(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect()
}

/// Fraction of pixels in an RGB buffer region that differ from white by more than `thr`.
pub fn non_white_fraction(
    rgb: &[u8],
    width: u32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    thr: u8,
) -> f64 {
    let mut n = 0u64;
    let mut tot = 0u64;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * width + x) * 3) as usize;
            tot += 1;
            if rgb[i..i + 3].iter().any(|c| 255 - *c > thr) {
                n += 1;
            }
        }
    }
    n as f64 / tot.max(1) as f64
}

/// True when the environment demands that independent-oracle checks (poppler) really run
/// (`BERG_REQUIRE_ORACLES=1`, set in CI where the tools are installed). Locally, a missing
/// tool makes those assertions skip rather than fail.
pub fn oracles_required() -> bool {
    std::env::var("BERG_REQUIRE_ORACLES").is_ok_and(|v| v == "1")
}

/// Full `pdfinfo` stdout (Title, Author, Pages, …) for `bytes`, if poppler is installed.
pub fn poppler_info_text(bytes: &[u8]) -> Option<String> {
    if !have_tool("pdfinfo") {
        return None;
    }
    let dir = tempfile::tempdir().ok()?;
    let p = dir.path().join("in.pdf");
    std::fs::write(&p, bytes).ok()?;
    let out = Command::new("pdfinfo").arg(&p).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Output of poppler's `pdfsig` for `bytes` (stdout + stderr), if the tool is installed.
pub fn poppler_pdfsig(bytes: &[u8]) -> Option<String> {
    if !have_tool("pdfsig") {
        return None;
    }
    let dir = tempfile::tempdir().ok()?;
    let p = dir.path().join("in.pdf");
    std::fs::write(&p, bytes).ok()?;
    let out = Command::new("pdfsig").arg(&p).output().ok()?;
    Some(format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    ))
}

/// Verify a detached DER CMS signature over `content` with OpenSSL (`openssl cms -verify
/// -noverify`: signature maths only). `None` when no suitable OpenSSL (1.1+/3.x) is installed.
pub fn openssl_cms_verify(cms_der: &[u8], content: &[u8]) -> Option<bool> {
    let ver = Command::new("openssl").arg("version").output().ok()?;
    let v = String::from_utf8_lossy(&ver.stdout).to_string();
    if !v.starts_with("OpenSSL") {
        return None; // LibreSSL has no `cms` command
    }
    let dir = tempfile::tempdir().ok()?;
    let (sig, data) = (dir.path().join("s.der"), dir.path().join("d.bin"));
    std::fs::write(&sig, cms_der).ok()?;
    std::fs::write(&data, content).ok()?;
    let out = Command::new("openssl")
        .args([
            "cms",
            "-verify",
            "-binary",
            "-inform",
            "DER",
            "-noverify",
            "-in",
        ])
        .arg(&sig)
        .arg("-content")
        .arg(&data)
        .args(["-out", "/dev/null"])
        .output()
        .ok()?;
    Some(out.status.success())
}

/// Plain text of one page (1-based) via `pdftotext`, if installed.
pub fn poppler_text_page(bytes: &[u8], page: usize) -> Option<String> {
    if !have_tool("pdftotext") {
        return None;
    }
    let dir = tempfile::tempdir().ok()?;
    let p = dir.path().join("in.pdf");
    std::fs::write(&p, bytes).ok()?;
    let out = Command::new("pdftotext")
        .args([
            "-f",
            &page.to_string(),
            "-l",
            &page.to_string(),
            "-enc",
            "UTF-8",
        ])
        .arg(&p)
        .arg("-")
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A word with its box in PDF points (top-left origin), from `pdftotext -bbox`.
#[derive(Clone, Debug)]
pub struct BBoxWord {
    /// Word text.
    pub text: String,
    /// Left.
    pub x0: f64,
    /// Top.
    pub y0: f64,
    /// Right.
    pub x1: f64,
    /// Bottom.
    pub y1: f64,
}

/// Words and boxes of a page (1-based) according to poppler.
pub fn poppler_bbox(bytes: &[u8], page: usize) -> Option<Vec<BBoxWord>> {
    if !have_tool("pdftotext") {
        return None;
    }
    let dir = tempfile::tempdir().ok()?;
    let p = dir.path().join("in.pdf");
    std::fs::write(&p, bytes).ok()?;
    let out = Command::new("pdftotext")
        .args(["-bbox", "-f", &page.to_string(), "-l", &page.to_string()])
        .arg(&p)
        .arg("-")
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let attr = |line: &str, k: &str| -> Option<f64> {
        let i = line.find(&format!("{k}=\""))? + k.len() + 2;
        line[i..].split('"').next()?.parse().ok()
    };
    let mut words = Vec::new();
    for line in s.lines().filter(|l| l.trim_start().starts_with("<word ")) {
        let text = line.split('>').nth(1)?.split('<').next()?.to_string();
        words.push(BBoxWord {
            text,
            x0: attr(line, "xMin")?,
            y0: attr(line, "yMin")?,
            x1: attr(line, "xMax")?,
            y1: attr(line, "yMax")?,
        });
    }
    Some(words)
}

/// Everything poppler's `pdftotext` writes to stderr while reading `bytes` (empty = it found
/// nothing to complain about). `None` when the tool is missing.
pub fn poppler_diagnostics(bytes: &[u8]) -> Option<String> {
    if !have_tool("pdftotext") {
        return None;
    }
    let dir = tempfile::tempdir().ok()?;
    let p = dir.path().join("in.pdf");
    std::fs::write(&p, bytes).ok()?;
    let out = Command::new("pdftotext").arg(&p).arg("-").output().ok()?;
    Some(String::from_utf8_lossy(&out.stderr).into_owned())
}

/// Result of a veraPDF validation.
#[derive(Debug, Clone)]
pub struct VeraReport {
    /// veraPDF's verdict.
    pub compliant: bool,
    /// One line per failed rule: `clause-test (n checks): description`.
    pub failures: Vec<String>,
}

/// Directory with the veraPDF jars (`BERG_VERAPDF`), if configured.
pub fn verapdf_dir() -> Option<PathBuf> {
    let d = PathBuf::from(std::env::var_os("BERG_VERAPDF")?);
    d.is_dir().then_some(d)
}

/// True when veraPDF validation must really run (`BERG_REQUIRE_VERAPDF=1`).
pub fn verapdf_required() -> bool {
    std::env::var("BERG_REQUIRE_VERAPDF").is_ok_and(|v| v == "1")
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let s = tag.find(&key)? + key.len();
    let e = tag[s..].find('"')?;
    Some(&tag[s..s + e])
}

/// Validate `bytes` against a PDF/A flavour (`"2b"`, `"1b"`…) with veraPDF, if it is installed
/// (see `docs/EVIDENCE.md` for how it was obtained). `None` when it is not available.
pub fn verapdf(bytes: &[u8], flavour: &str) -> Option<VeraReport> {
    let dir = verapdf_dir()?;
    if !have_tool("java") {
        return None;
    }
    let tmp = tempfile::tempdir().ok()?;
    let p = tmp.path().join("in.pdf");
    std::fs::write(&p, bytes).ok()?;
    let cp = format!("{}/*", dir.display());
    let out = Command::new("java")
        .env_remove("JAVA_TOOL_OPTIONS")
        .args([
            "-cp",
            &cp,
            "org.verapdf.apps.GreenfieldCliWrapper",
            "--flavour",
            flavour,
            "--format",
            "xml",
        ])
        .arg(&p)
        .output()
        .ok()?;
    let xml = String::from_utf8_lossy(&out.stdout).into_owned();
    let report_tag_start = xml.find("<validationReport")?;
    let report_tag = &xml[report_tag_start..xml[report_tag_start..].find('>')? + report_tag_start];
    let compliant = attr(report_tag, "isCompliant")? == "true";
    let mut failures = Vec::new();
    let mut rest = xml.as_str();
    while let Some(i) = rest.find("<rule ") {
        rest = &rest[i..];
        let end = rest.find('>').unwrap_or(rest.len());
        let tag = &rest[..end];
        if attr(tag, "status") == Some("failed") {
            let desc = rest
                .find("<description>")
                .and_then(|d| {
                    let s = d + "<description>".len();
                    rest[s..]
                        .find("</description>")
                        .map(|e| rest[s..s + e].to_string())
                })
                .unwrap_or_default();
            failures.push(format!(
                "{}-{} ({} checks): {}",
                attr(tag, "clause").unwrap_or("?"),
                attr(tag, "testNumber").unwrap_or("?"),
                attr(tag, "failedChecks").unwrap_or("?"),
                desc
            ));
        }
        rest = &rest[end..];
    }
    Some(VeraReport {
        compliant,
        failures,
    })
}
