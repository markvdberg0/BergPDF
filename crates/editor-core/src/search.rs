//! Per-document search state.

use pdf_engine::doc::PageId;
use pdf_engine::text::SearchHit;

/// One located match.
#[derive(Clone, Debug)]
pub struct Match {
    /// Page the match is on.
    pub page: PageId,
    /// Zero-based page index at search time.
    pub page_index: usize,
    /// The hit (glyph range + quads in PDF user space).
    pub hit: SearchHit,
    /// Short context snippet for the results list.
    pub snippet: String,
}

/// Search UI state.
#[derive(Clone, Debug, Default)]
pub struct SearchState {
    /// Query text.
    pub query: String,
    /// Match case.
    pub case_sensitive: bool,
    /// Matches found so far, in page order.
    pub matches: Vec<Match>,
    /// Index of the highlighted match.
    pub current: Option<usize>,
    /// Request id of the running search (stale results are dropped).
    pub request: u64,
    /// True while pages are still being scanned.
    pub running: bool,
    /// Pages scanned / total, for progress.
    pub progress: (usize, usize),
}

impl SearchState {
    /// Move to the next match (wraps).
    pub fn next(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.current = Some(self.current.map_or(0, |c| (c + 1) % self.matches.len()));
    }

    /// Move to the previous match (wraps).
    pub fn previous(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        let n = self.matches.len();
        self.current = Some(self.current.map_or(n - 1, |c| (c + n - 1) % n));
    }

    /// Reset results (keeps the query).
    pub fn clear_results(&mut self) {
        self.matches.clear();
        self.current = None;
        self.running = false;
        self.progress = (0, 0);
    }
}
