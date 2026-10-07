//! A document session: the single authoritative document plus transactional history.
//!
//! * Every edit is a [`pdf_engine::doc::ObjectDelta`] recorded as one history entry (a drag,
//!   a page batch, a property change — one undo step each).
//! * "Dirty" is computed against the *saved state id*, so undoing back to the saved state is
//!   clean again and redoing away from it is dirty.
//! * `revision` increases on every state change (edit, undo, redo); renderers consume
//!   revision-tagged byte snapshots and results for stale revisions are discarded.

use crate::search::SearchState;
use crate::selection::Selection;
use crate::view::ViewState;
use pdf_engine::doc::{ObjectDelta, OpenOptions, PageInfo, PdfDocument, Tx};
use pdf_engine::error::{EngineError, Result};
use pdf_engine::save::{self, FileStamp, SaveOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Process-unique document id (not reused while the process runs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocId(pub u64);

static NEXT_DOC: AtomicU64 = AtomicU64::new(1);

/// An immutable, revision-tagged copy of the document bytes for background workers.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// Owning document.
    pub doc: DocId,
    /// Revision this snapshot represents.
    pub revision: u64,
    /// Serialised document.
    pub bytes: Arc<Vec<u8>>,
}

type StateId = u64;

struct Entry {
    label: String,
    delta: ObjectDelta,
    before: StateId,
    after: StateId,
    coalesce: Option<String>,
}

/// Limits that bound history memory.
const MAX_ENTRIES: usize = 500;
const MAX_HISTORY_BYTES: usize = 256 * 1024 * 1024;

/// One open document.
pub struct DocumentSession {
    /// Process-unique id.
    pub id: DocId,
    doc: PdfDocument,
    /// File path, if the document has been saved or opened from disk.
    pub path: Option<PathBuf>,
    /// Display name for the tab.
    pub title: String,
    stamp: Option<FileStamp>,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    state: StateId,
    next_state: StateId,
    saved_state: Option<StateId>,
    revision: u64,
    snapshot: Option<Snapshot>,
    pages: Option<(u64, Arc<Vec<PageInfo>>)>,
    /// Selection (annotations, text).
    pub selection: Selection,
    /// View state (zoom, scroll, mode).
    pub view: ViewState,
    /// Search state.
    pub search: SearchState,
}

impl DocumentSession {
    fn from_doc(
        doc: PdfDocument,
        path: Option<PathBuf>,
        title: String,
        stamp: Option<FileStamp>,
    ) -> Self {
        Self {
            id: DocId(NEXT_DOC.fetch_add(1, Ordering::Relaxed)),
            doc,
            path,
            title,
            stamp,
            undo: Vec::new(),
            redo: Vec::new(),
            state: 0,
            next_state: 1,
            saved_state: Some(0),
            revision: 0,
            snapshot: None,
            pages: None,
            selection: Selection::default(),
            view: ViewState::default(),
            search: SearchState::default(),
        }
    }

    /// Open a file from disk.
    pub fn open_path(path: &Path) -> Result<Self> {
        Self::open_path_with_password(path, "")
    }

    /// Open a file from disk, unlocking it with `password` when it is password protected
    /// (`EngineError::PasswordRequired` / `WrongPassword` tell the caller to ask).
    pub fn open_path_with_password(path: &Path, password: &str) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        let stamp = FileStamp::of(path).ok();
        let doc = PdfDocument::open_with_password(bytes, &OpenOptions::default(), password)?;
        let title = path
            .file_name()
            .map_or_else(|| "Untitled".into(), |n| n.to_string_lossy().into_owned());
        Ok(Self::from_doc(doc, Some(path.to_path_buf()), title, stamp))
    }

    /// Open from memory (no file path; Save will ask for one).
    pub fn open_bytes(bytes: Vec<u8>, title: &str) -> Result<Self> {
        let doc = PdfDocument::open(bytes, &OpenOptions::default())?;
        Ok(Self::from_doc(doc, None, title.to_string(), None))
    }

    /// Wrap an engine document that has no file yet (extract/split results).
    pub fn from_new_document(doc: PdfDocument, title: &str) -> Self {
        let mut s = Self::from_doc(doc, None, title.to_string(), None);
        // A new, never-saved document starts dirty so closing prompts.
        s.saved_state = None;
        s
    }

    /// Read access to the engine document.
    pub fn doc(&self) -> &PdfDocument {
        &self.doc
    }

    /// Current revision.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Mark the current state as unsaved (e.g. a document restored from a recovery file).
    pub fn mark_unsaved(&mut self) {
        self.saved_state = None;
    }

    /// True when the current state differs from what is on disk.
    pub fn is_dirty(&self) -> bool {
        self.saved_state != Some(self.state)
    }

    /// Label of the change that undo would revert.
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|e| e.label.as_str())
    }

    /// Label of the change that redo would re-apply.
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|e| e.label.as_str())
    }

    /// Whether undo is available.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether redo is available.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn bump(&mut self) {
        self.revision += 1;
        self.snapshot = None;
        self.pages = None;
    }

    fn fresh_state(&mut self) -> StateId {
        let s = self.next_state;
        self.next_state += 1;
        s
    }

    /// Run an edit as one undoable transaction. On error nothing changes.
    pub fn execute<R>(
        &mut self,
        label: &str,
        f: impl FnOnce(&mut Tx<'_>) -> Result<R>,
    ) -> Result<R> {
        self.execute_inner(label, None, f)
    }

    /// Like [`Self::execute`], but merges into the previous entry when it has the same
    /// `key` (e.g. typing in a comment box or nudging with arrow keys → one undo step).
    pub fn execute_coalesced<R>(
        &mut self,
        key: &str,
        label: &str,
        f: impl FnOnce(&mut Tx<'_>) -> Result<R>,
    ) -> Result<R> {
        self.execute_inner(label, Some(key.to_string()), f)
    }

    fn execute_inner<R>(
        &mut self,
        label: &str,
        coalesce: Option<String>,
        f: impl FnOnce(&mut Tx<'_>) -> Result<R>,
    ) -> Result<R> {
        let (r, delta) = self.doc.transact(f)?;
        if delta.is_empty() {
            return Ok(r);
        }
        let before = self.state;
        let can_merge = coalesce.is_some()
            && self.redo.is_empty()
            && self.undo.last().is_some_and(|e| e.coalesce == coalesce && e.after == before)
            // Never merge across the saved state: undo must be able to return to it.
            && self.saved_state != Some(before);
        if can_merge {
            let new_state = self.fresh_state();
            if let Some(last) = self.undo.last_mut() {
                last.delta.merge(delta);
                last.after = new_state;
            }
            self.state = new_state;
        } else {
            let after = self.fresh_state();
            self.undo.push(Entry {
                label: label.to_string(),
                delta,
                before,
                after,
                coalesce,
            });
            self.state = after;
            self.redo.clear();
            self.trim_history();
        }
        self.bump();
        Ok(r)
    }

    fn trim_history(&mut self) {
        let mut bytes: usize = self.undo.iter().map(|e| e.delta.approx_bytes()).sum();
        while self.undo.len() > MAX_ENTRIES || (bytes > MAX_HISTORY_BYTES && self.undo.len() > 1) {
            let e = self.undo.remove(0);
            bytes = bytes.saturating_sub(e.delta.approx_bytes());
        }
    }

    /// Undo the last change. Returns its label.
    pub fn undo(&mut self) -> Option<String> {
        let e = self.undo.pop()?;
        self.doc.revert_delta(&e.delta);
        self.state = e.before;
        let label = e.label.clone();
        self.redo.push(e);
        self.bump();
        Some(label)
    }

    /// Redo the last undone change. Returns its label.
    pub fn redo(&mut self) -> Option<String> {
        let e = self.redo.pop()?;
        self.doc.apply_delta(&e.delta);
        self.state = e.after;
        let label = e.label.clone();
        self.undo.push(e);
        self.bump();
        Some(label)
    }

    /// Page list for the current revision (cached).
    pub fn pages(&mut self) -> Result<Arc<Vec<PageInfo>>> {
        if let Some((rev, p)) = &self.pages
            && *rev == self.revision
        {
            return Ok(p.clone());
        }
        let p = Arc::new(self.doc.pages()?);
        self.pages = Some((self.revision, p.clone()));
        Ok(p)
    }

    /// Byte snapshot of the current revision for background workers (cached per revision).
    pub fn snapshot(&mut self) -> Result<Snapshot> {
        if let Some(s) = &self.snapshot {
            return Ok(s.clone());
        }
        let bytes = if self.doc.has_changes_since_base() {
            self.doc.snapshot_bytes()?
        } else {
            self.doc.original_bytes().as_ref().clone()
        };
        let s = Snapshot {
            doc: self.id,
            revision: self.revision,
            bytes: Arc::new(bytes),
        };
        self.snapshot = Some(s.clone());
        Ok(s)
    }

    /// Save to the document's current path. A clean document is left untouched on disk.
    pub fn save(&mut self) -> Result<()> {
        let path = self.path.clone().ok_or_else(|| {
            EngineError::InvalidArgument("document has no file path; use Save As".into())
        })?;
        if !self.is_dirty() {
            return Ok(());
        }
        self.write_to(&path, true)
    }

    /// Save under a new path (becomes the document's path).
    pub fn save_as(&mut self, dest: &Path) -> Result<()> {
        // Save As is the explicit "overwrite anyway" route: the native dialog already asked for
        // overwrite confirmation, so the external-modification guard applies only to plain Save.
        self.write_to(dest, false)
    }

    fn write_to(&mut self, dest: &Path, check_external: bool) -> Result<()> {
        let bytes = if self.doc.has_changes_since_base() {
            self.doc.snapshot_bytes()?
        } else {
            self.doc.original_bytes().as_ref().clone()
        };
        self.write_bytes(dest, bytes, check_external)
    }

    /// `bytes` are the plain bytes of the current revision; a protected document is written encrypted again.
    fn write_bytes(&mut self, dest: &Path, bytes: Vec<u8>, check_external: bool) -> Result<()> {
        let pages = self.doc.page_count();
        let sealed = self.doc.seal(&bytes)?;
        let stamp = save::write_atomic(
            dest,
            &sealed,
            &SaveOptions {
                expected_stamp: if check_external { self.stamp } else { None },
                expected_pages: Some(pages),
                password: self.doc.protection_password().map(str::to_string),
                inject: None,
            },
        )?;
        drop(sealed);
        self.doc.rebase(bytes);
        self.saved_state = Some(self.state);
        self.stamp = Some(stamp);
        self.path = Some(dest.to_path_buf());
        self.title = dest
            .file_name()
            .map_or_else(|| self.title.clone(), |n| n.to_string_lossy().into_owned());
        Ok(())
    }

    /// Sign the document and save the signed file to `dest` (which becomes the document's path).
    ///
    /// Pending edits are part of what is signed. Signing is a milestone: the file on disk is
    /// the signed bytes and the undo history is cleared (undoing across a signature would
    /// silently diverge from the signed file).
    pub fn sign_and_save(
        &mut self,
        dest: &Path,
        identity: &pdf_engine::sign::Identity,
        opts: &pdf_engine::sign::SignOptions,
    ) -> Result<()> {
        if self.doc.protection().is_some() {
            return Err(EngineError::Unsupported(
                "Remove the password protection before signing; a signed file cannot be encrypted afterwards without breaking the signature.".into(),
            ));
        }
        let bytes = self.doc.sign(identity, opts)?;
        let check_external = self.path.as_deref() == Some(dest);
        self.write_bytes(dest, bytes, check_external)?;
        self.undo.clear();
        self.redo.clear();
        self.state = self.fresh_state();
        self.saved_state = Some(self.state);
        self.bump();
        Ok(())
    }

    /// Test hook: save with failure injection.
    #[doc(hidden)]
    pub fn save_with_injected_failure(&mut self, inject: save::InjectedFailure) -> Result<()> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| EngineError::InvalidArgument("no path".into()))?;
        let bytes = self.doc.snapshot_bytes()?;
        let sealed = self.doc.seal(&bytes)?;
        save::write_atomic(
            &path,
            &sealed,
            &SaveOptions {
                expected_stamp: self.stamp,
                expected_pages: None,
                password: self.doc.protection_password().map(str::to_string),
                inject: Some(inject),
            },
        )
        .map(|_| ())
    }

    /// Bytes suitable for crash-recovery files (current state, no side effects). A password-protected document
    /// has none: a recovery file would hold its content unencrypted.
    pub fn recovery_bytes(&mut self) -> Result<Vec<u8>> {
        if self.doc.protection().is_some() {
            return Err(EngineError::Unsupported(
                "protected documents are not written to recovery files".into(),
            ));
        }
        Ok(self.snapshot()?.bytes.as_ref().clone())
    }

    /// How the document is password protected, when it is.
    pub fn protection(&self) -> Option<pdf_engine::protect::ProtectionInfo> {
        self.doc.protection()
    }

    /// Protect the document with a password (applied when it is saved). Not an undo step.
    pub fn set_protection(
        &mut self,
        user: &str,
        owner: &str,
        rights: pdf_engine::protect::Rights,
    ) -> Result<()> {
        self.doc.set_protection(user, owner, rights)?;
        self.protection_changed();
        Ok(())
    }

    /// Remove the password protection (applied when it is saved). Not an undo step.
    pub fn remove_protection(&mut self) -> Result<()> {
        self.doc.remove_protection()?;
        self.protection_changed();
        Ok(())
    }

    /// A document opened with limited rights becomes fully editable with the owner password.
    pub fn unlock_as_owner(&mut self, password: &str) -> Result<()> {
        let unlocked = self.doc.unlock_as_owner(password, &OpenOptions::default())?;
        self.doc = unlocked;
        self.bump();
        Ok(())
    }

    /// The protection is part of what a save writes, so the document is unsaved afterwards.
    fn protection_changed(&mut self) {
        self.mark_unsaved();
        self.bump();
    }
}
