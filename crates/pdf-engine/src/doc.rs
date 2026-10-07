//! The authoritative editable document.
//!
//! [`PdfDocument`] owns the original bytes and a [`lopdf::Document`]. Every mutation goes
//! through [`PdfDocument::transact`], which records before/after images of every touched
//! object as an [`ObjectDelta`] — the unit editor-core stores for undo/redo. A failing
//! closure rolls the document back automatically.

use crate::caps::Capabilities;
use crate::error::{EngineError, Result, sanitize};
use crate::geom::{PageGeometry, Rotation};
use crate::objutil::{self, MAX_DEPTH};
use crate::protect::{self, Protection, ProtectionInfo, Rights};
use crate::serialize::{self, AppendObject, BaseInfo, TrailerInfo};
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Stable page identity: the object id of the page dictionary. It survives reordering,
/// rotation and deletion/undo, and is never reused for another page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PageId(pub ObjectId);

/// Limits applied when opening untrusted files.
#[derive(Clone, Debug)]
pub struct OpenOptions {
    /// Reject files larger than this many bytes.
    pub max_file_size: usize,
    /// Per-stream decompression cap while loading object/xref streams.
    pub max_decompressed_size: usize,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            max_file_size: 2 * 1024 * 1024 * 1024,
            max_decompressed_size: 256 * 1024 * 1024,
        }
    }
}

/// Before/after images for one committed change.
#[derive(Clone, Debug, Default)]
pub struct ObjectDelta {
    before: BTreeMap<ObjectId, Option<Object>>,
    after: BTreeMap<ObjectId, Option<Object>>,
    trailer: Option<(Dictionary, Dictionary)>,
}

impl ObjectDelta {
    /// Fold a later delta into this one so both undo as a single step (typing, nudging).
    pub fn merge(&mut self, later: ObjectDelta) {
        for (id, b) in later.before {
            self.before.entry(id).or_insert(b);
        }
        for (id, a) in later.after {
            self.after.insert(id, a);
        }
        match (&mut self.trailer, later.trailer) {
            (Some(t), Some(l)) => t.1 = l.1,
            (None, Some(l)) => self.trailer = Some(l),
            _ => {}
        }
    }

    /// Rough memory footprint, used to bound history size.
    pub fn approx_bytes(&self) -> usize {
        let sz = |o: &Option<Object>| match o {
            Some(Object::Stream(s)) => s.content.len() + 256,
            Some(_) => 256,
            None => 16,
        };
        self.before.values().map(sz).sum::<usize>() + self.after.values().map(sz).sum::<usize>()
    }

    /// Number of objects touched.
    pub fn len(&self) -> usize {
        self.before.len()
    }

    /// True when nothing changed.
    pub fn is_empty(&self) -> bool {
        self.before.is_empty() && self.trailer.is_none()
    }

    /// Ids of touched objects.
    pub fn touched(&self) -> impl Iterator<Item = ObjectId> + '_ {
        self.before.keys().copied()
    }
}

/// A transaction handle that records before-images on first mutation.
pub struct Tx<'a> {
    doc: &'a mut Document,
    before: BTreeMap<ObjectId, Option<Object>>,
    trailer_before: Option<Dictionary>,
}

impl Tx<'_> {
    /// Read-only access to the underlying document.
    pub fn doc(&self) -> &Document {
        self.doc
    }

    fn record(&mut self, id: ObjectId) {
        self.before
            .entry(id)
            .or_insert_with(|| self.doc.objects.get(&id).cloned());
    }

    /// Mutable access to an existing object (records its before-image).
    pub fn object_mut(&mut self, id: ObjectId) -> Result<&mut Object> {
        if !self.doc.objects.contains_key(&id) {
            return Err(EngineError::NoSuchObject(id.0, id.1));
        }
        self.record(id);
        self.doc
            .objects
            .get_mut(&id)
            .ok_or(EngineError::NoSuchObject(id.0, id.1))
    }

    /// Mutable access to a dictionary object (or a stream's dictionary).
    pub fn dict_mut(&mut self, id: ObjectId) -> Result<&mut Dictionary> {
        match self.object_mut(id)? {
            Object::Dictionary(d) => Ok(d),
            Object::Stream(s) => Ok(&mut s.dict),
            _ => Err(EngineError::Malformed(format!(
                "object {} {} is not a dictionary",
                id.0, id.1
            ))),
        }
    }

    /// Add a new object; returns its id.
    pub fn add(&mut self, obj: impl Into<Object>) -> ObjectId {
        let id = self.doc.add_object(obj);
        self.before.entry(id).or_insert(None);
        id
    }

    /// Replace (or create) the object at `id`.
    pub fn set(&mut self, id: ObjectId, obj: impl Into<Object>) {
        self.record(id);
        self.doc.objects.insert(id, obj.into());
        if id.0 > self.doc.max_id {
            self.doc.max_id = id.0;
        }
    }

    /// Remove an object.
    pub fn remove(&mut self, id: ObjectId) {
        self.record(id);
        self.doc.objects.remove(&id);
    }

    /// Mutable access to the trailer (records before-image).
    pub fn trailer_mut(&mut self) -> &mut Dictionary {
        if self.trailer_before.is_none() {
            self.trailer_before = Some(self.doc.trailer.clone());
        }
        &mut self.doc.trailer
    }
}

/// Basic per-page facts for UI layout.
#[derive(Clone, Debug)]
pub struct PageInfo {
    /// Stable id.
    pub id: PageId,
    /// Visible geometry.
    pub geometry: PageGeometry,
}

/// The editable PDF.
pub struct PdfDocument {
    pub(crate) doc: Document,
    original: Arc<Vec<u8>>,
    touched: BTreeSet<ObjectId>,
    trailer_touched: bool,
    base: Option<BaseInfo>,
    pub(crate) caps: Capabilities,
    /// How the file was protected; saving encrypts it again the same way (see [`crate::protect`]).
    protection: Option<Protection>,
}

impl PdfDocument {
    /// Parse a document from bytes. A password-protected file that opens with the empty password is unlocked;
    /// otherwise [`EngineError::PasswordRequired`] is returned (see [`PdfDocument::open_with_password`]).
    pub fn open(bytes: Vec<u8>, opts: &OpenOptions) -> Result<Self> {
        Self::open_with_password(bytes, opts, "")
    }

    /// Parse a document from bytes, unlocking it with `password` when it is password protected.
    ///
    /// The protected file is decrypted into a plain working copy; saving encrypts it again (see
    /// [`PdfDocument::seal`]). [`EngineError::WrongPassword`] when the password does not fit.
    pub fn open_with_password(bytes: Vec<u8>, opts: &OpenOptions, password: &str) -> Result<Self> {
        if bytes.len() > opts.max_file_size {
            return Err(EngineError::LimitExceeded(format!(
                "file is {} bytes, limit is {}",
                bytes.len(),
                opts.max_file_size
            )));
        }
        let (bytes, protection) =
            match protect::unlock(&bytes, opts.max_decompressed_size, password) {
                Ok(Some((plain, protection))) => (plain, Some(protection)),
                Ok(None) => (bytes, None),
                Err(e) => return Err(e),
            };
        let load = lopdf::LoadOptions {
            max_decompressed_size: Some(opts.max_decompressed_size),
            ..Default::default()
        };
        let doc = Document::load_mem_with_options(&bytes, load)
            .map_err(|e| EngineError::Parse(sanitize(&e.to_string())))?;
        let base = serialize::find_base_info(&bytes);
        let mut caps = Capabilities::detect(&doc, &bytes, protection.is_some(), base.is_some());
        if let Some(p) = &protection {
            caps.apply_protection(&p.info());
        }
        Ok(Self {
            doc,
            original: Arc::new(bytes),
            touched: BTreeSet::new(),
            trailer_touched: false,
            base,
            caps,
            protection,
        })
    }

    /// How the document is protected, when it is.
    pub fn protection(&self) -> Option<ProtectionInfo> {
        self.protection.as_ref().map(Protection::info)
    }

    /// The password the document was unlocked with (for checking a saved file), when protected.
    pub fn protection_password(&self) -> Option<&str> {
        self.protection.as_ref().map(Protection::password)
    }

    /// Unlock a document that was opened with limited rights by supplying the owner password. Returns a new
    /// document (the caller replaces the old one; nothing may have been edited, since editing was blocked).
    pub fn unlock_as_owner(&self, password: &str, opts: &OpenOptions) -> Result<PdfDocument> {
        let source = self
            .protection
            .as_ref()
            .and_then(Protection::source)
            .ok_or_else(|| EngineError::InvalidArgument("the document is not protected".into()))?;
        let doc = Self::open_with_password(source.as_ref().clone(), opts, password)?;
        if doc.protection().is_some_and(|p| p.owner) {
            Ok(doc)
        } else {
            Err(EngineError::WrongPassword)
        }
    }

    /// Encrypt `plain` (the bytes of this document's current revision) the way the document is protected, or
    /// return it unchanged when it is not. Every file written from this document goes through here.
    pub fn seal<'a>(&self, plain: &'a [u8]) -> Result<std::borrow::Cow<'a, [u8]>> {
        match &self.protection {
            Some(p) => Ok(std::borrow::Cow::Owned(p.seal(plain)?)),
            None => Ok(std::borrow::Cow::Borrowed(plain)),
        }
    }

    /// Protect the document with a password (AES 256). `owner` may be empty (then the user password is also the
    /// owner password). The next save writes the encrypted file.
    pub fn set_protection(&mut self, user: &str, owner: &str, rights: Rights) -> Result<()> {
        if self.restricted() {
            return Err(EngineError::Unsupported(
                "only the owner can change the protection".into(),
            ));
        }
        let owner = if owner.is_empty() { user } else { owner };
        self.protection = Some(Protection::new_aes256(user, owner, rights)?);
        if let Some(info) = self.protection() {
            self.caps.apply_protection(&info);
        }
        Ok(())
    }

    /// Remove the password protection; the next save writes a plain file. Only the owner may do this.
    pub fn remove_protection(&mut self) -> Result<()> {
        if self.restricted() {
            return Err(EngineError::Unsupported(
                "only the owner can remove the protection".into(),
            ));
        }
        self.protection = None;
        self.caps.encrypted = false;
        Ok(())
    }

    /// Opened with the user password and limited rights.
    fn restricted(&self) -> bool {
        self.protection().is_some_and(|p| !p.owner)
    }

    /// Capabilities and warnings detected at open time.
    pub fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    /// Original bytes of the base revision.
    pub fn original_bytes(&self) -> &Arc<Vec<u8>> {
        &self.original
    }

    /// Whether any object has been touched since the base bytes were established.
    pub fn has_changes_since_base(&self) -> bool {
        !self.touched.is_empty() || self.trailer_touched
    }

    /// Run `f` as one atomic transaction. On error the document is rolled back.
    pub fn transact<R>(
        &mut self,
        f: impl FnOnce(&mut Tx<'_>) -> Result<R>,
    ) -> Result<(R, ObjectDelta)> {
        if !self.caps.can_edit {
            return Err(EngineError::Unsupported(self.caps.edit_blocker_summary()));
        }
        let max_before = self.doc.max_id;
        let mut tx = Tx {
            doc: &mut self.doc,
            before: BTreeMap::new(),
            trailer_before: None,
        };
        let result = f(&mut tx);
        let Tx {
            before,
            trailer_before,
            ..
        } = tx;
        match result {
            Ok(r) => {
                let mut after = BTreeMap::new();
                for id in before.keys() {
                    after.insert(*id, self.doc.objects.get(id).cloned());
                }
                let trailer = trailer_before.map(|b| (b, self.doc.trailer.clone()));
                self.touched.extend(before.keys().copied());
                self.trailer_touched |= trailer.is_some();
                Ok((
                    r,
                    ObjectDelta {
                        before,
                        after,
                        trailer,
                    },
                ))
            }
            Err(e) => {
                for (id, b) in before {
                    match b {
                        Some(o) => {
                            self.doc.objects.insert(id, o);
                        }
                        None => {
                            self.doc.objects.remove(&id);
                        }
                    }
                }
                if let Some(t) = trailer_before {
                    self.doc.trailer = t;
                }
                self.doc.max_id = max_before.max(self.doc.max_id.min(max_before));
                Err(e)
            }
        }
    }

    /// Re-apply a delta (redo).
    pub fn apply_delta(&mut self, d: &ObjectDelta) {
        self.apply_side(&d.after, d.trailer.as_ref().map(|t| &t.1));
        self.touched.extend(d.before.keys().copied());
        self.trailer_touched |= d.trailer.is_some();
    }

    /// Revert a delta (undo).
    pub fn revert_delta(&mut self, d: &ObjectDelta) {
        self.apply_side(&d.before, d.trailer.as_ref().map(|t| &t.0));
        self.touched.extend(d.before.keys().copied());
        self.trailer_touched |= d.trailer.is_some();
    }

    fn apply_side(
        &mut self,
        side: &BTreeMap<ObjectId, Option<Object>>,
        trailer: Option<&Dictionary>,
    ) {
        for (id, o) in side {
            match o {
                Some(o) => {
                    self.doc.objects.insert(*id, o.clone());
                    if id.0 > self.doc.max_id {
                        self.doc.max_id = id.0;
                    }
                }
                None => {
                    self.doc.objects.remove(id);
                }
            }
        }
        if let Some(t) = trailer {
            self.doc.trailer = t.clone();
        }
    }

    /// Read-only access for engine modules.
    pub fn lopdf(&self) -> &Document {
        &self.doc
    }

    /// Untracked mutable access (for engine code that must reconcile the in-memory graph with
    /// bytes it produced itself, e.g. after patching a signature). Changes are not journaled.
    pub(crate) fn lopdf_mut(&mut self) -> &mut Document {
        &mut self.doc
    }

    // ---- pages -------------------------------------------------------------------

    fn catalog_id(&self) -> Result<ObjectId> {
        self.doc
            .trailer
            .get(b"Root")
            .and_then(Object::as_reference)
            .map_err(|_| EngineError::Malformed("missing /Root".into()))
    }

    /// Root `/Pages` node id.
    pub fn pages_root(&self) -> Result<ObjectId> {
        let cat = self.catalog_id()?;
        let d = self.doc.get_dictionary(cat)?;
        d.get(b"Pages")
            .and_then(Object::as_reference)
            .map_err(|_| EngineError::Malformed("catalog has no /Pages".into()))
    }

    /// Leaf page ids in document order. Cycle- and depth-safe.
    pub fn page_ids(&self) -> Result<Vec<PageId>> {
        let root = self.pages_root()?;
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        self.walk(root, 0, &mut seen, &mut out);
        Ok(out)
    }

    fn walk(
        &self,
        id: ObjectId,
        depth: usize,
        seen: &mut BTreeSet<ObjectId>,
        out: &mut Vec<PageId>,
    ) {
        if depth > MAX_DEPTH || !seen.insert(id) {
            return;
        }
        let Some(Object::Dictionary(d)) = self.doc.objects.get(&id) else {
            return;
        };
        let is_pages = d.get(b"Type").ok().and_then(|t| t.as_name().ok()) == Some(b"Pages")
            || (d.has(b"Kids") && !d.has(b"Contents") && !d.has(b"MediaBox"));
        if is_pages {
            if let Ok(kids) = d.get(b"Kids").and_then(Object::as_array) {
                for k in kids {
                    if let Ok(kid) = k.as_reference() {
                        self.walk(kid, depth + 1, seen, out);
                    }
                }
            }
        } else {
            out.push(PageId(id));
        }
    }

    /// Number of pages.
    pub fn page_count(&self) -> usize {
        self.page_ids().map(|p| p.len()).unwrap_or(0)
    }

    /// Look up an inheritable page attribute.
    pub fn inherited(&self, page: PageId, key: &[u8]) -> Option<&Object> {
        let mut cur = page.0;
        let mut seen = BTreeSet::new();
        for _ in 0..MAX_DEPTH {
            if !seen.insert(cur) {
                return None;
            }
            let Some(Object::Dictionary(d)) = self.doc.objects.get(&cur) else {
                return None;
            };
            if let Ok(v) = d.get(key) {
                return Some(v);
            }
            cur = d.get(b"Parent").ok()?.as_reference().ok()?;
        }
        None
    }

    /// Page dictionary.
    pub fn page_dict(&self, page: PageId) -> Result<&Dictionary> {
        self.doc
            .get_dictionary(page.0)
            .map_err(|_| EngineError::NoSuchObject(page.0.0, page.0.1))
    }

    /// Geometry (boxes, rotation, user unit) for a page.
    pub fn page_geometry(&self, page: PageId) -> Result<PageGeometry> {
        self.page_dict(page)?;
        let media = self
            .inherited(page, b"MediaBox")
            .and_then(|o| objutil::rect(&self.doc, o));
        let crop = self
            .inherited(page, b"CropBox")
            .and_then(|o| objutil::rect(&self.doc, o));
        let rot = self
            .inherited(page, b"Rotate")
            .and_then(|o| objutil::num(&self.doc, o))
            .and_then(|d| Rotation::from_degrees(d as i64))
            .unwrap_or(Rotation::R0);
        let uu = self
            .page_dict(page)?
            .get(b"UserUnit")
            .ok()
            .and_then(|o| objutil::num(&self.doc, o))
            .unwrap_or(1.0);
        Ok(PageGeometry::new(media, crop, rot, uu))
    }

    /// All pages with geometry.
    pub fn pages(&self) -> Result<Vec<PageInfo>> {
        self.page_ids()?
            .into_iter()
            .map(|id| {
                Ok(PageInfo {
                    id,
                    geometry: self.page_geometry(id)?,
                })
            })
            .collect()
    }

    /// Zero-based index of a page in the current order.
    pub fn page_index(&self, page: PageId) -> Option<usize> {
        self.page_ids().ok()?.iter().position(|p| *p == page)
    }

    // ---- serialisation ---------------------------------------------------------

    /// Bytes of the current revision as an incremental update over the base bytes.
    /// Falls back to a full rewrite when the base file cannot be extended safely.
    pub fn snapshot_bytes(&mut self) -> Result<Vec<u8>> {
        if !self.has_changes_since_base() {
            return Ok(self.original.as_ref().clone());
        }
        match self.base {
            Some(base) => self.incremental_bytes(base),
            None => self.rewrite_bytes(),
        }
    }

    /// Whether `snapshot_bytes` can append (preserving the original bytes and signatures).
    pub fn can_save_incrementally(&self) -> bool {
        self.base.is_some()
    }

    fn incremental_bytes(&self, base: BaseInfo) -> Result<Vec<u8>> {
        let objs: Vec<AppendObject<'_>> = self
            .touched
            .iter()
            .filter_map(|id| {
                self.doc
                    .objects
                    .get(id)
                    .map(|obj| AppendObject { id: *id, obj })
            })
            .collect();
        let root = self.catalog_id()?;
        let info = self
            .doc
            .trailer
            .get(b"Info")
            .and_then(Object::as_reference)
            .ok();
        let id = self.doc.trailer.get(b"ID").ok();
        let max_id = self
            .doc
            .max_id
            .max(self.doc.objects.keys().map(|k| k.0).max().unwrap_or(0));
        Ok(serialize::append_incremental(
            &self.original,
            base,
            &objs,
            &TrailerInfo {
                root,
                info,
                id,
                max_id,
            },
        ))
    }

    /// Full rewrite via lopdf (drops old revisions and unreferenced objects).
    pub fn rewrite_bytes(&mut self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.doc
            .save_to(&mut out)
            .map_err(|e| EngineError::Save(sanitize(&e.to_string())))?;
        Ok(out)
    }

    /// After content was *removed* on purpose (redaction): forget every object nothing refers to any more, and make
    /// the next save a complete rewrite of the file, so no earlier revision (which still holds the removed
    /// content) is carried along. Not undoable.
    pub fn discard_unreferenced_and_rewrite_on_save(&mut self) {
        self.doc.prune_objects();
        self.base = None;
    }

    /// Establish `bytes` (just written to disk) as the new base revision.
    pub fn rebase(&mut self, bytes: Vec<u8>) {
        self.base = serialize::find_base_info(&bytes);
        self.original = Arc::new(bytes);
        self.touched.clear();
        self.trailer_touched = false;
    }
}
