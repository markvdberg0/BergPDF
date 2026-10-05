//! Navigation structures: bookmarks (outlines) and link annotations.
//!
//! Only *navigation* is interpreted: GoTo destinations (explicit or named) and URI links.
//! Launch, JavaScript, GoToR, SubmitForm and every other action type are ignored — documents
//! can never make the application execute anything.

use crate::doc::{PageId, PdfDocument};
use crate::geom::Rect;
use crate::objutil::{self, MAX_DEPTH};
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::{BTreeSet, HashMap};

/// Maximum bookmark nodes read (guards pathological outlines).
pub const MAX_BOOKMARKS: usize = 20_000;

/// Where a link goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    /// Zero-based page index.
    Page(usize),
    /// External URI (must be confirmed by the user before opening).
    Uri(String),
}

/// A clickable link region.
#[derive(Clone, Debug)]
pub struct LinkInfo {
    /// Region in user space.
    pub rect: Rect,
    /// Destination.
    pub target: LinkTarget,
}

/// A bookmark tree node.
#[derive(Clone, Debug, Default)]
pub struct Bookmark {
    /// Title text.
    pub title: String,
    /// Destination page index, when resolvable.
    pub page: Option<usize>,
    /// Children.
    pub children: Vec<Bookmark>,
    /// Whether the document asks for it to be expanded initially.
    pub open: bool,
}

fn page_index_map(doc: &PdfDocument) -> HashMap<ObjectId, usize> {
    doc.page_ids()
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(i, p)| (p.0, i))
        .collect()
}

/// Links on a page.
pub fn page_links(doc: &PdfDocument, page: PageId) -> Vec<LinkInfo> {
    let d = doc.lopdf();
    let Ok(pd) = d.get_dictionary(page.0) else {
        return Vec::new();
    };
    let Some(arr) = objutil::dict_array(d, pd, b"Annots") else {
        return Vec::new();
    };
    let pages = page_index_map(doc);
    let mut out = Vec::new();
    for a in arr {
        let Some(Object::Dictionary(ad)) = objutil::deref(d, a) else {
            continue;
        };
        if objutil::dict_name(d, ad, b"Subtype") != Some(b"Link") {
            continue;
        }
        let Some(rect) = ad.get(b"Rect").ok().and_then(|r| objutil::rect(d, r)) else {
            continue;
        };
        if let Some(target) = link_target(d, ad, &pages) {
            out.push(LinkInfo { rect, target });
        }
    }
    out
}

fn link_target(
    d: &Document,
    ad: &Dictionary,
    pages: &HashMap<ObjectId, usize>,
) -> Option<LinkTarget> {
    if let Ok(dest) = ad.get(b"Dest") {
        return resolve_dest(d, dest, pages).map(LinkTarget::Page);
    }
    let action = objutil::dict_dict(d, ad, b"A")?;
    action_target(d, action, pages)
}

fn action_target(
    d: &Document,
    action: &Dictionary,
    pages: &HashMap<ObjectId, usize>,
) -> Option<LinkTarget> {
    match objutil::dict_name(d, action, b"S")? {
        b"GoTo" => resolve_dest(d, action.get(b"D").ok()?, pages).map(LinkTarget::Page),
        b"URI" => {
            let s = objutil::deref(d, action.get(b"URI").ok()?)?.as_str().ok()?;
            let uri = String::from_utf8_lossy(s).trim().to_string();
            (!uri.is_empty() && uri.len() < 4096).then_some(LinkTarget::Uri(uri))
        }
        _ => None,
    }
}

fn resolve_dest(d: &Document, dest: &Object, pages: &HashMap<ObjectId, usize>) -> Option<usize> {
    resolve_dest_depth(d, dest, pages, 0)
}

fn resolve_dest_depth(
    d: &Document,
    dest: &Object,
    pages: &HashMap<ObjectId, usize>,
    depth: usize,
) -> Option<usize> {
    if depth > 4 {
        return None;
    }
    match objutil::deref(d, dest)? {
        Object::Array(a) => match a.first()? {
            Object::Reference(r) => pages.get(r).copied(),
            Object::Integer(n) if *n >= 0 => Some(*n as usize),
            _ => None,
        },
        Object::Dictionary(dd) => resolve_dest_depth(d, dd.get(b"D").ok()?, pages, depth + 1),
        Object::Name(n) => named_dest(d, n, pages, depth),
        Object::String(s, _) => named_dest(d, s, pages, depth),
        _ => None,
    }
}

fn named_dest(
    d: &Document,
    name: &[u8],
    pages: &HashMap<ObjectId, usize>,
    depth: usize,
) -> Option<usize> {
    let cat_id = d.trailer.get(b"Root").and_then(Object::as_reference).ok()?;
    let cat = d.get_dictionary(cat_id).ok()?;
    // PDF 1.1 style: /Dests dictionary in the catalog.
    if let Some(dests) = objutil::dict_dict(d, cat, b"Dests")
        && let Ok(v) = dests.get(name)
    {
        return resolve_dest_depth(d, v, pages, depth + 1);
    }
    // PDF 1.2+: /Names /Dests name tree.
    let names = objutil::dict_dict(d, cat, b"Names")?;
    let tree = objutil::dict_dict(d, names, b"Dests")?;
    let v = name_tree_lookup(d, tree, name, 0)?;
    resolve_dest_depth(d, v, pages, depth + 1)
}

fn name_tree_lookup<'a>(
    d: &'a Document,
    node: &'a Dictionary,
    key: &[u8],
    depth: usize,
) -> Option<&'a Object> {
    if depth > MAX_DEPTH {
        return None;
    }
    if let Some(names) = objutil::dict_array(d, node, b"Names") {
        for pair in names.chunks_exact(2) {
            let k = objutil::deref(d, &pair[0])?.as_str().ok()?;
            if k == key {
                return Some(&pair[1]);
            }
        }
    }
    if let Some(kids) = objutil::dict_array(d, node, b"Kids") {
        for k in kids {
            let Some(Object::Dictionary(kd)) = objutil::deref(d, k) else {
                continue;
            };
            if let Some(limits) = objutil::dict_array(d, kd, b"Limits")
                && limits.len() >= 2
                && let (Some(lo), Some(hi)) = (
                    objutil::deref(d, &limits[0]).and_then(|o| o.as_str().ok()),
                    objutil::deref(d, &limits[1]).and_then(|o| o.as_str().ok()),
                )
                && (key < lo || key > hi)
            {
                continue;
            }
            if let Some(v) = name_tree_lookup(d, kd, key, depth + 1) {
                return Some(v);
            }
        }
    }
    None
}

/// Read the document's bookmarks.
pub fn outlines(doc: &PdfDocument) -> Vec<Bookmark> {
    let d = doc.lopdf();
    let Some(cat) = d
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .ok()
        .and_then(|id| d.get_dictionary(id).ok())
    else {
        return Vec::new();
    };
    let Some(root) = objutil::dict_dict(d, cat, b"Outlines") else {
        return Vec::new();
    };
    let pages = page_index_map(doc);
    let mut seen = BTreeSet::new();
    let mut count = 0;
    read_siblings(d, root, &pages, 0, &mut seen, &mut count)
}

fn read_siblings(
    d: &Document,
    parent: &Dictionary,
    pages: &HashMap<ObjectId, usize>,
    depth: usize,
    seen: &mut BTreeSet<ObjectId>,
    count: &mut usize,
) -> Vec<Bookmark> {
    let mut out = Vec::new();
    if depth > 32 {
        return out;
    }
    let mut cur = parent
        .get(b"First")
        .ok()
        .and_then(|o| o.as_reference().ok());
    while let Some(id) = cur {
        if !seen.insert(id) || *count >= MAX_BOOKMARKS {
            break;
        }
        *count += 1;
        let Ok(item) = d.get_dictionary(id) else {
            break;
        };
        let title = item
            .get(b"Title")
            .ok()
            .and_then(|o| objutil::deref(d, o))
            .and_then(|o| o.as_str().ok())
            .map(objutil::decode_text_string)
            .unwrap_or_default();
        let page = if let Ok(dest) = item.get(b"Dest") {
            resolve_dest(d, dest, pages)
        } else {
            objutil::dict_dict(d, item, b"A").and_then(|a| match action_target(d, a, pages) {
                Some(LinkTarget::Page(p)) => Some(p),
                _ => None,
            })
        };
        let open = objutil::dict_num(d, item, b"Count").is_some_and(|c| c > 0.0);
        let children = read_siblings(d, item, pages, depth + 1, seen, count);
        out.push(Bookmark {
            title: title.chars().filter(|c| !c.is_control()).collect(),
            page,
            children,
            open,
        });
        cur = item.get(b"Next").ok().and_then(|o| o.as_reference().ok());
    }
    out
}
