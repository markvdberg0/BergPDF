//! Headers, footers, Bates numbers, watermarks, page labels and cropping: the text lands on the
//! page as shown (also on rotated pages), survives saving, is extractable, and comes off again.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::doc::{OpenOptions, PageId, PdfDocument};
use pdf_engine::fontembed::FontStyle;
use pdf_engine::geom::{Rect, Rotation};
use pdf_engine::pagecontent::content_stream_ids;
use pdf_engine::pagelabels::{self, LabelRange, LabelStyle};
use pdf_engine::pagemarks::{
    self, Bates, HeaderFooter, MarkContext, MarkKind, Watermark, WatermarkContent,
};
use pdf_engine::pageops;
use pdf_engine::render::with_session;
use std::sync::Arc;
use test_support::fixtures;

fn open(bytes: Vec<u8>) -> PdfDocument {
    PdfDocument::open(bytes, &OpenOptions::default()).unwrap()
}

/// Text of every page, with the bounds of the glyphs of `needle` in the page as shown (y down).
fn find_shown(bytes: &[u8], page: usize, needle: &str) -> Option<(String, Rect)> {
    let doc = open(bytes.to_vec());
    let geometry = doc.page_geometry(doc.page_ids().unwrap()[page]).unwrap();
    let tp = with_session(Arc::new(bytes.to_vec()), |s| {
        s.extract_text(page, &geometry)
    })
    .unwrap();
    let text = tp.plain_text();
    let hit = tp.search(needle, true).into_iter().next()?;
    let to_view = geometry.pdf_to_view(Rotation::R0);
    let mut r: Option<Rect> = None;
    for q in hit.quads {
        let b = q.transform(to_view).bounds();
        r = Some(r.map_or(b, |x| x.union(b)));
    }
    Some((text, r?))
}

fn three_pages_one_turned() -> PdfDocument {
    let mut doc = open(fixtures::many_pages(3));
    let p = doc.page_ids().unwrap()[1];
    doc.transact(|tx| pageops::rotate_pages(tx, &[p], 1))
        .unwrap();
    doc
}

#[test]
fn header_footer_and_bates_land_on_the_shown_page_and_survive_saving() {
    let mut doc = three_pages_one_turned();
    let pages = doc.page_ids().unwrap();
    let spec = HeaderFooter {
        slots: [
            "{file}".into(),
            String::new(),
            "{date}".into(),
            String::new(),
            "Folio {page}/{pages}".into(),
            "{bates}".into(),
        ],
        font: FontStyle::default(),
        bates: Some(Bates {
            prefix: "ACME-".into(),
            start: 41,
            digits: 5,
            ..Bates::default()
        }),
        ..HeaderFooter::default()
    };
    let ctx = MarkContext {
        date: "2026-11-03".into(),
        file: "contract.pdf".into(),
        labels: Vec::new(),
    };
    let n = doc
        .transact(|tx| pagemarks::add_header_footer(tx, &pages, &spec, &ctx))
        .unwrap()
        .0;
    assert_eq!(n, 3);
    let bytes = doc.snapshot_bytes().unwrap();
    for (i, want_bates) in ["ACME-00041", "ACME-00042", "ACME-00043"]
        .iter()
        .enumerate()
    {
        let geometry = open(bytes.clone()).page_geometry(pages[i]).unwrap();
        let shown = geometry.view_size(Rotation::R0);
        let (text, footer) = find_shown(&bytes, i, &format!("Folio {}/3", i + 1))
            .unwrap_or_else(|| panic!("no page number on page {i}"));
        assert!(text.contains(want_bates), "page {i}: {text:?}");
        assert!(text.contains("contract.pdf") && text.contains("2026-11-03"));
        // The footer is at the bottom of the page as shown, centred, and reads horizontally.
        assert!(
            footer.y0 > shown.height - 60.0,
            "page {i} footer at {footer:?} of {shown:?}"
        );
        let centre = (footer.x0 + footer.x1) / 2.0;
        assert!(
            (centre - shown.width / 2.0).abs() < 3.0,
            "page {i}: {centre} vs {}",
            shown.width
        );
        assert!(footer.width() > footer.height());
        let (_, header) = find_shown(&bytes, i, "contract.pdf").unwrap();
        assert!(
            header.y1 < 60.0 && header.x0 < 60.0,
            "page {i} header at {header:?}"
        );
        let (_, bates) = find_shown(&bytes, i, want_bates).unwrap();
        assert!(bates.x1 > shown.width - 60.0, "page {i} Bates at {bates:?}");
    }
    // A second reader sees the same (when poppler is installed).
    if let Some(t) = test_support::poppler_text(&bytes) {
        assert!(t.contains("ACME-00043") && t.contains("Folio 2/3"), "{t}");
    }
}

#[test]
fn marks_come_off_again_and_leave_the_page_as_it_was() {
    let mut doc = three_pages_one_turned();
    let pages = doc.page_ids().unwrap();
    let before: Vec<Vec<_>> = pages
        .iter()
        .map(|p| content_stream_ids(doc.lopdf(), p.0))
        .collect();
    let hf = HeaderFooter {
        slots: [
            String::new(),
            "HEADER".into(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ],
        ..HeaderFooter::default()
    };
    let wm = Watermark {
        content: WatermarkContent::Text {
            text: "CONFIDENTIAL".into(),
            font: FontStyle::default(),
            size: 0.0,
            color: (0.8, 0.1, 0.1),
        },
        opacity: 0.3,
        rotation: 45.0,
        behind: true,
    };
    doc.transact(|tx| {
        pagemarks::add_header_footer(tx, &pages, &hf, &MarkContext::default())?;
        pagemarks::add_watermark(tx, &pages, &wm)
    })
    .unwrap();
    assert!(pagemarks::has_marks(doc.lopdf(), MarkKind::HeaderFooter));
    assert!(pagemarks::has_marks(doc.lopdf(), MarkKind::Watermark));
    let bytes = doc.snapshot_bytes().unwrap();
    let (text, _) = find_shown(&bytes, 0, "CONFIDENTIAL").expect("watermark text");
    assert!(text.contains("HEADER"));

    // Removing the header keeps the watermark, and the other way round; both gone = original streams.
    let n = doc
        .transact(|tx| pagemarks::remove_marks(tx, MarkKind::HeaderFooter))
        .unwrap()
        .0;
    assert_eq!(n, 3);
    let bytes = doc.snapshot_bytes().unwrap();
    let (text, _) = find_shown(&bytes, 2, "CONFIDENTIAL").expect("watermark stays");
    assert!(!text.contains("HEADER"));
    doc.transact(|tx| pagemarks::remove_marks(tx, MarkKind::Watermark))
        .unwrap();
    let after: Vec<Vec<_>> = pages
        .iter()
        .map(|p| content_stream_ids(doc.lopdf(), p.0))
        .collect();
    assert_eq!(before, after);
    assert!(!pagemarks::has_marks(doc.lopdf(), MarkKind::Watermark));
}

#[test]
fn a_watermark_on_top_and_a_picture_watermark() {
    let mut doc = open(fixtures::many_pages(2));
    let pages = doc.page_ids().unwrap();
    let mut png = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png, 4, 2);
        enc.set_color(png::ColorType::Rgb);
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&[200u8; 24]).unwrap();
    }
    let text = Watermark {
        content: WatermarkContent::Text {
            text: "DRAFT\nnot for use".into(),
            font: FontStyle::default(),
            size: 40.0,
            color: (0.2, 0.2, 0.9),
        },
        opacity: 0.5,
        rotation: 0.0,
        behind: false,
    };
    let picture = Watermark {
        content: WatermarkContent::Image {
            data: png,
            width: 0.5,
        },
        opacity: 0.2,
        rotation: 30.0,
        behind: true,
    };
    doc.transact(|tx| {
        pagemarks::add_watermark(tx, &pages[..1], &text)?;
        pagemarks::add_watermark(tx, &pages, &picture)
    })
    .unwrap();
    let bytes = doc.snapshot_bytes().unwrap();
    let geometry = open(bytes.clone()).page_geometry(pages[0]).unwrap();
    let shown = geometry.view_size(Rotation::R0);
    let (_, r) = find_shown(&bytes, 0, "DRAFT").unwrap();
    let c = ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
    assert!((c.0 - shown.width / 2.0).abs() < 4.0, "{c:?}");
    assert!(
        c.1 < shown.height / 2.0,
        "the first line is above the centre: {c:?}"
    );
    assert!(find_shown(&bytes, 1, "DRAFT").is_none());
    // Saved and re-opened, still well-formed.
    let re = open(bytes);
    assert_eq!(re.page_count(), 2);
}

#[test]
fn missing_characters_and_empty_specs_are_refused() {
    let mut doc = open(fixtures::many_pages(1));
    let pages = doc.page_ids().unwrap();
    let empty = HeaderFooter::default();
    assert!(
        doc.transact(|tx| pagemarks::add_header_footer(
            tx,
            &pages,
            &empty,
            &MarkContext::default()
        ))
        .is_err()
    );
    let wm = Watermark {
        content: WatermarkContent::Text {
            text: "   ".into(),
            font: FontStyle::default(),
            size: 0.0,
            color: (0.0, 0.0, 0.0),
        },
        opacity: 0.5,
        rotation: 0.0,
        behind: false,
    };
    assert!(
        doc.transact(|tx| pagemarks::add_watermark(tx, &pages, &wm))
            .is_err()
    );
}

#[test]
fn page_labels_round_trip_and_crop_boxes_change_the_shown_size() {
    let mut doc = open(fixtures::many_pages(6));
    let ranges = vec![
        LabelRange {
            start: 0,
            style: LabelStyle::RomanLower,
            prefix: String::new(),
            first: 1,
        },
        LabelRange {
            start: 2,
            style: LabelStyle::Decimal,
            prefix: "A-".into(),
            first: 1,
        },
    ];
    doc.transact(|tx| pagelabels::set_page_labels(tx, &ranges, 6))
        .unwrap();
    let re = open(doc.snapshot_bytes().unwrap());
    let read = pagelabels::read_page_labels(re.lopdf());
    assert_eq!(read, ranges);
    assert_eq!(
        pagelabels::all_labels(&read, 6),
        vec!["i", "ii", "A-1", "A-2", "A-3", "A-4"]
    );
    // A list that does not start at the first page gets 1, 2, … in front.
    doc.transact(|tx| pagelabels::set_page_labels(tx, &ranges[1..], 6))
        .unwrap();
    let read = pagelabels::read_page_labels(doc.lopdf());
    assert_eq!(read.len(), 2);
    assert_eq!(read[0], LabelRange::decimal(0));
    assert!(
        doc.transact(|tx| pagelabels::set_page_labels(tx, &[LabelRange::decimal(9)], 6))
            .is_err()
    );
    doc.transact(|tx| pagelabels::set_page_labels(tx, &[], 6))
        .unwrap();
    assert!(pagelabels::read_page_labels(doc.lopdf()).is_empty());

    // Cropping.
    let p: PageId = doc.page_ids().unwrap()[0];
    let full = doc.page_geometry(p).unwrap();
    let m = full.media_box;
    let crop = Rect::new(m.x0 + 20.0, m.y0 + 30.0, m.x1 - 20.0, m.y1 - 30.0);
    doc.transact(|tx| pageops::set_crop_box(tx, p, Some(crop)))
        .unwrap();
    let g = open(doc.snapshot_bytes().unwrap())
        .page_geometry(p)
        .unwrap();
    assert!((g.crop_box.width() - (m.width() - 40.0)).abs() < 1e-6);
    doc.transact(|tx| pageops::set_crop_box(tx, p, None))
        .unwrap();
    assert_eq!(doc.page_geometry(p).unwrap().crop_box, m);
}
