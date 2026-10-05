//! Selection state of one document.

use pdf_engine::annot::AnnotId;
use pdf_engine::doc::PageId;
use pdf_engine::pagecontent::ObjRef;
use std::ops::Range;

/// A selected piece of *page content* (as opposed to an annotation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentRef {
    /// A text run.
    Text(ObjRef),
    /// An image placement.
    Image(ObjRef),
}

/// A text selection on one page (glyph index range into that page's `TextPage`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSelection {
    /// Page the selection lives on.
    pub page: PageId,
    /// Glyph range in reading order.
    pub glyphs: Range<usize>,
}

/// Everything currently selected.
#[derive(Clone, Debug, Default)]
pub struct Selection {
    /// Selected annotations (page, id) — multi-selection supported.
    pub annotations: Vec<(PageId, AnnotId)>,
    /// Selected text, if any.
    pub text: Option<TextSelection>,
    /// Selected page thumbnails (organizer).
    pub pages: Vec<PageId>,
    /// Selected page-content object (Edit tools).
    pub content: Option<(PageId, ContentRef)>,
}

impl Selection {
    /// Clear annotation and text selection (page selection is kept).
    pub fn clear_content(&mut self) {
        self.annotations.clear();
        self.text = None;
        self.content = None;
    }

    /// Whether an annotation is selected.
    pub fn has_annotation(&self, id: AnnotId) -> bool {
        self.annotations.iter().any(|(_, a)| *a == id)
    }

    /// Select exactly one annotation, or toggle it when `additive`.
    pub fn select_annotation(&mut self, page: PageId, id: AnnotId, additive: bool) {
        if additive {
            if let Some(i) = self.annotations.iter().position(|(_, a)| *a == id) {
                self.annotations.remove(i);
            } else {
                self.annotations.push((page, id));
            }
        } else {
            self.annotations = vec![(page, id)];
        }
        self.text = None;
    }
}
