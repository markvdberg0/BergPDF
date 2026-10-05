//! Background work: bounded, prioritised, cancellable, revision-aware.
//!
//! One [`RenderPool`] exists per (document, revision). Each worker thread parses the
//! immutable byte snapshot into its own hayro session (sessions are `!Send`), so workers
//! never touch the editable document. Results carry (doc, revision, request id) and the
//! consumer drops anything stale.
//!
//! Isolation note: this is thread-level isolation with panic containment. Release builds are
//! meant to move parsing/rendering into restricted *processes* with time/memory limits
//! (see docs/PLAN.md, Milestone 5); the job/result types here are plain data to keep that
//! swap local.

use crate::session::{DocId, Snapshot};
use crate::tiles::{TileKey, TilePlan, dequantize_scale};
use pdf_engine::doc::PageId;
use pdf_engine::geom::PageGeometry;
use pdf_engine::render::{Bitmap, TileRequest, with_session};
use pdf_engine::text::{SearchHit, TextPage};
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

/// Maximum queued jobs per pool; the lowest-priority job is dropped beyond this.
pub const MAX_QUEUE: usize = 256;

/// What to compute.
#[derive(Clone, Debug)]
pub enum JobKind {
    /// Render a tile or whole page.
    Tile {
        /// Cache key the result belongs to.
        key: TileKey,
        /// Pixel rectangle within the full-page bitmap.
        plan: TilePlan,
        /// Page geometry (single source of truth for coordinates).
        geometry: PageGeometry,
        /// Index of the page in the snapshot.
        page_index: usize,
    },
    /// Extract page text.
    Text {
        /// Page id.
        page: PageId,
        /// Page index in the snapshot.
        page_index: usize,
        /// Geometry.
        geometry: PageGeometry,
    },
    /// Search the listed pages in order, streaming per-page results.
    Search {
        /// Query.
        query: String,
        /// Case sensitivity.
        case_sensitive: bool,
        /// Pages to scan: (id, index, geometry).
        pages: Vec<(PageId, usize, PageGeometry)>,
    },
}

/// A unit of work.
#[derive(Clone, Debug)]
pub struct Job {
    /// Owning document.
    pub doc: DocId,
    /// Revision of the snapshot this job must run against.
    pub revision: u64,
    /// Caller-chosen id echoed in results (search id, tile request id…).
    pub request: u64,
    /// Higher runs first.
    pub priority: i32,
    /// Set to abort before/while running.
    pub cancel: Arc<AtomicBool>,
    /// Payload.
    pub kind: JobKind,
}

/// A computed result.
#[derive(Debug)]
pub enum JobOutput {
    /// Rendered pixels.
    Tile {
        /// Which tile.
        key: TileKey,
        /// Pixels (RGBA8, opaque).
        bitmap: Bitmap,
    },
    /// Extracted text.
    Text {
        /// Page id.
        page: PageId,
        /// Text.
        text: Arc<TextPage>,
    },
    /// Hits on one page of a running search.
    SearchPage {
        /// Page.
        page: PageId,
        /// Page index.
        page_index: usize,
        /// Hits with snippets.
        hits: Vec<(SearchHit, String)>,
        /// Pages scanned so far.
        scanned: usize,
        /// Total pages in this search.
        total: usize,
    },
    /// Search finished (or was cancelled).
    SearchDone {
        /// True when cancelled before completion.
        cancelled: bool,
    },
    /// A tile/text job was dropped or cancelled without running.
    Dropped {
        /// Tile key, when the job was a tile.
        key: Option<TileKey>,
    },
    /// The job failed (panic contained, parse error, limit).
    Failed {
        /// Tile key, when the job was a tile.
        key: Option<TileKey>,
        /// Human-readable reason (sanitised).
        message: String,
    },
}

/// Result envelope.
#[derive(Debug)]
pub struct JobResult {
    /// Owning document.
    pub doc: DocId,
    /// Revision the job ran against.
    pub revision: u64,
    /// Request id from the job.
    pub request: u64,
    /// Payload.
    pub output: JobOutput,
}

struct Queued {
    seq: u64,
    job: Job,
}

struct QueueState {
    jobs: Vec<Queued>,
    closed: bool,
    seq: u64,
}

struct Queue {
    state: Mutex<QueueState>,
    cv: Condvar,
}

impl Queue {
    fn new() -> Self {
        Self {
            state: Mutex::new(QueueState {
                jobs: Vec::new(),
                closed: false,
                seq: 0,
            }),
            cv: Condvar::new(),
        }
    }

    /// Returns the job that was displaced, if the queue overflowed or a duplicate tile was replaced.
    fn push(&self, job: Job) -> Vec<Job> {
        let mut displaced = Vec::new();
        let Ok(mut st) = self.state.lock() else {
            return vec![job];
        };
        if st.closed {
            return vec![job];
        }
        // Replace a queued duplicate of the same tile.
        if let JobKind::Tile { key, .. } = &job.kind {
            let k = *key;
            if let Some(pos) = st
                .jobs
                .iter()
                .position(|q| matches!(&q.job.kind, JobKind::Tile { key, .. } if *key == k))
            {
                displaced.push(st.jobs.remove(pos).job);
            }
        }
        st.seq += 1;
        let seq = st.seq;
        st.jobs.push(Queued { seq, job });
        if st.jobs.len() > MAX_QUEUE {
            // Drop the lowest-priority, oldest job.
            if let Some((i, _)) = st
                .jobs
                .iter()
                .enumerate()
                .min_by_key(|(_, q)| (q.job.priority, std::cmp::Reverse(q.seq)))
            {
                displaced.push(st.jobs.remove(i).job);
            }
        }
        drop(st);
        self.cv.notify_one();
        displaced
    }

    fn pop(&self) -> Option<Job> {
        let Ok(mut st) = self.state.lock() else {
            return None;
        };
        loop {
            if st.closed {
                return None;
            }
            // Highest priority first, then oldest.
            if let Some((i, _)) = st
                .jobs
                .iter()
                .enumerate()
                .max_by_key(|(_, q)| (q.job.priority, std::cmp::Reverse(q.seq)))
            {
                return Some(st.jobs.remove(i).job);
            }
            st = match self.cv.wait(st) {
                Ok(g) => g,
                Err(_) => return None,
            };
        }
    }

    fn close(&self) -> Vec<Job> {
        let Ok(mut st) = self.state.lock() else {
            return Vec::new();
        };
        st.closed = true;
        let rest = st.jobs.drain(..).map(|q| q.job).collect();
        drop(st);
        self.cv.notify_all();
        rest
    }

    fn len(&self) -> usize {
        self.state.lock().map_or(0, |s| s.jobs.len())
    }
}

type Wake = Arc<dyn Fn() + Send + Sync>;

/// Workers for one (document, revision).
pub struct RenderPool {
    /// Document the pool serves.
    pub doc: DocId,
    /// Revision the pool serves.
    pub revision: u64,
    queue: Arc<Queue>,
    threads: Vec<JoinHandle<()>>,
    tx: Sender<JobResult>,
    wake: Wake,
}

impl RenderPool {
    /// Spawn workers for a snapshot.
    pub fn new(snapshot: &Snapshot, threads: usize, tx: Sender<JobResult>, wake: Wake) -> Self {
        let queue = Arc::new(Queue::new());
        let mut handles = Vec::new();
        for n in 0..threads.max(1) {
            let q = queue.clone();
            let bytes = snapshot.bytes.clone();
            let txc = tx.clone();
            let w = wake.clone();
            let (doc, revision) = (snapshot.doc, snapshot.revision);
            let h = std::thread::Builder::new()
                .name(format!("berg-render-{n}"))
                .stack_size(16 * 1024 * 1024)
                .spawn(move || worker_main(doc, revision, bytes, q, txc, w));
            if let Ok(h) = h {
                handles.push(h);
            }
        }
        Self {
            doc: snapshot.doc,
            revision: snapshot.revision,
            queue,
            threads: handles,
            tx,
            wake,
        }
    }

    /// Queue a job; displaced/overflowed jobs are reported as `Dropped`.
    pub fn submit(&self, job: Job) {
        for d in self.queue.push(job) {
            let key = match &d.kind {
                JobKind::Tile { key, .. } => Some(*key),
                _ => None,
            };
            let _ = self.tx.send(JobResult {
                doc: d.doc,
                revision: d.revision,
                request: d.request,
                output: JobOutput::Dropped { key },
            });
            (self.wake)();
        }
    }

    /// Number of queued (not running) jobs.
    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    /// Number of live worker threads.
    pub fn threads(&self) -> usize {
        self.threads.len()
    }
}

impl Drop for RenderPool {
    fn drop(&mut self) {
        for d in self.queue.close() {
            let key = match &d.kind {
                JobKind::Tile { key, .. } => Some(*key),
                _ => None,
            };
            let _ = self.tx.send(JobResult {
                doc: d.doc,
                revision: d.revision,
                request: d.request,
                output: JobOutput::Dropped { key },
            });
        }
        // Workers exit after their current job; they are detached on purpose so closing a
        // document never blocks the UI thread behind a long render.
        self.threads.clear();
    }
}

enum Loop {
    Exit,
    Restart,
}

fn worker_main(
    doc: DocId,
    revision: u64,
    bytes: Arc<Vec<u8>>,
    queue: Arc<Queue>,
    tx: Sender<JobResult>,
    wake: Wake,
) {
    loop {
        let q = queue.clone();
        let txc = tx.clone();
        let w = wake.clone();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            with_session(bytes.clone(), |session| {
                let mut text_cache: HashMap<usize, Arc<TextPage>> = HashMap::new();
                loop {
                    let Some(job) = q.pop() else {
                        return Ok(Loop::Exit);
                    };
                    let key = match &job.kind {
                        JobKind::Tile { key, .. } => Some(*key),
                        _ => None,
                    };
                    if job.cancel.load(Ordering::Relaxed) {
                        send(&txc, &w, &job, JobOutput::Dropped { key });
                        continue;
                    }
                    let res = catch_unwind(AssertUnwindSafe(|| {
                        run_job(session, &job, &mut text_cache, &txc, &w)
                    }));
                    match res {
                        Ok(()) => {}
                        Err(_) => {
                            send(
                                &txc,
                                &w,
                                &job,
                                JobOutput::Failed {
                                    key,
                                    message: "the renderer crashed on this page".into(),
                                },
                            );
                            // Session state may be inconsistent after a panic: rebuild it.
                            return Ok(Loop::Restart);
                        }
                    }
                }
            })
        }));
        match outcome {
            Ok(Ok(Loop::Exit)) => break,
            Ok(Ok(Loop::Restart)) | Err(_) => continue,
            Ok(Err(e)) => {
                // Cannot even parse the snapshot: fail everything that is queued, then stop.
                while let Some(job) = queue.pop() {
                    let key = match &job.kind {
                        JobKind::Tile { key, .. } => Some(*key),
                        _ => None,
                    };
                    let _ = tx.send(JobResult {
                        doc,
                        revision,
                        request: job.request,
                        output: JobOutput::Failed {
                            key,
                            message: e.to_string(),
                        },
                    });
                }
                wake();
                break;
            }
        }
    }
}

fn send(tx: &Sender<JobResult>, wake: &Wake, job: &Job, output: JobOutput) {
    let _ = tx.send(JobResult {
        doc: job.doc,
        revision: job.revision,
        request: job.request,
        output,
    });
    wake();
}

fn run_job(
    session: &pdf_engine::render::Session<'_>,
    job: &Job,
    text_cache: &mut HashMap<usize, Arc<TextPage>>,
    tx: &Sender<JobResult>,
    wake: &Wake,
) {
    match &job.kind {
        JobKind::Tile {
            key,
            plan,
            geometry,
            page_index,
        } => {
            let req = TileRequest {
                page_index: *page_index,
                geometry: *geometry,
                view_rotation: key.rotation,
                scale: dequantize_scale(key.scale_milli),
                x: plan.x,
                y: plan.y,
                width: plan.w,
                height: plan.h,
            };
            let out = match session.render_tile(&req) {
                Ok(bitmap) => JobOutput::Tile { key: *key, bitmap },
                Err(e) => JobOutput::Failed {
                    key: Some(*key),
                    message: e.to_string(),
                },
            };
            send(tx, wake, job, out);
        }
        JobKind::Text {
            page,
            page_index,
            geometry,
        } => {
            let text = match cached_text(session, text_cache, *page_index, geometry) {
                Ok(t) => JobOutput::Text {
                    page: *page,
                    text: t,
                },
                Err(e) => JobOutput::Failed {
                    key: None,
                    message: e,
                },
            };
            send(tx, wake, job, text);
        }
        JobKind::Search {
            query,
            case_sensitive,
            pages,
        } => {
            let total = pages.len();
            for (n, (page, idx, geometry)) in pages.iter().enumerate() {
                if job.cancel.load(Ordering::Relaxed) {
                    send(tx, wake, job, JobOutput::SearchDone { cancelled: true });
                    return;
                }
                let Ok(tp) = cached_text(session, text_cache, *idx, geometry) else {
                    continue;
                };
                let hits: Vec<(SearchHit, String)> = tp
                    .search(query, *case_sensitive)
                    .into_iter()
                    .map(|h| {
                        let s = h.glyphs.start.saturating_sub(24);
                        let e = (h.glyphs.end + 24).min(tp.glyphs.len());
                        let snippet = tp.text_of(s..e).replace('\n', " ");
                        (h, snippet)
                    })
                    .collect();
                send(
                    tx,
                    wake,
                    job,
                    JobOutput::SearchPage {
                        page: *page,
                        page_index: *idx,
                        hits,
                        scanned: n + 1,
                        total,
                    },
                );
            }
            send(tx, wake, job, JobOutput::SearchDone { cancelled: false });
        }
    }
}

fn cached_text(
    session: &pdf_engine::render::Session<'_>,
    cache: &mut HashMap<usize, Arc<TextPage>>,
    idx: usize,
    geometry: &PageGeometry,
) -> Result<Arc<TextPage>, String> {
    if let Some(t) = cache.get(&idx) {
        return Ok(t.clone());
    }
    let tp = session
        .extract_text(idx, geometry)
        .map_err(|e| e.to_string())?;
    let tp = Arc::new(tp);
    // Bound the cache: text pages can be large.
    if cache.len() > 64 {
        cache.clear();
    }
    cache.insert(idx, tp.clone());
    Ok(tp)
}

/// Owns one pool per open document and the shared result channel.
pub struct WorkerHub {
    pools: HashMap<DocId, RenderPool>,
    tx: Sender<JobResult>,
    rx: Receiver<JobResult>,
    wake: Wake,
    threads: usize,
}

impl WorkerHub {
    /// Create a hub. `wake` is called (from worker threads) whenever a result is ready.
    pub fn new(wake: Wake) -> Self {
        let (tx, rx) = channel();
        let threads = std::thread::available_parallelism()
            .map_or(2, |n| n.get().saturating_sub(1).clamp(1, 4));
        Self {
            pools: HashMap::new(),
            tx,
            rx,
            wake,
            threads,
        }
    }

    /// Make sure a pool exists for the snapshot's revision (replacing an older pool).
    pub fn ensure(&mut self, snap: &Snapshot) {
        let fresh = self
            .pools
            .get(&snap.doc)
            .is_some_and(|p| p.revision == snap.revision);
        if !fresh {
            let pool = RenderPool::new(snap, self.threads, self.tx.clone(), self.wake.clone());
            self.pools.insert(snap.doc, pool);
        }
    }

    /// Test helper: same as [`Self::ensure`].
    #[doc(hidden)]
    pub fn ensure_for_test(&mut self, snap: &Snapshot) {
        self.ensure(snap);
    }

    /// Submit a job. Returns `false` (and does nothing) when the document has no pool for the
    /// job's revision, so callers can roll back their own bookkeeping.
    pub fn submit(&self, job: Job) -> bool {
        match self.pools.get(&job.doc) {
            Some(p) if p.revision == job.revision => {
                p.submit(job);
                true
            }
            _ => false,
        }
    }

    /// Whether a pool for exactly this document revision exists.
    pub fn has_pool(&self, doc: DocId, revision: u64) -> bool {
        self.pools.get(&doc).is_some_and(|p| p.revision == revision)
    }

    /// Drop a document's workers.
    pub fn close(&mut self, doc: DocId) {
        self.pools.remove(&doc);
    }

    /// Drain finished results.
    pub fn poll(&self) -> Vec<JobResult> {
        self.rx.try_iter().collect()
    }

    /// Queued jobs across all pools (for the status bar).
    pub fn queued(&self) -> usize {
        self.pools.values().map(RenderPool::queued).sum()
    }
}
