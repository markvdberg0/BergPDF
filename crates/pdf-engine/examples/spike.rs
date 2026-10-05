//! Milestone-0 feasibility spike driver. Usage: spike <input.pdf> <out-dir>
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::geom::Rotation;
use pdf_engine::render::with_session;
use std::{fs, path::Path, sync::Arc, time::Instant};

fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) {
    let f = fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(f, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(rgba).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let input = &args[1];
    let out = Path::new(&args[2]);
    fs::create_dir_all(out).unwrap();
    let bytes = fs::read(input).unwrap();
    let t = Instant::now();
    let doc = PdfDocument::open(bytes, &OpenOptions::default()).unwrap();
    println!(
        "opened in {:?}; pages={}; caps={:?}",
        t.elapsed(),
        doc.page_count(),
        doc.capabilities()
    );
    let pages = doc.pages().unwrap();
    for (i, p) in pages.iter().enumerate() {
        println!("page {i}: {:?}", p.geometry);
    }
    let snap = Arc::new(doc.original_bytes().as_ref().clone());
    with_session(snap, |s| {
        println!("hayro pages: {}", s.page_count());
        for (i, p) in pages.iter().enumerate() {
            let t = Instant::now();
            let bmp = s.render_page(i, p.geometry, Rotation::R0, 1.5)?;
            println!(
                "rendered page {i} {}x{} in {:?}",
                bmp.width,
                bmp.height,
                t.elapsed()
            );
            write_png(
                &out.join(format!("page{i}.png")),
                bmp.width,
                bmp.height,
                &bmp.rgba,
            );
            let tp = s.extract_text(i, &p.geometry)?;
            println!(
                "--- text page {i} ({} glyphs, {} lines):\n{}",
                tp.glyphs.len(),
                tp.lines.len(),
                tp.plain_text()
            );
            if i == 0 {
                for g in tp.glyphs.iter().take(6) {
                    println!(
                        "  {:?} q={:?} fs={:.2}",
                        g.text,
                        g.quad.bounds(),
                        g.font_size
                    );
                }
            }
        }
        Ok(())
    })
    .unwrap();
}
