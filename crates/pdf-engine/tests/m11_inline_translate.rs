//! Inline translation: paragraphs are found, the original text goes away, the new text is in the
//! same place and the file stays valid. The "translation" here is just upper-casing, so no AI
//! service is needed to check the layout and editing machinery.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::inlinetr::{self, Item};
use pdf_engine::render::with_session;
use std::sync::Arc;
use test_support::*;

fn blocks_and_background(bytes: &[u8]) -> Vec<Item> {
    let doc = PdfDocument::open(bytes.to_vec(), &OpenOptions::default()).unwrap();
    let g = doc.pages().unwrap()[0].geometry;
    with_session(Arc::new(bytes.to_vec()), |s| {
        let tp = s.extract_text(0, &g)?;
        let bmp = s.render_page(0, g, pdf_engine::geom::Rotation::R0, 1.0)?;
        Ok(inlinetr::blocks_of(&tp)
            .into_iter()
            .map(|b| Item {
                translation: b.text.to_uppercase(),
                background: inlinetr::sample_background(&bmp, &g, 1.0, b.rect),
                block: b,
            })
            .collect())
    })
    .unwrap()
}

#[test]
fn paragraphs_are_found_and_replaced_in_place() {
    if !pdftotext_usable() {
        assert!(!oracles_required(), "poppler missing");
        return;
    }
    for name in ["report-chromium.pdf", "report-cairo.pdf"] {
        let original = fixture_bytes(name);
        let items = blocks_and_background(&original);
        assert!(items.len() >= 3, "{name}: only {} blocks", items.len());
        let before = poppler_text_page(&original, 1).unwrap();
        let sample = items
            .iter()
            .map(|i| i.block.text.clone())
            .find(|t| t.chars().count() >= 25)
            .expect("a block with some text");
        let probe: String = sample.chars().take(25).collect();
        assert!(
            before.replace('\n', " ").contains(&probe) || before.contains(&sample[..10]),
            "{name}: the extracted block is not in the page text? {probe:?}"
        );

        let mut doc = PdfDocument::open(original.clone(), &OpenOptions::default()).unwrap();
        let page = doc.pages().unwrap()[0].id;
        let (report, _) = doc
            .transact(|tx| inlinetr::apply_translation(tx, page, &items))
            .unwrap();
        assert_eq!(report.blocks, items.len());
        let bytes = doc.snapshot_bytes().unwrap();
        assert_eq!(poppler_diagnostics(&bytes).unwrap(), "", "{name}");
        if let Ok(dir) = std::env::var("BERG_DUMP") {
            let _ = std::fs::write(format!("{dir}/{name}.translated.pdf"), &bytes);
        }

        let after = poppler_text_page(&bytes, 1).unwrap();
        let upper_probe: String = probe.to_uppercase();
        assert!(
            after.replace('\n', " ").contains(upper_probe.trim()),
            "{name}: translated text missing from the page: {upper_probe:?}\n{after}"
        );
        // The original words are gone from the page content (not merely painted over).
        let first_word = probe.split_whitespace().next().unwrap();
        if first_word.chars().count() >= 5 && first_word.chars().any(char::is_lowercase) {
            let lower_hits = after.matches(first_word).count();
            let before_hits = before.matches(first_word).count();
            assert!(
                lower_hits < before_hits,
                "{name}: '{first_word}' still appears {lower_hits}x (was {before_hits}x)"
            );
        }
        // It still opens and renders.
        if let Some(r) = poppler_render(&bytes, 1, 72) {
            assert!(r.0 > 100 && r.1 > 100);
        } else {
            assert!(!oracles_required(), "pdftoppm required but missing");
        }
    }
}

#[test]
fn bare_numbers_and_symbols_are_not_translated() {
    if !pdftotext_usable() {
        return;
    }
    let items = blocks_and_background(&fixture_bytes("report-chromium.pdf"));
    assert!(
        items
            .iter()
            .all(|i| i.block.text.chars().filter(|c| c.is_alphabetic()).count() >= 2)
    );
}

#[test]
fn text_without_a_covering_font_is_reported_not_lost() {
    let original = fixture_bytes("report-chromium.pdf");
    let mut items = blocks_and_background(&original);
    items.truncate(1);
    items[0].translation = "Hello \u{10FFFD} world".into();
    let mut doc = PdfDocument::open(original, &OpenOptions::default()).unwrap();
    let page = doc.pages().unwrap()[0].id;
    let (report, _) = doc
        .transact(|tx| inlinetr::apply_translation(tx, page, &items))
        .unwrap();
    assert!(report.missing_chars.contains(&'\u{10FFFD}'), "{report:?}");
    let bytes = doc.snapshot_bytes().unwrap();
    if let Some(t) = poppler_text_page(&bytes, 1) {
        assert!(t.contains("Hello ? world"), "{t}");
    }
}

#[test]
fn cjk_text_uses_an_installed_font_when_there_is_one() {
    pdf_engine::sysfonts::scan_and_register();
    let probe = ['你', '好'];
    let Some(id) = pdf_engine::sysfonts::family_covering(&probe) else {
        eprintln!("no installed font covers CJK here; nothing to check");
        return;
    };
    let original = fixture_bytes("report-chromium.pdf");
    let mut items = blocks_and_background(&original);
    items.truncate(1);
    items[0].translation = "你好 world".into();
    let mut doc = PdfDocument::open(original, &OpenOptions::default()).unwrap();
    let page = doc.pages().unwrap()[0].id;
    let (report, _) = doc
        .transact(|tx| inlinetr::apply_translation(tx, page, &items))
        .unwrap();
    let name = pdf_engine::sysfonts::with_family(id, |f| f.name).unwrap();
    assert!(report.system_fonts.contains(name), "{report:?}");
    assert!(report.missing_chars.is_empty());
    let bytes = doc.snapshot_bytes().unwrap();
    if let Some(t) = poppler_text_page(&bytes, 1) {
        assert!(t.contains("你好"), "{t}");
    }
}
