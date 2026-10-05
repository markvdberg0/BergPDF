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
