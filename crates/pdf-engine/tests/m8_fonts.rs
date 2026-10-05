//! The bundled font families: new text, annotations and replaced runs really use them, they are
//! embedded as subsets, survive saving, and the results validate as PDF/A.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::annot::{self, Align, AnnotationKind, AnnotationSpec, Rgb};
use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::fontembed::{FontFamily, FontStyle};
use pdf_engine::geom::{Point, Rect};
use pdf_engine::pagecontent::{self, PageContent, TextEdit};
use pdf_engine::pdfa::convert_bytes;
use std::process::Command;
use test_support::*;

fn open(bytes: Vec<u8>) -> PdfDocument {
    PdfDocument::open(bytes, &OpenOptions::default()).unwrap()
}

/// Font names poppler's `pdffonts` reports (and whether each is embedded).
fn pdffonts(bytes: &[u8]) -> Option<Vec<(String, bool)>> {
    if !have_tool("pdffonts") {
        return None;
    }
    let dir = tempfile::tempdir().ok()?;
    let p = dir.path().join("a.pdf");
    std::fs::write(&p, bytes).ok()?;
    let out = Command::new("pdffonts").arg(&p).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    Some(
        text.lines()
            .skip(2)
            .filter_map(|l| {
                let mut it = l.split_whitespace();
                let name = it.next()?.to_string();
                let emb = l.split_whitespace().any(|w| w == "yes");
                Some((name, emb))
            })
            .collect(),
    )
}

fn all_styles() -> Vec<FontStyle> {
    let mut v = Vec::new();
    for f in FontFamily::ALL {
        for b in [false, true] {
            for i in [false, true] {
                let s = FontStyle::new(f, b, i);
                if !v.contains(&s) {
                    v.push(s);
                }
            }
        }
    }
    v
}

#[test]
fn every_family_and_style_adds_real_text_with_its_own_embedded_font() {
    // A blank page, so the samples cannot collide with other text.
    let mut doc = PdfDocument::new_empty().unwrap();
    let page = doc
        .transact(|tx| pdf_engine::pageops::insert_blank_page(tx, 0, 595.0, 842.0))
        .unwrap()
        .0;
    let styles = all_styles();
    doc.transact(|tx| {
        for (i, s) in styles.iter().enumerate() {
            pagecontent::add_text(
                tx,
                page,
                Point::new(40.0, 780.0 - i as f64 * 22.0),
                &format!("Sample {} ëïü € Ελλ Рус", s.base_name()),
                12.0,
                (0.0, 0.0, 0.0),
                *s,
            )?;
        }
        Ok(())
    })
    .unwrap();
    let bytes = doc.snapshot_bytes().unwrap();
    if let Some(t) = poppler_text(&bytes) {
        for s in &styles {
            assert!(
                t.contains(&format!("Sample {} ëïü € Ελλ Рус", s.base_name())),
                "{s:?}\n{t}"
            );
        }
    } else {
        assert!(!oracles_required());
    }
    if let Some(fonts) = pdffonts(&bytes) {
        for s in &styles {
            let n = s.base_name();
            assert!(
                fonts
                    .iter()
                    .any(|(f, emb)| f.ends_with(&format!("+{n}")) && *emb),
                "{n} not embedded: {fonts:?}"
            );
        }
    }
    // Different families really look different: render and compare two single-style pages.
    let render = |s: FontStyle| {
        let mut d = PdfDocument::new_empty().unwrap();
        d.transact(|tx| {
            let p = pdf_engine::pageops::insert_blank_page(tx, 0, 595.0, 842.0)?;
            pagecontent::add_text(
                tx,
                p,
                Point::new(40.0, 700.0),
                "Hamburgefonstiv 0123",
                28.0,
                (0.0, 0.0, 0.0),
                s,
            )
        })
        .unwrap();
        let b = d.snapshot_bytes().unwrap();
        poppler_render(&b, 1, 72)
    };
    if let (Some(a), Some(b), Some(c)) = (
        render(FontStyle::new(FontFamily::LiberationSans, false, false)),
        render(FontStyle::new(FontFamily::LiberationSerif, false, false)),
        render(FontStyle::new(FontFamily::LiberationSans, false, true)),
    ) {
        assert!(
            mean_abs_diff(&a.2, &b.2) > 0.05,
            "sans and serif render identically"
        );
        assert!(
            mean_abs_diff(&a.2, &c.2) > 0.05,
            "upright and italic render identically"
        );
    }
}

#[test]
fn text_with_a_chosen_family_stays_editable_and_the_missing_glyph_message_names_it() {
    let mut doc = open(fixtures::helvetica_lines());
    let page = doc.page_ids().unwrap()[0];
    let style = FontStyle::new(FontFamily::LiberationSerif, true, true);
    doc.transact(|tx| {
        pagecontent::add_text(
            tx,
            page,
            Point::new(72.0, 400.0),
            "Serif bold italic",
            16.0,
            (0.0, 0.0, 0.0),
            style,
        )
    })
    .unwrap();
    let r = doc.transact(|tx| {
        pagecontent::add_text(
            tx,
            page,
            Point::new(72.0, 300.0),
            "漢字",
            16.0,
            (0.0, 0.0, 0.0),
            style,
        )
    });
    match r {
        Err(pdf_engine::EngineError::MissingGlyphs { font, .. }) => {
            assert!(font.contains("Liberation Serif"), "{font}");
        }
        other => panic!("expected MissingGlyphs, got {other:?}"),
    }
    let runs = PageContent::load(doc.lopdf(), page.0).unwrap().text_runs();
    let r = runs.iter().find(|r| r.text == "Serif bold italic").unwrap();
    assert!(
        r.base_font.contains("LiberationSerif-BoldItalic"),
        "{}",
        r.base_font
    );
    assert!(r.editable.is_ok());
}

#[test]
fn replacing_the_font_of_an_existing_run_uses_the_chosen_family() {
    let mut doc = open(fixture_bytes("report-chromium.pdf"));
    let page = doc.page_ids().unwrap()[0];
    let run = PageContent::load(doc.lopdf(), page.0)
        .unwrap()
        .text_runs()
        .into_iter()
        .find(|r| r.text.contains("Quarterly"))
        .unwrap();
    let style = FontStyle::new(FontFamily::LiberationMono, true, false);
    let (rep, _) = doc
        .transact(|tx| {
            PageContent::load(tx.doc(), page.0)?.edit_text(
                tx,
                run.id,
                &TextEdit {
                    text: Some("Quarterly Report".into()),
                    substitute_font: Some(style),
                    ..Default::default()
                },
            )
        })
        .unwrap();
    assert!(
        rep.warnings.iter().any(|w| w.contains("Liberation Mono")),
        "{:?}",
        rep.warnings
    );
    let bytes = doc.snapshot_bytes().unwrap();
    let back = open(bytes.clone());
    let runs = PageContent::load(back.lopdf(), back.page_ids().unwrap()[0].0)
        .unwrap()
        .text_runs();
    let r = runs.iter().find(|r| r.text == "Quarterly Report").unwrap();
    assert!(
        r.base_font.contains("LiberationMono-Bold"),
        "{}",
        r.base_font
    );
    if let Some(t) = poppler_text(&bytes) {
        assert!(t.contains("Quarterly Report"));
    }
}

#[test]
fn freetext_remembers_its_font_across_save_and_reopen() {
    let mut doc = open(fixture_bytes("report-cairo.pdf"));
    let page = doc.page_ids().unwrap()[0];
    let style = FontStyle::new(FontFamily::LiberationSerif, true, true);
    let (id, _) = doc
        .transact(|tx| {
            let mut spec = AnnotationSpec::new(AnnotationKind::FreeText {
                rect: Rect::new(100.0, 500.0, 400.0, 560.0),
                font_size: 14.0,
                text_color: Rgb(0.0, 0.0, 0.5),
                align: Align::Left,
                font: style,
                callout: None,
            });
            spec.contents = "Bold italic serif note".into();
            annot::add_annotation(tx, page, &spec)
        })
        .unwrap();
    let bytes = doc.snapshot_bytes().unwrap();
    let back = open(bytes.clone());
    let list = annot::read_annotations(&back, back.page_ids().unwrap()[0]);
    let a = list.iter().find(|a| a.id == id).unwrap();
    let Some(AnnotationKind::FreeText { font, .. }) = a.spec.as_ref().map(|s| s.kind.clone())
    else {
        panic!("not a free text");
    };
    assert_eq!(font, style);
    // Its appearance really is in that font.
    if let Some(fonts) = pdffonts(&bytes) {
        assert!(
            fonts
                .iter()
                .any(|(f, emb)| f.ends_with("+LiberationSerif-BoldItalic") && *emb),
            "{fonts:?}"
        );
    }
    assert!(
        annot::freetext_required_height("a b c", 14.0, 200.0, style).unwrap()
            > annot::freetext_required_height("a", 14.0, 200.0, style).unwrap() - 1.0
    );
}

#[test]
fn documents_using_every_font_convert_to_validated_pdfa() {
    let mut doc = open(fixture_bytes("report-chromium.pdf"));
    let page = doc.page_ids().unwrap()[0];
    let styles = all_styles();
    doc.transact(|tx| {
        for (i, s) in styles.iter().enumerate() {
            pagecontent::add_text(
                tx,
                page,
                Point::new(40.0, 700.0 - i as f64 * 20.0),
                "Archive me ëü €",
                11.0,
                (0.0, 0.0, 0.0),
                *s,
            )?;
        }
        let mut spec = AnnotationSpec::new(AnnotationKind::FreeText {
            rect: Rect::new(300.0, 300.0, 500.0, 360.0),
            font_size: 12.0,
            text_color: Rgb(0.0, 0.0, 0.0),
            align: Align::Left,
            font: FontStyle::new(FontFamily::LiberationMono, false, true),
            callout: None,
        });
        spec.contents = "mono italic".into();
        annot::add_annotation(tx, page, &spec)?;
        Ok(())
    })
    .unwrap();
    let bytes = doc.snapshot_bytes().unwrap();
    let (out, rep) = convert_bytes(&bytes).unwrap_or_else(|e| panic!("{e}"));
    assert!(rep.blockers.is_empty());
    match verapdf(&out, "2b") {
        Some(r) => assert!(r.compliant, "{}", r.failures.join("\n")),
        None => assert!(!verapdf_required()),
    }
}
