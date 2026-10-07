//! Printing.
//!
//! * **Windows**: each page is drawn by the PDF renderer at the printer's resolution (at most 300 dpi, scaled up
//!   by the driver) and sent to the printer through GDI (`StartDoc` / `StretchDIBits` / `EndPage`). The pages go
//!   as pictures, so any printer driver can print them, including *Microsoft Print to PDF*. This is the only code
//!   in the workspace that uses `unsafe` (see `windows.rs`).
//! * **macOS and Linux**: the PDF is handed to CUPS with `lp`, which prints PDF natively (page ranges, copies and
//!   scaling are options of `lp`).
//!
//! The pure parts (page ranges, placement on the paper, the `lp` command line) are testable everywhere.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::path::PathBuf;
use std::sync::Arc;

#[cfg(not(windows))]
mod unix;
#[cfg(windows)]
mod windows;

/// A printer the system knows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Printer {
    /// Name to pass to [`print`].
    pub name: String,
    /// The system's default printer.
    pub is_default: bool,
}

/// How a page is sized on the paper.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Scale {
    /// Shrink a page that is larger than the printable area so that it fits; never enlarge.
    #[default]
    ShrinkToFit,
    /// Print at the page's own size (anything outside the printable area is cut off by the printer).
    ActualSize,
}

/// What to print.
#[derive(Clone, Debug)]
pub struct PrintRequest {
    /// Printer name from [`printers`].
    pub printer: String,
    /// Name of the job in the print queue.
    pub title: String,
    /// Number of copies (1 or more); each copy holds all pages in order.
    pub copies: u32,
    /// Zero-based page indexes to print, in order.
    pub pages: Vec<usize>,
    /// Page sizing.
    pub scale: Scale,
    /// Write the job to this file instead of the paper (the printer must be able to: *Microsoft Print to PDF*
    /// is). For tests.
    pub output_file: Option<PathBuf>,
}

/// Why printing did not happen.
#[derive(Debug, thiserror::Error)]
pub enum PrintError {
    /// The request itself is wrong (no pages, unknown printer…).
    #[error("{0}")]
    Invalid(String),
    /// The system or the printer refused.
    #[error("{0}")]
    System(String),
    /// A page could not be drawn.
    #[error("page {0} could not be drawn: {1}")]
    Render(usize, String),
    /// Printing is not available on this system.
    #[error("{0}")]
    Unavailable(String),
}

/// The printers of this computer, the default one first.
pub fn printers() -> Vec<Printer> {
    #[cfg(windows)]
    let mut v = windows::printers();
    #[cfg(not(windows))]
    let mut v = unix::printers();
    v.sort_by_key(|p| !p.is_default);
    v
}

/// Print `bytes` (a plain, unencrypted PDF). `progress(done, total)` is called after every page that went to the
/// printer (on Windows; CUPS reports nothing until it has the whole job).
pub fn print(
    bytes: Arc<Vec<u8>>,
    req: &PrintRequest,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<(), PrintError> {
    if req.pages.is_empty() {
        return Err(PrintError::Invalid("there are no pages to print".into()));
    }
    if req.copies == 0 || req.copies > 999 {
        return Err(PrintError::Invalid(
            "the number of copies must be between 1 and 999".into(),
        ));
    }
    platform_print(bytes, req, progress)
}

#[cfg(windows)]
use windows::print as platform_print;

#[cfg(not(windows))]
fn platform_print(
    bytes: Arc<Vec<u8>>,
    req: &PrintRequest,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<(), PrintError> {
    unix::print(&bytes, req, progress)
}

/// `"1-3, 5, 8-"` → zero-based page indexes `[0, 1, 2, 4, 7, …]` for a document of `total` pages. An empty text
/// means every page. Pages may be repeated and given in any order (the order is kept).
pub fn parse_page_ranges(text: &str, total: usize) -> Result<Vec<usize>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok((0..total).collect());
    }
    let mut out = Vec::new();
    for part in text.split([',', ';']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let num = |s: &str| -> Result<usize, String> {
            s.trim()
                .parse::<usize>()
                .ok()
                .filter(|n| *n >= 1)
                .ok_or_else(|| format!("“{}” is not a page number", s.trim()))
        };
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => {
                let a = if a.trim().is_empty() { 1 } else { num(a)? };
                let b = if b.trim().is_empty() { total } else { num(b)? };
                (a, b)
            }
            None => {
                let n = num(part)?;
                (n, n)
            }
        };
        if a > b {
            return Err(format!("“{part}” runs backwards"));
        }
        if b > total {
            return Err(format!("the document has {total} page(s), not {b}"));
        }
        out.extend((a - 1)..b);
    }
    if out.is_empty() {
        return Err("no pages chosen".into());
    }
    Ok(out)
}

/// Where a page goes on the paper.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// Turn the page by a quarter (a landscape page on portrait paper, or the other way round).
    pub rotate: bool,
    /// Left edge in device pixels, from the printable area's corner.
    pub x: i32,
    /// Top edge in device pixels.
    pub y: i32,
    /// Width in device pixels.
    pub width: i32,
    /// Height in device pixels.
    pub height: i32,
}

/// Place a `page_pt` (width, height in points) page on a printable area of `area_px` pixels at `dpi`
/// (horizontal, vertical), centred.
pub fn place(page_pt: (f64, f64), area_px: (i32, i32), dpi: (f64, f64), scale: Scale) -> Placement {
    let (area_w, area_h) = (f64::from(area_px.0.max(1)), f64::from(area_px.1.max(1)));
    let mut page = page_pt;
    let landscape = |w: f64, h: f64| w > h * 1.001;
    let portrait = |w: f64, h: f64| h > w * 1.001;
    let rotate = (landscape(page.0, page.1) && portrait(area_w / dpi.0, area_h / dpi.1))
        || (portrait(page.0, page.1) && landscape(area_w / dpi.0, area_h / dpi.1));
    if rotate {
        page = (page.1, page.0);
    }
    // The page at its own size, in device pixels.
    let (w, h) = (page.0 / 72.0 * dpi.0, page.1 / 72.0 * dpi.1);
    let factor = match scale {
        Scale::ActualSize => 1.0,
        Scale::ShrinkToFit => (area_w / w).min(area_h / h).min(1.0),
    };
    let (w, h) = ((w * factor).round().max(1.0), (h * factor).round().max(1.0));
    Placement {
        rotate,
        x: ((area_w - w) / 2.0).round() as i32,
        y: ((area_h - h) / 2.0).round() as i32,
        width: w as i32,
        height: h as i32,
    }
}

/// `[0, 1, 2, 4]` → `"1-3,5"` (CUPS page-ranges, one-based, in the given order).
pub fn cups_page_ranges(pages: &[usize]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < pages.len() {
        let start = pages[i];
        let mut end = start;
        while i + 1 < pages.len() && pages[i + 1] == end + 1 {
            end += 1;
            i += 1;
        }
        out.push(if start == end {
            format!("{}", start + 1)
        } else {
            format!("{}-{}", start + 1, end + 1)
        });
        i += 1;
    }
    out.join(",")
}

/// The `lp` arguments for a request (without the file name).
pub fn lp_args(req: &PrintRequest) -> Vec<String> {
    let mut a = vec![
        "-d".to_string(),
        req.printer.clone(),
        "-n".to_string(),
        req.copies.to_string(),
        "-t".to_string(),
        req.title.clone(),
        "-o".to_string(),
        format!("page-ranges={}", cups_page_ranges(&req.pages)),
        "-o".to_string(),
    ];
    a.push(match req.scale {
        Scale::ShrinkToFit => "print-scaling=fit".to_string(),
        Scale::ActualSize => "print-scaling=none".to_string(),
    });
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_ranges_in_words_and_numbers() {
        assert_eq!(parse_page_ranges("", 4).unwrap(), vec![0, 1, 2, 3]);
        assert_eq!(parse_page_ranges("2", 4).unwrap(), vec![1]);
        assert_eq!(parse_page_ranges("1-3, 5", 9).unwrap(), vec![0, 1, 2, 4]);
        assert_eq!(parse_page_ranges("8-", 10).unwrap(), vec![7, 8, 9]);
        assert_eq!(parse_page_ranges("-2", 10).unwrap(), vec![0, 1]);
        assert_eq!(parse_page_ranges("3,1", 5).unwrap(), vec![2, 0]);
        assert!(parse_page_ranges("0", 5).is_err());
        assert!(parse_page_ranges("6", 5).is_err());
        assert!(parse_page_ranges("4-2", 5).is_err());
        assert!(parse_page_ranges("a", 5).is_err());
        assert!(parse_page_ranges(",", 5).is_err());
    }

    #[test]
    fn a_page_is_centred_shrunk_and_turned_to_fit() {
        // A4 portrait on a 600-dpi printer with a printable area of 4960 × 7016 pixels (a bit under A4).
        let a4 = (595.0, 842.0);
        let p = place(a4, (4900, 6900), (600.0, 600.0), Scale::ShrinkToFit);
        assert!(!p.rotate);
        assert!(p.width <= 4900 && p.height <= 6900);
        assert!(p.x >= 0 && p.y >= 0);
        // It was shrunk by the same factor in both directions.
        let (ow, oh) = (595.0 / 72.0 * 600.0, 842.0 / 72.0 * 600.0);
        let fw = f64::from(p.width) / ow;
        let fh = f64::from(p.height) / oh;
        assert!((fw - fh).abs() < 0.001, "{fw} {fh}");
        // A small page is not enlarged.
        let small = place(
            (100.0, 200.0),
            (4900, 6900),
            (600.0, 600.0),
            Scale::ShrinkToFit,
        );
        assert_eq!(small.width, (100.0f64 / 72.0 * 600.0).round() as i32);
        assert!((small.x - (4900 - small.width) / 2).abs() <= 1);
        // Actual size keeps the size even when it does not fit.
        let big = place(
            (2000.0, 3000.0),
            (4900, 6900),
            (600.0, 600.0),
            Scale::ActualSize,
        );
        assert_eq!(big.width, (2000.0f64 / 72.0 * 600.0).round() as i32);
        // A landscape page on portrait paper is turned.
        let l = place(
            (842.0, 595.0),
            (4900, 6900),
            (600.0, 600.0),
            Scale::ShrinkToFit,
        );
        assert!(l.rotate);
        assert!(l.height >= 4000 && l.width <= 4900);
        // A square page is never turned.
        assert!(
            !place(
                (500.0, 500.0),
                (4900, 6900),
                (600.0, 600.0),
                Scale::ShrinkToFit
            )
            .rotate
        );
    }

    #[test]
    fn cups_page_ranges_are_one_based_and_compact() {
        assert_eq!(cups_page_ranges(&[0, 1, 2, 4]), "1-3,5");
        assert_eq!(cups_page_ranges(&[3]), "4");
        assert_eq!(cups_page_ranges(&[2, 0]), "3,1");
    }

    #[test]
    fn the_lp_command_line_carries_everything() {
        let req = PrintRequest {
            printer: "Office".into(),
            title: "Report".into(),
            copies: 2,
            pages: vec![0, 1, 4],
            scale: Scale::ShrinkToFit,
            output_file: None,
        };
        let a = lp_args(&req);
        assert_eq!(&a[..4], ["-d", "Office", "-n", "2"]);
        assert!(a.contains(&"page-ranges=1-2,5".to_string()));
        assert!(a.contains(&"print-scaling=fit".to_string()));
        assert!(a.contains(&"Report".to_string()));
    }

    #[test]
    fn nothing_to_print_is_refused_before_any_printer_is_touched() {
        let mut req = PrintRequest {
            printer: "x".into(),
            title: "t".into(),
            copies: 1,
            pages: vec![],
            scale: Scale::default(),
            output_file: None,
        };
        assert!(matches!(
            print(Arc::new(Vec::new()), &req, &mut |_, _| {}),
            Err(PrintError::Invalid(_))
        ));
        req.pages = vec![0];
        req.copies = 0;
        assert!(matches!(
            print(Arc::new(Vec::new()), &req, &mut |_, _| {}),
            Err(PrintError::Invalid(_))
        ));
    }
}
