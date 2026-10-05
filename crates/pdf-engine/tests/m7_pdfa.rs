//! PDF/A-2b conversion, judged by veraPDF (an independent validator) where it is installed
//! (`BERG_VERAPDF=<dir with the jars>`; `BERG_REQUIRE_VERAPDF=1` makes its absence a failure).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::pdfa::{analyze, convert_bytes};
use test_support::*;

fn vera_or_skip(bytes: &[u8], what: &str) {
    match verapdf(bytes, "2b") {
        Some(r) => assert!(
            r.compliant,
            "{what}: veraPDF rejects the converted file:\n{}",
            r.failures.join("\n")
        ),
        None => assert!(
            !verapdf_required(),
            "veraPDF required but not available (BERG_VERAPDF)"
        ),
    }
}

#[test]
fn originals_are_not_pdfa_so_the_validator_is_meaningful() {
    // Guard against a validator that says yes to everything.
    let input = fixture_bytes("report-chromium.pdf");
    if let Some(r) = verapdf(&input, "2b") {
        assert!(!r.compliant);
        assert!(
            r.failures.iter().any(|f| f.starts_with("6.6.2.1")),
            "{:?}",
            r.failures
        );
    } else {
        assert!(!verapdf_required());
    }
}

#[test]
fn chromium_and_cairo_reports_convert_to_validated_pdfa() {
    for name in ["report-chromium.pdf", "report-cairo.pdf"] {
        let input = fixture_bytes(name);
        let (out, rep) = convert_bytes(&input).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(rep.fixes.iter().any(|f| f.contains("XMP")), "{rep:?}");
        assert!(rep.blockers.is_empty());
        vera_or_skip(&out, name);
        // Pages still read and look the same.
        if let (Some(a), Some(b)) = (poppler_text(&input), poppler_text(&out)) {
            assert_eq!(a, b, "{name}: text changed");
            let ra = poppler_render(&input, 1, 80).unwrap();
            let rb = poppler_render(&out, 1, 80).unwrap();
            assert!(mean_abs_diff(&ra.2, &rb.2) < 1.0, "{name}: render changed");
        }
        if let Some(d) = poppler_diagnostics(&out) {
            assert!(d.trim().is_empty(), "{name}: poppler complains: {d}");
        }
    }
}

#[test]
fn documents_with_non_embedded_fonts_are_refused_with_the_font_names() {
    let input = fixtures::helvetica_lines();
    let rep = analyze(&input).unwrap();
    assert!(!rep.can_convert());
    assert!(rep.blockers[0].contains("Helvetica"), "{:?}", rep.blockers);
    let e = convert_bytes(&input).unwrap_err().to_string();
    assert!(e.contains("Helvetica"), "{e}");
}

#[test]
fn damaged_input_is_an_error() {
    assert!(convert_bytes(b"%PDF-1.4\ngarbage").is_err());
    assert!(analyze(b"nope").is_err());
}

// ---- harder inputs ------------------------------------------------------------------------

use image::{ImageEncoder, codecs::jpeg::JpegEncoder};
use lopdf::{Document, Object, dictionary};
use pdf_engine::annot::{self, AnnotationKind, AnnotationSpec, Rgb};
use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::geom::{Point, Rect};
use pdf_engine::pagecontent::add_text;

fn png_rgb(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().unwrap();
    let px: Vec<u8> = (0..w * h)
        .flat_map(|i| [(i % 251) as u8, (i % 199) as u8, (i % 97) as u8])
        .collect();
    wr.write_image_data(&px).unwrap();
    drop(wr);
    out
}

fn jpeg_gray(w: u32, h: u32) -> Vec<u8> {
    let px: Vec<u8> = (0..w * h).map(|i| (i % 256) as u8).collect();
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 90)
        .write_image(&px, w, h, image::ExtendedColorType::L8)
        .unwrap();
    out
}

fn bytes_of(mut doc: PdfDocument) -> Vec<u8> {
    doc.snapshot_bytes().unwrap()
}

#[test]
fn picture_documents_rgb_and_gray_convert_to_validated_pdfa() {
    for (name, data) in [
        ("png-rgb", png_rgb(64, 48)),
        ("jpeg-gray", jpeg_gray(64, 48)),
    ] {
        let input = bytes_of(PdfDocument::from_image(&data).unwrap());
        let (out, rep) = convert_bytes(&input).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(rep.blockers.is_empty());
        vera_or_skip(&out, name);
        if let (Some(a), Some(b)) = (poppler_render(&input, 1, 40), poppler_render(&out, 1, 40)) {
            assert!(mean_abs_diff(&a.2, &b.2) < 1.0, "{name}: render changed");
        }
    }
}

#[test]
fn transparency_is_allowed_in_pdfa2() {
    let input = fixtures::reused_form_and_transparency();
    let (out, _) = convert_bytes(&input).unwrap();
    vera_or_skip(&out, "transparency");
}

#[test]
fn editor_made_annotations_survive_conversion_and_validate() {
    let mut doc = PdfDocument::open(
        fixture_bytes("report-chromium.pdf"),
        &OpenOptions::default(),
    )
    .unwrap();
    let page = doc.page_ids().unwrap()[0];
    doc.transact(|tx| {
        let mut hl = AnnotationSpec::new(AnnotationKind::Highlight {
            quads: vec![pdf_engine::geom::Quad::from_rect(Rect::new(
                72.0, 700.0, 300.0, 716.0,
            ))],
        });
        hl.contents = "note".into();
        annot::add_annotation(tx, page, &hl)?;
        let mut sq = AnnotationSpec::new(AnnotationKind::Rectangle {
            rect: Rect::new(100.0, 400.0, 300.0, 500.0),
        });
        sq.color = Rgb(0.8, 0.1, 0.1);
        annot::add_annotation(tx, page, &sq)?;
        let note = AnnotationSpec::new(AnnotationKind::Note {
            pos: Point::new(400.0, 600.0),
        });
        annot::add_annotation(tx, page, &note)?;
        Ok(())
    })
    .unwrap();
    let input = bytes_of(doc);
    let (out, rep) = convert_bytes(&input).unwrap_or_else(|e| panic!("{e}"));
    assert!(rep.blockers.is_empty());
    vera_or_skip(&out, "annotations");
    let back = PdfDocument::open(out, &OpenOptions::default()).unwrap();
    let list = annot::read_annotations(&back, back.page_ids().unwrap()[0]);
    assert!(list.len() >= 3, "annotations kept: {}", list.len());
}

#[test]
fn text_added_by_the_editor_with_embedded_fonts_validates() {
    let mut doc =
        PdfDocument::open(fixture_bytes("report-cairo.pdf"), &OpenOptions::default()).unwrap();
    let page = doc.page_ids().unwrap()[0];
    doc.transact(|tx| {
        add_text(
            tx,
            page,
            Point::new(72.0, 500.0),
            "Added text: café, ünïcode € ≠",
            14.0,
            (0.1, 0.2, 0.6),
            false,
        )?;
        add_text(
            tx,
            page,
            Point::new(72.0, 470.0),
            "Bold line",
            14.0,
            (0.0, 0.0, 0.0),
            true,
        )
    })
    .unwrap();
    let input = bytes_of(doc);
    let (out, _) = convert_bytes(&input).unwrap_or_else(|e| panic!("{e}"));
    vera_or_skip(&out, "editor text");
    if let Some(t) = poppler_text(&out) {
        assert!(t.contains("café"), "{t}");
    }
}

#[test]
fn scripts_launch_actions_and_attachments_are_removed() {
    // Start from a real document and bolt on everything PDF/A forbids.
    let mut d = Document::load_mem(&fixture_bytes("report-chromium.pdf")).unwrap();
    let cat_id = d.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let js = d.add_object(
        dictionary! { "S" => "JavaScript", "JS" => Object::string_literal("app.alert(1)") },
    );
    let ef_stream = d.add_object(lopdf::Stream::new(
        dictionary! { "Type" => "EmbeddedFile" },
        b"secret".to_vec(),
    ));
    let spec = d.add_object(dictionary! {
        "Type" => "Filespec", "F" => Object::string_literal("a.txt"),
        "EF" => dictionary! { "F" => Object::Reference(ef_stream) },
    });
    let launch =
        d.add_object(dictionary! { "S" => "Launch", "F" => Object::string_literal("calc.exe") });
    let link = d.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link", "Rect" => vec![10.into(), 10.into(), 50.into(), 30.into()],
        "A" => Object::Reference(launch), "F" => 2i64,
    });
    let page_id = *d.get_pages().values().next().unwrap();
    {
        let Object::Dictionary(p) = d.objects.get_mut(&page_id).unwrap() else {
            panic!()
        };
        p.set("Annots", vec![Object::Reference(link)]);
        p.set("AA", dictionary! { "O" => Object::Reference(js) });
    }
    {
        let Object::Dictionary(c) = d.objects.get_mut(&cat_id).unwrap() else {
            panic!()
        };
        c.set("OpenAction", Object::Reference(js));
        c.set("Names", dictionary! {
            "JavaScript" => dictionary! { "Names" => vec![Object::string_literal("x"), Object::Reference(js)] },
            "EmbeddedFiles" => dictionary! { "Names" => vec![Object::string_literal("a.txt"), Object::Reference(spec)] },
        });
        c.set("AF", vec![Object::Reference(spec)]);
    }
    let mut input = Vec::new();
    d.save_to(&mut input).unwrap();
    let rep = analyze(&input).unwrap();
    assert!(rep.removed.iter().any(|r| r.contains("action")), "{rep:?}");
    assert!(
        rep.removed.iter().any(|r| r.contains("embedded file")),
        "{rep:?}"
    );
    let (out, _) = convert_bytes(&input).unwrap_or_else(|e| panic!("{e}"));
    vera_or_skip(&out, "forbidden features");
    let hay = String::from_utf8_lossy(&out);
    for needle in [
        "JavaScript",
        "app.alert",
        "calc.exe",
        "EmbeddedFile",
        "OpenAction",
    ] {
        assert!(!hay.contains(needle), "{needle} must be gone");
    }
}

#[test]
fn converting_twice_is_stable_and_still_valid() {
    let (once, _) = convert_bytes(&fixture_bytes("report-cairo.pdf")).unwrap();
    let (twice, rep) = convert_bytes(&once).unwrap();
    assert!(rep.blockers.is_empty());
    vera_or_skip(&twice, "second pass");
    assert_eq!(
        poppler_text(&once).is_some(),
        poppler_text(&twice).is_some()
    );
}

#[test]
fn the_validator_notices_when_a_converted_file_is_damaged_again() {
    let input = bytes_of(PdfDocument::from_image(&png_rgb(32, 32)).unwrap());
    let (out, _) = convert_bytes(&input).unwrap();
    let Some(good) = verapdf(&out, "2b") else {
        assert!(!verapdf_required());
        return;
    };
    assert!(good.compliant);
    // Remove the output intent and the file identifier: two independent breakages.
    let mut d = Document::load_mem(&out).unwrap();
    let cat_id = d.trailer.get(b"Root").unwrap().as_reference().unwrap();
    if let Object::Dictionary(c) = d.objects.get_mut(&cat_id).unwrap() {
        c.remove(b"OutputIntents");
    }
    d.trailer.remove(b"ID");
    let mut broken = Vec::new();
    d.save_to(&mut broken).unwrap();
    let bad = verapdf(&broken, "2b").unwrap();
    assert!(!bad.compliant);
    assert!(
        bad.failures.iter().any(|f| f.starts_with("6.2.4.3")),
        "{:?}",
        bad.failures
    );
    assert!(
        bad.failures.iter().any(|f| f.starts_with("6.1.3")),
        "{:?}",
        bad.failures
    );
}
