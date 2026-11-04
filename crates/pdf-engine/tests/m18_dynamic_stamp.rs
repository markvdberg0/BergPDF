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

/// The `Tf` size and the `Td` baseline of the text drawn with `font` in an appearance stream.
fn size_and_baseline(content: &str, font: &str) -> (f64, f64) {
    let line = content
        .lines()
        .find(|l| l.starts_with(&format!("BT {font} ")))
        .unwrap_or_else(|| panic!("no {font} text in {content}"));
    let t: Vec<&str> = line.split_whitespace().collect();
    (t[2].parse().unwrap(), t[5].parse().unwrap())
}

#[test]
fn the_name_and_date_line_sits_below_the_stamp_word() {
    for height in [40.0, 60.0, 90.0] {
        let mut doc = PdfDocument::new_empty().unwrap();
        let page = doc
            .transact(|tx| pageops::insert_blank_page(tx, 0, 595.0, 842.0))
            .unwrap()
            .0;
        let spec = AnnotationSpec::new(AnnotationKind::StampText {
            rect: Rect::new(100.0, 600.0, 300.0, 600.0 + height),
            label: "APPROVED\nAnna de Vries · 2026-11-04 09:07".into(),
        });
        let id = doc
            .transact(|tx| annot::add_annotation(tx, page, &spec))
            .unwrap()
            .0;
        let d = doc.lopdf().get_dictionary(id).unwrap();
        let ap = d.get(b"AP").unwrap().as_dict().unwrap();
        let n = ap.get(b"N").unwrap().as_reference().unwrap();
        let content = doc
            .lopdf()
            .get_object(n)
            .unwrap()
            .as_stream()
            .unwrap()
            .decompressed_content()
            .unwrap();
        let text = String::from_utf8_lossy(&content);
        let (fs, y) = size_and_baseline(&text, "/F1");
        let (sfs, dy) = size_and_baseline(&text, "/F2");
        // The capitals of the second line end clearly below the bottom of the stamp word…
        assert!(
            dy + 0.75 * sfs < y - 0.15 * fs,
            "height {height}: word at {y} ({fs} pt), details at {dy} ({sfs} pt)"
        );
        // …and both lines stay inside the box.
        assert!(
            dy - 0.25 * sfs >= 0.0 && y + 0.95 * fs <= height,
            "height {height}: {text}"
        );
    }
}
