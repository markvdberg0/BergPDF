//! Text boxes, callouts and stamps on rotated pages: upright when asked, rotatable, and the box
//! of a callout survives a save/re-open (it used to grow to include the leader line).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::annot::{self, Align, AnnotationKind, AnnotationSpec, Rgb};
use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::fontembed::FontStyle;
use pdf_engine::geom::{Point, Rect};
use pdf_engine::pageops;
use test_support::*;

fn open(bytes: Vec<u8>) -> PdfDocument {
    PdfDocument::open(bytes, &OpenOptions::default()).unwrap()
}

/// A blank A4 page shown rotated by `page_rotate` degrees clockwise, with one text box whose text
/// is rotated `rotation` degrees counter-clockwise. Returns the saved bytes.
fn page_with_box(page_rotate: i64, rotation: i32, callout: bool, stamp: bool) -> Vec<u8> {
    let mut doc = PdfDocument::new_empty().unwrap();
    let page = doc
        .transact(|tx| pageops::insert_blank_page(tx, 0, 595.0, 842.0))
        .unwrap()
        .0;
    doc.transact(|tx| pageops::rotate_pages(tx, &[page], page_rotate / 90))
        .unwrap();
    // Roomy box, long axis along the rotated text direction.
    let rect = if rotation % 180 == 0 {
        Rect::new(100.0, 400.0, 400.0, 460.0)
    } else {
        Rect::new(100.0, 300.0, 160.0, 600.0)
    };
    let kind = if stamp {
        AnnotationKind::StampText {
            rect,
            label: "APPROVED".into(),
        }
    } else {
        AnnotationKind::FreeText {
            rect,
            font_size: 14.0,
            text_color: Rgb::BLACK,
            align: Align::Left,
            font: FontStyle::default(),
            callout: callout.then(|| vec![Point::new(40.0, 380.0), Point::new(100.0, 430.0)]),
        }
    };
    let mut spec = AnnotationSpec::new(kind);
    spec.contents = "Hello world".into();
    spec.rotation = rotation;
    spec.border_width = 1.0;
    doc.transact(|tx| annot::add_annotation(tx, page, &spec))
        .unwrap();
    doc.snapshot_bytes().unwrap()
}

fn word<'a>(words: &'a [BBoxWord], text: &str) -> &'a BBoxWord {
    words
        .iter()
        .find(|w| w.text == text)
        .unwrap_or_else(|| panic!("{text:?} not found in {words:?}"))
}

#[test]
fn text_reads_upright_on_a_page_shown_rotated() {
    if !have_tool("pdftotext") {
        assert!(!oracles_required(), "poppler missing");
        return;
    }
    for shown in [0, 90, 180, 270] {
        let bytes = page_with_box(shown, annot::upright_for(shown), false, false);
        assert_eq!(
            poppler_diagnostics(&bytes).unwrap(),
            "",
            "poppler complains at {shown}"
        );
        let words = poppler_bbox(&bytes, 1).unwrap();
        let w = word(&words, "Hello");
        assert!(
            w.x1 - w.x0 > (w.y1 - w.y0) * 1.5,
            "page shown {shown}° clockwise: 'Hello' is not horizontal: {w:?}"
        );
        // Reading order: Hello then world, left to right (not mirrored or reversed).
        let w2 = word(&words, "world");
        assert!(w2.x0 > w.x1 - 0.5, "{w:?} {w2:?}");
    }
}

#[test]
fn without_a_rotation_text_follows_the_page_and_a_quarter_turn_makes_it_vertical() {
    if !have_tool("pdftotext") {
        return;
    }
    // Page shown 90° clockwise, text not compensated: it is vertical on screen.
    let bytes = page_with_box(90, 0, false, false);
    let words = poppler_bbox(&bytes, 1).unwrap();
    let w = word(&words, "Hello");
    assert!(w.y1 - w.y0 > w.x1 - w.x0, "{w:?}");
    // Asking for a further quarter turn on an upright page makes it vertical too.
    let bytes = page_with_box(0, 90, false, false);
    let words = poppler_bbox(&bytes, 1).unwrap();
    let w = word(&words, "Hello");
    assert!(w.y1 - w.y0 > w.x1 - w.x0, "{w:?}");
}

#[test]
fn stamps_are_upright_on_rotated_pages_too() {
    if !have_tool("pdftotext") {
        return;
    }
    for shown in [90, 270] {
        let bytes = page_with_box(shown, annot::upright_for(shown), false, true);
        assert_eq!(poppler_diagnostics(&bytes).unwrap(), "");
        let words = poppler_bbox(&bytes, 1).unwrap();
        let w = word(&words, "APPROVED");
        assert!(w.x1 - w.x0 > (w.y1 - w.y0) * 2.0, "{shown}: {w:?}");
    }
}

#[test]
fn rotation_and_the_callout_box_survive_saving() {
    let bytes = page_with_box(90, 90, true, false);
    let doc = open(bytes);
    let page = doc.pages().unwrap()[0].id;
    let infos = annot::read_annotations(&doc, page);
    assert_eq!(infos.len(), 1);
    let spec = infos[0].spec.clone().unwrap();
    assert_eq!(spec.rotation, 90);
    match spec.kind {
        AnnotationKind::FreeText { rect, callout, .. } => {
            assert_eq!(
                rect,
                Rect::new(100.0, 300.0, 160.0, 600.0),
                "box, not box+leader"
            );
            assert_eq!(callout.unwrap().len(), 2);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn growing_a_box_keeps_the_top_of_the_text_in_place() {
    let r = Rect::new(100.0, 300.0, 160.0, 600.0);
    // Text rotated 90° runs bottom-to-top; its top edge is the left edge (x0).
    let (w, h) = annot::local_size(r, 90);
    assert_eq!((w, h), (300.0, 60.0));
    let g = annot::grow_box(r, 90, 100.0);
    assert!(
        (g.x0 - 100.0).abs() < 1e-9 && (g.x1 - 200.0).abs() < 1e-9,
        "{g:?}"
    );
    let g0 = annot::grow_box(Rect::new(0.0, 100.0, 100.0, 120.0), 0, 50.0);
    assert!(
        (g0.y1 - 120.0).abs() < 1e-9 && (g0.y0 - 70.0).abs() < 1e-9,
        "{g0:?}"
    );
}

#[test]
fn added_page_text_can_be_upright_on_a_rotated_page() {
    if !have_tool("pdftotext") {
        return;
    }
    for shown in [90, 180, 270] {
        let mut doc = PdfDocument::new_empty().unwrap();
        let page = doc
            .transact(|tx| pageops::insert_blank_page(tx, 0, 595.0, 842.0))
            .unwrap()
            .0;
        doc.transact(|tx| pageops::rotate_pages(tx, &[page], shown / 90))
            .unwrap();
        doc.transact(|tx| {
            pdf_engine::pagecontent::add_text_rotated(
                tx,
                page,
                Point::new(300.0, 400.0),
                "Upright line\nSecond line",
                14.0,
                (0.0, 0.0, 0.0),
                FontStyle::default(),
                annot::upright_for(shown),
            )
        })
        .unwrap();
        let bytes = doc.snapshot_bytes().unwrap();
        assert_eq!(poppler_diagnostics(&bytes).unwrap(), "");
        let words = poppler_bbox(&bytes, 1).unwrap();
        let a = word(&words, "Upright");
        assert!(a.x1 - a.x0 > (a.y1 - a.y0) * 1.5, "{shown}: {a:?}");
        let b = word(&words, "Second");
        assert!(
            b.y0 > a.y0,
            "second line is below the first on screen: {a:?} {b:?}"
        );
    }
}

/// Test-support: with `BERG_DUMP=<dir>` writes a landscape (page `/Rotate 90`) copy of a real
/// fixture, for trying the application by hand.
#[test]
fn dump_a_rotated_fixture_for_manual_checks() {
    let Ok(dir) = std::env::var("BERG_DUMP") else {
        return;
    };
    let mut doc = open(fixture_bytes("report-chromium.pdf"));
    let pages: Vec<_> = doc.pages().unwrap().iter().map(|p| p.id).collect();
    doc.transact(|tx| pageops::rotate_pages(tx, &pages, 1))
        .unwrap();
    std::fs::write(
        format!("{dir}/report-landscape.pdf"),
        doc.snapshot_bytes().unwrap(),
    )
    .unwrap();
}
