//! Temporary: which combinations does poppler read? (prints a matrix by failing)
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::protect::Rights;
use std::process::Command;
use test_support::fixtures::helvetica_lines;

fn text(bytes: &[u8], flag: &str, pw: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("in.pdf");
    std::fs::write(&p, bytes).unwrap();
    let out = Command::new("pdftotext").args([flag, pw, "-enc", "UTF-8"]).arg(&p).arg("-").output().unwrap();
    let t = String::from_utf8_lossy(&out.stdout).into_owned();
    if t.contains("Second line") { "OK".into() } else { format!("EMPTY/garbled (exit {:?})", out.status.code()) }
}

#[test]
fn matrix() {
    if !test_support::have_tool("pdftotext") { return; }
    let only_view = Rights { print: true, copy: false, modify: false, annotate: false, fill: false, assemble: false };
    let mut report = String::new();
    for (rname, rights) in [("ALL", Rights::ALL), ("only_view", only_view)] {
        for owner in ["", "chief"] {
            let mut doc = PdfDocument::open(helvetica_lines(), &OpenOptions::default()).unwrap();
            doc.set_protection("reader", owner, rights).unwrap();
            let plain = doc.snapshot_bytes().unwrap();
            let sealed = doc.seal(&plain).unwrap().into_owned();
            report.push_str(&format!(
                "rights={rname} owner={owner:?}: -upw reader: {} | -opw reader: {} | -opw chief: {}\n",
                text(&sealed, "-upw", "reader"), text(&sealed, "-opw", "reader"), text(&sealed, "-opw", "chief")
            ));
        }
    }
    panic!("MATRIX\n{report}");
}
