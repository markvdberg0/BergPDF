//! The translation export: a real, searchable text PDF.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::pdfa::convert_bytes;
use pdf_engine::textdoc::build_text_pdf;
use test_support::*;

fn sections(n: usize) -> Vec<(String, String)> {
    (1..=n)
        .map(|i| {
            (
                format!("Page {i}"),
                format!(
                    "Café über naïve résumé — price €{i}.\nThe quick brown fox jumps over the lazy dog and keeps running across the whole line until it wraps around.\n\n- first item {i}\n- second item {i}\nA sentence that continues\non the next source line.\n{}",
                    "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ".repeat(12)
                ),
            )
        })
        .collect()
}

#[test]
fn long_text_paginates_and_stays_searchable_with_accents() {
    let (bytes, rep) = build_text_pdf(
        "Translation of Contract.pdf",
        "Dutch → English, machine translation by BergPDF Copilot.",
        &sections(12),
    )
    .unwrap();
    assert!(rep.pages >= 3, "{rep:?}");
    assert!(rep.replaced.is_empty());
    let doc = PdfDocument::open(bytes.clone(), &OpenOptions::default()).unwrap();
    assert_eq!(doc.page_count(), rep.pages);
    if let Some(t) = poppler_text(&bytes) {
        assert!(t.contains("Translation of Contract.pdf"), "title missing");
        assert!(
            t.contains("Café über naïve résumé"),
            "accents lost:\n{}",
            &t[..t.len().min(400)]
        );
        assert!(t.contains("Page 12"));
        assert!(
            t.contains("A sentence that continues on the next source line."),
            "paragraph joining"
        );
        assert!(t.contains("- first item 3"));
    } else {
        assert!(!oracles_required());
    }
    if let Some(d) = poppler_diagnostics(&bytes) {
        assert!(d.trim().is_empty(), "{d}");
    }
    if let Some((n, _)) = poppler_info(&bytes) {
        assert_eq!(n, Some(rep.pages));
    }
}

#[test]
fn characters_the_font_lacks_are_replaced_and_reported() {
    let (bytes, rep) =
        build_text_pdf("T", "", &[("Page 1".into(), "Hello 漢字 world Ω".into())]).unwrap();
    assert_eq!(rep.replaced, vec!['字', '漢']);
    if let Some(t) = poppler_text(&bytes) {
        assert!(t.contains("Hello ?? world Ω"), "{t}");
    }
}

#[test]
fn very_long_words_and_empty_input_do_not_break_layout() {
    let (bytes, rep) = build_text_pdf("T", "", &[("Page 1".into(), "x".repeat(900))]).unwrap();
    assert!(rep.pages >= 1);
    PdfDocument::open(bytes, &OpenOptions::default()).unwrap();
    let (bytes, rep) = build_text_pdf("Only a title", "", &[]).unwrap();
    assert_eq!(rep.pages, 1);
    PdfDocument::open(bytes, &OpenOptions::default()).unwrap();
}

#[test]
fn the_export_converts_to_validated_pdfa() {
    // The embedded subset fonts written by BergPDF are checked by an independent validator.
    let (bytes, _) = build_text_pdf("Translation", "note", &sections(3)).unwrap();
    let (out, rep) = convert_bytes(&bytes).unwrap_or_else(|e| panic!("{e}"));
    assert!(rep.blockers.is_empty());
    match verapdf(&out, "2b") {
        Some(r) => assert!(r.compliant, "{}", r.failures.join("\n")),
        None => assert!(!verapdf_required()),
    }
}
