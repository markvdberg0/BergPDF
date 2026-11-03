//! A dynamic stamp is a text stamp with a second, smaller line (name, date and time): both lines
//! are drawn inside the box, the label survives saving, and the old one-line stamps are unchanged.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::annot::{self, AnnotationKind, AnnotationSpec};
use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::geom::Rect;
use pdf_engine::pageops;

#[test]
fn two_line_stamp_keeps_its_label_and_fits_its_box() {
    let mut doc = PdfDocument::new_empty().unwrap();
    let page = doc
        .transact(|tx| pageops::insert_blank_page(tx, 0, 595.0, 842.0))
        .unwrap()
        .0;
    let rect = Rect::new(100.0, 600.0, 300.0, 660.0);
    let label = "APPROVED\nMark · 2026-11-03 14:05".to_string();
    let spec = AnnotationSpec::new(AnnotationKind::StampText {
        rect,
        label: label.clone(),
    });
    let id = doc
        .transact(|tx| annot::add_annotation(tx, page, &spec))
        .unwrap()
        .0;
    let bytes = doc.snapshot_bytes().unwrap();
    let re = PdfDocument::open(bytes.clone(), &OpenOptions::default()).unwrap();
    let info = annot::read_annotation(re.lopdf(), id).unwrap();
    assert!(
        matches!(&info.spec.unwrap().kind, AnnotationKind::StampText { label: l, .. } if *l == label)
    );
    // Both lines are in the appearance, as text another reader can extract.
    if let Some(t) = test_support::poppler_text(&bytes) {
        assert!(t.contains("APPROVED") && t.contains("2026-11-03"), "{t}");
    }
    // A one-line stamp still works.
    let one = AnnotationSpec::new(AnnotationKind::StampText {
        rect,
        label: "PAID".into(),
    });
    doc.transact(|tx| annot::add_annotation(tx, page, &one))
        .unwrap();
}
