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
        .filter(|n| n.ends_with(".berg-tmp"))
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

#[test]
fn sign_and_save_signs_pending_edits_clears_history_and_survives_reopen() {
    use pdf_engine::sign::{Identity, SignOptions, SignatureStatus, list_signatures};
    let (_d, path) = tmp_copy();
    let mut s = DocumentSession::open_path(&path).unwrap();
    add_rect(&mut s);
    assert!(s.is_dirty() && s.can_undo());
    let p12 = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../pdf-sign/tests/fixtures/rsa-aes.p12");
    let id = Identity::from_pkcs12(&fs::read(p12).unwrap(), "test123").unwrap();
    s.sign_and_save(&path, &id, &SignOptions::default())
        .unwrap();
    assert!(!s.is_dirty(), "the signed file is what is on disk");
    assert!(
        !s.can_undo() && !s.can_redo(),
        "history must not cross a signature"
    );
    // Reopen from disk: the edit and a valid signature are both there.
    let again = DocumentSession::open_path(&path).unwrap();
    let sigs = list_signatures(again.doc());
    assert_eq!(sigs.len(), 1);
    assert_eq!(sigs[0].status, SignatureStatus::IntegrityOk);
    let page = again.doc().page_ids().unwrap()[0];
    let markup = annot::read_annotations(again.doc(), page)
        .into_iter()
        .filter(|a| a.subtype != "Widget")
        .count();
    assert_eq!(
        markup, 1,
        "the edit made before signing is part of the signed file"
    );
    // Saving again appends and keeps the first signature intact.
    let mut s = again;
    add_rect(&mut s);
    s.save().unwrap();
    let third = DocumentSession::open_path(&path).unwrap();
    assert_eq!(
        list_signatures(third.doc())[0].status,
        SignatureStatus::IntegrityOk
    );
}

#[test]
fn a_failed_signing_destination_leaves_the_original_untouched() {
    use pdf_engine::sign::{Identity, SignOptions};
    let (d, path) = tmp_copy();
    let before = fs::read(&path).unwrap();
    let mut s = DocumentSession::open_path(&path).unwrap();
    let p12 = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../pdf-sign/tests/fixtures/rsa-aes.p12");
    let id = Identity::from_pkcs12(&fs::read(p12).unwrap(), "test123").unwrap();
    let bad = d.path().join("no-such-dir/out.pdf");
    assert!(s.sign_and_save(&bad, &id, &SignOptions::default()).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
}

// ---- password protection ---------------------------------------------------------------------

/// A protected copy of the fixture on disk (AES-256, separate user and owner passwords).
fn protected_copy(rights: pdf_engine::protect::Rights) -> (tempfile::TempDir, std::path::PathBuf) {
    use pdf_engine::doc::{OpenOptions, PdfDocument};
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("secret.pdf");
    let mut doc =
        PdfDocument::open(fixture_bytes("report-chromium.pdf"), &OpenOptions::default()).unwrap();
    doc.set_protection("reader", "chief", rights).unwrap();
    let plain = doc.snapshot_bytes().unwrap();
    fs::write(&p, doc.seal(&plain).unwrap()).unwrap();
    (dir, p)
}

#[test]
fn a_protected_file_asks_for_its_password_and_saves_protected_again() {
    let (dir, p) = protected_copy(pdf_engine::protect::Rights::ALL);
    assert!(matches!(
        DocumentSession::open_path(&p),
        Err(EngineError::PasswordRequired)
    ));
    assert!(matches!(
        DocumentSession::open_path_with_password(&p, "nope"),
        Err(EngineError::WrongPassword)
    ));
    let mut s = DocumentSession::open_path_with_password(&p, "reader").unwrap();
    assert!(s.protection().is_some());
    add_rect(&mut s);
    s.save().unwrap();
    assert!(tmp_files(dir.path()).is_empty());

    // On disk it is still encrypted, and the edit is in it.
    let on_disk = fs::read(&p).unwrap();
    assert!(on_disk.windows(8).any(|w| w == b"/Encrypt"));
    assert!(matches!(
        DocumentSession::open_path(&p),
        Err(EngineError::PasswordRequired)
    ));
    let mut again = DocumentSession::open_path_with_password(&p, "chief").unwrap();
    let page = again.pages().unwrap()[0].id;
    let annots = pdf_engine::annot::read_annotations(again.doc(), page);
    assert_eq!(annots.len(), 1, "the rectangle was saved");
    // A second save in the same session keeps working (the base was rebased on plain bytes).
    add_rect(&mut s);
    s.save().unwrap();
    let third = DocumentSession::open_path_with_password(&p, "reader").unwrap();
    assert_eq!(third.doc().page_count(), s.doc().page_count());
}

#[test]
fn protected_documents_are_never_written_to_recovery_files_or_signed() {
    let (_d, p) = protected_copy(pdf_engine::protect::Rights::ALL);
    let mut s = DocumentSession::open_path_with_password(&p, "reader").unwrap();
    add_rect(&mut s);
    assert!(s.recovery_bytes().is_err());
    let (_d2, p2) = tmp_copy();
    let mut plain = DocumentSession::open_path(&p2).unwrap();
    assert!(plain.recovery_bytes().is_ok());
}

#[test]
fn restricted_documents_are_read_only_until_the_owner_password_is_given() {
    let rights = pdf_engine::protect::Rights {
        modify: false,
        ..pdf_engine::protect::Rights::ALL
    };
    let (_d, p) = protected_copy(rights);
    let mut s = DocumentSession::open_path_with_password(&p, "reader").unwrap();
    assert!(!s.doc().capabilities().can_edit);
    assert!(
        s.execute("x", |tx| {
            let page = pdf_engine::doc::PageId(tx.doc().get_pages()[&1]);
            pageops::rotate_pages(tx, &[page], 1)
        })
        .is_err()
    );
    assert!(matches!(
        s.unlock_as_owner("reader"),
        Err(EngineError::WrongPassword)
    ));
    let rev = s.revision();
    s.unlock_as_owner("chief").unwrap();
    assert!(s.revision() > rev);
    assert!(s.doc().capabilities().can_edit);
    add_rect(&mut s);
    s.save().unwrap();
}

#[test]
fn protection_is_added_and_removed_through_the_session() {
    let (_d, p) = tmp_copy();
    let mut s = DocumentSession::open_path(&p).unwrap();
    assert!(!s.is_dirty());
    s.set_protection("pw", "", pdf_engine::protect::Rights::ALL)
        .unwrap();
    assert!(s.is_dirty(), "the protection is written by the next save");
    s.save().unwrap();
    assert!(matches!(
        DocumentSession::open_path(&p),
        Err(EngineError::PasswordRequired)
    ));
    let mut s = DocumentSession::open_path_with_password(&p, "pw").unwrap();
    s.remove_protection().unwrap();
    assert!(s.is_dirty());
    s.save().unwrap();
    assert!(DocumentSession::open_path(&p).is_ok(), "plain again");
}

// ---- redaction ------------------------------------------------------------------------------

#[test]
fn redaction_marks_are_undoable_but_applying_them_is_final_and_saved_as_a_clean_file() {
    use pdf_engine::geom::Rect as GRect;
    use pdf_engine::redact::RedactOptions;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("doc.pdf");
    fs::write(&p, test_support::fixtures::helvetica_lines()).unwrap();
    let mut s = DocumentSession::open_path(&p).unwrap();
    let page = s.pages().unwrap()[0].id;
    let rect = GRect::new(70.0, 676.0, 300.0, 696.0);
    s.mark_for_redaction(page, &[rect]).unwrap();
    assert_eq!(s.redaction_marks().len(), 1);
    assert_eq!(s.undo_label(), Some("Mark for redaction"));
    s.undo();
    assert!(s.redaction_marks().is_empty());
    s.redo();
    assert_eq!(s.redaction_marks().len(), 1);

    let report = s.apply_redactions(&RedactOptions::default()).unwrap();
    assert_eq!(report.words_removed, 4);
    assert!(s.redaction_marks().is_empty());
    assert!(!s.can_undo() && !s.can_redo(), "no way back to the removed text");
    assert!(s.is_dirty());

    s.save().unwrap();
    let on_disk = fs::read(&p).unwrap();
    let has = |needle: &str| on_disk.windows(needle.len()).any(|w| w == needle.as_bytes());
    assert!(!has("Second line"), "the removed text is in the saved file");
    assert_eq!(
        on_disk.windows(5).filter(|w| w == b"%%EOF").count(),
        1,
        "the file holds one revision only"
    );
    // The session goes on working on the saved file.
    assert!(!s.is_dirty());
    add_rect(&mut s);
    s.save().unwrap();
    let again = DocumentSession::open_path(&p).unwrap();
    assert_eq!(again.doc().page_count(), 1);
}

#[test]
fn a_protected_document_stays_protected_after_redaction() {
    use pdf_engine::geom::Rect as GRect;
    use pdf_engine::redact::RedactOptions;
    let (_d, p) = protected_copy(pdf_engine::protect::Rights::ALL);
    let mut s = DocumentSession::open_path_with_password(&p, "reader").unwrap();
    let page = s.pages().unwrap()[0].id;
    s.mark_for_redaction(page, &[GRect::new(50.0, 600.0, 400.0, 760.0)])
        .unwrap();
    s.apply_redactions(&RedactOptions::default()).unwrap();
    s.save().unwrap();
    let bytes = fs::read(&p).unwrap();
    assert!(bytes.windows(8).any(|w| w == b"/Encrypt"));
    assert!(DocumentSession::open_path_with_password(&p, "reader").is_ok());
}
