//! Structural page operations. Every function runs inside a [`Tx`] so a batch is a single
//! undoable transaction.
//!
//! The page tree is flattened (inherited attributes pushed down) the first time a
//! structural change needs it, so reordering never loses `Resources`, boxes or `Rotate`
//! that were inherited from intermediate `/Pages` nodes.

use crate::doc::{PageId, PdfDocument, Tx};
use crate::error::{EngineError, Result};
use crate::geom::{Rect, Rotation};
use crate::objutil::{self, MAX_DEPTH, num_array, reference};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};
use std::collections::{BTreeMap, BTreeSet};

const INHERITABLE: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

fn pages_root(doc: &Document) -> Result<ObjectId> {
    let cat = doc
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .map_err(|_| EngineError::Malformed("missing /Root".into()))?;
    doc.get_dictionary(cat)?
        .get(b"Pages")
        .and_then(Object::as_reference)
        .map_err(|_| EngineError::Malformed("catalog has no /Pages".into()))
}

fn ordered_pages(doc: &Document) -> Result<Vec<ObjectId>> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    walk(doc, pages_root(doc)?, 0, &mut seen, &mut out);
    Ok(out)
}

fn walk(
    doc: &Document,
    id: ObjectId,
    depth: usize,
    seen: &mut BTreeSet<ObjectId>,
    out: &mut Vec<ObjectId>,
) {
    if depth > MAX_DEPTH || !seen.insert(id) {
        return;
    }
    let Some(Object::Dictionary(d)) = doc.objects.get(&id) else {
        return;
    };
    if is_pages_node(d) {
        if let Ok(kids) = d.get(b"Kids").and_then(Object::as_array) {
            for k in kids {
                if let Ok(r) = k.as_reference() {
                    walk(doc, r, depth + 1, seen, out);
                }
            }
        }
    } else {
        out.push(id);
    }
}

fn is_pages_node(d: &Dictionary) -> bool {
    d.get(b"Type").ok().and_then(|t| t.as_name().ok()) == Some(b"Pages")
        || (d.has(b"Kids") && !d.has(b"Contents") && !d.has(b"MediaBox"))
}

fn inherited_in(doc: &Document, page: ObjectId, key: &[u8]) -> Option<Object> {
    let mut cur = page;
    let mut seen = BTreeSet::new();
    for _ in 0..MAX_DEPTH {
        if !seen.insert(cur) {
            return None;
        }
        let Some(Object::Dictionary(d)) = doc.objects.get(&cur) else {
            return None;
        };
        if let Ok(v) = d.get(key) {
            return Some(v.clone());
        }
        cur = d.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

/// Make the page tree a single `/Pages` node whose kids are the page leaves, pushing
/// inherited attributes down. Returns the root id and ordered pages.
pub fn ensure_flat(tx: &mut Tx<'_>) -> Result<(ObjectId, Vec<ObjectId>)> {
    let root = pages_root(tx.doc())?;
    let pages = ordered_pages(tx.doc())?;
    let already_flat = {
        let r = tx.doc().get_dictionary(root)?;
        let kids: Vec<ObjectId> = r
            .get(b"Kids")
            .and_then(Object::as_array)
            .map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect())
            .unwrap_or_default();
        kids == pages
            && pages.iter().all(|p| {
                tx.doc()
                    .get_dictionary(*p)
                    .ok()
                    .and_then(|d| d.get(b"Parent").ok().and_then(|x| x.as_reference().ok()))
                    == Some(root)
            })
    };
    if already_flat {
        return Ok((root, pages));
    }
    // Collect inherited values first (against the unmodified tree).
    type Inherited<'a> = Vec<(&'a [u8], Object)>;
    let mut push_down: Vec<(ObjectId, Inherited<'_>)> = Vec::new();
    for p in &pages {
        let d = tx.doc().get_dictionary(*p)?;
        let mut add = Vec::new();
        for key in INHERITABLE {
            if !d.has(key)
                && let Some(v) = inherited_in(tx.doc(), *p, key)
            {
                add.push((key, v));
            }
        }
        push_down.push((*p, add));
    }
    for (p, add) in push_down {
        let d = tx.dict_mut(p)?;
        for (k, v) in add {
            d.set(k.to_vec(), v);
        }
        d.set("Parent", reference(root));
    }
    let n = pages.len() as i64;
    let rd = tx.dict_mut(root)?;
    rd.set(
        "Kids",
        Object::Array(pages.iter().map(|p| reference(*p)).collect()),
    );
    rd.set("Count", n);
    Ok((root, pages))
}

fn set_kids(tx: &mut Tx<'_>, root: ObjectId, order: &[ObjectId]) -> Result<()> {
    let rd = tx.dict_mut(root)?;
    rd.set(
        "Kids",
        Object::Array(order.iter().map(|p| reference(*p)).collect()),
    );
    rd.set("Count", order.len() as i64);
    Ok(())
}

/// Rotate pages by `quarter_turns` clockwise (negative = counter-clockwise). This changes
/// the document's page `/Rotate` (saved), unlike the view rotation which is display-only.
pub fn rotate_pages(tx: &mut Tx<'_>, pages: &[PageId], quarter_turns: i64) -> Result<()> {
    for p in pages {
        let cur = inherited_in(tx.doc(), p.0, b"Rotate")
            .and_then(|o| objutil::num(tx.doc(), &o))
            .and_then(|d| Rotation::from_degrees(d as i64))
            .unwrap_or(Rotation::R0);
        let new = cur.rotated_by(quarter_turns);
        tx.dict_mut(p.0)?.set("Rotate", new.degrees());
    }
    Ok(())
}

/// Delete pages. Refuses to delete every page.
pub fn delete_pages(tx: &mut Tx<'_>, pages: &[PageId]) -> Result<()> {
    let (root, order) = ensure_flat(tx)?;
    let del: BTreeSet<ObjectId> = pages.iter().map(|p| p.0).collect();
    let keep: Vec<ObjectId> = order.iter().copied().filter(|p| !del.contains(p)).collect();
    if keep.is_empty() {
        return Err(EngineError::InvalidArgument(
            "a document must keep at least one page".into(),
        ));
    }
    if keep.len() == order.len() {
        return Err(EngineError::InvalidArgument(
            "no matching pages to delete".into(),
        ));
    }
    set_kids(tx, root, &keep)?;
    strip_dead_destinations(tx, &del)?;
    Ok(())
}

/// Replace the page order. `new_order` must be a permutation of the current pages.
pub fn reorder_pages(tx: &mut Tx<'_>, new_order: &[PageId]) -> Result<()> {
    let (root, cur) = ensure_flat(tx)?;
    let a: BTreeSet<ObjectId> = cur.iter().copied().collect();
    let b: BTreeSet<ObjectId> = new_order.iter().map(|p| p.0).collect();
    if a != b || new_order.len() != cur.len() {
        return Err(EngineError::InvalidArgument(
            "new order is not a permutation of the existing pages".into(),
        ));
    }
    let ids: Vec<ObjectId> = new_order.iter().map(|p| p.0).collect();
    set_kids(tx, root, &ids)
}

/// Move a set of pages so the block starts at `to_index` of the *remaining* pages.
pub fn move_pages(tx: &mut Tx<'_>, pages: &[PageId], to_index: usize) -> Result<()> {
    let (_, cur) = ensure_flat(tx)?;
    let moving: BTreeSet<ObjectId> = pages.iter().map(|p| p.0).collect();
    let mut block: Vec<ObjectId> = cur.iter().copied().filter(|p| moving.contains(p)).collect();
    let mut rest: Vec<ObjectId> = cur
        .iter()
        .copied()
        .filter(|p| !moving.contains(p))
        .collect();
    let at = to_index.min(rest.len());
    let tail = rest.split_off(at);
    rest.append(&mut block);
    rest.extend(tail);
    let order: Vec<PageId> = rest.into_iter().map(PageId).collect();
    reorder_pages(tx, &order)
}

/// Insert a blank page of the given size (points) at `index`.
pub fn insert_blank_page(tx: &mut Tx<'_>, index: usize, width: f64, height: f64) -> Result<PageId> {
    if !(width.is_finite() && height.is_finite())
        || width < 1.0
        || height < 1.0
        || width > 14_400.0
        || height > 14_400.0
    {
        return Err(EngineError::InvalidArgument(
            "page size out of range (1–14400 pt)".into(),
        ));
    }
    let (root, mut order) = ensure_flat(tx)?;
    let contents = tx.add(Object::Stream(Stream::new(dictionary! {}, Vec::new())));
    let page = tx.add(Object::Dictionary(dictionary! {
        "Type" => "Page",
        "Parent" => reference(root),
        "MediaBox" => num_array(&[0.0, 0.0, width, height]),
        "Resources" => dictionary! {},
        "Contents" => contents,
    }));
    order.insert(index.min(order.len()), page);
    set_kids(tx, root, &order)?;
    Ok(PageId(page))
}

/// Set (or clear) a page's `CropBox`. Cropping changes the visible area only: content
/// outside the box remains in the file and can be recovered.
pub fn set_crop_box(tx: &mut Tx<'_>, page: PageId, crop: Option<Rect>) -> Result<()> {
    let d = tx.dict_mut(page.0)?;
    match crop {
        Some(r) => {
            let r = r.abs();
            d.set("CropBox", num_array(&[r.x0, r.y0, r.x1, r.y1]));
        }
        None => {
            d.remove(b"CropBox");
        }
    }
    Ok(())
}

/// How form fields on duplicated/imported pages are handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormPolicy {
    /// Duplicated widgets share the same field (and value).
    Linked,
    /// Duplicated widgets become new fields with collision-free names.
    Independent,
    /// Widgets are removed on the duplicate (appearance baked into the page content).
    Flatten,
}

/// Duplicate pages, inserting each copy directly after its original. Content streams and
/// annotations are deep-copied (so editing the copy never touches the original); fonts,
/// images and other resources are shared by reference — all engine edits copy-on-write them.
pub fn duplicate_pages(
    tx: &mut Tx<'_>,
    pages: &[PageId],
    policy: FormPolicy,
) -> Result<Vec<PageId>> {
    let (root, mut order) = ensure_flat(tx)?;
    let mut created = Vec::new();
    let mut dup = crate::forms::DupState::default();
    for p in pages {
        let pos = order
            .iter()
            .position(|x| *x == p.0)
            .ok_or(EngineError::NoSuchPage(p.0.0 as usize))?;
        let has_widgets = page_has_widgets(tx.doc(), p.0);
        let new = clone_page_shallow(tx, p.0, root, policy, &mut dup)?;
        if has_widgets && policy == FormPolicy::Flatten {
            crate::forms::flatten_page_widgets(tx, PageId(new))?;
        }
        order.insert(pos + 1, new);
        created.push(PageId(new));
    }
    set_kids(tx, root, &order)?;
    Ok(created)
}

fn page_has_widgets(doc: &Document, page: ObjectId) -> bool {
    let Ok(d) = doc.get_dictionary(page) else {
        return false;
    };
    objutil::dict_array(doc, d, b"Annots").is_some_and(|a| {
        a.iter().any(|o| {
            objutil::deref(doc, o)
                .and_then(|o| o.as_dict().ok())
                .is_some_and(|ad| objutil::dict_name(doc, ad, b"Subtype") == Some(b"Widget"))
        })
    })
}

fn clone_page_shallow(
    tx: &mut Tx<'_>,
    page: ObjectId,
    root: ObjectId,
    policy: FormPolicy,
    dup: &mut crate::forms::DupState,
) -> Result<ObjectId> {
    let mut d = tx.doc().get_dictionary(page)?.clone();
    d.set("Parent", reference(root));
    d.remove(b"StructParents");
    // Deep-copy content streams.
    if let Ok(c) = d.get(b"Contents").cloned() {
        let new = match c {
            Object::Reference(r) => copy_content_ref(tx, r)?,
            Object::Array(a) => Object::Array(
                a.iter()
                    .map(|o| match o.as_reference() {
                        Ok(r) => copy_content_ref(tx, r),
                        Err(_) => Ok(o.clone()),
                    })
                    .collect::<Result<Vec<_>>>()?,
            ),
            other => other,
        };
        d.set("Contents", new);
    }
    // Own copy of the Resources dictionary (resources inside remain shared).
    if let Ok(Object::Reference(r)) = d.get(b"Resources").cloned()
        && let Ok(res) = tx.doc().get_object(r).cloned()
    {
        let nr = tx.add(res);
        d.set("Resources", reference(nr));
    }
    let new_page = tx.add(Object::Dictionary(Dictionary::new())); // reserve id
    // Clone annotations.
    let annots: Vec<Object> = objutil::dict_array(tx.doc(), &d, b"Annots")
        .cloned()
        .unwrap_or_default();
    let mut new_annots = Vec::new();
    for a in annots {
        let Ok(aid) = a.as_reference() else {
            new_annots.push(a);
            continue;
        };
        let Ok(ad) = tx.doc().get_dictionary(aid).cloned() else {
            continue;
        };
        let is_widget = objutil::dict_name(tx.doc(), &ad, b"Subtype") == Some(b"Widget");
        if is_widget {
            if let Some(w) = crate::forms::duplicate_widget(tx, aid, new_page, policy, dup)? {
                new_annots.push(reference(w));
            }
            continue;
        }
        if objutil::dict_name(tx.doc(), &ad, b"Subtype") == Some(b"Popup") {
            continue; // re-created by viewers; avoids dangling /Parent
        }
        let mut nd = ad.clone();
        nd.set("P", reference(new_page));
        nd.remove(b"Popup");
        // Deep-copy the appearance so later regeneration cannot alias the original.
        let nid = tx.add(Object::Dictionary(nd));
        new_annots.push(reference(nid));
    }
    if new_annots.is_empty() {
        d.remove(b"Annots");
    } else {
        d.set("Annots", Object::Array(new_annots));
    }
    tx.set(new_page, Object::Dictionary(d));
    Ok(new_page)
}

fn copy_content_ref(tx: &mut Tx<'_>, r: ObjectId) -> Result<Object> {
    let obj = tx
        .doc()
        .get_object(r)
        .map_err(|_| EngineError::NoSuchObject(r.0, r.1))?
        .clone();
    Ok(reference(tx.add(obj)))
}

/// Remove link destinations / bookmarks that point at deleted pages so they do not keep the
/// deleted pages reachable (and so viewers do not jump to nowhere).
fn strip_dead_destinations(tx: &mut Tx<'_>, deleted: &BTreeSet<ObjectId>) -> Result<()> {
    let ids: Vec<ObjectId> = tx.doc().objects.keys().copied().collect();
    for id in ids {
        let Some(Object::Dictionary(d)) = tx.doc().objects.get(&id) else {
            continue;
        };
        let is_annot_link = objutil::dict_name(tx.doc(), d, b"Subtype") == Some(b"Link");
        let is_outline_item =
            d.has(b"Title") && (d.has(b"Parent") || d.has(b"Dest") || d.has(b"A"));
        if !is_annot_link && !is_outline_item {
            continue;
        }
        let dest_dead = |o: &Object| dest_targets(tx.doc(), o, deleted);
        let dead_dest = d.get(b"Dest").ok().is_some_and(dest_dead);
        let dead_action = d.get(b"A").ok().is_some_and(|a| {
            objutil::deref(tx.doc(), a)
                .and_then(|a| a.as_dict().ok())
                .and_then(|ad| ad.get(b"D").ok())
                .is_some_and(dest_dead)
        });
        if dead_dest || dead_action {
            let dm = tx.dict_mut(id)?;
            if dead_dest {
                dm.remove(b"Dest");
            }
            if dead_action {
                dm.remove(b"A");
            }
        }
    }
    Ok(())
}

fn dest_targets(doc: &Document, dest: &Object, deleted: &BTreeSet<ObjectId>) -> bool {
    match objutil::deref(doc, dest) {
        Some(Object::Array(a)) => a
            .first()
            .and_then(|o| o.as_reference().ok())
            .is_some_and(|r| deleted.contains(&r)),
        _ => false,
    }
}

// -------------------------------------------------------------------------------------
// Importing pages from another document (merge / extract / split)
// -------------------------------------------------------------------------------------

struct Importer<'a> {
    src: &'a Document,
    map: BTreeMap<ObjectId, ObjectId>,
    page_set: BTreeSet<ObjectId>,
    new_pages: BTreeMap<ObjectId, ObjectId>,
}

impl Importer<'_> {
    fn import_ref(&mut self, tx: &mut Tx<'_>, id: ObjectId, depth: usize) -> Result<Object> {
        if let Some(n) = self.map.get(&id) {
            return Ok(reference(*n));
        }
        if depth > 512 {
            return Err(EngineError::LimitExceeded(
                "object graph too deep to import".into(),
            ));
        }
        let Some(obj) = self.src.objects.get(&id) else {
            return Ok(Object::Null);
        };
        // Do not drag other pages or the source page tree along.
        if let Object::Dictionary(d) = obj {
            let is_page = d.get(b"Type").ok().and_then(|t| t.as_name().ok()) == Some(b"Page");
            if (is_page && !self.page_set.contains(&id)) || is_pages_node(d) {
                return Ok(Object::Null);
            }
        }
        let new = tx.add(Object::Null);
        self.map.insert(id, new);
        let converted = self.convert(tx, obj, depth + 1)?;
        tx.set(new, converted);
        Ok(reference(new))
    }

    fn convert(&mut self, tx: &mut Tx<'_>, obj: &Object, depth: usize) -> Result<Object> {
        Ok(match obj {
            Object::Reference(r) => self.import_ref(tx, *r, depth)?,
            Object::Array(a) => Object::Array(
                a.iter()
                    .map(|o| self.convert(tx, o, depth))
                    .collect::<Result<Vec<_>>>()?,
            ),
            Object::Dictionary(d) => Object::Dictionary(self.convert_dict(tx, d, depth)?),
            Object::Stream(s) => {
                let dict = self.convert_dict(tx, &s.dict, depth)?;
                let mut ns = Stream::new(dict, s.content.clone());
                ns.allows_compression = s.allows_compression;
                Object::Stream(ns)
            }
            other => other.clone(),
        })
    }

    fn convert_dict(
        &mut self,
        tx: &mut Tx<'_>,
        d: &Dictionary,
        depth: usize,
    ) -> Result<Dictionary> {
        let mut out = Dictionary::new();
        for (k, v) in d.iter() {
            if k == b"Parent" || k == b"StructParents" || k == b"StructParent" || k == b"Popup" {
                continue;
            }
            out.set(k.clone(), self.convert(tx, v, depth)?);
        }
        Ok(out)
    }
}

/// Copy pages from another document into this one at `at_index`, remapping object ids.
/// Returns the new page ids. Pages with form widgets are imported according to `policy`.
pub fn import_pages(
    tx: &mut Tx<'_>,
    src: &PdfDocument,
    src_pages: &[PageId],
    at_index: usize,
    policy: FormPolicy,
) -> Result<Vec<PageId>> {
    let sdoc = src.lopdf();
    let (root, mut order) = ensure_flat(tx)?;
    let mut imp = Importer {
        src: sdoc,
        map: BTreeMap::new(),
        page_set: src_pages.iter().map(|p| p.0).collect(),
        new_pages: BTreeMap::new(),
    };
    // Pre-allocate new page ids so cross references (annotation /P) resolve to them.
    for p in src_pages {
        let nid = tx.add(Object::Null);
        imp.map.insert(p.0, nid);
        imp.new_pages.insert(p.0, nid);
    }
    let mut result = Vec::new();
    let insert_at = at_index.min(order.len());
    for (offset, p) in src_pages.iter().enumerate() {
        let mut d = sdoc
            .get_dictionary(p.0)
            .map_err(|_| EngineError::NoSuchPage(p.0.0 as usize))?
            .clone();
        for key in INHERITABLE {
            if !d.has(key)
                && let Some(v) = inherited_in(sdoc, p.0, key)
            {
                d.set(key.to_vec(), v);
            }
        }
        let widgets = crate::forms::page_widgets(sdoc, p.0);
        if !widgets.is_empty() {
            let keep: Vec<Object> = objutil::dict_array(sdoc, &d, b"Annots")
                .map(|a| {
                    a.iter()
                        .filter(|o| o.as_reference().ok().is_none_or(|r| !widgets.contains(&r)))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            d.set("Annots", Object::Array(keep));
        }
        let mut converted = imp.convert_dict(tx, &d, 0)?;
        if !widgets.is_empty() {
            let nid = imp.new_pages.get(&p.0).copied().unwrap_or((0, 0));
            let added = crate::forms::import_widgets(tx, sdoc, &widgets, nid, policy, &mut imp)?;
            let mut annots = converted
                .get(b"Annots")
                .ok()
                .and_then(|o| o.as_array().ok())
                .cloned()
                .unwrap_or_default();
            annots.extend(added);
            converted.set("Annots", Object::Array(annots));
        }
        converted.set("Parent", reference(root));
        let nid = imp.new_pages.get(&p.0).copied().unwrap_or((0, 0));
        tx.set(nid, Object::Dictionary(converted));
        order.insert(insert_at + offset, nid);
        result.push(PageId(nid));
        if !widgets.is_empty() && policy == FormPolicy::Flatten {
            crate::forms::flatten_page_widgets(tx, PageId(nid))?;
        }
    }
    set_kids(tx, root, &order)?;
    Ok(result)
}

impl PdfDocument {
    /// Create a new empty document (one blank A4 page is *not* added; callers import pages).
    pub fn new_empty() -> Result<PdfDocument> {
        let mut doc = Document::with_version("1.7");
        let pages = doc.new_object_id();
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => Vec::<Object>::new(),
                "Count" => 0i64,
            }),
        );
        doc.trailer.set("Root", reference(cat));
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes)
            .map_err(|e| EngineError::Save(e.to_string()))?;
        PdfDocument::open(bytes, &crate::doc::OpenOptions::default())
    }
}

impl crate::forms::ImportConv for Importer<'_> {
    fn import_ref(&mut self, tx: &mut Tx<'_>, id: ObjectId) -> Result<Object> {
        Importer::import_ref(self, tx, id, 0)
    }
    fn convert(&mut self, tx: &mut Tx<'_>, obj: &Object) -> Result<Object> {
        Importer::convert(self, tx, obj, 0)
    }
}
