//! Editing bookmarks (the document outline).
//!
//! [`read_outline`] gives the whole tree as plain values; the caller changes it (add, rename,
//! delete, move, indent, re-target) and [`write_outline`] writes it back as a fresh outline: new
//! item objects linked with `First`/`Last`/`Prev`/`Next`/`Parent` and correct `Count`s. An item
//! whose destination was not changed keeps its original `/Dest` or `/A` object exactly (zoom,
//! named destinations, links to other files), so editing one title never disturbs the others.
//!
//! Only navigation is written: new destinations are `[page /XYZ null top null]`. Actions are never
//! created and existing ones are only carried over, never run (see [`crate::nav`]).

use crate::doc::{PdfDocument, Tx};
use crate::error::{EngineError, Result};
use crate::nav::MAX_BOOKMARKS;
use crate::objutil::{self, reference, text_obj};
use lopdf::{Dictionary, Document, Object, ObjectId, dictionary};
use std::collections::BTreeSet;

/// Longest bookmark title written.
pub const MAX_TITLE: usize = 500;

/// One bookmark.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineItem {
    /// Shown text.
    pub title: String,
    /// Target page (0-based), when it is a page of this document.
    pub page: Option<usize>,
    /// Top of the view on that page in user space (`None` = keep the reader's position).
    pub top: Option<f64>,
    /// The original `/Dest` or `/A` entry, kept while the target is unchanged.
    pub original: Option<(Vec<u8>, Object)>,
    /// Sub-bookmarks.
    pub children: Vec<OutlineItem>,
    /// Shown expanded.
    pub open: bool,
    /// Text colour (`/C`).
    pub color: Option<[f32; 3]>,
    /// `/F` flags: 1 = italic, 2 = bold.
    pub flags: i64,
}

impl OutlineItem {
    /// A new bookmark to a page.
    pub fn new(title: &str, page: usize, top: Option<f64>) -> Self {
        Self {
            title: title.to_string(),
            page: Some(page),
            top,
            original: None,
            children: Vec::new(),
            open: true,
            color: None,
            flags: 0,
        }
    }

    /// Point the bookmark at another page (drops the original destination).
    pub fn retarget(&mut self, page: usize, top: Option<f64>) {
        self.page = Some(page);
        self.top = top;
        self.original = None;
    }
}

fn root_of(d: &Document) -> Option<&Dictionary> {
    let cat = d
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .ok()
        .and_then(|id| d.get_dictionary(id).ok())?;
    objutil::dict_dict(d, cat, b"Outlines")
}

/// The whole bookmark tree.
pub fn read_outline(doc: &PdfDocument) -> Vec<OutlineItem> {
    let d = doc.lopdf();
    let Some(root) = root_of(d) else {
        return Vec::new();
    };
    // The page of each item comes from the reader in `nav` (it resolves named destinations too);
    // both walk the same tree in the same order.
    let pages = crate::nav::outlines(doc);
    let mut seen = BTreeSet::new();
    let mut count = 0;
    read_level(d, root, &pages, 0, &mut seen, &mut count)
}

fn read_level(
    d: &Document,
    parent: &Dictionary,
    known: &[crate::nav::Bookmark],
    depth: usize,
    seen: &mut BTreeSet<ObjectId>,
    count: &mut usize,
) -> Vec<OutlineItem> {
    let mut out = Vec::new();
    if depth > 32 {
        return out;
    }
    let mut cur = parent
        .get(b"First")
        .ok()
        .and_then(|o| o.as_reference().ok());
    let mut n = 0;
    while let Some(id) = cur {
        if !seen.insert(id) || *count >= MAX_BOOKMARKS {
            break;
        }
        *count += 1;
        let Ok(item) = d.get_dictionary(id) else {
            break;
        };
        let info = known.get(n);
        let title = info.map(|b| b.title.clone()).unwrap_or_default();
        let original = ["Dest", "A"].iter().find_map(|k| {
            item.get(k.as_bytes())
                .ok()
                .map(|o| (k.as_bytes().to_vec(), o.clone()))
        });
        let color = objutil::dict_array(d, item, b"C").and_then(|a| {
            let v: Vec<f32> = a
                .iter()
                .filter_map(|o| objutil::num(d, o))
                .map(|x| x as f32)
                .collect();
            (v.len() == 3).then(|| [v[0], v[1], v[2]])
        });
        let flags = objutil::dict_num(d, item, b"F").map_or(0, |f| f as i64 & 3);
        let children = read_level(
            d,
            item,
            info.map(|b| b.children.as_slice()).unwrap_or(&[]),
            depth + 1,
            seen,
            count,
        );
        out.push(OutlineItem {
            title,
            page: info.and_then(|b| b.page),
            top: None,
            original,
            children,
            open: objutil::dict_num(d, item, b"Count").is_some_and(|c| c > 0.0),
            color,
            flags,
        });
        n += 1;
        cur = item.get(b"Next").ok().and_then(|o| o.as_reference().ok());
    }
    out
}

fn count_items(items: &[OutlineItem]) -> usize {
    items.iter().map(|i| 1 + count_items(&i.children)).sum()
}

/// Items visible below an item when it is open.
fn visible_below(item: &OutlineItem) -> i64 {
    item.children
        .iter()
        .map(|c| 1 + if c.open { visible_below(c) } else { 0 })
        .sum()
}

/// Replace the document's bookmarks with `items`. An empty list removes the outline.
pub fn write_outline(tx: &mut Tx<'_>, items: &[OutlineItem]) -> Result<()> {
    let cat = tx
        .doc()
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .map_err(|_| EngineError::Malformed("missing /Root".into()))?;
    if items.is_empty() {
        tx.dict_mut(cat)?.remove(b"Outlines");
        return Ok(());
    }
    if count_items(items) > MAX_BOOKMARKS {
        return Err(EngineError::LimitExceeded("too many bookmarks".into()));
    }
    let pages = crate::pageops::page_order(tx.doc())?;
    let root = tx.add(Object::Dictionary(dictionary! { "Type" => "Outlines" }));
    let (first, last) = write_level(tx, root, items, &pages, 0)?;
    let top_count: i64 = items
        .iter()
        .map(|i| 1 + if i.open { visible_below(i) } else { 0 })
        .sum();
    let rd = tx.dict_mut(root)?;
    rd.set("First", reference(first));
    rd.set("Last", reference(last));
    rd.set("Count", top_count);
    tx.dict_mut(cat)?.set("Outlines", reference(root));
    Ok(())
}

fn write_level(
    tx: &mut Tx<'_>,
    parent: ObjectId,
    items: &[OutlineItem],
    pages: &[crate::doc::PageId],
    depth: usize,
) -> Result<(ObjectId, ObjectId)> {
    if depth > 32 {
        return Err(EngineError::LimitExceeded(
            "bookmarks nested too deeply".into(),
        ));
    }
    let ids: Vec<ObjectId> = items.iter().map(|_| tx.add(Object::Null)).collect();
    for (i, item) in items.iter().enumerate() {
        let mut d = Dictionary::new();
        let title: String = item
            .title
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_TITLE)
            .collect();
        d.set("Title", text_obj(&title));
        d.set("Parent", reference(parent));
        if i > 0 {
            d.set("Prev", reference(ids[i - 1]));
        }
        if let Some(n) = ids.get(i + 1) {
            d.set("Next", reference(*n));
        }
        match (&item.original, item.page.and_then(|p| pages.get(p))) {
            (Some((k, o)), _) => d.set(k.clone(), o.clone()),
            (None, Some(p)) => d.set(
                "Dest",
                Object::Array(vec![
                    reference(p.0),
                    Object::Name(b"XYZ".to_vec()),
                    Object::Null,
                    item.top.map_or(Object::Null, objutil::real),
                    Object::Null,
                ]),
            ),
            (None, None) => {}
        }
        if let Some(c) = item.color {
            d.set("C", objutil::num_array(&c.map(f64::from)));
        }
        if item.flags & 3 != 0 {
            d.set("F", item.flags & 3);
        }
        if !item.children.is_empty() {
            let (f, l) = write_level(tx, ids[i], &item.children, pages, depth + 1)?;
            d.set("First", reference(f));
            d.set("Last", reference(l));
            let below = visible_below(item);
            d.set("Count", if item.open { below } else { -below });
        }
        tx.set(ids[i], Object::Dictionary(d));
    }
    Ok((ids[0], ids[ids.len() - 1]))
}

/// The item at `path` (indexes from the top level down).
pub fn get<'a>(items: &'a [OutlineItem], path: &[usize]) -> Option<&'a OutlineItem> {
    let (first, rest) = path.split_first()?;
    let item = items.get(*first)?;
    if rest.is_empty() {
        Some(item)
    } else {
        get(&item.children, rest)
    }
}

/// The item at `path`, mutable.
pub fn get_mut<'a>(items: &'a mut [OutlineItem], path: &[usize]) -> Option<&'a mut OutlineItem> {
    let (first, rest) = path.split_first()?;
    let item = items.get_mut(*first)?;
    if rest.is_empty() {
        Some(item)
    } else {
        get_mut(&mut item.children, rest)
    }
}

fn siblings_mut<'a>(
    items: &'a mut Vec<OutlineItem>,
    parent: &[usize],
) -> Option<&'a mut Vec<OutlineItem>> {
    if parent.is_empty() {
        Some(items)
    } else {
        get_mut(items, parent).map(|i| &mut i.children)
    }
}

/// Insert `item` right after `after` (at the same level), or at the end when `after` is `None`.
/// Returns the new item's path.
pub fn insert_after(
    items: &mut Vec<OutlineItem>,
    after: Option<&[usize]>,
    item: OutlineItem,
) -> Vec<usize> {
    match after {
        Some(p) if !p.is_empty() => {
            let (parent, last) = p.split_at(p.len() - 1);
            if let Some(sib) = siblings_mut(items, parent) {
                let at = (last[0] + 1).min(sib.len());
                sib.insert(at, item);
                let mut path = parent.to_vec();
                path.push(at);
                return path;
            }
            items.push(item);
            vec![items.len() - 1]
        }
        _ => {
            items.push(item);
            vec![items.len() - 1]
        }
    }
}

/// Add `item` as the last child of `parent`. Returns its path.
pub fn insert_child(
    items: &mut [OutlineItem],
    parent: &[usize],
    item: OutlineItem,
) -> Option<Vec<usize>> {
    let p = get_mut(items, parent)?;
    p.children.push(item);
    p.open = true;
    let mut path = parent.to_vec();
    path.push(p.children.len() - 1);
    Some(path)
}

/// Remove the item at `path` (with its children).
pub fn remove(items: &mut Vec<OutlineItem>, path: &[usize]) -> Option<OutlineItem> {
    let (parent, last) = path.split_at(path.len().checked_sub(1)?);
    let sib = siblings_mut(items, parent)?;
    (last[0] < sib.len()).then(|| sib.remove(last[0]))
}

/// Move the item one place up (`-1`) or down (`1`) among its siblings. Returns the new path.
pub fn move_by(items: &mut Vec<OutlineItem>, path: &[usize], delta: i64) -> Option<Vec<usize>> {
    let (parent, last) = path.split_at(path.len().checked_sub(1)?);
    let sib = siblings_mut(items, parent)?;
    let i = last[0];
    let j = usize::try_from(i as i64 + delta)
        .ok()
        .filter(|j| *j < sib.len())?;
    sib.swap(i, j);
    let mut p = parent.to_vec();
    p.push(j);
    Some(p)
}

/// Make the item a child of the sibling above it. Returns the new path.
pub fn indent(items: &mut Vec<OutlineItem>, path: &[usize]) -> Option<Vec<usize>> {
    let (parent, last) = path.split_at(path.len().checked_sub(1)?);
    let i = last[0];
    if i == 0 {
        return None;
    }
    let sib = siblings_mut(items, parent)?;
    let item = sib.remove(i);
    let above = sib.get_mut(i - 1)?;
    above.children.push(item);
    above.open = true;
    let mut p = parent.to_vec();
    p.push(i - 1);
    p.push(above.children.len() - 1);
    Some(p)
}

/// Move the item out one level, right after its parent. Returns the new path.
pub fn outdent(items: &mut Vec<OutlineItem>, path: &[usize]) -> Option<Vec<usize>> {
    if path.len() < 2 {
        return None;
    }
    let item = remove(items, path)?;
    let parent = &path[..path.len() - 1];
    Some(insert_after(items, Some(parent), item))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(t: &str) -> OutlineItem {
        OutlineItem::new(t, 0, None)
    }

    #[test]
    fn tree_edits() {
        let mut v = vec![item("A"), item("B"), item("C")];
        let p = indent(&mut v, &[1]).unwrap();
        assert_eq!(p, vec![0, 0]);
        assert_eq!(v[0].children[0].title, "B");
        let p = outdent(&mut v, &p).unwrap();
        assert_eq!(p, vec![1]);
        assert_eq!(
            v.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(),
            ["A", "B", "C"]
        );
        let p = move_by(&mut v, &[2], -1).unwrap();
        assert_eq!(p, vec![1]);
        assert_eq!(v[1].title, "C");
        assert!(move_by(&mut v, &[0], -1).is_none());
        let c = insert_child(&mut v, &[0], item("A.1")).unwrap();
        assert_eq!(c, vec![0, 0]);
        let n = insert_after(&mut v, Some(&c), item("A.2"));
        assert_eq!(n, vec![0, 1]);
        assert_eq!(get(&v, &[0, 1]).unwrap().title, "A.2");
        remove(&mut v, &[0]).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(
            visible_below(&OutlineItem {
                children: vec![item("x"), item("y")],
                ..item("p")
            }),
            2
        );
    }
}
