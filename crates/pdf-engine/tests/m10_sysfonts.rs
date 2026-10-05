//! Installed ("system") fonts: found by scanning, embedded as subsets, remembered in annotations.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::annot::{self, Align, AnnotationKind, AnnotationSpec, Rgb};
use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::fontembed::{FontFamily, FontStyle};
use pdf_engine::geom::{Point, Rect};
use pdf_engine::{pagecontent, pageops, sysfonts};
use std::process::Command;
use test_support::*;

/// A folder with two faces of a pretend installed family ("Liberation Sans" under another name
/// is not possible without rewriting tables, so the real name is used).
fn install_test_fonts() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let files: [(&str, &[u8]); 4] = [
        (
            "r.ttf",
            include_bytes!("../assets/fonts/LiberationSerif-Regular.ttf"),
        ),
        (
            "b.ttf",
            include_bytes!("../assets/fonts/LiberationSerif-Bold.ttf"),
        ),
        (
            "i.ttf",
            include_bytes!("../assets/fonts/LiberationSerif-Italic.ttf"),
        ),
        (
            "bi.ttf",
            include_bytes!("../assets/fonts/LiberationSerif-BoldItalic.ttf"),
        ),
    ];
    for (n, b) in files {
        std::fs::write(dir.path().join(n), b).unwrap();
    }
    sysfonts::install(sysfonts::scan(&[dir.path().to_path_buf()]));
    dir
}

fn pdffonts(bytes: &[u8]) -> Option<String> {
    if !have_tool("pdffonts") {
        return None;
    }
    let d = tempfile::tempdir().ok()?;
    let p = d.path().join("a.pdf");
    std::fs::write(&p, bytes).ok()?;
    let out = Command::new("pdffonts").arg(&p).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn an_installed_font_is_embedded_as_a_subset_and_text_is_extractable() {
    let _dir = install_test_fonts();
    let id = sysfonts::find_by_name("Liberation Serif").expect("family registered");
    let fam = FontFamily::System(id);
    assert_eq!(fam.title(), "Liberation Serif");
    assert!(fam.has_italic() && fam.has_bold());
    assert_eq!(FontFamily::from_key(fam.key()), Some(fam));

    let mut doc = PdfDocument::new_empty().unwrap();
    let page = doc
        .transact(|tx| pageops::insert_blank_page(tx, 0, 595.0, 842.0))
        .unwrap()
        .0;
    let style = FontStyle::new(fam, true, true);
    doc.transact(|tx| {
        pagecontent::add_text(
            tx,
            page,
            Point::new(60.0, 700.0),
            "Installed font sample ëü €",
            14.0,
            (0.0, 0.0, 0.0),
            style,
        )
    })
    .unwrap();
    let bytes = doc.snapshot_bytes().unwrap();
    if let Some(text) = poppler_text(&bytes) {
        assert!(text.contains("Installed font sample"), "{text}");
    }
    if let Some(fonts) = pdffonts(&bytes) {
        let line = fonts
            .lines()
            .find(|l| l.contains("LiberationSerif"))
            .unwrap_or_else(|| panic!("no embedded Liberation Serif in:\n{fonts}"));
        assert!(line.contains("yes"), "embedded: {line}");
        assert!(!line.contains("DejaVu"), "{line}");
    }
    assert_eq!(poppler_diagnostics(&bytes).unwrap_or_default(), "");
}

#[test]
fn a_text_box_remembers_its_system_font_across_saving() {
    let _dir = install_test_fonts();
    let id = sysfonts::find_by_name("Liberation Serif").unwrap();
    let style = FontStyle::new(FontFamily::System(id), false, true);
    assert_eq!(FontStyle::from_base_name(&style.base_name()), Some(style));
    let mut doc = PdfDocument::new_empty().unwrap();
    let page = doc
        .transact(|tx| pageops::insert_blank_page(tx, 0, 595.0, 842.0))
        .unwrap()
        .0;
    let mut spec = AnnotationSpec::new(AnnotationKind::FreeText {
        rect: Rect::new(50.0, 600.0, 300.0, 680.0),
        font_size: 13.0,
        text_color: Rgb::BLACK,
        align: Align::Left,
        font: style,
        callout: None,
    });
    spec.contents = "Boxed text".into();
    doc.transact(|tx| annot::add_annotation(tx, page, &spec))
        .unwrap();
    let bytes = doc.snapshot_bytes().unwrap();
    let reopened = PdfDocument::open(bytes, &OpenOptions::default()).unwrap();
    let p = reopened.pages().unwrap()[0].id;
    let infos = annot::read_annotations(&reopened, p);
    let AnnotationKind::FreeText { font, .. } = infos[0].spec.clone().unwrap().kind else {
        panic!()
    };
    assert_eq!(font, style);
}

#[test]
fn a_font_that_covers_the_characters_can_be_found() {
    let _dir = install_test_fonts();
    let id = sysfonts::family_covering(&['a', 'é']).expect("some installed font covers Latin");
    assert!(sysfonts::with_family(id, |f| !f.name.is_empty()).unwrap());
    // Nothing installed has this private-use character.
    assert!(sysfonts::family_covering(&['\u{10FFFD}']).is_none());
}
