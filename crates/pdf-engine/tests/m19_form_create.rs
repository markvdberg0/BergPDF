//! Making forms: every field kind can be created, is read back as that kind after saving, can be
//! filled, changed, moved and deleted, and the scripts stored on it come back unchanged.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::doc::{OpenOptions, PageId, PdfDocument};
use pdf_engine::formedit::{self, FieldProps, NewField};
use pdf_engine::forms::{self, FieldKind};
use pdf_engine::formscripts::{self, FieldScripts};
use pdf_engine::geom::Rect;
use pdf_engine::pageops;

fn open(bytes: Vec<u8>) -> PdfDocument {
    PdfDocument::open(bytes, &OpenOptions::default()).unwrap()
}

fn blank() -> (PdfDocument, PageId) {
    let mut doc = PdfDocument::new_empty().unwrap();
    let page = doc
        .transact(|tx| pageops::insert_blank_page(tx, 0, 595.0, 842.0))
        .unwrap()
        .0;
    (doc, page)
}

fn create(
    doc: &mut PdfDocument,
    page: PageId,
    rect: Rect,
    props: &FieldProps,
) -> pdf_engine::ObjectId {
    let view = open(doc.snapshot_bytes().unwrap());
    doc.transact(|tx| formedit::create_field(tx, &view, page, rect, props))
        .unwrap()
        .0
}

#[test]
fn every_kind_of_field_is_created_saved_and_read_back() {
    let (mut doc, page) = blank();
    let text = FieldProps {
        value: "Hello form".into(),
        tooltip: "Your name".into(),
        required: true,
        ..FieldProps::new(
            NewField::Text {
                multiline: false,
                password: false,
                comb: false,
                max_len: Some(30),
            },
            "name",
        )
    };
    create(&mut doc, page, Rect::new(50.0, 700.0, 300.0, 724.0), &text);
    create(
        &mut doc,
        page,
        Rect::new(50.0, 650.0, 66.0, 666.0),
        &FieldProps::new(NewField::Checkbox, "agree"),
    );
    let mut r1 = FieldProps::new(NewField::Radio, "size");
    r1.export = "Small".into();
    create(&mut doc, page, Rect::new(50.0, 600.0, 66.0, 616.0), &r1);
    let mut r2 = FieldProps::new(NewField::Radio, "size");
    r2.export = "Large".into();
    create(&mut doc, page, Rect::new(90.0, 600.0, 106.0, 616.0), &r2);
    let mut combo = FieldProps::new(NewField::Combo { editable: false }, "country");
    combo.options = vec![
        ("NL".into(), "Netherlands".into()),
        ("DE".into(), "Germany".into()),
    ];
    combo.value = "DE".into();
    create(&mut doc, page, Rect::new(50.0, 550.0, 250.0, 570.0), &combo);
    let mut list = FieldProps::new(NewField::List { multi: false }, "colour");
    list.options = vec![("red".into(), "red".into()), ("blue".into(), "blue".into())];
    create(&mut doc, page, Rect::new(50.0, 470.0, 250.0, 530.0), &list);
    let mut button = FieldProps::new(NewField::Button, "reset");
    button.caption = "Start again".into();
    button.reset_form = true;
    create(
        &mut doc,
        page,
        Rect::new(50.0, 420.0, 150.0, 445.0),
        &button,
    );
    create(
        &mut doc,
        page,
        Rect::new(300.0, 400.0, 500.0, 450.0),
        &FieldProps::new(NewField::Signature, "sign"),
    );

    let bytes = doc.snapshot_bytes().unwrap();
    let re = open(bytes.clone());
    let form = forms::read_form(&re);
    let by = |n: &str| {
        form.fields
            .iter()
            .find(|f| f.name == n)
            .unwrap_or_else(|| panic!("{n} missing"))
    };
    assert!(matches!(
        by("name").kind,
        FieldKind::Text {
            max_len: Some(30),
            ..
        }
    ));
    assert!(by("name").required);
    assert_eq!(by("name").value, "Hello form");
    assert_eq!(by("agree").kind, FieldKind::Checkbox);
    assert_eq!(by("size").kind, FieldKind::Radio);
    assert_eq!(by("size").widgets.len(), 2, "one group, two buttons");
    assert!(
        matches!(&by("country").kind, FieldKind::Choice { combo: true, options } if options.len() == 2)
    );
    assert!(matches!(
        by("colour").kind,
        FieldKind::Choice { combo: false, .. }
    ));
    assert_eq!(by("reset").kind, FieldKind::PushButton);
    assert_eq!(by("sign").kind, FieldKind::Signature);
    // A second reader shows the generated appearances.
    if let Some(t) = test_support::poppler_text(&bytes) {
        assert!(
            t.contains("Hello form") && t.contains("Germany") && t.contains("Start again"),
            "{t}"
        );
    }

    // Filling works on the new fields like on any other.
    let mut doc = re;
    let (agree, size, name) = (by("agree").id, by("size").id, by("name").id);
    doc.transact(|tx| {
        forms::set_checkbox(tx, agree, true)?;
        forms::set_radio(tx, size, "Large")?;
        forms::set_text_value(tx, name, "Changed")
    })
    .unwrap();
    let form = forms::read_form(&open(doc.snapshot_bytes().unwrap()));
    let by = |n: &str| {
        form.fields
            .iter()
            .find(|f| f.name == n)
            .unwrap()
            .value
            .clone()
    };
    assert_eq!(by("agree"), "Yes");
    assert_eq!(by("size"), "Large");
    assert_eq!(by("name"), "Changed");
}

#[test]
fn fields_are_changed_moved_and_deleted() {
    let (mut doc, page) = blank();
    let props = FieldProps::new(
        NewField::Text {
            multiline: false,
            password: false,
            comb: false,
            max_len: None,
        },
        "first",
    );
    let w = create(&mut doc, page, Rect::new(50.0, 700.0, 200.0, 720.0), &props);
    // The same name twice is refused; a unique one is offered.
    let view = open(doc.snapshot_bytes().unwrap());
    assert!(
        doc.transact(|tx| formedit::create_field(
            tx,
            &view,
            page,
            Rect::new(50.0, 600.0, 200.0, 620.0),
            &props
        ))
        .is_err()
    );
    assert_eq!(formedit::unique_field_name(doc.lopdf(), "first"), "first_2");

    let view = open(doc.snapshot_bytes().unwrap());
    let mut p = formedit::read_props(&view, w, w).unwrap();
    p.name = "total".into();
    p.read_only = true;
    p.align = 2;
    p.scripts = FieldScripts {
        calculate: "AFSimple_Calculate(\"SUM\", [\"a\", \"b\"]);".into(),
        format: "AFNumber_Format(2, 0, 0, 0, \"€ \", true);".into(),
        ..FieldScripts::default()
    };
    doc.transact(|tx| formedit::update_field(tx, w, w, &p))
        .unwrap();
    let re = open(doc.snapshot_bytes().unwrap());
    let back = formedit::read_props(&re, w, w).unwrap();
    assert_eq!(back.name, "total");
    assert!(back.read_only);
    assert_eq!(back.align, 2);
    assert_eq!(back.scripts, p.scripts);
    assert_eq!(formscripts::calculation_order(re.lopdf()), vec![w]);
    let all = formscripts::read_form_scripts(&re);
    assert_eq!(all.order, vec![w]);

    doc.transact(|tx| formedit::move_widget(tx, &re, w, Rect::new(100.0, 500.0, 300.0, 530.0)))
        .unwrap();
    let form = forms::read_form(&open(doc.snapshot_bytes().unwrap()));
    assert_eq!(
        form.fields[0].widgets[0].rect,
        Rect::new(100.0, 500.0, 300.0, 530.0)
    );

    // Removing the calculation takes the field out of the calculation order.
    let mut p2 = p.clone();
    p2.scripts.calculate.clear();
    doc.transact(|tx| formedit::update_field(tx, w, w, &p2))
        .unwrap();
    assert!(formscripts::calculation_order(doc.lopdf()).is_empty());

    doc.transact(|tx| formedit::delete_widget(tx, w)).unwrap();
    let re = open(doc.snapshot_bytes().unwrap());
    assert!(forms::read_form(&re).fields.is_empty());
    assert!(pdf_engine::annot::annotation_ids(&re, page).is_empty());
}

#[test]
fn document_scripts_round_trip() {
    let (mut doc, _) = blank();
    assert!(formscripts::document_scripts(doc.lopdf()).is_empty());
    let scripts = vec![
        (
            "helpers".to_string(),
            "function twice(x) { return 2 * x; }".to_string(),
        ),
        ("init".to_string(), "var ready = true;".to_string()),
    ];
    doc.transact(|tx| formscripts::set_document_scripts(tx, &scripts))
        .unwrap();
    let re = open(doc.snapshot_bytes().unwrap());
    assert_eq!(formscripts::document_scripts(re.lopdf()), scripts);
    assert!(re.capabilities().has_javascript);
    doc.transact(|tx| formscripts::set_document_scripts(tx, &[]))
        .unwrap();
    assert!(formscripts::document_scripts(doc.lopdf()).is_empty());
}
