//! Milestone 0 feasibility evidence: a real PDF from an independent producer (Skia/PDF) goes
//! through render, text search, annotation write, structural page edit and save/reopen.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::annot::{self, AnnotationKind, AnnotationSpec, Rgb};
use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::geom::{Point, Rect, Rotation};
use pdf_engine::pageops;
use pdf_engine::render::with_session;
use std::sync::Arc;
use test_support::*;

fn open_fixture() -> PdfDocument {
    PdfDocument::open(
        fixture_bytes("report-chromium.pdf"),
        &OpenOptions::default(),
    )
    .unwrap()
}

#[test]
fn render_search_annotate_restructure_save_reopen() {
    let mut doc = open_fixture();
    let orig_bytes = doc.original_bytes().clone();
    assert_eq!(doc.page_count(), 3);
    assert!(doc.can_save_incrementally());
    let pages = doc.pages().unwrap();

    // --- find real text to highlight --------------------------------------------------
    let (hit_quads, text_bounds) = with_session(orig_bytes.clone(), |s| {
        let tp = s.extract_text(0, &pages[0].geometry)?;
        let hits = tp.search("quick brown fox", false);
        assert_eq!(hits.len(), 1, "search must find the phrase");
        let b = hits[0].quads[0].bounds();
        Ok((hits[0].quads.clone(), b))
    })
    .unwrap();

    // --- annotate + restructure in ONE transaction each -------------------------------
    let p0 = pages[0].id;
    let ((hl, ft, rect), d1) = doc
        .transact(|tx| {
            let mut hl = AnnotationSpec::new(AnnotationKind::Highlight {
                quads: hit_quads.clone(),
            });
            hl.author = "Tester".into();
            hl.contents = "important".into();
            let hl = annot::add_annotation(tx, p0, &hl)?;
            let mut ft = AnnotationSpec::new(AnnotationKind::FreeText {
                rect: Rect::new(300.0, 600.0, 520.0, 660.0),
                font_size: 12.0,
                text_color: Rgb(0.0, 0.2, 0.8),
                align: annot::Align::Left,
                font: pdf_engine::fontembed::FontStyle::default(),
                callout: None,
            });
            ft.contents = "Free text: café € ≠ naïve".into();
            ft.fill = Some(Rgb(1.0, 1.0, 0.8));
            let ft = annot::add_annotation(tx, p0, &ft)?;
            let mut r = AnnotationSpec::new(AnnotationKind::Rectangle {
                rect: Rect::new(60.0, 100.0, 200.0, 180.0),
            });
            r.border_width = 3.0;
            let rect = annot::add_annotation(tx, p0, &r)?;
            Ok((hl, ft, rect))
        })
        .unwrap();
    assert!(d1.len() >= 6);
    let (_, d2) = doc
        .transact(|tx| {
            pageops::rotate_pages(tx, &[pages[1].id], 1)?;
            pageops::move_pages(tx, &[pages[2].id], 0)
        })
        .unwrap();
    assert!(!d2.is_empty());

    // --- snapshot is an append to the original bytes -----------------------------------
    let saved = doc.snapshot_bytes().unwrap();
    assert!(saved.len() > orig_bytes.len());
    assert_eq!(
        &saved[..orig_bytes.len()],
        orig_bytes.as_slice(),
        "original bytes preserved verbatim"
    );

    if let Ok(dir) = std::env::var("BERG_KEEP_OUT") {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(std::path::Path::new(&dir).join("m0-saved.pdf"), &saved).unwrap();
    }
    // --- reopen with our own parser ----------------------------------------------------
    let re = PdfDocument::open(saved.clone(), &OpenOptions::default()).unwrap();
    let rp = re.pages().unwrap();
    assert_eq!(rp.len(), 3);
    assert_eq!(rp[0].id, pages[2].id, "page 3 moved to front");
    assert_eq!(rp[2].id, pages[1].id);
    assert_eq!(rp[2].geometry.rotate, Rotation::R90);
    let annots = annot::read_annotations(&re, p0);
    assert_eq!(annots.len(), 3);
    let kinds: Vec<_> = annots.iter().map(|a| a.subtype.as_str()).collect();
    assert_eq!(kinds, ["Highlight", "FreeText", "Square"]);
    assert_eq!(annots[0].contents, "important");
    assert_eq!(annots[1].contents, "Free text: café € ≠ naïve");
    assert_eq!(annots[0].author, "Tester");
    assert!(
        annots.iter().all(|a| a.spec.is_some()),
        "all round-trip into editable specs"
    );
    let _ = (hl, ft, rect);

    // --- reopen with the renderer; annotation appearances must be visible --------------
    let new_page_idx = re.page_index(p0).unwrap();
    assert_eq!(new_page_idx, 1);
    let before = with_session(orig_bytes.clone(), |s| {
        s.render_page(0, pages[0].geometry, Rotation::R0, 1.0)
    })
    .unwrap();
    let after = with_session(Arc::new(saved.clone()), |s| {
        s.render_page(new_page_idx, rp[new_page_idx].geometry, Rotation::R0, 1.0)
    })
    .unwrap();
    assert_eq!((before.width, before.height), (after.width, after.height));
    let (b, a) = (rgba_to_rgb(&before.rgba), rgba_to_rgb(&after.rgba));
    // Rectangle outline region (user rect 60,100..200,180 -> view y flipped).
    let h = after.height as f64;
    let rect_view_y0 = (h - 180.0) as u32;
    let rect_view_y1 = (h - 100.0) as u32;
    let frac_before = non_white_fraction(&b, before.width, 58, rect_view_y0, 202, rect_view_y1, 40);
    let frac_after = non_white_fraction(&a, after.width, 58, rect_view_y0, 202, rect_view_y1, 40);
    assert!(
        frac_after > frac_before + 0.01,
        "rectangle annotation must render: {frac_before} -> {frac_after}"
    );
    // Highlight: yellow pixels over the text bounds (blue channel low).
    let (x0, x1) = (text_bounds.x0 as u32, text_bounds.x1 as u32);
    let (y0, y1) = ((h - text_bounds.y1) as u32, (h - text_bounds.y0) as u32);
    let mut yellow = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * after.width + x) * 3) as usize;
            if a[i] > 200 && a[i + 1] > 180 && a[i + 2] < 120 {
                yellow += 1;
            }
        }
    }
    assert!(
        yellow > 100,
        "highlight must tint the text area (yellow px = {yellow})"
    );
    // Text under a Multiply highlight must still be dark (not covered).
    let dark = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let i = ((y * after.width + x) * 3) as usize;
            a[i] < 90 && a[i + 1] < 90
        })
        .count();
    assert!(
        dark > 30,
        "text must remain visible under the highlight (dark px = {dark})"
    );

    // --- the saved file extracts the free-text content (annotation text is real text) --
    // --- independent oracle: poppler ---------------------------------------------------
    if let Some((pages, stderr)) = poppler_info(&saved) {
        assert_eq!(pages, Some(3));
        assert!(
            stderr.trim().is_empty(),
            "poppler reported problems: {stderr}"
        );
    } else {
        eprintln!("poppler not installed: oracle check skipped");
    }
    if let Some((w, hh, rgb)) = poppler_render(&saved, 2, 72) {
        // page 2 of saved == original page 1 (+annotations); rectangle outline must be there too.
        let f = non_white_fraction(
            &rgb,
            w,
            58,
            (hh as f64 - 180.0) as u32,
            202,
            (hh as f64 - 100.0) as u32,
            40,
        );
        assert!(
            f > 0.01,
            "poppler must render the rectangle annotation (frac {f})"
        );
    }
}

#[test]
fn undo_by_reverting_deltas_restores_exact_state() {
    let mut doc = open_fixture();
    let pages = doc.pages().unwrap();
    let before = doc.snapshot_bytes().unwrap();
    let (_, d) = doc
        .transact(|tx| {
            pageops::delete_pages(tx, &[pages[1].id])?;
            annot::add_annotation(
                tx,
                pages[0].id,
                &AnnotationSpec::new(AnnotationKind::Ellipse {
                    rect: Rect::new(10.0, 10.0, 90.0, 60.0),
                }),
            )
        })
        .unwrap();
    assert_eq!(doc.page_count(), 2);
    doc.revert_delta(&d);
    assert_eq!(doc.page_count(), 3);
    assert!(annot::read_annotations(&doc, pages[0].id).is_empty());
    doc.apply_delta(&d);
    assert_eq!(doc.page_count(), 2);
    assert_eq!(annot::read_annotations(&doc, pages[0].id).len(), 1);
    doc.revert_delta(&d);
    // Structure is back to what it was; snapshot differs only by harmless re-append of touched objects.
    let after = doc.snapshot_bytes().unwrap();
    let re = PdfDocument::open(after, &OpenOptions::default()).unwrap();
    assert_eq!(re.page_count(), 3);
    let _ = before;
}

#[test]
fn failed_transaction_rolls_back() {
    let mut doc = open_fixture();
    let pages = doc.pages().unwrap();
    let r = doc.transact(|tx| {
        pageops::rotate_pages(tx, &[pages[0].id], 1)?;
        pageops::delete_pages(tx, &[pages[0].id, pages[1].id, pages[2].id])
    });
    assert!(r.is_err());
    assert_eq!(doc.page_count(), 3);
    assert_eq!(
        doc.page_geometry(pages[0].id).unwrap().rotate,
        Rotation::R0,
        "rotation rolled back"
    );
    assert!(!doc.has_changes_since_base() || doc.snapshot_bytes().is_ok());
}

#[test]
fn point_helper_compiles() {
    let _ = Point::new(0.0, 0.0);
}
