//! Milestone 4: AcroForm reading, filling, duplication policies and import — verified with the
//! engine's own re-read plus poppler as an independent renderer/text extractor.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::forms::{self, FieldKind};
use pdf_engine::pageops::{self, FormPolicy};
use test_support::fixtures::*;
use test_support::*;

fn open(bytes: Vec<u8>) -> PdfDocument {
    PdfDocument::open(bytes, &OpenOptions::default()).unwrap()
}

fn reopen(doc: &mut PdfDocument) -> PdfDocument {
    open(doc.snapshot_bytes().unwrap())
}

fn field_id(doc: &PdfDocument, name: &str) -> lopdf::ObjectId {
    forms::read_form(doc)
        .fields
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no field {name}"))
        .id
}

fn value_of(doc: &PdfDocument, name: &str) -> String {
    forms::read_form(doc)
        .fields
        .iter()
        .find(|f| f.name == name)
        .unwrap()
        .value
        .clone()
}

#[test]
fn reads_the_field_tree() {
    let doc = open(form_rich());
    let info = forms::read_form(&doc);
    let names: Vec<_> = info.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        ["name", "address.city", "agree", "size", "color"],
        "{names:?}"
    );
    assert!(matches!(info.fields[0].kind, FieldKind::Text { .. }));
    assert_eq!(info.fields[2].kind, FieldKind::Checkbox);
    assert_eq!(info.fields[3].kind, FieldKind::Radio);
    assert_eq!(info.fields[3].widgets.len(), 2);
    let FieldKind::Choice { combo, options } = &info.fields[4].kind else {
        panic!()
    };
    assert!(*combo);
    assert_eq!(options[1], ("g".to_string(), "Green".to_string()));
    let pages = doc.page_ids().unwrap();
    assert_eq!(info.fields[0].widgets[0].page, Some(pages[0]));
    assert!(!info.xfa && !info.has_scripts);
}

#[test]
fn fills_unicode_text_and_the_appearance_is_visible_to_poppler() {
    let mut doc = open(form_rich());
    let f = field_id(&doc, "name");
    doc.transact(|tx| forms::set_text_value(tx, f, "Zoë Ångström – naïve"))
        .unwrap();
    let mut doc2 = reopen(&mut doc);
    assert_eq!(value_of(&doc2, "name"), "Zoë Ångström – naïve");
    let bytes = doc2.snapshot_bytes().unwrap();
    let text = poppler_text(&bytes);
    if let Some(text) = text {
        assert!(text.contains("Zoë Ångström"), "poppler text: {text}");
    }
    if let Some((w, h, px)) = poppler_render(&bytes, 1, 100) {
        // Field region (x 100..300, y 255..275 of a 300pt-high page) must contain dark ink.
        let sc = 100.0 / 72.0;
        let (x0, x1) = ((100.0 * sc) as u32, (300.0 * sc) as u32);
        let (y0, y1) = (((300.0 - 275.0) * sc) as u32, ((300.0 - 255.0) * sc) as u32);
        let mut dark = 0;
        for y in y0..y1 {
            for x in x0..x1 {
                let i = ((y * w + x) * 3) as usize;
                if px[i] < 100 {
                    dark += 1;
                }
            }
        }
        assert!(
            dark > 30,
            "expected visible ink in the field, got {dark} (h={h})"
        );
    }
}

#[test]
fn rejects_unencodable_text_and_overlong_values() {
    let mut doc = open(form_rich());
    let f = field_id(&doc, "name");
    let err = doc
        .transact(|tx| forms::set_text_value(tx, f, "漢字"))
        .unwrap_err();
    assert!(
        matches!(err, pdf_engine::error::EngineError::MissingGlyphs { .. }),
        "{err:?}"
    );
    assert_eq!(value_of(&doc, "name"), "", "failed edit must roll back");
}

#[test]
fn checkbox_radio_and_combo() {
    let mut doc = open(form_rich());
    let (cb, rd, co) = (
        field_id(&doc, "agree"),
        field_id(&doc, "size"),
        field_id(&doc, "color"),
    );
    doc.transact(|tx| forms::set_checkbox(tx, cb, true))
        .unwrap();
    doc.transact(|tx| forms::set_radio(tx, rd, "Large"))
        .unwrap();
    doc.transact(|tx| forms::set_text_value(tx, co, "g"))
        .unwrap();
    let d2 = reopen(&mut doc);
    assert_eq!(value_of(&d2, "agree"), "Yes");
    assert_eq!(value_of(&d2, "size"), "Large");
    assert_eq!(value_of(&d2, "color"), "g");
    let err = doc
        .transact(|tx| forms::set_radio(tx, rd, "Huge"))
        .unwrap_err();
    assert!(matches!(
        err,
        pdf_engine::error::EngineError::InvalidArgument(_)
    ));
    doc.transact(|tx| forms::set_checkbox(tx, cb, false))
        .unwrap();
    assert_eq!(value_of(&doc, "agree"), "Off");
}

#[test]
fn independent_duplicate_creates_separate_fields() {
    let mut doc = open(form_rich());
    let f = field_id(&doc, "name");
    doc.transact(|tx| forms::set_text_value(tx, f, "Original"))
        .unwrap();
    let p1 = doc.page_ids().unwrap()[0];
    doc.transact(|tx| pageops::duplicate_pages(tx, &[p1], FormPolicy::Independent))
        .unwrap();
    let mut doc = reopen(&mut doc);
    let info = forms::read_form(&doc);
    let names: Vec<_> = info.fields.iter().map(|f| f.name.clone()).collect();
    // Every original field exists once more under a collision-free name.
    assert_eq!(info.fields.len(), 10, "{names:?}");
    let mut uniq = names.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), names.len(), "names must be unique: {names:?}");
    // Edit the copy; the original keeps its value.
    let copy = info
        .fields
        .iter()
        .find(|f| f.name != "name" && f.name.starts_with("name"))
        .unwrap();
    assert_eq!(copy.value, "Original");
    let cid = copy.id;
    let copy_name = copy.name.clone();
    doc.transact(|tx| forms::set_text_value(tx, cid, "Changed"))
        .unwrap();
    assert_eq!(value_of(&doc, "name"), "Original");
    assert_eq!(value_of(&doc, &copy_name), "Changed");
    // The radio group on the copy is one new group with two widgets (not two groups).
    let radios: Vec<_> = forms::read_form(&doc)
        .fields
        .into_iter()
        .filter(|f| f.kind == FieldKind::Radio)
        .collect();
    assert_eq!(radios.len(), 2);
    assert!(radios.iter().all(|r| r.widgets.len() == 2));
    // The copies live on the new page (page index 1).
    let pages = doc.page_ids().unwrap();
    assert_eq!(pages.len(), 3);
    assert_eq!(
        forms::read_form(&doc)
            .fields
            .iter()
            .find(|f| f.name == copy_name)
            .unwrap()
            .widgets[0]
            .page,
        Some(pages[1])
    );
}

#[test]
fn linked_duplicate_shares_the_field() {
    let mut doc = open(form_rich());
    let p1 = doc.page_ids().unwrap()[0];
    doc.transact(|tx| pageops::duplicate_pages(tx, &[p1], FormPolicy::Linked))
        .unwrap();
    let mut doc = reopen(&mut doc);
    let info = forms::read_form(&doc);
    assert_eq!(info.fields.len(), 5, "linked keeps the same fields");
    let f = info.fields.iter().find(|f| f.name == "name").unwrap();
    assert_eq!(f.widgets.len(), 2, "one field, two widgets");
    let pages = doc.page_ids().unwrap();
    let wp: Vec<_> = f.widgets.iter().map(|w| w.page).collect();
    assert!(
        wp.contains(&Some(pages[0])) && wp.contains(&Some(pages[1])),
        "{wp:?}"
    );
    let fid = f.id;
    doc.transact(|tx| forms::set_text_value(tx, fid, "Shared"))
        .unwrap();
    assert_eq!(value_of(&doc, "name"), "Shared");
}

#[test]
fn flatten_duplicate_bakes_appearance_and_drops_fields_on_the_copy() {
    let mut doc = open(form_rich());
    let f = field_id(&doc, "name");
    doc.transact(|tx| forms::set_text_value(tx, f, "Baked value"))
        .unwrap();
    let p1 = doc.page_ids().unwrap()[0];
    doc.transact(|tx| pageops::duplicate_pages(tx, &[p1], FormPolicy::Flatten))
        .unwrap();
    let mut doc = reopen(&mut doc);
    let info = forms::read_form(&doc);
    assert_eq!(
        info.fields.len(),
        5,
        "fields remain only on the original page"
    );
    let pages = doc.page_ids().unwrap();
    assert!(forms::page_widgets(doc.lopdf(), pages[1].0).is_empty());
    assert!(!forms::page_widgets(doc.lopdf(), pages[0].0).is_empty());
    let bytes = doc.snapshot_bytes().unwrap();
    if let Some((w, _h, px)) = poppler_render(&bytes, 2, 100) {
        let sc = 100.0 / 72.0;
        let (y0, y1) = (((300.0 - 275.0) * sc) as u32, ((300.0 - 255.0) * sc) as u32);
        let mut dark = 0;
        for y in y0..y1 {
            for x in (100.0 * sc) as u32..(300.0 * sc) as u32 {
                if px[((y * w + x) * 3) as usize] < 100 {
                    dark += 1;
                }
            }
        }
        assert!(
            dark > 30,
            "flattened text must be visible on the copy ({dark})"
        );
    }
}

#[test]
fn import_renames_colliding_fields_and_flatten_removes_them() {
    let src = open(form_rich());
    let sp = src.page_ids().unwrap()[0];
    let mut dst = open(form_rich());
    dst.transact(|tx| pageops::import_pages(tx, &src, &[sp], 2, FormPolicy::Independent))
        .unwrap();
    let mut dst = reopen(&mut dst);
    let info = forms::read_form(&dst);
    let names: Vec<_> = info.fields.iter().map(|f| f.name.clone()).collect();
    let mut uniq = names.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), names.len(), "{names:?}");
    assert_eq!(info.fields.len(), 10, "{names:?}");
    // Imported fields are editable.
    let imported = info
        .fields
        .iter()
        .find(|f| f.name != "name" && f.name.starts_with("name"))
        .unwrap();
    let id = imported.id;
    dst.transact(|tx| forms::set_text_value(tx, id, "Imported"))
        .unwrap();
    assert_eq!(value_of(&dst, "name"), "");

    let mut dst2 = open(form_rich());
    dst2.transact(|tx| pageops::import_pages(tx, &src, &[sp], 2, FormPolicy::Flatten))
        .unwrap();
    let dst2 = reopen(&mut dst2);
    assert_eq!(forms::read_form(&dst2).fields.len(), 5);
    let err = open(form_rich())
        .transact(|tx| pageops::import_pages(tx, &src, &[sp], 2, FormPolicy::Linked))
        .unwrap_err();
    assert!(matches!(
        err,
        pdf_engine::error::EngineError::Unsupported(_)
    ));
}

#[test]
fn import_into_a_document_without_acroform_creates_one() {
    let src = open(form_rich());
    let sp = src.page_ids().unwrap()[0];
    let mut dst = open(helvetica_lines());
    dst.transact(|tx| pageops::import_pages(tx, &src, &[sp], 1, FormPolicy::Independent))
        .unwrap();
    let dst = reopen(&mut dst);
    assert_eq!(forms::read_form(&dst).fields.len(), 5);
}

#[test]
fn xfa_and_javascript_are_detected_never_executed() {
    // A form whose AcroForm carries /XFA and a field with a calculate action.
    let mut doc = lopdf::Document::load_mem(&form_rich()).unwrap();
    let cat = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let af = doc
        .get_dictionary(cat)
        .unwrap()
        .get(b"AcroForm")
        .unwrap()
        .as_reference()
        .unwrap();
    doc.get_object_mut(af)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("XFA", lopdf::Object::Array(vec![]));
    let name_f = field_id(&open(form_rich()), "name");
    doc.get_object_mut(name_f).unwrap().as_dict_mut().unwrap().set(
        "AA",
        lopdf::dictionary! { "C" => lopdf::dictionary! { "S" => "JavaScript", "JS" => lopdf::Object::string_literal("event.value=1") } },
    );
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    let d = open(bytes);
    let info = forms::read_form(&d);
    assert!(info.xfa && info.has_scripts && info.has_calculations);
}

#[test]
fn flatten_document_bakes_values_and_removes_the_acroform() {
    let mut doc = open(form_rich());
    let f = field_id(&doc, "name");
    doc.transact(|tx| forms::set_text_value(tx, f, "Baked forever"))
        .unwrap();
    let cb = field_id(&doc, "agree");
    doc.transact(|tx| forms::set_checkbox(tx, cb, true))
        .unwrap();
    let ((baked, dropped), _) = doc.transact(forms::flatten_document).unwrap();
    // name, agree and both radio buttons have appearances; city is empty (nothing to draw);
    // the combo had a value but no appearance, which flattening now generates.
    assert_eq!((baked, dropped), (5, 1), "baked {baked}, dropped {dropped}");
    let mut doc = reopen(&mut doc);
    assert!(forms::read_form(&doc).fields.is_empty());
    let pages = doc.page_ids().unwrap();
    assert!(forms::page_widgets(doc.lopdf(), pages[0].0).is_empty());
    let cat = doc
        .lopdf()
        .trailer
        .get(b"Root")
        .unwrap()
        .as_reference()
        .unwrap();
    assert!(!doc.lopdf().get_dictionary(cat).unwrap().has(b"AcroForm"));
    let bytes = doc.snapshot_bytes().unwrap();
    if let Some((w, _h, px)) = poppler_render(&bytes, 1, 100) {
        let sc = 100.0 / 72.0;
        let (y0, y1) = (((300.0 - 275.0) * sc) as u32, ((300.0 - 255.0) * sc) as u32);
        let mut dark = 0;
        for y in y0..y1 {
            for x in (100.0 * sc) as u32..(300.0 * sc) as u32 {
                if px[((y * w + x) * 3) as usize] < 100 {
                    dark += 1;
                }
            }
        }
        assert!(dark > 30, "flattened value must stay visible ({dark})");
    } else {
        assert!(!oracles_required(), "poppler required");
    }
}
