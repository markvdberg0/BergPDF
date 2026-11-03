//! Editing bookmarks: a written outline is read back by the reader the viewer uses, edits keep the
//! untouched destinations exactly, and the counts readers rely on are right.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::nav;
use pdf_engine::outline::{self, OutlineItem};
use test_support::fixtures;

fn open(bytes: Vec<u8>) -> PdfDocument {
    PdfDocument::open(bytes, &OpenOptions::default()).unwrap()
}

#[test]
fn a_written_outline_reads_back_and_edits_keep_the_rest() {
    let mut doc = open(fixtures::many_pages(5));
    assert!(outline::read_outline(&doc).is_empty());
    let mut items = vec![
        OutlineItem::new("Introduction", 0, None),
        OutlineItem::new("Chapter 1", 1, Some(700.0)),
        OutlineItem::new("Appendix", 4, None),
    ];
    outline::insert_child(&mut items, &[1], OutlineItem::new("Section 1.1", 2, None)).unwrap();
    outline::insert_child(
        &mut items,
        &[1],
        OutlineItem::new("Section 1.2 — Überblick", 3, None),
    )
    .unwrap();
    items[2].open = false;
    outline::insert_child(&mut items, &[2], OutlineItem::new("A.1", 4, None)).unwrap();
    items[2].open = false;
    doc.transact(|tx| outline::write_outline(tx, &items))
        .unwrap();

    let re = open(doc.snapshot_bytes().unwrap());
    let read = nav::outlines(&re);
    assert_eq!(read.len(), 3);
    assert_eq!(read[1].children[1].title, "Section 1.2 — Überblick");
    assert_eq!(read[1].children[1].page, Some(3));
    assert_eq!(read[2].children[0].page, Some(4));
    assert!(read[1].open && !read[2].open);
    // Counts: the root sees 3 + 2 open children; the closed appendix says -1.
    let d = re.lopdf();
    let cat = d.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let root = d
        .get_dictionary(cat)
        .unwrap()
        .get(b"Outlines")
        .unwrap()
        .as_reference()
        .unwrap();
    assert_eq!(
        d.get_dictionary(root)
            .unwrap()
            .get(b"Count")
            .unwrap()
            .as_i64()
            .unwrap(),
        5
    );

    // Rename one, move another: the untouched destination object is carried over as it was.
    let mut items = outline::read_outline(&re);
    let before = items[1].original.clone().unwrap();
    items[0].title = "Preface".into();
    let p = outline::move_by(&mut items, &[2], -1).unwrap();
    assert_eq!(p, vec![1]);
    let mut doc = re;
    doc.transact(|tx| outline::write_outline(tx, &items))
        .unwrap();
    let again = outline::read_outline(&open(doc.snapshot_bytes().unwrap()));
    assert_eq!(again[0].title, "Preface");
    assert_eq!(again[1].title, "Appendix");
    assert_eq!(again[2].title, "Chapter 1");
    assert_eq!(again[2].original.as_ref().unwrap(), &before);
    assert_eq!(again[2].page, Some(1));

    // Removing everything removes the outline.
    doc.transact(|tx| outline::write_outline(tx, &[])).unwrap();
    assert!(nav::outlines(&doc).is_empty());
}
