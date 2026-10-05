//! Deterministic fixture PDFs generated with `pdf-writer` (a different library from the one
//! the application uses for editing). Each builder documents what it is for. All content is
//! original to this repository (see tests/fixtures/README.md for provenance).

use pdf_writer::types::{ActionType, AnnotationFlags, AnnotationType, FieldType};
use pdf_writer::{Content, Name, Pdf, Rect, Ref, Str, TextStr};

struct Ids(i32);

impl Ids {
    fn next(&mut self) -> Ref {
        self.0 += 1;
        Ref::new(self.0)
    }
}

/// One A4 page with simple Helvetica/WinAnsi text: a `Tj` line, a relative `Td` line and a
/// `TJ` line with kerning. Exercises base-14 metrics (no `/Widths`) and relative positioning.
pub fn helvetica_lines() -> Vec<u8> {
    let mut pdf = Pdf::new();
    let (catalog, tree, page, font, content) = (
        Ref::new(1),
        Ref::new(2),
        Ref::new(3),
        Ref::new(4),
        Ref::new(5),
    );
    pdf.catalog(catalog).pages(tree);
    pdf.pages(tree).kids([page]).count(1);
    {
        let mut p = pdf.page(page);
        p.media_box(Rect::new(0.0, 0.0, 595.0, 842.0))
            .parent(tree)
            .contents(content);
        p.resources().fonts().pair(Name(b"F1"), font);
    }
    pdf.type1_font(font)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    let mut c = Content::new();
    c.begin_text();
    c.set_font(Name(b"F1"), 14.0);
    c.next_line(72.0, 700.0);
    c.show(Str(b"Hello World"));
    c.next_line(0.0, -20.0);
    c.show(Str(b"Second line of text"));
    c.next_line(0.0, -20.0);
    c.show_positioned()
        .items()
        .show(Str(b"T"))
        .adjust(80.0)
        .show(Str(b"est kerning (parens)"));
    c.set_font(Name(b"F1"), 10.0);
    c.next_line(0.0, -30.0);
    c.show(Str(b"Small print: price \x80 99"));
    c.end_text();
    pdf.stream(content, &c.finish());
    pdf.finish()
}

/// Two pages that **share one content stream object** and one font. Editing text on page 1
/// must not change page 2 (copy-on-write).
pub fn shared_content_stream() -> Vec<u8> {
    let mut pdf = Pdf::new();
    let (catalog, tree, p1, p2, font, content) = (
        Ref::new(1),
        Ref::new(2),
        Ref::new(3),
        Ref::new(4),
        Ref::new(5),
        Ref::new(6),
    );
    pdf.catalog(catalog).pages(tree);
    pdf.pages(tree).kids([p1, p2]).count(2);
    for p in [p1, p2] {
        let mut page = pdf.page(p);
        page.media_box(Rect::new(0.0, 0.0, 400.0, 300.0))
            .parent(tree)
            .contents(content);
        page.resources().fonts().pair(Name(b"F1"), font);
    }
    pdf.type1_font(font)
        .base_font(Name(b"Times-Roman"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    let mut c = Content::new();
    c.begin_text()
        .set_font(Name(b"F1"), 18.0)
        .next_line(50.0, 200.0)
        .show(Str(b"Shared page text"))
        .end_text();
    pdf.stream(content, &c.finish());
    pdf.finish()
}

/// A page with a 4×4 RGB image placed under a flipped, scaled CTM plus some text.
pub fn image_page() -> Vec<u8> {
    let mut pdf = Pdf::new();
    let (catalog, tree, page, font, content, image) = (
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
        p.media_box(Rect::new(0.0, 0.0, 400.0, 400.0))
            .parent(tree)
            .contents(content);
        let mut r = p.resources();
        r.fonts().pair(Name(b"F1"), font);
        r.x_objects().pair(Name(b"Im1"), image);
    }
    pdf.type1_font(font)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    let mut px = Vec::new();
    for y in 0..4u8 {
        for x in 0..4u8 {
            px.extend_from_slice(&[x * 60, y * 60, 128]);
        }
    }
    {
        let mut im = pdf.image_xobject(image, &px);
        im.width(4).height(4).color_space().device_rgb();
        im.bits_per_component(8);
    }
    let mut c = Content::new();
    // Page-level flip like Chromium output, then the image drawn in the flipped space.
    c.transform([1.0, 0.0, 0.0, -1.0, 0.0, 400.0]);
    c.save_state()
        .transform([120.0, 0.0, 0.0, 80.0, 40.0, 60.0])
        .x_object(Name(b"Im1"))
        .restore_state();
    c.begin_text()
        .set_font(Name(b"F1"), 12.0)
        .set_text_matrix([1.0, 0.0, 0.0, -1.0, 40.0, 200.0])
        .show(Str(b"Caption under image"))
        .end_text();
    pdf.stream(content, &c.finish());
    pdf.finish()
}

/// Pages with awkward geometry: page 1 `/Rotate 90` with an offset CropBox and a MediaBox with
/// negative origin; page 2 with `/UserUnit 2`; page 3 inherits MediaBox/Rotate from the tree.
pub fn odd_geometry() -> Vec<u8> {
    let mut pdf = Pdf::new();
    let mut ids = Ids(0);
    let (catalog, tree, p1, p2, p3, font) = (
        ids.next(),
        ids.next(),
        ids.next(),
        ids.next(),
        ids.next(),
        ids.next(),
    );
    let (c1, c2, c3) = (ids.next(), ids.next(), ids.next());
    pdf.catalog(catalog).pages(tree);
    {
        let mut t = pdf.pages(tree);
        t.kids([p1, p2, p3]).count(3);
        t.media_box(Rect::new(0.0, 0.0, 300.0, 200.0));
        t.pair(Name(b"Rotate"), 270i32);
    }
    {
        let mut p = pdf.page(p1);
        p.parent(tree)
            .media_box(Rect::new(-100.0, -50.0, 500.0, 750.0))
            .crop_box(Rect::new(20.0, 30.0, 320.0, 330.0))
            .rotate(90)
            .contents(c1);
        p.resources().fonts().pair(Name(b"F1"), font);
    }
    {
        let mut p = pdf.page(p2);
        p.parent(tree)
            .media_box(Rect::new(0.0, 0.0, 200.0, 200.0))
            .rotate(0)
            .contents(c2);
        p.pair(Name(b"UserUnit"), 2.0f32);
        p.resources().fonts().pair(Name(b"F1"), font);
    }
    {
        let mut p = pdf.page(p3);
        p.parent(tree).contents(c3);
        p.resources().fonts().pair(Name(b"F1"), font);
    }
    pdf.type1_font(font)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    for (c, txt) in [
        (c1, "Rotated cropped page"),
        (c2, "UserUnit two"),
        (c3, "Inherited attributes"),
    ] {
        let mut cc = Content::new();
        cc.begin_text()
            .set_font(Name(b"F1"), 12.0)
            .next_line(30.0, 100.0)
            .show(Str(txt.as_bytes()))
            .end_text();
        pdf.stream(c, &cc.finish());
    }
    pdf.finish()
}

/// Pages of different sizes (A4 portrait, A3 landscape, tiny label).
pub fn mixed_sizes() -> Vec<u8> {
    let mut pdf = Pdf::new();
    let (catalog, tree) = (Ref::new(1), Ref::new(2));
    pdf.catalog(catalog).pages(tree);
    let pages = [Ref::new(3), Ref::new(4), Ref::new(5)];
    pdf.pages(tree).kids(pages).count(3);
    let sizes = [(595.0, 842.0), (1191.0, 842.0), (144.0, 72.0)];
    for (p, (w, h)) in pages.iter().zip(sizes) {
        pdf.page(*p)
            .parent(tree)
            .media_box(Rect::new(0.0, 0.0, w, h));
    }
    pdf.finish()
}

/// A form XObject painted twice plus a transparent rectangle (ExtGState alpha).
pub fn reused_form_and_transparency() -> Vec<u8> {
    let mut pdf = Pdf::new();
    let (catalog, tree, page, content, form, gs) = (
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
        p.media_box(Rect::new(0.0, 0.0, 300.0, 300.0))
            .parent(tree)
            .contents(content);
        let mut r = p.resources();
        r.x_objects().pair(Name(b"Fm1"), form);
        r.ext_g_states().pair(Name(b"GS1"), gs);
    }
    let mut fc = Content::new();
    fc.set_fill_rgb(0.1, 0.4, 0.9)
        .rect(0.0, 0.0, 50.0, 50.0)
        .fill_nonzero();
    let form_data = fc.finish();
    pdf.form_xobject(form, &form_data)
        .bbox(Rect::new(0.0, 0.0, 50.0, 50.0));
    pdf.ext_graphics(gs).non_stroking_alpha(0.5);
    let mut c = Content::new();
    c.save_state()
        .transform([1.0, 0.0, 0.0, 1.0, 20.0, 20.0])
        .x_object(Name(b"Fm1"))
        .restore_state();
    c.save_state()
        .transform([1.0, 0.0, 0.0, 1.0, 120.0, 20.0])
        .x_object(Name(b"Fm1"))
        .restore_state();
    c.set_parameters(Name(b"GS1"))
        .set_fill_rgb(1.0, 0.0, 0.0)
        .rect(40.0, 40.0, 120.0, 80.0)
        .fill_nonzero();
    pdf.stream(content, &c.finish());
    pdf.finish()
}

/// A page with pre-existing annotations from "another producer": a highlight with appearance, a
/// URI link, a GoTo link, a sticky note and a popup — used to prove they are preserved.
pub fn existing_annotations() -> Vec<u8> {
    let mut pdf = Pdf::new();
    let mut ids = Ids(0);
    let (catalog, tree, p1, p2, content) =
        (ids.next(), ids.next(), ids.next(), ids.next(), ids.next());
    let (hl, hl_ap, uri_link, goto_link, note, popup, font) = (
        ids.next(),
        ids.next(),
        ids.next(),
        ids.next(),
        ids.next(),
        ids.next(),
        ids.next(),
    );
    pdf.catalog(catalog).pages(tree);
    pdf.pages(tree).kids([p1, p2]).count(2);
    {
        let mut p = pdf.page(p1);
        p.media_box(Rect::new(0.0, 0.0, 400.0, 300.0))
            .parent(tree)
            .contents(content);
        p.resources().fonts().pair(Name(b"F1"), font);
        p.annotations([hl, uri_link, goto_link, note, popup]);
    }
    pdf.page(p2)
        .media_box(Rect::new(0.0, 0.0, 400.0, 300.0))
        .parent(tree);
    pdf.type1_font(font).base_font(Name(b"Helvetica"));
    let mut c = Content::new();
    c.begin_text()
        .set_font(Name(b"F1"), 14.0)
        .next_line(50.0, 200.0)
        .show(Str(b"Highlighted words here"))
        .end_text();
    pdf.stream(content, &c.finish());
    let mut ap = Content::new();
    ap.set_fill_rgb(1.0, 1.0, 0.0)
        .rect(0.0, 0.0, 120.0, 16.0)
        .fill_nonzero();
    let ap_data = ap.finish();
    pdf.form_xobject(hl_ap, &ap_data)
        .bbox(Rect::new(0.0, 0.0, 120.0, 16.0));
    {
        let mut a = pdf.annotation(hl);
        a.subtype(AnnotationType::Highlight)
            .rect(Rect::new(50.0, 196.0, 170.0, 212.0))
            .flags(AnnotationFlags::PRINT);
        a.contents(TextStr("legacy highlight"));
        a.author(TextStr("Other Producer"));
        a.quad_points([50.0, 212.0, 170.0, 212.0, 50.0, 196.0, 170.0, 196.0]);
        a.color_rgb(1.0, 1.0, 0.0);
        a.appearance().normal().stream(hl_ap);
    }
    {
        let mut a = pdf.annotation(uri_link);
        a.subtype(AnnotationType::Link)
            .rect(Rect::new(50.0, 100.0, 150.0, 120.0))
            .flags(AnnotationFlags::PRINT);
        a.action()
            .action_type(ActionType::Uri)
            .uri(Str(b"https://example.com/docs"));
    }
    {
        let mut a = pdf.annotation(goto_link);
        a.subtype(AnnotationType::Link)
            .rect(Rect::new(200.0, 100.0, 300.0, 120.0))
            .flags(AnnotationFlags::PRINT);
        a.insert(Name(b"Dest"))
            .array()
            .item(p2)
            .item(Name(b"XYZ"))
            .item(0.0f32)
            .item(300.0f32)
            .item(0.0f32);
    }
    {
        let mut a = pdf.annotation(note);
        a.subtype(AnnotationType::Text)
            .rect(Rect::new(300.0, 250.0, 320.0, 270.0))
            .contents(TextStr("legacy note"));
        a.author(TextStr("Other Producer"));
        a.pair(Name(b"Popup"), popup);
    }
    {
        let mut a = pdf.annotation(popup);
        a.pair(Name(b"Type"), Name(b"Annot"));
        a.pair(Name(b"Subtype"), Name(b"Popup"));
        a.rect(Rect::new(320.0, 200.0, 400.0, 270.0));
        a.pair(Name(b"Parent"), note);
    }
    pdf.finish()
}

/// A page with an AcroForm: text field, checkbox and a choice field (with appearances that
/// are deliberately minimal). For form-filling and field-duplication tests.
pub fn acroform_basic() -> Vec<u8> {
    let mut pdf = Pdf::new();
    let mut ids = Ids(0);
    let (catalog, tree, page, font, content) =
        (ids.next(), ids.next(), ids.next(), ids.next(), ids.next());
    let (name_f, check_f, choice_f) = (ids.next(), ids.next(), ids.next());
    let check_on = ids.next();
    let check_off = ids.next();
    let name_ap = ids.next();
    let acroform = ids_acroform(&mut pdf, [name_f, check_f, choice_f], font);
    pdf.catalog(catalog)
        .pages(tree)
        .pair(Name(b"AcroForm"), acroform);
    pdf.pages(tree).kids([page]).count(1);
    {
        let mut p = pdf.page(page);
        p.media_box(Rect::new(0.0, 0.0, 400.0, 300.0))
            .parent(tree)
            .contents(content);
        p.resources().fonts().pair(Name(b"Helv"), font);
        p.annotations([name_f, check_f, choice_f]);
    }
    pdf.type1_font(font)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    let mut c = Content::new();
    c.begin_text()
        .set_font(Name(b"Helv"), 12.0)
        .next_line(40.0, 260.0)
        .show(Str(b"Name:"))
        .next_line(0.0, -40.0)
        .show(Str(b"Agree:"))
        .end_text();
    pdf.stream(content, &c.finish());
    let mut ap = Content::new();
    ap.begin_text()
        .set_font(Name(b"Helv"), 12.0)
        .next_line(2.0, 4.0)
        .show(Str(b""))
        .end_text();
    let name_ap_data = ap.finish();
    pdf.form_xobject(name_ap, &name_ap_data)
        .bbox(Rect::new(0.0, 0.0, 200.0, 20.0));
    let mut on = Content::new();
    on.move_to(2.0, 2.0)
        .line_to(14.0, 14.0)
        .move_to(2.0, 14.0)
        .line_to(14.0, 2.0)
        .stroke();
    let on_data = on.finish();
    pdf.form_xobject(check_on, &on_data)
        .bbox(Rect::new(0.0, 0.0, 16.0, 16.0));
    pdf.form_xobject(check_off, b"")
        .bbox(Rect::new(0.0, 0.0, 16.0, 16.0));
    {
        let mut a = pdf.form_field(name_f);
        a.partial_name(TextStr("name"));
        a.field_type(FieldType::Text);
        let mut w = a.into_annotation();
        w.rect(Rect::new(100.0, 255.0, 300.0, 275.0))
            .flags(AnnotationFlags::PRINT);
        w.pair(Name(b"DA"), Str(b"/Helv 12 Tf 0 g"));
        w.appearance().normal().stream(name_ap);
        w.pair(Name(b"V"), TextStr(""));
    }
    {
        let mut a = pdf.form_field(check_f);
        a.partial_name(TextStr("agree"));
        a.field_type(FieldType::Button);
        let mut w = a.into_annotation();
        w.rect(Rect::new(100.0, 215.0, 116.0, 231.0))
            .flags(AnnotationFlags::PRINT);
        w.pair(Name(b"V"), Name(b"Off"));
        w.pair(Name(b"AS"), Name(b"Off"));
        let mut ap = w.appearance();
        let n = ap.normal();
        let mut d = n.streams();
        d.pair(Name(b"Yes"), check_on);
        d.pair(Name(b"Off"), check_off);
    }
    {
        let mut a = pdf.form_field(choice_f);
        a.partial_name(TextStr("color"));
        a.field_type(FieldType::Choice);
        let mut w = a.into_annotation();
        w.rect(Rect::new(100.0, 170.0, 220.0, 190.0))
            .flags(AnnotationFlags::PRINT);
        w.pair(Name(b"V"), TextStr("Red"));
        w.pair(Name(b"Opt"), 0i32); // overwritten below by raw bytes? keep simple
    }
    pdf.finish()
}

fn ids_acroform(pdf: &mut Pdf, fields: [Ref; 3], font: Ref) -> Ref {
    // pdf-writer cannot nest a dictionary under `pair` without a Ref; write it as an object.
    let id = Ref::new(900);
    {
        let mut af = pdf.indirect(id).dict();
        af.insert(Name(b"Fields")).array().items(fields);
        af.pair(Name(b"DA"), Str(b"/Helv 0 Tf 0 g"));
        af.insert(Name(b"DR"))
            .dict()
            .insert(Name(b"Font"))
            .dict()
            .pair(Name(b"Helv"), font);
        af.pair(Name(b"NeedAppearances"), false);
    }
    id
}

/// A document with a (structural, not cryptographically valid) signature field: `/ByteRange` +
/// `/Contents` placeholder. Used only to test *detection* and warnings — never to claim
/// signature validity.
pub fn signature_structure_only() -> Vec<u8> {
    let mut pdf = Pdf::new();
    let (catalog, tree, page, sig_field, sig_val) = (
        Ref::new(1),
        Ref::new(2),
        Ref::new(3),
        Ref::new(4),
        Ref::new(5),
    );
    pdf.catalog(catalog)
        .pages(tree)
        .pair(Name(b"AcroForm"), Ref::new(6));
    pdf.pages(tree).kids([page]).count(1);
    {
        let mut p = pdf.page(page);
        p.media_box(Rect::new(0.0, 0.0, 300.0, 300.0)).parent(tree);
        p.annotations([sig_field]);
    }
    {
        let mut af = pdf.indirect(Ref::new(6)).dict();
        af.insert(Name(b"Fields")).array().items([sig_field]);
        af.pair(Name(b"SigFlags"), 3i32);
    }
    {
        let mut a = pdf.form_field(sig_field);
        a.partial_name(TextStr("Signature1"));
        a.field_type(FieldType::Signature);
        a.pair(Name(b"V"), sig_val);
        let mut w = a.into_annotation();
        w.rect(Rect::new(50.0, 50.0, 200.0, 100.0))
            .flags(AnnotationFlags::PRINT);
    }
    {
        let mut v = pdf.indirect(sig_val).dict();
        v.pair(Name(b"Type"), Name(b"Sig"));
        v.pair(Name(b"Filter"), Name(b"Adobe.PPKLite"));
        v.pair(Name(b"SubFilter"), Name(b"adbe.pkcs7.detached"));
        v.insert(Name(b"ByteRange"))
            .array()
            .items([0i32, 100, 200, 100]);
        v.pair(Name(b"Contents"), Str(&[0u8; 16]));
    }
    pdf.finish()
}

/// Malformed inputs for safe-failure tests. Each must produce an error (or a documented
/// repair), never a panic or hang.
pub fn malformed() -> Vec<(&'static str, Vec<u8>)> {
    let good = helvetica_lines();
    let mut truncated = good.clone();
    truncated.truncate(good.len() / 2);
    let mut bad_xref = good.clone();
    if let Some(p) = bad_xref.windows(9).rposition(|w| w == b"startxref") {
        let tail = &mut bad_xref[p + 10..];
        for b in tail.iter_mut().take(4) {
            if b.is_ascii_digit() {
                *b = b'9';
            }
        }
    }
    let mut no_pages = good.clone();
    if let Some(w) = no_pages.windows(6).position(|w| w == b"/Pages") {
        no_pages[w + 1] = b'X';
    }
    vec![
        ("empty", Vec::new()),
        ("garbage", b"this is not a pdf at all \x00\x01\x02".to_vec()),
        ("header_only", b"%PDF-1.7\n".to_vec()),
        ("truncated", truncated),
        ("bad_startxref", bad_xref),
        ("catalog_without_pages", no_pages),
        ("huge_numbers", b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 99999999999999999999 0 R >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF".to_vec()),
    ]
}

/// A two-page document with a realistic AcroForm on page 1: merged text field `name`, a
/// hierarchical field `address.city`, checkbox `agree`, radio group `size` (Small/Large), and
/// combo box `color` (export/display option pairs). Page 2 is plain text. Used for form
/// filling, duplication (Linked / Independent / Flatten) and import tests.
pub fn form_rich() -> Vec<u8> {
    let mut pdf = Pdf::new();
    let (catalog, tree, page1, page2, font, c1, c2) = (
        Ref::new(1),
        Ref::new(2),
        Ref::new(3),
        Ref::new(4),
        Ref::new(5),
        Ref::new(6),
        Ref::new(7),
    );
    let (name_f, addr_f, city_f, agree_f, size_f, small_w, large_w, color_f, af) = (
        Ref::new(10),
        Ref::new(11),
        Ref::new(12),
        Ref::new(13),
        Ref::new(14),
        Ref::new(15),
        Ref::new(16),
        Ref::new(17),
        Ref::new(18),
    );
    let (box_on, box_off, rad_on, rad_off) =
        (Ref::new(20), Ref::new(21), Ref::new(22), Ref::new(23));
    pdf.catalog(catalog).pages(tree).pair(Name(b"AcroForm"), af);
    pdf.pages(tree).kids([page1, page2]).count(2);
    {
        let mut p = pdf.page(page1);
        p.media_box(Rect::new(0.0, 0.0, 400.0, 300.0))
            .parent(tree)
            .contents(c1);
        p.resources().fonts().pair(Name(b"Helv"), font);
        p.annotations([name_f, city_f, agree_f, small_w, large_w, color_f]);
    }
    {
        let mut p = pdf.page(page2);
        p.media_box(Rect::new(0.0, 0.0, 400.0, 300.0))
            .parent(tree)
            .contents(c2);
        p.resources().fonts().pair(Name(b"Helv"), font);
    }
    pdf.type1_font(font)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    let mut c = Content::new();
    c.begin_text()
        .set_font(Name(b"Helv"), 12.0)
        .next_line(20.0, 260.0)
        .show(Str(b"Name"))
        .next_line(0.0, -30.0)
        .show(Str(b"City"))
        .next_line(0.0, -30.0)
        .show(Str(b"Agree"))
        .next_line(0.0, -30.0)
        .show(Str(b"Size"))
        .next_line(0.0, -30.0)
        .show(Str(b"Colour"))
        .end_text();
    pdf.stream(c1, &c.finish());
    let mut c = Content::new();
    c.begin_text()
        .set_font(Name(b"Helv"), 12.0)
        .next_line(20.0, 260.0)
        .show(Str(b"Second page"))
        .end_text();
    pdf.stream(c2, &c.finish());
    {
        let mut d = pdf.indirect(af).dict();
        d.insert(Name(b"Fields"))
            .array()
            .items([name_f, addr_f, agree_f, size_f, color_f]);
        d.pair(Name(b"DA"), Str(b"/Helv 12 Tf 0 g"));
        d.insert(Name(b"DR"))
            .dict()
            .insert(Name(b"Font"))
            .dict()
            .pair(Name(b"Helv"), font);
    }
    let mark = |pdf: &mut Pdf, id: Ref, on: bool, w: f32| {
        let mut ct = Content::new();
        if on {
            ct.move_to(2.0, 2.0)
                .line_to(w - 2.0, w - 2.0)
                .move_to(2.0, w - 2.0)
                .line_to(w - 2.0, 2.0)
                .stroke();
        }
        pdf.form_xobject(id, &ct.finish())
            .bbox(Rect::new(0.0, 0.0, w, w));
    };
    mark(&mut pdf, box_on, true, 16.0);
    mark(&mut pdf, box_off, false, 16.0);
    mark(&mut pdf, rad_on, true, 16.0);
    mark(&mut pdf, rad_off, false, 16.0);
    // name: merged field + widget
    {
        let mut a = pdf.form_field(name_f);
        a.partial_name(TextStr("name"));
        a.field_type(FieldType::Text);
        let mut w = a.into_annotation();
        w.rect(Rect::new(100.0, 255.0, 300.0, 275.0))
            .flags(AnnotationFlags::PRINT)
            .page(page1);
        w.pair(Name(b"DA"), Str(b"/Helv 12 Tf 0 g"));
    }
    // address (parent only) with kid city
    {
        let mut d = pdf.indirect(addr_f).dict();
        d.pair(Name(b"T"), TextStr("address"));
        d.insert(Name(b"Kids")).array().items([city_f]);
    }
    {
        let mut a = pdf.form_field(city_f);
        a.partial_name(TextStr("city"));
        a.field_type(FieldType::Text);
        a.pair(Name(b"Parent"), addr_f);
        let mut w = a.into_annotation();
        w.rect(Rect::new(100.0, 225.0, 300.0, 245.0))
            .flags(AnnotationFlags::PRINT)
            .page(page1);
    }
    // checkbox
    {
        let mut a = pdf.form_field(agree_f);
        a.partial_name(TextStr("agree"));
        a.field_type(FieldType::Button);
        let mut w = a.into_annotation();
        w.rect(Rect::new(100.0, 195.0, 116.0, 211.0))
            .flags(AnnotationFlags::PRINT)
            .page(page1);
        w.pair(Name(b"V"), Name(b"Off"));
        w.pair(Name(b"AS"), Name(b"Off"));
        let mut ap = w.appearance();
        let n = ap.normal();
        let mut s = n.streams();
        s.pair(Name(b"Yes"), box_on);
        s.pair(Name(b"Off"), box_off);
    }
    // radio group
    {
        let mut d = pdf.indirect(size_f).dict();
        d.pair(Name(b"T"), TextStr("size"));
        d.pair(Name(b"FT"), Name(b"Btn"));
        d.pair(Name(b"Ff"), 32768i32);
        d.pair(Name(b"V"), Name(b"Off"));
        d.insert(Name(b"Kids")).array().items([small_w, large_w]);
    }
    for (id, state, x) in [(small_w, "Small", 100.0f32), (large_w, "Large", 150.0)] {
        let mut d = pdf.indirect(id).dict();
        d.pair(Name(b"Type"), Name(b"Annot"));
        d.pair(Name(b"Subtype"), Name(b"Widget"));
        d.pair(Name(b"Parent"), size_f);
        d.pair(Name(b"P"), page1);
        d.pair(Name(b"F"), 4i32);
        d.insert(Name(b"Rect"))
            .array()
            .items([x, 165.0f32, x + 16.0, 181.0]);
        d.pair(Name(b"AS"), Name(b"Off"));
        let mut ap = d.insert(Name(b"AP")).dict();
        let mut n = ap.insert(Name(b"N")).dict();
        n.pair(Name(state.as_bytes()), rad_on);
        n.pair(Name(b"Off"), rad_off);
    }
    // combo
    {
        let mut a = pdf.form_field(color_f);
        a.partial_name(TextStr("color"));
        a.field_type(FieldType::Choice);
        a.pair(Name(b"Ff"), 131072i32);
        let mut w = a.into_annotation();
        w.rect(Rect::new(100.0, 135.0, 220.0, 155.0))
            .flags(AnnotationFlags::PRINT)
            .page(page1);
        w.pair(Name(b"V"), TextStr("r"));
        let mut opt = w.insert(Name(b"Opt")).array();
        for (e, d) in [("r", "Red"), ("g", "Green")] {
            let mut pair = opt.push().array();
            pair.push().primitive(TextStr(e));
            pair.push().primitive(TextStr(d));
        }
    }
    pdf.finish()
}

/// A document with `n` simple text pages (for scale/performance checks).
pub fn many_pages(n: usize) -> Vec<u8> {
    let mut pdf = Pdf::new();
    let (catalog, tree, font) = (Ref::new(1), Ref::new(2), Ref::new(3));
    pdf.catalog(catalog).pages(tree);
    let first_page = 10;
    let kids: Vec<Ref> = (0..n)
        .map(|i| Ref::new(first_page + (i as i32) * 2))
        .collect();
    pdf.pages(tree).kids(kids.iter().copied()).count(n as i32);
    pdf.type1_font(font)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    for i in 0..n {
        let (page, content) = (
            Ref::new(first_page + (i as i32) * 2),
            Ref::new(first_page + (i as i32) * 2 + 1),
        );
        let mut p = pdf.page(page);
        p.media_box(Rect::new(0.0, 0.0, 595.0, 842.0))
            .parent(tree)
            .contents(content);
        p.resources().fonts().pair(Name(b"F1"), font);
        drop(p);
        let mut c = Content::new();
        c.begin_text()
            .set_font(Name(b"F1"), 14.0)
            .next_line(50.0, 780.0);
        c.show(Str(format!("Page {} of {n}", i + 1).as_bytes()));
        for l in 0..40 {
            c.next_line(0.0, -16.0);
            c.show(Str(format!(
                "Line {l}: The quick brown fox jumps over the lazy dog."
            )
            .as_bytes()));
        }
        c.end_text();
        pdf.stream(content, &c.finish());
    }
    pdf.finish()
}

/// One A0 page (841 × 1189 mm) filled with `n` small stroked and filled vector shapes — a
/// stand-in for a dense CAD-style drawing.
pub fn vector_a0(n: usize) -> Vec<u8> {
    let mut pdf = Pdf::new();
    let (catalog, tree, page, content) = (Ref::new(1), Ref::new(2), Ref::new(3), Ref::new(4));
    pdf.catalog(catalog).pages(tree);
    pdf.pages(tree).kids([page]).count(1);
    {
        let mut p = pdf.page(page);
        p.media_box(Rect::new(0.0, 0.0, 2384.0, 3370.0))
            .parent(tree)
            .contents(content);
    }
    let mut c = Content::new();
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % 100_000) as f32 / 100_000.0
    };
    for i in 0..n {
        let (x, y) = (rnd() * 2300.0, rnd() * 3300.0);
        c.set_line_width(0.25 + rnd());
        c.set_stroke_rgb(rnd(), rnd(), rnd());
        if i % 3 == 0 {
            c.set_fill_rgb(rnd(), rnd(), rnd());
            c.rect(x, y, 4.0 + rnd() * 30.0, 4.0 + rnd() * 30.0);
            c.fill_nonzero_and_stroke();
        } else {
            c.move_to(x, y);
            c.cubic_to(
                x + 10.0,
                y + 30.0 * rnd(),
                x + 20.0,
                y - 20.0 * rnd(),
                x + 40.0 * rnd(),
                y + 5.0,
            );
            c.stroke();
        }
    }
    pdf.stream(content, &c.finish());
    pdf.finish()
}
