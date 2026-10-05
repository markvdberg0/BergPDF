//! Snap-to-geometry: background index building and the hover marker.

use crate::canvas::ViewCtx;
use crate::state::*;
use editor_core::session::{DocId, Snapshot};
use editor_core::tools::Tool;
use egui::{Color32, Pos2, Stroke};
use pdf_engine::doc::{OpenOptions, PageId, PdfDocument};
use pdf_engine::snap::{SnapIndex, SnapKind};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};

type Key = (DocId, PageId, u64);

struct Built {
    key: Key,
    index: SnapIndex,
}

type ParsedDoc = Arc<OnceLock<Option<Arc<PdfDocument>>>>;

/// Builds and caches one [`SnapIndex`] per (document, page, revision) off the UI thread.
pub struct SnapService {
    cache: HashMap<(DocId, PageId), (u64, Arc<SnapIndex>)>,
    pending: HashSet<Key>,
    parsed: Arc<Mutex<HashMap<(DocId, u64), ParsedDoc>>>,
    tx: Sender<Built>,
    rx: Receiver<Built>,
}

impl SnapService {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self {
            cache: HashMap::new(),
            pending: HashSet::new(),
            parsed: Arc::new(Mutex::new(HashMap::new())),
            tx,
            rx,
        }
    }

    pub fn get(&self, doc: DocId, page: PageId, rev: u64) -> Option<Arc<SnapIndex>> {
        self.cache
            .get(&(doc, page))
            .filter(|(r, _)| *r == rev)
            .map(|(_, ix)| ix.clone())
    }

    /// Start building (once) the index of `page` (index `page_index`) for `snap`'s revision.
    pub fn request(&mut self, snap: &Snapshot, page_index: usize, page: PageId) {
        let key = (snap.doc, page, snap.revision);
        if !self.pending.insert(key) {
            return;
        }
        let entry = {
            let mut map = self.parsed.lock().unwrap_or_else(|e| e.into_inner());
            // Parsed copies of older revisions of this document are no longer needed.
            map.retain(|(d, r), _| *d != snap.doc || *r == snap.revision);
            map.entry((snap.doc, snap.revision))
                .or_insert_with(|| Arc::new(OnceLock::new()))
                .clone()
        };
        let bytes = snap.bytes.clone();
        let tx = self.tx.clone();
        let spawned = std::thread::Builder::new()
            .name("snap-index".into())
            .spawn(move || {
                let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let doc = entry.get_or_init(|| {
                        PdfDocument::open(bytes.as_ref().clone(), &OpenOptions::default())
                            .ok()
                            .map(Arc::new)
                    });
                    let doc = doc.as_ref()?;
                    let id = *doc.page_ids().ok()?.get(page_index)?;
                    Some(SnapIndex::build(doc.lopdf(), id.0))
                }));
                // On any failure an empty index is delivered so the request is not repeated.
                let index = built.ok().flatten().unwrap_or_default();
                let _ = tx.send(Built { key, index });
            });
        if spawned.is_err() {
            self.pending.remove(&key);
        }
    }

    /// Collect finished indexes. Returns true while more are on their way.
    pub fn poll(&mut self) -> bool {
        while let Ok(b) = self.rx.try_recv() {
            self.pending.remove(&b.key);
            let (doc, page, rev) = b.key;
            self.cache.insert((doc, page), (rev, Arc::new(b.index)));
        }
        !self.pending.is_empty()
    }
}

impl App {
    /// Index for page `i` of the active document, requesting it if it is not built yet.
    pub fn snap_index(&mut self, i: usize, vc: &ViewCtx) -> Option<Arc<SnapIndex>> {
        let ti = self.active;
        let page = vc.pages.get(i)?.id;
        let (doc, rev) = {
            let t = self.tabs.get(ti)?;
            (t.session.id, t.session.revision())
        };
        if let Some(ix) = self.snaps.get(doc, page, rev) {
            return Some(ix);
        }
        if let Ok(snap) = self.tabs[ti].session.snapshot() {
            self.snaps.request(&snap, i, page);
        }
        None
    }

    /// Draw the snap marker under the pointer while a measurement tool is active.
    pub fn paint_snap_hover(&mut self, painter: &egui::Painter, vc: &ViewCtx) {
        let measuring = self.tool_is_measure() || self.tool == Tool::Calibrate;
        if !measuring || self.dialog.is_some() || self.palette_open {
            return;
        }
        let Some(h) = painter.ctx().input(|i| i.pointer.hover_pos()) else {
            return;
        };
        if !vc.viewport.contains(h) {
            return;
        }
        let Some(i) = vc.page_at(h) else { return };
        let shift = painter.ctx().input(|i| i.modifiers.shift);
        let (existing, last) = match &self.tabs[self.active].ui.interaction {
            Interaction::Draw { points, .. } => (points.clone(), points.last().copied()),
            _ => (Vec::new(), None),
        };
        let (p, kind) = self.snap_point(vc, i, h, &existing, last, shift);
        let Some(kind) = kind else { return };
        let c = vc.pdf_to_screen(i, p);
        let col = Color32::from_rgb(255, 140, 0);
        let st = Stroke::new(2.0, col);
        let r = 6.0;
        match kind {
            SnapKind::Endpoint => {
                painter.rect_stroke(
                    egui::Rect::from_center_size(c, egui::vec2(2.0 * r, 2.0 * r)),
                    0.0,
                    st,
                    egui::StrokeKind::Middle,
                );
            }
            SnapKind::Intersection => {
                painter.line_segment([c + egui::vec2(-r, -r), c + egui::vec2(r, r)], st);
                painter.line_segment([c + egui::vec2(-r, r), c + egui::vec2(r, -r)], st);
            }
            SnapKind::Midpoint => {
                let tri = vec![
                    c + egui::vec2(0.0, -r),
                    c + egui::vec2(r, r * 0.8),
                    c + egui::vec2(-r, r * 0.8),
                ];
                painter.add(egui::epaint::PathShape::closed_line(tri, st));
            }
            SnapKind::Edge => {
                painter.circle_stroke(c, r * 0.7, st);
            }
        }
        // Small caption so the reason for the jump is clear.
        let galley = painter.layout_no_wrap(
            kind.label().to_string(),
            egui::FontId::proportional(11.0),
            Color32::WHITE,
        );
        let pos = Pos2::new(c.x + 10.0, c.y - 20.0);
        let rect = egui::Rect::from_min_size(pos, galley.size()).expand(3.0);
        painter.rect_filled(rect, 3.0, Color32::from_rgba_unmultiplied(40, 30, 10, 220));
        painter.galley(pos, galley, Color32::WHITE);
    }
}
