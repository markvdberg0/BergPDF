//! Document properties and PNG export.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::geom::Rotation;
use pdf_engine::meta::{self, InfoEdit};
use pdf_engine::render::{export_scale, with_session};
use std::sync::Arc;
use test_support::fixtures::*;
use test_support::*;

fn open(bytes: Vec<u8>) -> PdfDocument {
    PdfDocument::open(bytes, &OpenOptions::default()).unwrap()
}

#[test]
fn info_roundtrip_with_unicode_and_removal() {
    let mut doc = open(helvetica_lines());
    assert_eq!(meta::read_info(&doc).title, "");
    let edit = InfoEdit {
        title: "Jaarverslag — Zoë".into(),
        author: "M. van den Berg".into(),
        subject: "Q3".into(),
        keywords: "pdf, test".into(),
    };
    doc.transact(|tx| meta::set_info(tx, &edit)).unwrap();
    let mut doc = open(doc.snapshot_bytes().unwrap());
    let i = meta::read_info(&doc);
    assert_eq!(i.title, "Jaarverslag — Zoë");
    assert_eq!(i.author, "M. van den Berg");
    assert!(i.modified.starts_with("D:"), "{i:?}");
    // poppler (independent) reads the same title.
    let bytes = doc.snapshot_bytes().unwrap();
    if let Some(info) = poppler_info_text(&bytes) {
        assert!(info.contains("Jaarverslag"), "poppler: {info}");
        assert!(info.contains("van den Berg"), "poppler: {info}");
    } else {
        assert!(!oracles_required(), "poppler required");
    }
    // Clearing a field removes it.
    let cleared = InfoEdit {
        subject: String::new(),
        ..edit
    };
    doc.transact(|tx| meta::set_info(tx, &cleared)).unwrap();
    assert_eq!(meta::read_info(&doc).subject, "");
    assert_eq!(meta::read_info(&doc).keywords, "pdf, test");
    // Overlong values are rejected and roll back.
    let long = InfoEdit {
        title: "x".repeat(meta::MAX_LEN + 1),
        ..Default::default()
    };
    assert!(doc.transact(|tx| meta::set_info(tx, &long)).is_err());
    assert_eq!(meta::read_info(&doc).keywords, "pdf, test");
    assert!(!meta::has_xmp(&doc));
}

#[test]
fn png_export_is_a_valid_png_with_the_right_size() {
    let doc = open(helvetica_lines());
    let pages = doc.pages().unwrap();
    let g = pages[0].geometry;
    let size = g.view_size(Rotation::R0);
    let scale = export_scale(size.width, size.height, 150.0);
    let png = with_session(Arc::new(doc.original_bytes().as_ref().clone()), |s| {
        s.render_page(0, g, Rotation::R0, scale)?.to_png()
    })
    .unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let dec = png::Decoder::new(std::io::Cursor::new(&png));
    let r = dec.read_info().unwrap();
    let info = r.info();
    assert_eq!(info.width, (size.width * 150.0 / 72.0).ceil() as u32);
    // An A0 page at 300 dpi would exceed the budget: the scale is reduced, never refused.
    let s = export_scale(2384.0, 3370.0, 300.0);
    assert!(s < 300.0 / 72.0);
    assert!((2384.0 * s).ceil() * (3370.0 * s).ceil() <= 4096.0 * 4096.0);
}
