//! Prints through the real Windows GDI path into *Microsoft Print to PDF* (which writes a file instead of using
//! paper) and looks at what came out. Skipped where that printer does not exist unless `BERG_REQUIRE_PRINT=1`.
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::geom::Rotation;
use pdf_engine::render::{Bitmap, with_session};
use printing::{PrintRequest, Scale, print, printers};
use std::sync::Arc;
use test_support::fixtures::{helvetica_lines, many_pages, mixed_sizes};

const PRINTER: &str = "Microsoft Print to PDF";

fn have_printer() -> bool {
    let have = printers().iter().any(|p| p.name == PRINTER);
    if !have {
        assert!(
            !std::env::var("BERG_REQUIRE_PRINT").is_ok_and(|v| v == "1"),
            "{PRINTER} is required (BERG_REQUIRE_PRINT=1)"
        );
        eprintln!("skipped: no {PRINTER} printer");
    }
    have
}

fn render(bytes: &[u8], page: usize, scale: f64) -> Bitmap {
    let doc = PdfDocument::open(bytes.to_vec(), &OpenOptions::default()).unwrap();
    let id = doc.page_ids().unwrap()[page];
    let geom = doc.page_geometry(id).unwrap();
    with_session(Arc::new(bytes.to_vec()), |s| {
        s.render_page(page, geom, Rotation::R0, scale)
    })
    .unwrap()
}

/// Bounding box of the dark pixels: (x0, y0, x1, y1), and the fraction of dark pixels.
fn dark_box(b: &Bitmap) -> Option<(u32, u32, u32, u32, f64)> {
    let (mut x0, mut y0, mut x1, mut y1, mut n) = (u32::MAX, u32::MAX, 0, 0, 0u64);
    for y in 0..b.height {
        for x in 0..b.width {
            let i = ((y * b.width + x) * 4) as usize;
            if u32::from(b.rgba[i]) + u32::from(b.rgba[i + 1]) + u32::from(b.rgba[i + 2]) < 300 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
                n += 1;
            }
        }
    }
    (n > 0).then(|| {
        (
            x0,
            y0,
            x1,
            y1,
            n as f64 / (u64::from(b.width) * u64::from(b.height)) as f64,
        )
    })
}

fn job(pages: Vec<usize>, copies: u32, out: &std::path::Path) -> PrintRequest {
    PrintRequest {
        printer: PRINTER.into(),
        title: "BergPDF test".into(),
        copies,
        pages,
        scale: Scale::ShrinkToFit,
        output_file: Some(out.to_path_buf()),
    }
}

#[test]
fn a_printed_page_has_the_same_text_in_the_same_proportions() {
    if !have_printer() {
        return;
    }
    let original = helvetica_lines();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("printed.pdf");
    let mut reports = Vec::new();
    print(
        Arc::new(original.clone()),
        &job(vec![0], 1, &out),
        &mut |d, t| reports.push((d, t)),
    )
    .unwrap();
    assert_eq!(reports, vec![(1, 1)]);
    let printed = std::fs::read(&out).unwrap();
    assert!(printed.starts_with(b"%PDF"), "not a PDF file");
    let doc = PdfDocument::open(printed.clone(), &OpenOptions::default()).unwrap();
    assert_eq!(doc.page_count(), 1);

    // The text of the original, as a dark box; the printed page shows the same box, shrunk uniformly.
    let (ox0, oy0, ox1, oy1, _) = dark_box(&render(&original, 0, 1.0)).unwrap();
    let (px0, py0, px1, py1, frac) = dark_box(&render(&printed, 0, 1.0)).unwrap();
    assert!(frac > 0.001, "the printed page is nearly blank ({frac})");
    let o_ratio = f64::from(ox1 - ox0) / f64::from(oy1 - oy0);
    let p_ratio = f64::from(px1 - px0) / f64::from(py1 - py0);
    assert!(
        (o_ratio / p_ratio - 1.0).abs() < 0.08,
        "text block proportions changed: {o_ratio} vs {p_ratio}"
    );
    // Upright: the text block is in the upper half, as in the original.
    assert!(
        py1 < render(&printed, 0, 1.0).height * 2 / 3,
        "text block is not in the upper part of the page"
    );
    let _ = (oy0, py0);
}

#[test]
fn page_ranges_copies_and_mixed_page_sizes() {
    if !have_printer() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // Three pages: A4 portrait, A3 landscape (turned and shrunk), a tiny label.
    let original = mixed_sizes();
    let out = dir.path().join("mixed.pdf");
    let mut seen = 0;
    print(
        Arc::new(original.clone()),
        &job(vec![0, 1, 2], 2, &out),
        &mut |d, t| {
            seen = d;
            assert_eq!(t, 6);
        },
    )
    .unwrap();
    assert_eq!(seen, 6);
    let printed = PdfDocument::open(std::fs::read(&out).unwrap(), &OpenOptions::default()).unwrap();
    assert_eq!(printed.page_count(), 6, "3 pages × 2 copies");

    // A range of a longer document.
    let many = many_pages(5);
    let out2 = dir.path().join("many.pdf");
    print(Arc::new(many), &job(vec![1, 3], 1, &out2), &mut |_, _| {}).unwrap();
    let p2 = PdfDocument::open(std::fs::read(&out2).unwrap(), &OpenOptions::default()).unwrap();
    assert_eq!(p2.page_count(), 2);
    for i in 0..2 {
        let (_, _, _, _, frac) = dark_box(&render(&std::fs::read(&out2).unwrap(), i, 0.5)).unwrap();
        assert!(
            frac > 0.002,
            "page {i} of the printout is nearly blank ({frac})"
        );
    }
}

#[test]
fn an_unknown_printer_or_page_is_a_clear_error() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("x.pdf");
    let mut req = job(vec![0], 1, &out);
    req.printer = "No such printer".into();
    assert!(print(Arc::new(helvetica_lines()), &req, &mut |_, _| {}).is_err());
    if have_printer() {
        let req = job(vec![5], 1, &out);
        let e = print(Arc::new(helvetica_lines()), &req, &mut |_, _| {}).unwrap_err();
        assert!(e.to_string().contains("page 6"), "{e}");
        assert!(!out.exists(), "a refused job left a file behind");
    }
}

#[test]
fn the_printer_list_has_a_default_first() {
    let list = printers();
    if let Some(first) = list.first() {
        assert!(list.iter().filter(|p| p.is_default).count() <= 1);
        if list.iter().any(|p| p.is_default) {
            assert!(first.is_default);
        }
    }
}
