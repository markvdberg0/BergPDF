//! Performance measurements. Usage (release build!):
//! `cargo run --release -p pdf-engine --example bench -- [fixture.pdf]`
//! Prints wall-clock timings and peak resident memory for the operations a user waits on.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout
)]
use pdf_engine::annot::{self, AnnotationKind, AnnotationSpec};
use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::geom::{Point, Rect, Rotation};
use pdf_engine::render::with_session;
use std::sync::Arc;
use std::time::Instant;
use test_support::fixtures::{many_pages, vector_a0};

fn peak_rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
        })
        .map_or(f64::NAN, |kb| kb / 1024.0)
}

fn time<R>(label: &str, f: impl FnOnce() -> R) -> R {
    let t = Instant::now();
    let r = f();
    println!(
        "{label:<58} {:>9.1} ms   (peak RSS {:.0} MB)",
        t.elapsed().as_secs_f64() * 1000.0,
        peak_rss_mb()
    );
    r
}

fn first_page_ms(label: &str, bytes: Vec<u8>) {
    println!("\n== {label} ({:.1} KB)", bytes.len() as f64 / 1024.0);
    let t0 = Instant::now();
    let doc = time("open + parse", || {
        PdfDocument::open(bytes, &OpenOptions::default()).unwrap()
    });
    let pages = time("page list", || doc.pages().unwrap());
    let snap = Arc::new(doc.original_bytes().as_ref().clone());
    time("hayro load + render page 1 @ 1.0x (fit-width-like)", || {
        with_session(snap, |s| {
            s.render_page(0, pages[0].geometry, Rotation::R0, 1.0)
                .map(|b| b.width)
        })
        .unwrap()
    });
    println!(
        "{:<58} {:>9.1} ms",
        "TOTAL time to first rendered page",
        t0.elapsed().as_secs_f64() * 1000.0
    );
}

fn main() {
    if let Some(path) = std::env::args().nth(1) {
        first_page_ms(&path, std::fs::read(&path).unwrap());
        return;
    }
    first_page_ms(
        "report-chromium.pdf (3 pages, embedded fonts)",
        test_support::fixture_bytes("report-chromium.pdf"),
    );
    first_page_ms("1000-page text document", many_pages(1000));
    first_page_ms("A0 page, 60 000 vector shapes", vector_a0(60_000));

    println!("\n== 1000-page document operations");
    let mut doc = PdfDocument::open(many_pages(1000), &OpenOptions::default()).unwrap();
    let ids = time("page ids (walk page tree)", || doc.page_ids().unwrap());
    assert_eq!(ids.len(), 1000);
    let snap = Arc::new(doc.original_bytes().as_ref().clone());
    let pages = doc.pages().unwrap();
    time("render page 500 thumbnail (0.2x)", || {
        with_session(snap.clone(), |s| {
            s.render_page(499, pages[499].geometry, Rotation::R0, 0.2)
                .map(|b| b.width)
        })
        .unwrap()
    });
    time("render 50 thumbnails (0.2x) in one session", || {
        with_session(snap.clone(), |s| {
            for i in 0..50 {
                s.render_page(i * 20, pages[i * 20].geometry, Rotation::R0, 0.2)?;
            }
            Ok(())
        })
        .unwrap()
    });
    time(
        "search 'quick' across all 1000 pages (text extraction)",
        || {
            with_session(snap.clone(), |s| {
                let mut n = 0usize;
                for (i, p) in pages.iter().enumerate() {
                    n += s.extract_text(i, &p.geometry)?.glyphs.len();
                }
                Ok(n)
            })
            .unwrap()
        },
    );
    let page = ids[0];
    let spec = AnnotationSpec::new(AnnotationKind::Rectangle {
        rect: Rect::new(50.0, 50.0, 150.0, 120.0),
    });
    time("add one annotation (transaction)", || {
        doc.transact(|tx| annot::add_annotation(tx, page, &spec))
            .unwrap()
    });
    let bytes = time("incremental snapshot after the edit", || {
        doc.snapshot_bytes().unwrap()
    });
    println!(
        "{:<58} {:>9} bytes appended to a {} byte original",
        "incremental save size",
        bytes.len() - doc.original_bytes().len(),
        doc.original_bytes().len()
    );
    let _ = Point::new(0.0, 0.0);
    time("delete 500 pages (one transaction)", || {
        let victims: Vec<_> = ids.iter().step_by(2).copied().collect();
        doc.transact(|tx| pdf_engine::pageops::delete_pages(tx, &victims))
            .unwrap()
    });
}
