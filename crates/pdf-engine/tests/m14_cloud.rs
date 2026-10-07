//! A cloud annotation: a `Square` with a cloudy border effect that reads back as a cloud, moves as one,
//! and is drawn with scallops (checked by our renderer and by poppler, an independent one).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::annot::{self, AnnotationKind, AnnotationSpec};
use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::geom::{Rect, Rotation};
use pdf_engine::render::with_session;
use std::sync::Arc;
use test_support::*;

fn cloud_doc() -> (PdfDocument, pdf_engine::doc::PageId, annot::AnnotId) {
    let mut doc = PdfDocument::open(
        fixture_bytes("report-chromium.pdf"),
        &OpenOptions::default(),
    )
    .unwrap();
    let page = doc.pages().unwrap()[0].id;
    let (id, _) = doc
        .transact(|tx| {
            let mut spec = AnnotationSpec::new(AnnotationKind::Cloud {
                rect: Rect::new(60.0, 100.0, 260.0, 220.0),
            });
            spec.border_width = 2.0;
            annot::add_annotation(tx, page, &spec)
        })
        .unwrap();
    (doc, page, id)
}

#[test]
fn a_cloud_reads_back_as_a_cloud_and_moves_as_one() {
    let (mut doc, page, id) = cloud_doc();
    let info = annot::read_annotation(doc.lopdf(), id).unwrap();
    assert_eq!(info.subtype, "Square");
    let spec = info.spec.unwrap();
    assert!(
        matches!(spec.kind, AnnotationKind::Cloud { .. }),
        "{spec:?}"
    );

    // The border effect is what other programs use to draw their own cloud.
    let d = doc.lopdf().get_dictionary(id).unwrap();
    let be = d.get(b"BE").unwrap().as_dict().unwrap();
    assert_eq!(be.get(b"S").unwrap().as_name().unwrap(), b"C");

    doc.transact(|tx| annot::move_annotation(tx, id, 10.0, 5.0))
        .unwrap();
    let moved = annot::read_annotations(&doc, page);
    let AnnotationKind::Cloud { rect } = moved[0].spec.as_ref().unwrap().kind else {
        panic!("not a cloud after the move");
    };
    assert!((rect.x0 - 70.0).abs() < 1e-6 && (rect.y0 - 105.0).abs() < 1e-6);
}

#[test]
fn the_cloud_border_is_scalloped_not_straight() {
    let (mut doc, _, id) = cloud_doc();
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
    let curves = text
        .lines()
        .filter(|l| l.trim_end().ends_with(" c"))
        .count();
    assert!(curves >= 12, "scallops are curves, found {curves}: {text}");
    assert!(!text.contains(" re"), "not a plain rectangle: {text}");

    // And it is drawn: ink along the top edge of the cloud, where the page is white.
    let bytes = doc.snapshot_bytes().unwrap();
    let pages = doc.pages().unwrap();
    let img = with_session(Arc::new(bytes), |s| {
        s.render_page(0, pages[0].geometry, Rotation::R0, 1.0)
    })
    .unwrap();
    let rgb = rgba_to_rgb(&img.rgba);
    let h = f64::from(img.height);
    let (y0, y1) = ((h - 224.0) as u32, (h - 214.0) as u32);
    let ink = non_white_fraction(&rgb, img.width, 62, y0, 258, y1, 40);
    assert!(ink > 0.02, "the cloud must be drawn (ink {ink})");
}
