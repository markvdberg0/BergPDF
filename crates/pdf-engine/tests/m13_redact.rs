//! Secure redaction, checked from the outside: with poppler's `pdftotext` as an independent reader, by
//! searching every byte and every decoded stream of the saved file for the removed words, and by comparing
//! renderings of the page before and after.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lopdf::{Object, dictionary};
use pdf_engine::annot::{self, AnnotationKind, AnnotationSpec};
use pdf_engine::doc::{OpenOptions, PageId, PdfDocument};
use pdf_engine::geom::{Point, Rect, Rotation};
use pdf_engine::pageops;
use pdf_engine::redact::{self, ImageMode, RedactOptions};
use pdf_engine::render::{Bitmap, with_session};
use std::sync::Arc;
use test_support::fixtures::{helvetica_lines, image_page, odd_geometry};
use test_support::*;

fn open(bytes: Vec<u8>) -> PdfDocument {
    PdfDocument::open(bytes, &OpenOptions::default()).unwrap()
}

/// "Second line of text" in `helvetica_lines` (14 pt at 72,680): a box around it.
const SECOND_LINE: Rect = Rect {
    x0: 70.0,
    y0: 676.0,
    x1: 300.0,
    y1: 696.0,
};

fn mark(doc: &mut PdfDocument, page: PageId, rect: Rect) {
    doc.transact(|tx| redact::add_mark(tx, page, rect)).unwrap();
}

fn first_page(doc: &PdfDocument) -> PageId {
    doc.page_ids().unwrap()[0]
}

fn render(bytes: &[u8], page_index: usize, scale: f64) -> (Bitmap, pdf_engine::geom::PageGeometry) {
    let doc = open(bytes.to_vec());
    let page = doc.page_ids().unwrap()[page_index];
    let geom = doc.page_geometry(page).unwrap();
    let bmp = with_session(Arc::new(bytes.to_vec()), |s| {
        s.render_page(page_index, geom, Rotation::R0, scale)
    })
    .unwrap();
    (bmp, geom)
}

/// `needle` anywhere in the file: as it is written, or in any stream after decoding it.
fn file_contains(bytes: &[u8], needle: &str) -> bool {
    if bytes.windows(needle.len()).any(|w| w == needle.as_bytes()) {
        return true;
    }
    let doc = lopdf::Document::load_mem(bytes).unwrap();
    doc.objects.values().any(|o| match o {
        Object::Stream(s) => {
            let data = s
                .decompressed_content()
                .unwrap_or_else(|_| s.content.clone());
            data.windows(needle.len()).any(|w| w == needle.as_bytes())
        }
        Object::String(b, _) => b.windows(needle.len()).any(|w| w == needle.as_bytes()),
        _ => false,
    })
}

/// Pixel box of `rect` (user space) in a bitmap rendered at `scale`.
fn pixel_box(
    geom: &pdf_engine::geom::PageGeometry,
    rect: Rect,
    scale: f64,
) -> (u32, u32, u32, u32) {
    let t = geom.pdf_to_view(Rotation::R0);
    let pts = [
        t * Point::new(rect.x0, rect.y0),
        t * Point::new(rect.x1, rect.y1),
    ];
    let x0 = pts[0].x.min(pts[1].x) * scale;
    let x1 = pts[0].x.max(pts[1].x) * scale;
    let y0 = pts[0].y.min(pts[1].y) * scale;
    let y1 = pts[0].y.max(pts[1].y) * scale;
    (
        x0.ceil() as u32 + 1,
        y0.ceil() as u32 + 1,
        (x1.floor() as u32).saturating_sub(1),
        (y1.floor() as u32).saturating_sub(1),
    )
}

/// Mean absolute difference over everything *outside* `skip` (pixel box, grown by `grow`).
fn diff_outside(a: &Bitmap, b: &Bitmap, skip: (u32, u32, u32, u32), grow: u32) -> f64 {
    assert_eq!(
        (a.width, a.height),
        (b.width, b.height),
        "page size changed"
    );
    let (mut sum, mut n) = (0u64, 0u64);
    for y in 0..a.height {
        for x in 0..a.width {
            if x + grow >= skip.0 && x <= skip.2 + grow && y + grow >= skip.1 && y <= skip.3 + grow
            {
                continue;
            }
            let i = ((y * a.width + x) * 4) as usize;
            for c in 0..3 {
                sum += u64::from(a.rgba[i + c].abs_diff(b.rgba[i + c]));
            }
            n += 3;
        }
    }
    sum as f64 / n.max(1) as f64
}

fn mean_inside(b: &Bitmap, r: (u32, u32, u32, u32)) -> f64 {
    let (mut sum, mut n) = (0u64, 0u64);
    for y in r.1..=r.3.min(b.height - 1) {
        for x in r.0..=r.2.min(b.width - 1) {
            let i = ((y * b.width + x) * 4) as usize;
            sum += u64::from(b.rgba[i]) + u64::from(b.rgba[i + 1]) + u64::from(b.rgba[i + 2]);
            n += 3;
        }
    }
    sum as f64 / n.max(1) as f64
}

/// Page text from poppler with every run of white space reduced to one space (the readers differ in how they
/// space words that were placed one by one).
fn poppler_text(bytes: &[u8], page: usize) -> Option<String> {
    let t =
        poppler_text_page(bytes, page).map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "));
    if t.is_none() {
        assert!(
            !oracles_required(),
            "poppler is required for this check (BERG_REQUIRE_ORACLES=1)"
        );
    }
    t
}

#[test]
fn marks_are_listed_and_do_not_change_the_page() {
    let mut doc = open(helvetica_lines());
    let page = first_page(&doc);
    assert!(redact::marks(&doc).is_empty());
    mark(&mut doc, page, SECOND_LINE);
    let marks = redact::marks(&doc);
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].page, page);
    assert!((marks[0].rect.x0 - 70.0).abs() < 1e-6);
    // Too small to be a mark.
    assert!(
        doc.transact(|tx| redact::add_mark(tx, page, Rect::new(10.0, 10.0, 10.2, 10.2)))
            .is_err()
    );
    // The marked page still holds everything: the text is there until the marks are applied.
    let bytes = doc.snapshot_bytes().unwrap();
    assert!(file_contains(&bytes, "Second line of text"));
    // A mark can be deleted like any annotation.
    doc.transact(|tx| annot::delete_annotation(tx, page, marks[0].id))
        .unwrap();
    assert!(redact::marks(&doc).is_empty());
}

#[test]
fn applying_nothing_is_an_error_and_changes_nothing() {
    let mut doc = open(helvetica_lines());
    assert!(redact::apply(&mut doc, &RedactOptions::default()).is_err());
    assert!(!doc.has_changes_since_base());
}

#[test]
fn the_text_under_a_mark_is_gone_for_every_reader_and_the_rest_stays() {
    let original = helvetica_lines();
    let mut doc = open(original.clone());
    let page = first_page(&doc);
    mark(&mut doc, page, SECOND_LINE);
    let report = redact::apply(&mut doc, &RedactOptions::default()).unwrap();
    assert_eq!((report.pages, report.marks), (1, 1));
    assert_eq!(report.words_removed, 4, "Second line of text");
    assert!(report.words_kept >= 8, "the other lines stay searchable");
    assert!((report.lowest_dpi - 300.0).abs() < 1.0);
    assert!(redact::marks(&doc).is_empty(), "marks are used up");

    let saved = doc.snapshot_bytes().unwrap();
    // The removed words are nowhere in the file: not in the page, not in an old revision, not in a leftover
    // object (the original file kept them in the clear).
    assert!(file_contains(&original, "Second line of text"));
    for word in ["Second", "line of"] {
        assert!(
            !file_contains(&saved, word),
            "{word:?} is still in the file"
        );
    }
    assert!(file_contains(&saved, "%PDF-"));
    assert_eq!(
        saved.windows(5).filter(|w| w == b"%%EOF").count(),
        1,
        "one revision only"
    );

    // Another reader sees the other text and not the removed one.
    if let Some(t) = poppler_text(&saved, 1) {
        assert!(t.contains("Hello World"), "{t:?}");
        assert!(t.contains("Test kerning"), "{t:?}");
        assert!(!t.contains("Second"), "{t:?}");
        assert!(!t.contains("line of text"), "{t:?}");
    }
    // And so does our own extraction after reopening.
    let reopened = open(saved.clone());
    assert_eq!(reopened.page_count(), 1);
    assert!(redact::marks(&reopened).is_empty());
    let (_, found) = with_session(Arc::new(saved), |s| {
        let g = reopened.page_geometry(first_page(&reopened)).unwrap();
        let tp = s.extract_text(0, &g)?;
        Ok(((), tp.plain_text()))
    })
    .unwrap();
    assert!(found.contains("Hello"), "{found:?}");
    assert!(!found.contains("Second"), "{found:?}");
}

#[test]
fn the_page_looks_the_same_except_for_a_black_box() {
    let original = helvetica_lines();
    let mut doc = open(original.clone());
    let page = first_page(&doc);
    mark(&mut doc, page, SECOND_LINE);
    redact::apply(&mut doc, &RedactOptions::default()).unwrap();
    let saved = doc.snapshot_bytes().unwrap();
    let scale = 100.0 / 72.0;
    let (before, geom) = render(&original, 0, scale);
    let (after, _) = render(&saved, 0, scale);
    let b = pixel_box(&geom, SECOND_LINE, scale);
    assert!(mean_inside(&after, b) < 4.0, "the box is black");
    assert!(
        mean_inside(&before, b) > 40.0,
        "the original has text there"
    );
    let d = diff_outside(&before, &after, b, 3);
    assert!(
        d < 1.5,
        "the rest of the page changed (mean difference {d})"
    );
}

#[test]
fn rotated_cropped_and_scaled_pages_land_exactly_where_they_were() {
    // Page 1: /Rotate 90, offset CropBox, MediaBox with a negative origin. Page 2: /UserUnit 2.
    // Page 3: attributes inherited from the page tree (/Rotate 270).
    let original = odd_geometry();
    for (index, rect, word) in [
        (0usize, Rect::new(25.0, 95.0, 120.0, 118.0), "Rotated"),
        (1, Rect::new(25.0, 95.0, 130.0, 118.0), "UserUnit two"),
        (2, Rect::new(25.0, 95.0, 150.0, 118.0), "Inherited"),
    ] {
        let mut doc = open(original.clone());
        let page = doc.page_ids().unwrap()[index];
        mark(&mut doc, page, rect);
        redact::apply(&mut doc, &RedactOptions::default()).unwrap();
        let saved = doc.snapshot_bytes().unwrap();
        let scale = 2.0;
        let (before, geom) = render(&original, index, scale);
        let (after, geom_after) = render(&saved, index, scale);
        assert_eq!(geom, geom_after, "page {index}: the page geometry changed");
        let b = pixel_box(&geom, rect, scale);
        assert!(mean_inside(&after, b) < 6.0, "page {index}: no black box");
        let d = diff_outside(&before, &after, b, 4);
        assert!(d < 2.5, "page {index}: the page moved or changed ({d})");
        assert!(
            !file_contains(&saved, word),
            "page {index}: {word} is still in the file"
        );
        if let Some(t) = poppler_text(&saved, index + 1) {
            assert!(!t.contains(word), "page {index}: {t:?}");
        }
    }
}

#[test]
fn the_other_pages_are_not_touched() {
    let original = odd_geometry();
    let mut doc = open(original.clone());
    let p0 = doc.page_ids().unwrap()[0];
    mark(&mut doc, p0, Rect::new(25.0, 95.0, 120.0, 118.0));
    redact::apply(&mut doc, &RedactOptions::default()).unwrap();
    let saved = doc.snapshot_bytes().unwrap();
    assert_eq!(open(saved.clone()).page_count(), 3);
    for i in [1usize, 2] {
        let (a, _) = render(&original, i, 1.0);
        let (b, _) = render(&saved, i, 1.0);
        assert_eq!(a.rgba, b.rgba, "page {i} changed");
    }
    // Their text is still there as real text, not a picture.
    if let Some(t) = poppler_text(&saved, 2) {
        assert!(t.contains("UserUnit two"), "{t:?}");
    }
}

#[test]
fn pictures_under_a_mark_are_blacked_out_in_the_pixels() {
    let original = image_page();
    let mut doc = open(original.clone());
    let page = first_page(&doc);
    // The picture is drawn at 40..160 × 60..140 from the top of a 400-pt page: user space y 260..340.
    let rect = Rect::new(60.0, 280.0, 120.0, 320.0);
    mark(&mut doc, page, rect);
    redact::apply(&mut doc, &RedactOptions::default()).unwrap();
    let saved = doc.snapshot_bytes().unwrap();
    let scale = 2.0;
    let (before, geom) = render(&original, 0, scale);
    let (after, _) = render(&saved, 0, scale);
    let b = pixel_box(&geom, rect, scale);
    assert!(mean_inside(&before, b) > 60.0, "the picture is there");
    assert!(mean_inside(&after, b) < 3.0, "and now it is black");
    // The picture's own pixels are no longer in the file as a picture of 4×4.
    let doc2 = lopdf::Document::load_mem(&saved).unwrap();
    for o in doc2.objects.values() {
        if let Object::Stream(s) = o
            && s.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image")
        {
            let w = s.dict.get(b"Width").and_then(Object::as_i64).unwrap();
            assert!(w > 100, "the 4×4 original picture is still in the file");
        }
    }
    let d = diff_outside(&before, &after, b, 3);
    assert!(d < 2.0, "the rest of the page changed ({d})");
}

#[test]
fn comments_and_fields_under_a_mark_are_removed_and_the_others_stay() {
    let mut doc = open(helvetica_lines());
    let page = first_page(&doc);
    let note_under = |doc: &mut PdfDocument, pos: Point, text: &str| {
        let mut spec = AnnotationSpec::new(AnnotationKind::Note { pos });
        spec.contents = text.into();
        doc.transact(|tx| annot::add_annotation(tx, page, &spec))
            .unwrap()
            .0
    };
    let under = note_under(&mut doc, Point::new(100.0, 692.0), "secret comment");
    let away = note_under(&mut doc, Point::new(450.0, 300.0), "harmless comment");
    mark(&mut doc, page, SECOND_LINE);
    let report = redact::apply(&mut doc, &RedactOptions::default()).unwrap();
    assert_eq!(report.annotations_removed, 1);
    let saved = doc.snapshot_bytes().unwrap();
    assert!(!file_contains(&saved, "secret comment"));
    assert!(file_contains(&saved, "harmless comment"));
    let after = open(saved);
    let ids: Vec<_> = annot::annotation_ids(&after, first_page(&after));
    assert_eq!(ids.len(), 1);
    assert_ne!(ids[0], under);
    let _ = away;
}

#[test]
fn other_places_that_repeat_the_text_are_reported() {
    // A document whose title, a bookmark-like entry and a comment on another page repeat a removed word.
    let mut doc = open(helvetica_lines());
    let page = first_page(&doc);
    doc.transact(|tx| {
        let mut spec = AnnotationSpec::new(AnnotationKind::Note {
            pos: Point::new(450.0, 300.0),
        });
        spec.contents = "remember the Second line".into();
        annot::add_annotation(tx, page, &spec)?;
        let info = tx.add(Object::Dictionary(dictionary! {
            "Title" => Object::string_literal("About the Second line"),
        }));
        tx.trailer_mut().set("Info", Object::Reference(info));
        Ok(())
    })
    .unwrap();
    mark(&mut doc, page, SECOND_LINE);
    let report = redact::apply(&mut doc, &RedactOptions::default()).unwrap();
    let all = report.leaks.join("\n");
    assert!(all.contains("Document property Title"), "{all}");
    assert!(all.contains("Comment"), "{all}");
    assert!(all.to_lowercase().contains("second"), "{all}");
    // Removing the metadata is an option; the comment is the person's to deal with.
    let mut doc = open(helvetica_lines());
    let page = first_page(&doc);
    doc.transact(|tx| {
        let info = tx.add(Object::Dictionary(dictionary! {
            "Title" => Object::string_literal("About the Second line"),
            "Author" => Object::string_literal("Somebody"),
        }));
        tx.trailer_mut().set("Info", Object::Reference(info));
        Ok(())
    })
    .unwrap();
    mark(&mut doc, page, SECOND_LINE);
    let report = redact::apply(
        &mut doc,
        &RedactOptions {
            remove_metadata: true,
            ..RedactOptions::default()
        },
    )
    .unwrap();
    assert!(report.leaks.is_empty(), "{:?}", report.leaks);
    let saved = doc.snapshot_bytes().unwrap();
    assert!(!file_contains(&saved, "Somebody"));
    assert!(!file_contains(&saved, "About the Second"));
}

#[test]
fn jpeg_pictures_and_lower_resolutions_work_and_are_smaller() {
    let original = helvetica_lines();
    let sizes: Vec<usize> = [
        RedactOptions::default(),
        RedactOptions {
            dpi: 150.0,
            ..RedactOptions::default()
        },
        RedactOptions {
            image: ImageMode::Jpeg(70),
            ..RedactOptions::default()
        },
    ]
    .iter()
    .map(|opts| {
        let mut doc = open(original.clone());
        let page = first_page(&doc);
        mark(&mut doc, page, SECOND_LINE);
        let report = redact::apply(&mut doc, opts).unwrap();
        assert!((report.lowest_dpi - opts.dpi).abs() < 1.0);
        let saved = doc.snapshot_bytes().unwrap();
        if let Some(t) = poppler_text(&saved, 1) {
            assert!(t.contains("Hello World") && !t.contains("Second"));
        }
        let (bmp, _) = render(&saved, 0, 1.0);
        assert!(bmp.width > 100);
        saved.len()
    })
    .collect();
    assert!(
        sizes[1] < sizes[0],
        "150 dpi is smaller than 300 dpi: {sizes:?}"
    );
    assert!(sizes.iter().all(|s| *s < 2_000_000), "{sizes:?}");
}

#[test]
fn a_page_too_large_for_the_wanted_resolution_is_drawn_at_a_lower_one() {
    let mut doc = open(helvetica_lines());
    let page = first_page(&doc);
    // A0-sized page: 2384 × 3370 pt.
    doc.transact(|tx| {
        tx.dict_mut(page.0)?.set(
            "MediaBox",
            Object::Array(vec![0.into(), 0.into(), 2384.into(), 3370.into()]),
        );
        Ok(())
    })
    .unwrap();
    mark(&mut doc, page, SECOND_LINE);
    let report = redact::apply(&mut doc, &RedactOptions::default()).unwrap();
    assert!(report.lowest_dpi < 300.0, "{}", report.lowest_dpi);
    assert!(report.lowest_dpi > 80.0, "{}", report.lowest_dpi);
}

#[test]
fn a_redacted_page_survives_more_edits_and_can_be_saved_again() {
    let mut doc = open(helvetica_lines());
    let page = first_page(&doc);
    mark(&mut doc, page, SECOND_LINE);
    redact::apply(&mut doc, &RedactOptions::default()).unwrap();
    // More editing afterwards works, and saving twice in a row stays a clean file.
    doc.transact(|tx| pageops::rotate_pages(tx, &[page], 1))
        .unwrap();
    let first = doc.snapshot_bytes().unwrap();
    doc.rebase(first.clone());
    doc.transact(|tx| pageops::rotate_pages(tx, &[page], 1))
        .unwrap();
    let second = doc.snapshot_bytes().unwrap();
    for bytes in [&first, &second] {
        assert!(!file_contains(bytes, "Second"));
        assert_eq!(open(bytes.clone()).page_count(), 1);
    }
}

/// Text a viewer does not show but a text extractor finds: white text, invisible (render mode 3) text,
/// text inside a form XObject. Whatever lies under a mark must be gone for every reader.
fn hidden_text_page() -> Vec<u8> {
    use pdf_writer::types::TextRenderingMode;
    use pdf_writer::{Content, Finish, Name, Pdf, Rect as PRect, Ref, Str};
    let mut pdf = Pdf::new();
    let (catalog, tree, page, font, content, form) = (
        Ref::new(1),
        Ref::new(2),
        Ref::new(3),
        Ref::new(4),
        Ref::new(5),
        Ref::new(6),
    );
    pdf.catalog(catalog).pages(tree);
    pdf.pages(tree).kids([page]).count(1);
    {
        let mut p = pdf.page(page);
        p.media_box(PRect::new(0.0, 0.0, 400.0, 300.0))
            .parent(tree)
            .contents(content);
        let mut r = p.resources();
        r.fonts().pair(Name(b"F1"), font);
        r.x_objects().pair(Name(b"Fm1"), form);
        r.finish();
        p.finish();
    }
    pdf.type1_font(font)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    let mut f = Content::new();
    f.begin_text()
        .set_font(Name(b"F1"), 12.0)
        .next_line(50.0, 200.0)
        .show(Str(b"CHARLIE inside a form"))
        .end_text();
    let form_content = f.finish();
    {
        let mut xo = pdf.form_xobject(form, &form_content);
        xo.bbox(PRect::new(0.0, 0.0, 400.0, 300.0));
        xo.resources().fonts().pair(Name(b"F1"), font);
    }
    let mut c = Content::new();
    c.begin_text()
        .set_font(Name(b"F1"), 12.0)
        .next_line(50.0, 250.0)
        .show(Str(b"ALPHA visible secret"))
        .end_text();
    c.set_fill_gray(1.0);
    c.begin_text()
        .set_font(Name(b"F1"), 12.0)
        .next_line(50.0, 150.0)
        .show(Str(b"BRAVO white on white"))
        .end_text();
    c.set_fill_gray(0.0);
    c.x_object(Name(b"Fm1"));
    c.begin_text()
        .set_font(Name(b"F1"), 12.0)
        .set_text_rendering_mode(TextRenderingMode::Invisible)
        .next_line(50.0, 100.0)
        .show(Str(b"ECHO invisible mode"))
        .end_text();
    c.begin_text()
        .set_font(Name(b"F1"), 12.0)
        .set_text_rendering_mode(TextRenderingMode::Fill)
        .next_line(50.0, 30.0)
        .show(Str(b"FOXTROT elsewhere"))
        .end_text();
    pdf.stream(content, &c.finish());
    pdf.finish()
}

#[test]
fn hidden_white_invisible_and_form_text_under_a_mark_is_removed_too() {
    let original = hidden_text_page();
    for needle in ["ALPHA", "BRAVO", "CHARLIE", "ECHO", "FOXTROT"] {
        assert!(file_contains(&original, needle), "fixture sanity: {needle}");
    }
    if let Some(t) = poppler_text(&original, 1) {
        for needle in ["ALPHA", "BRAVO", "CHARLIE", "ECHO"] {
            assert!(
                t.contains(needle),
                "an extractor finds {needle} before: {t:?}"
            );
        }
    }
    let mut doc = open(original);
    let page = first_page(&doc);
    mark(&mut doc, page, Rect::new(40.0, 90.0, 300.0, 270.0));
    let report = redact::apply(&mut doc, &RedactOptions::default()).unwrap();
    assert!(report.words_removed >= 10, "{report:?}");
    let saved = doc.snapshot_bytes().unwrap();
    for needle in [
        "ALPHA",
        "BRAVO",
        "CHARLIE",
        "ECHO",
        "secret",
        "white on",
        "inside a form",
    ] {
        assert!(
            !file_contains(&saved, needle),
            "{needle} survived in the file"
        );
    }
    if let Some(t) = poppler_text(&saved, 1) {
        for needle in ["ALPHA", "BRAVO", "CHARLIE", "ECHO", "secret"] {
            assert!(
                !t.contains(needle),
                "{needle} survived for an extractor: {t:?}"
            );
        }
        assert!(t.contains("FOXTROT"), "text outside the marks stays: {t:?}");
    }
}

#[test]
fn pages_that_were_not_redacted_but_repeat_a_removed_word_are_reported() {
    // Two pages that show the same line; only page 1 is redacted.
    let mut doc = open(test_support::fixtures::many_pages(3));
    let p = doc.page_ids().unwrap();
    // many_pages: every page has its own number as text; mark the first page's text area.
    let (tp, _) = {
        let bytes = doc.snapshot_bytes().unwrap();
        let g = doc.page_geometry(p[0]).unwrap();
        with_session(Arc::new(bytes), |s| Ok((s.extract_text(0, &g)?, ()))).unwrap()
    };
    let first_word = tp
        .plain_text()
        .split_whitespace()
        .find(|w| w.chars().count() >= 3)
        .unwrap()
        .to_string();
    let mut rect = tp.glyphs[0].quad.bounds();
    for g in tp.glyphs.iter().filter(|g| !g.synthetic) {
        rect = rect.union(g.quad.bounds());
    }
    mark(&mut doc, p[0], rect.inflate(2.0, 2.0));
    let report = redact::apply(&mut doc, &RedactOptions::default()).unwrap();
    assert_eq!(report.pages, 1);
    let all = report.leaks.join("\n").to_lowercase();
    assert!(
        all.contains("page 2") && all.contains("page 3"),
        "the word {first_word:?} also shows on the other pages: {all}"
    );
}
