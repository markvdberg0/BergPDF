//! Milestone 3: genuine editing of existing page text, verified against independent tools
//! (poppler) and by inspecting decoded content — never by looking only at our own output.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::doc::{OpenOptions, PageId, PdfDocument};
use pdf_engine::error::EngineError;
use pdf_engine::pagecontent::{PageContent, TextEdit, TextRunInfo};
use pdf_engine::render::with_session;
use std::sync::Arc;
use test_support::fixtures::*;
use test_support::*;

fn open(bytes: Vec<u8>) -> PdfDocument {
    PdfDocument::open(bytes, &OpenOptions::default()).unwrap()
}

fn runs(doc: &PdfDocument, page: PageId) -> Vec<TextRunInfo> {
    PageContent::load(doc.lopdf(), page.0).unwrap().text_runs()
}

fn find_run(doc: &PdfDocument, page: PageId, needle: &str) -> TextRunInfo {
    let rs = runs(doc, page);
    rs.iter()
        .find(|r| r.text.contains(needle))
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "no run containing {needle:?} in {:?}",
                rs.iter().map(|r| r.text.clone()).collect::<Vec<_>>()
            )
        })
}

/// Apply an edit, snapshot, reopen — returns the reopened document and the saved bytes.
fn edit_and_reopen(
    doc: &mut PdfDocument,
    page: PageId,
    run: &TextRunInfo,
    edit: &TextEdit,
) -> Result<(PdfDocument, Vec<u8>, pdf_engine::pagecontent::EditReport), EngineError> {
    let (report, _) = doc.transact(|tx| {
        let pc = PageContent::load(tx.doc(), page.0)?;
        pc.edit_text(tx, run.id, edit)
    })?;
    let bytes = doc.snapshot_bytes().unwrap();
    Ok((open(bytes.clone()), bytes, report))
}

fn all_text(bytes: &[u8]) -> String {
    poppler_text(bytes).unwrap_or_else(|| {
        eprintln!("poppler missing; falling back to hayro text");
        with_session(Arc::new(bytes.to_vec()), |s| {
            let d = open(bytes.to_vec());
            let mut out = String::new();
            for (i, p) in d.pages().unwrap().iter().enumerate() {
                out.push_str(&s.extract_text(i, &p.geometry)?.plain_text());
                out.push('\n');
            }
            Ok(out)
        })
        .unwrap()
    })
}

#[test]
fn chromium_per_glyph_runs_are_grouped_into_one_editable_line() {
    let doc = open(fixture_bytes("report-chromium.pdf"));
    let page = doc.pages().unwrap()[2].id;
    let r = find_run(&doc, page, "ALPHA-7731");
    assert!(
        r.text
            .starts_with("Confidential appendix: the secret code is ALPHA-7731"),
        "{:?}",
        r.text
    );
    assert!(r.editable.is_ok(), "{:?}", r.editable);
    assert!(r.embedded && r.subset);
    assert!(r.size_pt > 9.0 && r.size_pt < 11.0, "size {}", r.size_pt);
    // Its box is where the text is on the page (compare to hayro's independent extraction).
    let q = r.quad.bounds();
    let bytes = doc.original_bytes().clone();
    let (gx0, gx1) = with_session(bytes, |s| {
        let tp = s.extract_text(2, &doc.page_geometry(page).unwrap())?;
        let hit = &tp.search("secret code", false)[0];
        let b = hit.quads[0].bounds();
        Ok((b.x0, b.x1))
    })
    .unwrap();
    assert!(
        q.x0 <= gx0 + 1.0 && q.x1 >= gx1 - 1.0,
        "run box {q:?} must contain hayro's glyph span {gx0}..{gx1}"
    );
}

#[test]
fn edit_existing_text_replaces_real_content_not_a_cover_up() {
    let mut doc = open(fixture_bytes("report-chromium.pdf"));
    let page = doc.pages().unwrap()[2].id;
    let before_runs = runs(&doc, page);
    let run = find_run(&doc, page, "ALPHA-7731");
    let new_text = run.text.replace("7731", "1377");
    let (re, bytes, report) = edit_and_reopen(
        &mut doc,
        page,
        &run,
        &TextEdit {
            text: Some(new_text.clone()),
            ..Default::default()
        },
    )
    .unwrap();

    // 1. Independent extraction sees the new text and not the old.
    let txt = all_text(&bytes);
    assert!(txt.contains("ALPHA-1377"), "{txt}");
    assert!(
        !txt.contains("ALPHA-7731"),
        "old text must be gone from the extracted text"
    );
    // 2. The *decoded content stream* no longer contains the old glyph sequence: compare the
    //    run list of the reopened document (decoded from the stream).
    let after = runs(&re, re.pages().unwrap()[2].id);
    assert!(after.iter().any(|r| r.text == new_text));
    assert!(!after.iter().any(|r| r.text.contains("7731")));
    // 3. Nothing else on the page changed: same other runs, same boxes.
    let others_before: Vec<_> = before_runs
        .iter()
        .filter(|r| r.id != run.id)
        .map(|r| (r.text.clone(), r.quad.bounds()))
        .collect();
    let others_after: Vec<_> = after
        .iter()
        .filter(|r| r.text != new_text)
        .map(|r| (r.text.clone(), r.quad.bounds()))
        .collect();
    assert_eq!(others_before.len(), others_after.len());
    for ((t0, b0), (t1, b1)) in others_before.iter().zip(&others_after) {
        assert_eq!(t0, t1);
        assert!(
            (b0.x0 - b1.x0).abs() < 1e-3 && (b0.y0 - b1.y0).abs() < 1e-3,
            "{t0}: {b0:?} vs {b1:?}"
        );
    }
    // 4. No white-box cover-up: the new content contains no filled white rectangles.
    let pc = PageContent::load(re.lopdf(), re.pages().unwrap()[2].id.0).unwrap();
    let _ = pc;
    let content = re.lopdf().get_page_content(re.pages().unwrap()[2].id.0);
    assert!(
        !String::from_utf8_lossy(&content).contains("1 1 1 rg"),
        "a white fill would indicate a cover-up"
    );
    // 5. The report is honest about layout (same-width digits => no width change).
    assert!(!report.overlaps_following);
    assert!(
        (report.width_after_pt - report.width_before_pt).abs() < 1.0,
        "{report:?}"
    );
    // 6. It still renders: the edited line has ink where the text is, per poppler.
    if let Some((w, h, rgb)) = poppler_render(&bytes, 3, 72) {
        let b = run.quad.bounds();
        let f = non_white_fraction(
            &rgb,
            w,
            b.x0 as u32,
            (h as f64 - b.y1) as u32,
            b.x1 as u32,
            (h as f64 - b.y0) as u32,
            60,
        );
        assert!(f > 0.03, "edited line must be visible (ink fraction {f})");
    }
    // 7. Original file bytes are preserved verbatim (incremental update).
    assert!(bytes.starts_with(doc.original_bytes().as_slice()));
}

#[test]
fn size_change_and_move_keep_following_content_in_place() {
    let mut doc = open(fixture_bytes("report-chromium.pdf"));
    let page = doc.pages().unwrap()[0].id;
    let rs = runs(&doc, page);
    let run = rs
        .iter()
        .find(|r| r.text.starts_with("Quarterly"))
        .cloned()
        .unwrap();
    let (re, _bytes, rep) = edit_and_reopen(
        &mut doc,
        page,
        &run,
        &TextEdit {
            size_pt: Some(run.size_pt * 1.5),
            shift: Some((10.0, -4.0)),
            ..Default::default()
        },
    )
    .unwrap();
    let after = runs(&re, re.pages().unwrap()[0].id);
    let moved = after
        .iter()
        .find(|r| r.text.starts_with("Quarterly"))
        .unwrap();
    assert!(
        (moved.size_pt / run.size_pt - 1.5).abs() < 0.02,
        "{} vs {}",
        moved.size_pt,
        run.size_pt
    );
    assert!(
        (moved.quad.bounds().x0 - run.quad.bounds().x0 - 10.0).abs() < 0.05,
        "moved x"
    );
    assert!(moved.width_pt > run.width_pt * 1.4, "text scales with size");
    assert!(rep.warnings.is_empty() || !rep.overlaps_following);
    // Every other run is unchanged (compared by order; texts can repeat).
    let before: Vec<_> = rs.iter().filter(|r| r.id != run.id).collect();
    let after_others: Vec<_> = after
        .iter()
        .filter(|r| !r.text.starts_with("Quarterly"))
        .collect();
    assert_eq!(before.len(), after_others.len());
    for (r, a) in before.iter().zip(after_others) {
        assert_eq!(r.text, a.text);
        assert!(
            (a.quad.bounds().x0 - r.quad.bounds().x0).abs() < 1e-3
                && (a.quad.bounds().y0 - r.quad.bounds().y0).abs() < 1e-3,
            "run {:?} moved: {:?} -> {:?}",
            r.text,
            r.quad.bounds(),
            a.quad.bounds()
        );
    }
}

#[test]
fn characters_missing_from_a_subset_font_are_reported_and_substitution_is_explicit() {
    let mut doc = open(fixture_bytes("report-chromium.pdf"));
    let page = doc.pages().unwrap()[2].id;
    let run = find_run(&doc, page, "ALPHA-7731");
    // 'Z' and 'Q' do not occur in this subset.
    let err = edit_and_reopen(
        &mut doc,
        page,
        &run,
        &TextEdit {
            text: Some("QZ-1377".into()),
            ..Default::default()
        },
    )
    .err()
    .expect("must fail");
    match err {
        EngineError::MissingGlyphs { chars, font } => {
            assert!(chars.contains('Z') && chars.contains('Q'), "{chars}");
            assert!(font.contains("DejaVu"), "{font}");
        }
        other => panic!("expected MissingGlyphs, got {other:?}"),
    }
    // The failed attempt changed nothing.
    assert!(!doc.has_changes_since_base());
    // Explicit substitution with the bundled font works and warns about the font change.
    let (re, bytes, report) = edit_and_reopen(
        &mut doc,
        page,
        &run,
        &TextEdit {
            text: Some("QZ-1377".into()),
            substitute_font: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        report.warnings.iter().any(|w| w.contains("Font changed")),
        "{:?}",
        report.warnings
    );
    assert!(all_text(&bytes).contains("QZ-1377"));
    let after = runs(&re, re.pages().unwrap()[2].id);
    let r = after.iter().find(|r| r.text == "QZ-1377").unwrap();
    assert!(r.base_font.contains("DejaVuSans"));
}

#[test]
fn wider_text_warns_about_overlap_and_fit_width_keeps_the_width() {
    let mut doc = open(fixture_bytes("report-cairo.pdf"));
    let page = doc.pages().unwrap()[0].id;
    let run = find_run(&doc, page, "Quarterly");
    // cairo: simple TrueType WinAnsi subset; use letters already present on the page.
    let t = "Quarterly Report Report";
    let rep = {
        let (rep, _) = doc
            .transact(|tx| {
                let pc = PageContent::load(tx.doc(), page.0)?;
                pc.edit_text(
                    tx,
                    run.id,
                    &TextEdit {
                        text: Some(t.into()),
                        ..Default::default()
                    },
                )
            })
            .unwrap();
        rep
    };
    assert!(rep.width_after_pt > rep.width_before_pt);
    assert!(
        rep.warnings
            .iter()
            .any(|w| w.contains("wider") || w.contains("overlap")),
        "{:?}",
        rep.warnings
    );
    // fit_width keeps the original width by tightening character spacing.
    let mut doc2 = open(fixture_bytes("report-cairo.pdf"));
    let run2 = find_run(&doc2, page, "Quarterly");
    let (rep2, _) = doc2
        .transact(|tx| {
            let pc = PageContent::load(tx.doc(), page.0)?;
            pc.edit_text(
                tx,
                run2.id,
                &TextEdit {
                    text: Some(t.into()),
                    fit_width: true,
                    ..Default::default()
                },
            )
        })
        .unwrap();
    assert!(
        (rep2.width_after_pt - rep2.width_before_pt).abs() < 0.01,
        "{rep2:?}"
    );
    assert!(!rep2.overlaps_following);
    let bytes = doc2.snapshot_bytes().unwrap();
    // poppler drops spaces when character spacing is tightened, so compare without them.
    assert!(
        all_text(&bytes)
            .replace([' ', '\n'], "")
            .contains("QuarterlyReportReport")
    );
    let re = open(bytes);
    let r = find_run(&re, re.pages().unwrap()[0].id, "Quarterly Report Report");
    assert!(
        (r.width_pt - run2.width_pt).abs() < 0.5,
        "{} vs {}",
        r.width_pt,
        run2.width_pt
    );
}

#[test]
fn cairo_simple_truetype_fonts_edit_and_other_text_is_untouched() {
    let mut doc = open(fixture_bytes("report-cairo.pdf"));
    let page = doc.pages().unwrap()[2].id;
    let run = find_run(&doc, page, "ALPHA-7731");
    assert!(run.editable.is_ok(), "{:?}", run.editable);
    let before_all = runs(&doc, page);
    let nt = run.text.replace("ALPHA-7731", "ALPHA-3177");
    let (re, bytes, _) = edit_and_reopen(
        &mut doc,
        page,
        &run,
        &TextEdit {
            text: Some(nt),
            ..Default::default()
        },
    )
    .unwrap();
    let txt = all_text(&bytes);
    assert!(
        txt.contains("ALPHA-3177") && !txt.contains("ALPHA-7731"),
        "{txt}"
    );
    let after = runs(&re, re.pages().unwrap()[2].id);
    assert_eq!(after.len(), before_all.len());
    assert!(
        after
            .iter()
            .filter(|r| r.text.contains("Appendix") || r.text.contains("sentence"))
            .count()
            >= 2
    );
}

#[test]
fn base14_fonts_edit_delete_and_preserve_relative_positioning() {
    let mut doc = open(helvetica_lines());
    let page = doc.pages().unwrap()[0].id;
    let rs = runs(&doc, page);
    assert_eq!(
        rs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
        [
            "Hello World",
            "Second line of text",
            "Test kerning (parens)",
            "Small print: price € 99"
        ]
    );
    let second = rs[1].clone();
    // Change the 2nd line's text; the 3rd line is positioned *relative* to it (Td) and must stay put.
    let third_before = rs[2].quad.bounds();
    let (re, bytes, _) = edit_and_reopen(
        &mut doc,
        page,
        &second,
        &TextEdit {
            text: Some("A much longer replacement line".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let after = runs(&re, re.pages().unwrap()[0].id);
    let third = after
        .iter()
        .find(|r| r.text.starts_with("Test"))
        .unwrap()
        .quad
        .bounds();
    assert!(
        (third.x0 - third_before.x0).abs() < 1e-3 && (third.y0 - third_before.y0).abs() < 1e-3,
        "{third:?} vs {third_before:?}"
    );
    assert!(all_text(&bytes).contains("A much longer replacement line"));
    assert!(!all_text(&bytes).contains("Second line of text"));
    // WinAnsi euro sign: encodable and round-trips.
    let small = after
        .iter()
        .find(|r| r.text.starts_with("Small"))
        .cloned()
        .unwrap();
    let mut doc = re;
    let (_, b2, _) = edit_and_reopen(
        &mut doc,
        page,
        &small,
        &TextEdit {
            text: Some("Small print: price € 100".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(all_text(&b2).contains('€'));
    // A character outside WinAnsi is refused, not dropped.
    let hello = runs(&doc, page).into_iter().next().unwrap();
    match edit_and_reopen(
        &mut doc,
        page,
        &hello,
        &TextEdit {
            text: Some("Hello → World".into()),
            ..Default::default()
        },
    ) {
        Err(EngineError::MissingGlyphs { chars, .. }) => assert!(chars.contains('→')),
        other => panic!("{:?}", other.err()),
    }
    // Delete a run: its glyphs disappear, the others keep their place.
    let hello = runs(&doc, page).into_iter().next().unwrap();
    let keep: Vec<_> = runs(&doc, page)
        .into_iter()
        .skip(1)
        .map(|r| (r.text, r.quad.bounds()))
        .collect();
    doc.transact(|tx| PageContent::load(tx.doc(), page.0)?.delete_text(tx, hello.id))
        .unwrap();
    let re = open(doc.snapshot_bytes().unwrap());
    let rest: Vec<_> = runs(&re, re.pages().unwrap()[0].id)
        .into_iter()
        .map(|r| (r.text, r.quad.bounds()))
        .collect();
    assert_eq!(rest.len(), keep.len());
    for ((t0, b0), (t1, b1)) in keep.iter().zip(&rest) {
        assert_eq!(t0, t1);
        assert!(
            (b0.x0 - b1.x0).abs() < 1e-3 && (b0.y0 - b1.y0).abs() < 1e-3,
            "{t0}"
        );
    }
}

#[test]
fn shared_content_streams_are_copied_before_editing() {
    let mut doc = open(shared_content_stream());
    let pages = doc.pages().unwrap();
    let run = runs(&doc, pages[0].id).into_iter().next().unwrap();
    assert_eq!(run.text, "Shared page text");
    let (re, _, _) = edit_and_reopen(
        &mut doc,
        pages[0].id,
        &run,
        &TextEdit {
            text: Some("Edited only here".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let rp = re.pages().unwrap();
    assert_eq!(runs(&re, rp[0].id)[0].text, "Edited only here");
    assert_eq!(
        runs(&re, rp[1].id)[0].text,
        "Shared page text",
        "page 2 must still show the original"
    );
}

#[test]
fn stale_references_are_rejected() {
    let mut doc = open(helvetica_lines());
    let page = doc.pages().unwrap()[0].id;
    let rs = runs(&doc, page);
    doc.transact(|tx| {
        PageContent::load(tx.doc(), page.0)?.edit_text(
            tx,
            rs[0].id,
            &TextEdit {
                text: Some("Changed".into()),
                ..Default::default()
            },
        )
    })
    .unwrap();
    // The old reference no longer matches the page content.
    let r = doc.transact(|tx| {
        PageContent::load(tx.doc(), page.0)?.edit_text(
            tx,
            rs[0].id,
            &TextEdit {
                text: Some("Again".into()),
                ..Default::default()
            },
        )
    });
    assert!(
        matches!(r, Err(EngineError::StaleReference)),
        "{:?}",
        r.err()
    );
}

#[test]
fn unsupported_text_is_listed_with_a_reason_not_corrupted() {
    // A Type3-font page: runs are located but marked non-editable with a clear reason.
    use pdf_writer::{Content, Name, Pdf, Rect, Ref, Str};
    let mut pdf = Pdf::new();
    let (cat, tree, page, font, content) = (
        Ref::new(1),
        Ref::new(2),
        Ref::new(3),
        Ref::new(4),
        Ref::new(5),
    );
    pdf.catalog(cat).pages(tree);
    pdf.pages(tree).kids([page]).count(1);
    {
        let mut p = pdf.page(page);
        p.media_box(Rect::new(0.0, 0.0, 200.0, 200.0))
            .parent(tree)
            .contents(content);
        p.resources().fonts().pair(Name(b"F3"), font);
    }
    {
        let mut f = pdf.indirect(font).dict();
        f.pair(Name(b"Type"), Name(b"Font"));
        f.pair(Name(b"Subtype"), Name(b"Type3"));
    }
    let mut c = Content::new();
    c.begin_text()
        .set_font(Name(b"F3"), 10.0)
        .next_line(10.0, 100.0)
        .show(Str(b"abc"))
        .end_text();
    pdf.stream(content, &c.finish());
    let doc = open(pdf.finish());
    let rs = runs(&doc, doc.pages().unwrap()[0].id);
    assert_eq!(rs.len(), 1);
    assert!(rs[0].editable.as_ref().unwrap_err().contains("Type 3"));
}
