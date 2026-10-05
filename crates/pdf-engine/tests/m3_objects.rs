//! Milestone 3: images and added content, checked with an independent renderer.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::doc::{OpenOptions, PageId, PdfDocument};
use pdf_engine::error::EngineError;
use pdf_engine::geom::{Point, Rect};
use pdf_engine::pagecontent::{self, PageContent, TextEdit};
use pdf_engine::render::with_session;
use std::sync::Arc;
use test_support::fixtures::*;
use test_support::*;

fn open(bytes: Vec<u8>) -> PdfDocument {
    PdfDocument::open(bytes, &OpenOptions::default()).unwrap()
}

fn solid_png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().unwrap();
    let px: Vec<u8> = (0..w * h).flat_map(|_| rgba).collect();
    wr.write_image_data(&px).unwrap();
    drop(wr);
    out
}

/// Pixel (RGB) at PDF user-space point on page 1 (400×400 page, no rotation) via hayro.
fn hayro_pixel(bytes: &[u8], doc: &PdfDocument, page_index: usize, x: f64, y: f64) -> [u8; 3] {
    let g = doc.pages().unwrap()[page_index].geometry;
    let bmp = with_session(Arc::new(bytes.to_vec()), |s| {
        s.render_page(page_index, g, pdf_engine::geom::Rotation::R0, 1.0)
    })
    .unwrap();
    let h = bmp.height as f64;
    let (px, py) = (x as u32, (h - y) as u32);
    let i = ((py * bmp.width + px) * 4) as usize;
    [bmp.rgba[i], bmp.rgba[i + 1], bmp.rgba[i + 2]]
}

fn first_page(doc: &PdfDocument) -> PageId {
    doc.pages().unwrap()[0].id
}

#[test]
fn image_is_located_through_a_flipped_ctm() {
    let doc = open(image_page());
    let pc = PageContent::load(doc.lopdf(), first_page(&doc).0).unwrap();
    let imgs = pc.images(doc.lopdf());
    assert_eq!(imgs.len(), 1);
    let b = imgs[0].quad.bounds();
    // Drawn at x 40..160, and (flipped y) 400-60-80 .. 400-60 = 260..340 in user space.
    assert!(
        (b.x0 - 40.0).abs() < 1e-6 && (b.x1 - 160.0).abs() < 1e-6,
        "{b:?}"
    );
    assert!(
        (b.y0 - 260.0).abs() < 1e-6 && (b.y1 - 340.0).abs() < 1e-6,
        "{b:?}"
    );
    assert_eq!((imgs[0].width_px, imgs[0].height_px), (4, 4));
}

#[test]
fn move_resize_delete_replace_image() {
    let mut doc = open(image_page());
    let page = first_page(&doc);
    let img = PageContent::load(doc.lopdf(), page.0)
        .unwrap()
        .images(doc.lopdf())
        .remove(0);

    // Move + resize to a new box (user space).
    let new_box = Rect::new(200.0, 100.0, 300.0, 150.0);
    doc.transact(|tx| PageContent::load(tx.doc(), page.0)?.place_image(tx, img.id, new_box))
        .unwrap();
    let bytes = doc.snapshot_bytes().unwrap();
    let re = open(bytes.clone());
    let moved = PageContent::load(re.lopdf(), first_page(&re).0)
        .unwrap()
        .images(re.lopdf())
        .remove(0);
    let mb = moved.quad.bounds();
    assert!(
        (mb.x0 - 200.0).abs() < 1e-3
            && (mb.y0 - 100.0).abs() < 1e-3
            && (mb.x1 - 300.0).abs() < 1e-3
            && (mb.y1 - 150.0).abs() < 1e-3,
        "{mb:?}"
    );
    // Pixels moved: new location has image colour, old location is white.
    let inside = hayro_pixel(&bytes, &re, 0, 250.0, 125.0);
    let old = hayro_pixel(&bytes, &re, 0, 100.0, 300.0);
    assert_ne!(inside, [255, 255, 255], "image must be at its new place");
    assert_eq!(old, [255, 255, 255], "old place must be empty");
    // The caption text is untouched and still extractable.
    assert!(all_text_hayro(&bytes).contains("Caption under image"));

    // Replace with a solid red PNG; box preserved (stretch).
    let page = first_page(&doc);
    let img2 = PageContent::load(doc.lopdf(), page.0)
        .unwrap()
        .images(doc.lopdf())
        .remove(0);
    let red = solid_png(8, 8, [255, 0, 0, 255]);
    doc.transact(|tx| PageContent::load(tx.doc(), page.0)?.replace_image(tx, img2.id, &red, true))
        .unwrap();
    let bytes = doc.snapshot_bytes().unwrap();
    let re = open(bytes.clone());
    let px = hayro_pixel(&bytes, &re, 0, 250.0, 125.0);
    assert!(
        px[0] > 200 && px[1] < 60 && px[2] < 60,
        "replaced image must be red, got {px:?}"
    );
    if let Some(rgb) = poppler_render(&bytes, 1, 72) {
        let (w, h, buf) = rgb;
        let i = (((h as f64 - 125.0) as u32 * w + 250) * 3) as usize;
        assert!(
            buf[i] > 200 && buf[i + 1] < 60,
            "poppler agrees the image is red"
        );
    }

    // Delete it.
    let page = first_page(&doc);
    let img3 = PageContent::load(doc.lopdf(), page.0)
        .unwrap()
        .images(doc.lopdf())
        .remove(0);
    doc.transact(|tx| PageContent::load(tx.doc(), page.0)?.delete_image(tx, img3.id))
        .unwrap();
    let bytes = doc.snapshot_bytes().unwrap();
    let re = open(bytes.clone());
    assert!(
        PageContent::load(re.lopdf(), first_page(&re).0)
            .unwrap()
            .images(re.lopdf())
            .is_empty()
    );
    assert_eq!(hayro_pixel(&bytes, &re, 0, 250.0, 125.0), [255, 255, 255]);
    assert!(
        all_text_hayro(&bytes).contains("Caption under image"),
        "text survives image deletion"
    );
}

fn all_text_hayro(bytes: &[u8]) -> String {
    let d = open(bytes.to_vec());
    with_session(Arc::new(bytes.to_vec()), |s| {
        let mut out = String::new();
        for (i, p) in d.pages().unwrap().iter().enumerate() {
            out.push_str(&s.extract_text(i, &p.geometry)?.plain_text());
            out.push('\n');
        }
        Ok(out)
    })
    .unwrap()
}

#[test]
fn added_text_is_real_extractable_page_text_and_stays_editable() {
    let mut doc = open(helvetica_lines());
    let page = first_page(&doc);
    doc.transact(|tx| {
        pagecontent::add_text(
            tx,
            page,
            Point::new(72.0, 400.0),
            "Added café – Ünïcode line\nsecond added line",
            16.0,
            (0.0, 0.2, 0.7),
            false,
        )
    })
    .unwrap();
    let bytes = doc.snapshot_bytes().unwrap();
    // Extractable by two independent engines.
    assert!(all_text_hayro(&bytes).contains("Added café – Ünïcode line"));
    if let Some(t) = poppler_text(&bytes) {
        assert!(
            t.contains("Added café") && t.contains("second added line"),
            "{t}"
        );
    }
    // Listed as editable runs; the original text is intact.
    let re = open(bytes.clone());
    let pc = PageContent::load(re.lopdf(), first_page(&re).0).unwrap();
    let texts: Vec<String> = pc.text_runs().into_iter().map(|r| r.text).collect();
    assert!(texts.contains(&"Hello World".to_string()), "{texts:?}");
    let added = pc
        .text_runs()
        .into_iter()
        .find(|r| r.text.starts_with("Added"))
        .unwrap();
    assert!(added.editable.is_ok() && added.embedded);
    assert!((added.size_pt - 16.0).abs() < 0.01);
    // The placement is where we asked (baseline x = 72).
    assert!(
        (added.quad.bounds().x0 - 72.0).abs() < 0.5,
        "{:?}",
        added.quad.bounds()
    );
    // Characters the bundled font lacks are refused, not dropped.
    let mut d2 = open(helvetica_lines());
    let r = d2.transact(|tx| {
        pagecontent::add_text(
            tx,
            page,
            Point::new(10.0, 10.0),
            "漢字",
            12.0,
            (0.0, 0.0, 0.0),
            false,
        )
    });
    assert!(
        matches!(r, Err(EngineError::MissingGlyphs { .. })),
        "{:?}",
        r.err()
    );
    assert!(!d2.has_changes_since_base());
}

#[test]
fn added_content_respects_rotation_cropping_and_a_flipped_ctm() {
    // Page 1: Rotate 90, offset CropBox, negative-origin MediaBox.
    let mut doc = open(odd_geometry());
    let page = doc.pages().unwrap()[0].id;
    let target = Point::new(100.0, 200.0); // inside the crop box (20..320, 30..330)
    doc.transact(|tx| {
        pagecontent::add_text(tx, page, target, "Marker", 14.0, (0.0, 0.0, 0.0), false)
    })
    .unwrap();
    let bytes = doc.snapshot_bytes().unwrap();
    let re = open(bytes.clone());
    let geom = re.pages().unwrap()[0].geometry;
    let found = with_session(Arc::new(bytes), |s| {
        let tp = s.extract_text(0, &geom)?;
        let hit = tp.search("Marker", false);
        assert_eq!(hit.len(), 1);
        Ok(hit[0].quads[0].bounds())
    })
    .unwrap();
    assert!(
        (found.x0 - 100.0).abs() < 1.0 && found.y0 < 205.0 && found.y1 > 200.0,
        "marker at {found:?}, wanted near (100, 200)"
    );
    // Image on a page whose content ends under a flipped CTM (Chromium fixture).
    let mut doc = open(fixture_bytes("report-chromium.pdf"));
    let page = doc.pages().unwrap()[0].id;
    doc.transact(|tx| {
        pagecontent::add_image(
            tx,
            page,
            Rect::new(300.0, 500.0, 400.0, 560.0),
            &solid_png(4, 4, [0, 160, 0, 255]),
        )
    })
    .unwrap();
    let bytes = doc.snapshot_bytes().unwrap();
    let re = open(bytes.clone());
    let px = {
        let g = re.pages().unwrap()[0].geometry;
        let bmp = with_session(Arc::new(bytes.clone()), |s| {
            s.render_page(0, g, pdf_engine::geom::Rotation::R0, 1.0)
        })
        .unwrap();
        let (x, y) = (350u32, (bmp.height as f64 - 530.0) as u32);
        let i = ((y * bmp.width + x) * 4) as usize;
        [bmp.rgba[i], bmp.rgba[i + 1], bmp.rgba[i + 2]]
    };
    assert!(
        px[1] > 120 && px[0] < 60,
        "green image must appear at the requested place despite the flipped CTM: {px:?}"
    );
    // The original page text is unaffected.
    assert!(all_text_hayro(&bytes).contains("Quarterly Report"));
}

#[test]
fn adding_resources_never_leaks_into_pages_that_share_them() {
    let mut doc = open(shared_content_stream());
    let pages = doc.pages().unwrap();
    doc.transact(|tx| {
        pagecontent::add_text(
            tx,
            pages[0].id,
            Point::new(20.0, 20.0),
            "Only on page one",
            12.0,
            (0.0, 0.0, 0.0),
            false,
        )
    })
    .unwrap();
    let bytes = doc.snapshot_bytes().unwrap();
    let t = all_text_hayro(&bytes);
    assert_eq!(t.matches("Only on page one").count(), 1, "{t}");
    let re = open(bytes);
    let rp = re.pages().unwrap();
    let p2 = PageContent::load(re.lopdf(), rp[1].id.0).unwrap();
    assert_eq!(p2.text_runs().len(), 1);
}

#[test]
fn text_added_with_the_bundled_font_can_be_edited_within_its_subset() {
    let mut doc = open(helvetica_lines());
    let page = first_page(&doc);
    doc.transact(|tx| {
        pagecontent::add_text(
            tx,
            page,
            Point::new(72.0, 300.0),
            "Draft note",
            12.0,
            (0.0, 0.0, 0.0),
            false,
        )
    })
    .unwrap();
    let run = PageContent::load(doc.lopdf(), page.0)
        .unwrap()
        .text_runs()
        .into_iter()
        .find(|r| r.text == "Draft note")
        .unwrap();
    // Letters already in the embedded subset can be recombined.
    let (_, _) = doc
        .transact(|tx| {
            PageContent::load(tx.doc(), page.0)?.edit_text(
                tx,
                run.id,
                &TextEdit {
                    text: Some("Dart not'".replace("'", "e")),
                    ..Default::default()
                },
            )
        })
        .unwrap();
    let bytes = doc.snapshot_bytes().unwrap();
    assert!(all_text_hayro(&bytes).contains("Dart note"));
    assert!(!all_text_hayro(&bytes).contains("Draft note"));
    // A letter that is not in the subset is refused (explicit substitution is the way out).
    let run = PageContent::load(doc.lopdf(), page.0)
        .unwrap()
        .text_runs()
        .into_iter()
        .find(|r| r.text == "Dart note")
        .unwrap();
    let r = doc.transact(|tx| {
        PageContent::load(tx.doc(), page.0)?.edit_text(
            tx,
            run.id,
            &TextEdit {
                text: Some("Dart Zone".into()),
                ..Default::default()
            },
        )
    });
    match r {
        Err(EngineError::MissingGlyphs { chars, .. }) => assert!(chars.contains('Z'), "{chars}"),
        other => panic!("expected MissingGlyphs, got {:?}", other.err()),
    }
}
