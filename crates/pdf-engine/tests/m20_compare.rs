//! Comparing two versions: real PDFs are read back as words, the changes are found, and the
//! annotated copy marks them where they are on the newer version.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::annot::{self, AnnotationKind};
use pdf_engine::compare::{self, ChangeKind};
use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::textdoc::build_text_pdf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

fn version(body: &str, second: &str) -> Vec<u8> {
    build_text_pdf(
        "Contract",
        "",
        &[
            ("Terms".to_string(), body.to_string()),
            ("Payment".to_string(), second.to_string()),
        ],
    )
    .unwrap()
    .0
}

#[test]
fn two_versions_are_compared_and_the_changes_marked() {
    let old = version(
        "The tenant pays the rent on the first day of each month. The landlord repairs the roof.",
        "Payment is due within thirty days by bank transfer.",
    );
    let new = version(
        "The tenant pays the rent on the fifth day of each month. The landlord repairs the roof and the gutters.",
        "Payment is due within days by bank transfer.",
    );
    let cancel = AtomicBool::new(false);
    let cmp = compare::compare(Arc::new(old.clone()), Arc::new(new.clone()), &cancel).unwrap();
    assert!(cmp.old_words > 20 && cmp.new_words > 20);
    assert_eq!(cmp.old_pages_without_text, 0);
    let found: Vec<(ChangeKind, String, String)> = cmp
        .changes
        .iter()
        .map(|c| (c.kind, c.old_text.clone(), c.new_text.clone()))
        .collect();
    assert!(
        found.contains(&(ChangeKind::Replaced, "first".into(), "fifth".into())),
        "{found:?}"
    );
    assert!(
        found
            .iter()
            .any(|(k, _, n)| *k == ChangeKind::Inserted && n.contains("gutters")),
        "{found:?}"
    );
    assert!(
        found.contains(&(ChangeKind::Deleted, "thirty".into(), String::new())),
        "{found:?}"
    );
    assert_eq!(
        cmp.counts().0 + cmp.counts().1 + cmp.counts().2,
        cmp.changes.len()
    );

    // Marked on a copy of the newer version.
    let mut doc = PdfDocument::open(new, &OpenOptions::default()).unwrap();
    let pages = doc.page_ids().unwrap();
    let n = doc
        .transact(|tx| {
            compare::annotate(
                tx,
                &pages,
                &cmp,
                ["Inserted", "Deleted", "Replaced", "Was:"],
            )
        })
        .unwrap()
        .0;
    assert!(n >= 3);
    let re = PdfDocument::open(doc.snapshot_bytes().unwrap(), &OpenOptions::default()).unwrap();
    let all: Vec<_> = re
        .page_ids()
        .unwrap()
        .into_iter()
        .flat_map(|p| annot::read_annotations(&re, p))
        .collect();
    assert!(
        all.iter().any(|a| a.contents == "Was: first"),
        "{:?}",
        all.iter().map(|a| &a.contents).collect::<Vec<_>>()
    );
    assert!(all.iter().any(|a| a.contents == "Deleted: thirty"
        && matches!(
            a.spec.as_ref().map(|s| &s.kind),
            Some(AnnotationKind::Note { .. })
        )));
    // Identical versions have no changes.
    let same = compare::compare(Arc::new(old.clone()), Arc::new(old), &cancel).unwrap();
    assert!(same.changes.is_empty());
}
