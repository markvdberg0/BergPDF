//! Session-level behaviour: history, saved-revision dirty tracking, safe saving.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use editor_core::session::DocumentSession;
use pdf_engine::annot::{self, AnnotationKind, AnnotationSpec};
use pdf_engine::error::EngineError;
use pdf_engine::geom::{Rect, Rotation};
use pdf_engine::pageops;
use pdf_engine::save::InjectedFailure;
use std::fs;
use test_support::fixture_bytes;

fn tmp_copy() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("doc.pdf");
    fs::write(&p, fixture_bytes("report-chromium.pdf")).unwrap();
    (dir, p)
}

fn add_rect(s: &mut DocumentSession) {
    let page = s.pages().unwrap()[0].id;
    s.execute("Add rectangle", |tx| {
        annot::add_annotation(
            tx,
            page,
            &AnnotationSpec::new(AnnotationKind::Rectangle {
                rect: Rect::new(50.0, 50.0, 150.0, 120.0),
            }),
        )
    })
    .unwrap();
}

fn tmp_files(dir: &std::path::Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".ferrum-tmp"))
        .collect()
}

#[test]
fn dirty_state_is_tracked_against_the_saved_revision() {
    let (_d, p) = tmp_copy();
    let mut s = DocumentSession::open_path(&p).unwrap();
    assert!(!s.is_dirty());
    add_rect(&mut s);
    assert!(s.is_dirty());
    assert_eq!(s.undo_label(), Some("Add rectangle"));
    s.undo().unwrap();
    assert!(!s.is_dirty(), "undo back to the saved state is clean");
    s.redo().unwrap();
    assert!(s.is_dirty());
    s.save().unwrap();
    assert!(!s.is_dirty());
    s.undo().unwrap();
    assert!(s.is_dirty(), "undoing past the saved state is dirty");
    s.redo().unwrap();
    assert!(!s.is_dirty(), "redo back to the saved state is clean");
    // A new edit after undo discards redo and is dirty.
    s.undo().unwrap();
    add_rect(&mut s);
    assert!(!s.can_redo());
    assert!(s.is_dirty());
}

#[test]
fn revisions_increase_on_every_state_change() {
    let (_d, p) = tmp_copy();
    let mut s = DocumentSession::open_path(&p).unwrap();
    let r0 = s.revision();
    add_rect(&mut s);
    let r1 = s.revision();
    s.undo();
    let r2 = s.revision();
    s.redo();
    let r3 = s.revision();
    assert!(r0 < r1 && r1 < r2 && r2 < r3);
    // Snapshots are tagged and cached per revision.
    let a = s.snapshot().unwrap();
    let b = s.snapshot().unwrap();
    assert_eq!(a.revision, r3);
    assert!(std::sync::Arc::ptr_eq(&a.bytes, &b.bytes));
}

#[test]
fn coalesced_edits_are_one_undo_step_but_do_not_cross_the_saved_state() {
    let (_d, p) = tmp_copy();
    let mut s = DocumentSession::open_path(&p).unwrap();
    add_rect(&mut s);
    let page = s.pages().unwrap()[0].id;
    let id = annot::annotation_ids(s.doc(), page)[0];
    for dx in 0..5 {
        s.execute_coalesced("nudge", "Move annotation", |tx| {
            annot::move_annotation(tx, id, 1.0 + f64::from(dx), 0.0)
        })
        .unwrap();
    }
    s.undo().unwrap(); // reverts all five nudges at once
    let moved = annot::read_annotation(s.doc().lopdf(), id).unwrap();
    assert!(
        (moved.rect.x0 - 50.0).abs() < 1e-3,
        "all nudges undone together: {:?}",
        moved.rect
    );
    assert_eq!(s.undo_label(), Some("Add rectangle"));
    // Save between coalesced edits: the next edit must not merge into the saved entry.
    s.redo().unwrap();
    s.save().unwrap();
    s.execute_coalesced("nudge", "Move annotation", |tx| {
        annot::move_annotation(tx, id, 5.0, 0.0)
    })
    .unwrap();
    assert!(s.is_dirty());
    s.undo().unwrap();
    assert!(!s.is_dirty(), "undo returns exactly to the saved state");
}

#[test]
fn reorder_rotate_save_as_reopen() {
    let (dir, p) = tmp_copy();
    let mut s = DocumentSession::open_path(&p).unwrap();
    let pages = s.pages().unwrap();
    let (a, b, c) = (pages[0].id, pages[1].id, pages[2].id);
    s.execute("Reorder and rotate", |tx| {
        pageops::move_pages(tx, &[c], 0)?;
        pageops::rotate_pages(tx, &[a], 2)
    })
    .unwrap();
    let dest = dir.path().join("copy.pdf");
    s.save_as(&dest).unwrap();
    assert_eq!(s.path.as_deref(), Some(dest.as_path()));
    assert!(!s.is_dirty());
    // The source file was not touched by Save As to a new path.
    assert_eq!(fs::read(&p).unwrap(), fixture_bytes("report-chromium.pdf"));
    let mut re = DocumentSession::open_path(&dest).unwrap();
    let rp = re.pages().unwrap();
    assert_eq!(rp.iter().map(|p| p.id).collect::<Vec<_>>(), vec![c, a, b]);
    assert_eq!(rp[1].geometry.rotate, Rotation::R180);
    assert!(!re.is_dirty());
}

#[test]
fn failed_save_keeps_the_original_intact() {
    let (dir, p) = tmp_copy();
    let original = fs::read(&p).unwrap();
    let mut s = DocumentSession::open_path(&p).unwrap();
    add_rect(&mut s);
    for inject in [
        InjectedFailure::WriteAfter(100),
        InjectedFailure::BeforeRename,
    ] {
        let err = s.save_with_injected_failure(inject).unwrap_err();
        assert!(matches!(err, EngineError::Save(_)), "{err:?}");
        assert_eq!(
            fs::read(&p).unwrap(),
            original,
            "original must be byte-identical after a failed save"
        );
        assert!(
            tmp_files(dir.path()).is_empty(),
            "temp file must be cleaned up"
        );
        assert!(s.is_dirty(), "a failed save must not clear the dirty flag");
    }
    // And a real save afterwards still works.
    s.save().unwrap();
    assert!(!s.is_dirty());
    assert!(fs::read(&p).unwrap().starts_with(&original));
}

#[test]
fn external_modification_is_detected_and_blocks_overwrite() {
    let (_d, p) = tmp_copy();
    let mut s = DocumentSession::open_path(&p).unwrap();
    add_rect(&mut s);
    // Another program rewrites the file.
    let mut other = fixture_bytes("report-chromium.pdf");
    other.extend_from_slice(b"\n% touched by another program\n");
    fs::write(&p, &other).unwrap();
    let err = s.save().unwrap_err();
    assert!(matches!(err, EngineError::ExternallyModified), "{err:?}");
    assert_eq!(
        fs::read(&p).unwrap(),
        other,
        "the other program's file is untouched"
    );
    // Save As to the same path is the explicit "overwrite anyway" route and is allowed.
    let p2 = p.clone();
    s.save_as(&p2).unwrap();
    assert!(!s.is_dirty());
}

#[test]
fn clean_save_does_not_rewrite_the_file() {
    let (_d, p) = tmp_copy();
    let mut s = DocumentSession::open_path(&p).unwrap();
    let before = fs::metadata(&p).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(30));
    s.save().unwrap();
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), before);
    // A failed transaction leaves history and dirty state untouched.
    let pages = s.pages().unwrap();
    let r = s.execute("bad", |tx| {
        pageops::delete_pages(tx, &pages.iter().map(|p| p.id).collect::<Vec<_>>())
    });
    assert!(r.is_err());
    assert!(!s.is_dirty() && !s.can_undo());
}
