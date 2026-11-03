//! OCR end to end: a scanned page (a picture of text) becomes searchable text at the right place.
//! Needs the OCR model files: set `BERG_OCR_MODELS` to their directory (see
//! `cargo xtask fetch-ocr-models`). Without them the test is skipped, unless `BERG_REQUIRE_OCR=1`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use editor_core::ocr::{OcrPageSpec, recognize_pages};
use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::geom::Rotation;
use pdf_engine::pagecontent::{add_ocr_text_layers, page_has_text};
use pdf_engine::render::with_session;
use std::collections::BTreeSet;
use std::sync::Arc;
use test_support::*;

fn engine() -> Option<pdf_ocr::Engine> {
    let dir = std::env::var_os("BERG_OCR_MODELS").map(std::path::PathBuf::from);
    match dir.filter(|d| pdf_ocr::models_present(d)) {
        Some(d) => Some(pdf_ocr::Engine::load(&d).unwrap()),
        None => {
            assert!(
                std::env::var("BERG_REQUIRE_OCR").map_or(true, |v| v != "1"),
                "BERG_REQUIRE_OCR=1 but no models in BERG_OCR_MODELS"
            );
            eprintln!("skipping: OCR models not available");
            None
        }
    }
}

fn words(text: &str) -> BTreeSet<String> {
    text.split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| w.len() >= 3)
        .collect()
}

/// A scan of page 1 of the Chromium fixture: render it, wrap the bitmap in an image-only PDF.
fn scanned_report_page() -> (PdfDocument, Vec<u8>) {
    let original = fixture_bytes("report-chromium.pdf");
    let doc = PdfDocument::open(original.clone(), &OpenOptions::default()).unwrap();
    let g = doc.pages().unwrap()[0].geometry;
    let png = with_session(Arc::new(original.clone()), |s| {
        s.render_page(0, g, Rotation::R0, 300.0 / 72.0)?.to_png()
    })
    .unwrap();
    (PdfDocument::from_image(&png).unwrap(), original)
}

#[test]
fn a_scanned_page_becomes_searchable_and_the_text_sits_where_it_was_printed() {
    let Some(engine) = engine() else { return };
    let (mut scan, original) = scanned_report_page();
    let page = scan.page_ids().unwrap()[0];
    assert!(
        !page_has_text(scan.lopdf(), page),
        "the scan starts with no text"
    );
    if let Some(t) = poppler_text(&scan.snapshot_bytes().unwrap()) {
        assert!(
            t.trim().is_empty(),
            "poppler sees no text in the scan: {t:?}"
        );
    }

    let spec = OcrPageSpec {
        page,
        index: 0,
        geometry: scan.pages().unwrap()[0].geometry,
    };
    let snap = Arc::new(scan.snapshot_bytes().unwrap());
    let mut ticks = Vec::new();
    let result = recognize_pages(&engine, snap, &[spec], &mut |d, t| {
        ticks.push((d, t));
        true
    })
    .unwrap();
    assert_eq!(ticks, vec![(0, 1), (1, 1)]);
    assert!(result[0].1.len() > 40, "found {} words", result[0].1.len());

    let (n, _) = scan
        .transact(|tx| add_ocr_text_layers(tx, &result))
        .unwrap();
    assert!(n > 40);
    let bytes = scan.snapshot_bytes().unwrap();
    assert!(
        page_has_text(scan.lopdf(), page),
        "OCR text counts as text, so it is not recognised twice"
    );

    // Independent check 1: poppler extracts the recognised words; most of the real text is there.
    let Some(got) = poppler_text(&bytes) else {
        assert!(!oracles_required(), "poppler required");
        return;
    };
    let truth_text = poppler_text_page(&original, 1).unwrap();
    let (truth, found) = (words(&truth_text), words(&got));
    let hit = truth.intersection(&found).count();
    let recall = hit as f64 / truth.len() as f64;
    eprintln!(
        "OCR: {} words recognised; recall of printed words {recall:.2} ({hit}/{})",
        result[0].1.len(),
        truth.len()
    );
    assert!(
        recall >= 0.8,
        "word recall {recall:.2} ({hit}/{}); got: {got}",
        truth.len()
    );

    // Independent check 2: the invisible words are where the printed words are (±8 pt), using
    // poppler's own word boxes for both documents.
    let (Some(tb), Some(gb)) = (poppler_bbox(&original, 1), poppler_bbox(&bytes, 1)) else {
        assert!(!oracles_required(), "poppler's pdftotext -bbox required");
        return;
    };
    let mut checked = 0;
    for key in ["quarterly", "report", "brown", "workflow"] {
        let (Some(a), Some(b)) = (
            tb.iter().find(|w| w.text.to_lowercase().starts_with(key)),
            gb.iter().find(|w| w.text.to_lowercase().starts_with(key)),
        ) else {
            continue;
        };
        let (ca, cb) = (
            ((a.x0 + a.x1) / 2.0, (a.y0 + a.y1) / 2.0),
            ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0),
        );
        assert!(
            (ca.0 - cb.0).abs() < 8.0 && (ca.1 - cb.1).abs() < 8.0,
            "{key}: printed {ca:?} vs recognised {cb:?}"
        );
        checked += 1;
    }
    assert!(checked >= 3, "only {checked} reference words matched");

    // The page still looks the same: the layer is invisible (render mode 3).
    let before = poppler_render(&scan_png_pdf_bytes(&original), 1, 50);
    let after = poppler_render(&bytes, 1, 50);
    if let (Some((_, _, a)), Some((_, _, b))) = (before, after) {
        assert!(
            mean_abs_diff(&a, &b) < 2.0,
            "invisible text must not change the picture"
        );
    }
}

/// Dutch, German and French text with the accents each language needs.
const ACCENTED: &str = "De ideeën van België over één coördinatie zijn geëvalueerd.\n\
Die Größe der Straße wurde für die Übersicht gemessen.\n\
Le café de l'élève est très français et un peu naïf.\n\
Skärgårdens søster läser på Dvořák och Łódź.";

#[test]
fn accented_letters_of_dutch_german_and_french_survive_ocr_at_the_right_place() {
    let Some(engine) = engine() else { return };
    let (original, _) = pdf_engine::textdoc::build_text_pdf(
        "Overzicht",
        "Übersicht",
        &[("Één".into(), ACCENTED.into())],
    )
    .unwrap();
    let mut scan = {
        let doc = PdfDocument::open(original.clone(), &OpenOptions::default()).unwrap();
        let g = doc.pages().unwrap()[0].geometry;
        let png = with_session(Arc::new(original.clone()), |s| {
            s.render_page(0, g, Rotation::R0, 300.0 / 72.0)?.to_png()
        })
        .unwrap();
        PdfDocument::from_image(&png).unwrap()
    };
    let page = scan.page_ids().unwrap()[0];
    let geometry = scan.pages().unwrap()[0].geometry;
    let spec = OcrPageSpec {
        page,
        index: 0,
        geometry,
    };
    let snap = Arc::new(scan.snapshot_bytes().unwrap());
    let result = recognize_pages(&engine, snap, &[spec], &mut |_, _| true).unwrap();
    scan.transact(|tx| add_ocr_text_layers(tx, &result))
        .unwrap();
    let bytes = scan.snapshot_bytes().unwrap();

    // Read back with the engine's own text extraction (no OCR involved), from both files.
    let text_of = |b: &[u8]| {
        let g = PdfDocument::open(b.to_vec(), &OpenOptions::default())
            .unwrap()
            .pages()
            .unwrap()[0]
            .geometry;
        with_session(Arc::new(b.to_vec()), |s| s.extract_text(0, &g)).unwrap()
    };
    let (truth, got) = (text_of(&original), text_of(&bytes));
    let plain = got.plain_text();
    eprintln!("OCR read:\n{plain}");
    let accented: Vec<&str> = ACCENTED
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| !w.is_ascii())
        .collect();
    let found: Vec<&&str> = accented
        .iter()
        .filter(|w| !got.search(w, true).is_empty())
        .collect();
    eprintln!("accented words found: {}/{}", found.len(), accented.len());
    // Every word of the Dutch and German lines; most of the rest (on 2026-11-02 the misses were
    // naïf → naîf, på → pà, Łódź → Lódž).
    for w in ACCENTED.lines().take(2).flat_map(str::split_whitespace) {
        let w = w.trim_matches(|c: char| !c.is_alphanumeric());
        assert!(
            !got.search(w, true).is_empty(),
            "{w} not read; got:\n{plain}"
        );
    }
    assert!(
        found.len() * 10 >= accented.len() * 8,
        "only {found:?} of {accented:?}"
    );

    // The recognised word sits where it was printed (±8 pt).
    let mut checked = 0;
    for w in ["ideeën", "Größe", "Straße", "élève", "français"] {
        let (Some(a), Some(b)) = (
            truth.search(w, true).first().map(|h| h.quads[0].bounds()),
            got.search(w, true).first().map(|h| h.quads[0].bounds()),
        ) else {
            continue;
        };
        let (ca, cb) = (
            ((a.x0 + a.x1) / 2.0, (a.y0 + a.y1) / 2.0),
            ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0),
        );
        assert!(
            (ca.0 - cb.0).abs() < 8.0 && (ca.1 - cb.1).abs() < 8.0,
            "{w}: printed {ca:?} vs recognised {cb:?}"
        );
        checked += 1;
    }
    assert!(checked >= 4, "only {checked} reference words matched");
}

fn scan_png_pdf_bytes(original: &[u8]) -> Vec<u8> {
    let doc = PdfDocument::open(original.to_vec(), &OpenOptions::default()).unwrap();
    let g = doc.pages().unwrap()[0].geometry;
    let png = with_session(Arc::new(original.to_vec()), |s| {
        s.render_page(0, g, Rotation::R0, 300.0 / 72.0)?.to_png()
    })
    .unwrap();
    let mut d = PdfDocument::from_image(&png).unwrap();
    d.snapshot_bytes().unwrap()
}

#[test]
fn cancelling_returns_what_was_done_and_empty_pages_add_nothing() {
    let Some(engine) = engine() else { return };
    let (mut scan, _) = scanned_report_page();
    let page = scan.page_ids().unwrap()[0];
    let spec = OcrPageSpec {
        page,
        index: 0,
        geometry: scan.pages().unwrap()[0].geometry,
    };
    let snap = Arc::new(scan.snapshot_bytes().unwrap());
    let r = recognize_pages(&engine, snap, &[spec, spec], &mut |_, _| false).unwrap();
    assert!(r.is_empty(), "cancelled before the first page");
    let (n, _) = scan
        .transact(|tx| add_ocr_text_layers(tx, &[(page, vec![])]))
        .unwrap();
    assert_eq!(n, 0);
}
