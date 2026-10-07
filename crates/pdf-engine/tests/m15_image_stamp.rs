//! A picture as a stamp or signature: it is a `Stamp` annotation whose appearance is the picture, reads
//! back with the picture intact (so it can be moved, resized and saved), keeps its transparency, and
//! shows in our renderer and in poppler (an independent one).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::annot::{self, AnnotationKind, AnnotationSpec};
use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::geom::{Rect, Rotation};
use pdf_engine::render::with_session;
use pdf_engine::stampimage::StampImage;
use std::sync::Arc;
use test_support::*;

/// 40×20 picture: the left half solid red, the right half fully transparent.
fn red_half() -> Arc<StampImage> {
    let mut rgba = Vec::new();
    for _y in 0..20 {
        for x in 0..40 {
            rgba.extend_from_slice(&if x < 20 {
                [220, 0, 0, 255]
            } else {
                [0, 0, 0, 0]
            });
        }
    }
    Arc::new(StampImage::from_rgba(40, 20, &rgba).unwrap())
}

fn doc_with_stamp() -> (PdfDocument, pdf_engine::doc::PageId, annot::AnnotId) {
    let mut doc = PdfDocument::open(
        fixture_bytes("report-chromium.pdf"),
        &OpenOptions::default(),
    )
    .unwrap();
    let page = doc.pages().unwrap()[0].id;
    let (id, _) = doc
        .transact(|tx| {
            let spec = AnnotationSpec::new(AnnotationKind::ImageStamp {
                rect: Rect::new(100.0, 300.0, 300.0, 400.0),
                image: red_half(),
            });
            annot::add_annotation(tx, page, &spec)
        })
        .unwrap();
    (doc, page, id)
}

#[test]
fn the_picture_survives_reading_saving_and_moving() {
    let (mut doc, page, id) = doc_with_stamp();
    let read = |doc: &PdfDocument| {
        let info = annot::read_annotation(doc.lopdf(), id).unwrap();
        assert_eq!(info.subtype, "Stamp");
        match info.spec.unwrap().kind {
            AnnotationKind::ImageStamp { rect, image } => (rect, image),
            other => panic!("not an image stamp: {other:?}"),
        }
    };
    let (rect, image) = read(&doc);
    assert_eq!(*image, *red_half(), "the picture reads back unchanged");
    assert_eq!((rect.x0, rect.y1), (100.0, 400.0));

    doc.transact(|tx| annot::move_annotation(tx, id, 10.0, -20.0))
        .unwrap();
    let (rect, _) = read(&doc);
    assert_eq!((rect.x0, rect.y0), (110.0, 280.0));

    // Saved and opened again it is still an image stamp with the same picture.
    let bytes = doc.snapshot_bytes().unwrap();
    let re = PdfDocument::open(bytes, &OpenOptions::default()).unwrap();
    let all = annot::read_annotations(&re, page);
    let AnnotationKind::ImageStamp { image, .. } = &all[0].spec.as_ref().unwrap().kind else {
        panic!("not an image stamp after saving");
    };
    assert_eq!(**image, *red_half());
}

#[test]
fn it_is_drawn_with_its_transparency() {
    let (mut doc, _, _) = doc_with_stamp();
    let bytes = doc.snapshot_bytes().unwrap();
    let pages = doc.pages().unwrap();
    let img = with_session(Arc::new(bytes.clone()), |s| {
        s.render_page(0, pages[0].geometry, Rotation::R0, 1.0)
    })
    .unwrap();
    let rgb = rgba_to_rgb(&img.rgba);
    let h = f64::from(img.height);
    // Stamp box x 100..300, y 300..400 (user space). Left half red, right half shows the page (white here).
    let band = |x0: f64, x1: f64| {
        non_white_fraction(
            &rgb,
            img.width,
            x0 as u32,
            (h - 390.0) as u32,
            x1 as u32,
            (h - 310.0) as u32,
            40,
        )
    };
    assert!(band(110.0, 190.0) > 0.9, "the opaque half is drawn");
    assert!(band(210.0, 290.0) < 0.05, "the transparent half is not");

    if let Some((w, hh, px)) = poppler_render(&bytes, 1, 72) {
        let f = |x0: f64, x1: f64| {
            non_white_fraction(
                &px,
                w,
                x0 as u32,
                (hh as f64 - 390.0) as u32,
                x1 as u32,
                (hh as f64 - 310.0) as u32,
                40,
            )
        };
        assert!(f(110.0, 190.0) > 0.9, "poppler draws the picture");
        assert!(f(210.0, 290.0) < 0.05, "poppler keeps the transparency");
    }
}
