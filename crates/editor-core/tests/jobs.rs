//! Worker pool behaviour: results arrive, stale revisions are distinguishable, searches stream.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use editor_core::jobs::{Job, JobKind, JobOutput, WorkerHub};
use editor_core::session::DocumentSession;
use editor_core::tiles::{TileKey, TilePlan, quantize_scale};
use pdf_engine::annot::{self, AnnotationKind, AnnotationSpec};
use pdf_engine::geom::{Rect, Rotation};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};
use test_support::fixture_bytes;

fn wait_for(hub: &WorkerHub, want: usize, secs: u64) -> Vec<editor_core::jobs::JobResult> {
    let mut got = Vec::new();
    let end = Instant::now() + Duration::from_secs(secs);
    while got.len() < want && Instant::now() < end {
        got.extend(hub.poll());
        std::thread::sleep(Duration::from_millis(10));
    }
    got
}

fn tile_job(
    s: &mut DocumentSession,
    page_index: usize,
    scale: f64,
    request: u64,
) -> (Job, TileKey) {
    let pages = s.pages().unwrap();
    let p = &pages[page_index];
    let size = p.geometry.view_size(Rotation::R0);
    let (w, h) = (
        (size.width * scale).ceil() as u32,
        (size.height * scale).ceil() as u32,
    );
    let key = TileKey {
        doc: s.id,
        revision: s.revision(),
        page: p.id,
        rotation: Rotation::R0,
        scale_milli: quantize_scale(scale),
        tx: 0,
        ty: 0,
        whole: true,
    };
    let job = Job {
        doc: s.id,
        revision: s.revision(),
        request,
        priority: 50,
        cancel: Arc::new(AtomicBool::new(false)),
        kind: JobKind::Tile {
            key,
            plan: TilePlan {
                tx: 0,
                ty: 0,
                whole: true,
                x: 0,
                y: 0,
                w,
                h,
            },
            geometry: p.geometry,
            page_index,
        },
    };
    (job, key)
}

#[test]
fn every_submitted_tile_comes_back() {
    let mut s = DocumentSession::open_bytes(fixture_bytes("report-chromium.pdf"), "t.pdf").unwrap();
    let mut hub = WorkerHub::new(Arc::new(|| {}));
    let snap = s.snapshot().unwrap();
    hub.ensure_for_test(&snap);
    let mut keys = Vec::new();
    for i in 0..3 {
        let (j, k) = tile_job(&mut s, i, 0.4, i as u64);
        keys.push(k);
        hub.submit(j);
        // Also a main-size render of each page.
        let (j2, k2) = tile_job(&mut s, i, 1.5, 10 + i as u64);
        keys.push(k2);
        hub.submit(j2);
    }
    let res = wait_for(&hub, 6, 20);
    assert_eq!(
        res.len(),
        6,
        "all six jobs must complete: {:?}",
        res.iter()
            .map(|r| format!("{:?}", std::mem::discriminant(&r.output)))
            .collect::<Vec<_>>()
    );
    for r in &res {
        match &r.output {
            JobOutput::Tile { key, bitmap } => {
                assert!(keys.contains(key));
                assert_eq!(
                    bitmap.rgba.len(),
                    (bitmap.width * bitmap.height * 4) as usize
                );
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[test]
fn results_are_tagged_with_revision_so_stale_ones_can_be_dropped() {
    let mut s = DocumentSession::open_bytes(fixture_bytes("report-chromium.pdf"), "t.pdf").unwrap();
    let mut hub = WorkerHub::new(Arc::new(|| {}));
    hub.ensure_for_test(&s.snapshot().unwrap());
    let (j, _) = tile_job(&mut s, 0, 1.0, 1);
    let old_rev = s.revision();
    hub.submit(j);
    let page = s.pages().unwrap()[0].id;
    s.execute("edit", |tx| {
        annot::add_annotation(
            tx,
            page,
            &AnnotationSpec::new(AnnotationKind::Rectangle {
                rect: Rect::new(10.0, 10.0, 50.0, 50.0),
            }),
        )
    })
    .unwrap();
    assert!(s.revision() > old_rev);
    let res = wait_for(&hub, 1, 10);
    assert_eq!(
        res[0].revision, old_rev,
        "a result is tagged with the revision it was rendered from"
    );
    // Jobs for a revision with no pool are ignored rather than rendered against the wrong bytes.
    let (j2, _) = tile_job(&mut s, 0, 1.0, 2);
    hub.submit(j2);
    assert!(wait_for(&hub, 1, 1).is_empty());
}

#[test]
fn search_streams_pages_and_reports_done() {
    let mut s = DocumentSession::open_bytes(fixture_bytes("report-chromium.pdf"), "t.pdf").unwrap();
    let mut hub = WorkerHub::new(Arc::new(|| {}));
    hub.ensure_for_test(&s.snapshot().unwrap());
    let pages: Vec<_> = s
        .pages()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id, i, p.geometry))
        .collect();
    hub.submit(Job {
        doc: s.id,
        revision: s.revision(),
        request: 7,
        priority: 20,
        cancel: Arc::new(AtomicBool::new(false)),
        kind: JobKind::Search {
            query: "revenue".into(),
            case_sensitive: false,
            pages,
        },
    });
    let res = wait_for(&hub, 4, 15);
    let mut hits = 0;
    let mut done = false;
    for r in res {
        assert_eq!(r.request, 7);
        match r.output {
            JobOutput::SearchPage {
                hits: h,
                page_index,
                ..
            } => {
                if !h.is_empty() {
                    assert_eq!(page_index, 1, "only page 2 mentions revenue");
                }
                hits += h.len();
            }
            JobOutput::SearchDone { cancelled } => {
                assert!(!cancelled);
                done = true;
            }
            other => panic!("{other:?}"),
        }
    }
    assert!(done && hits == 2, "hits={hits} done={done}");
}

#[test]
fn cancelled_jobs_do_not_run() {
    let mut s = DocumentSession::open_bytes(fixture_bytes("report-chromium.pdf"), "t.pdf").unwrap();
    let mut hub = WorkerHub::new(Arc::new(|| {}));
    hub.ensure_for_test(&s.snapshot().unwrap());
    let (j, _) = tile_job(&mut s, 0, 1.0, 1);
    j.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    hub.submit(j);
    let res = wait_for(&hub, 1, 5);
    assert!(matches!(res[0].output, JobOutput::Dropped { .. }));
}
