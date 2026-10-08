//! Make a "scanned" document from a page of a PDF, to try text recognition on:
//! `cargo run -p pdf-engine --example make_scan -- <in.pdf> <out.pdf> [page] [dpi]`.
//! The page is drawn to a picture (grey, a little paper noise) and the picture becomes the only content
//! of a new one-page PDF, so the output has no text layer.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout)]

use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::geom::Rotation;
use pdf_engine::render::with_session;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).expect("read input");
    let page: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
    let dpi: f64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(200.0);
    let doc = PdfDocument::open(bytes.clone(), &OpenOptions::default()).expect("open");
    let geometry = doc.pages().expect("pages")[page].geometry;
    let bmp = with_session(Arc::new(bytes), |s| {
        s.render_page(page, geometry, Rotation::R0, dpi / 72.0)
    })
    .expect("render");
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rgb = Vec::with_capacity((bmp.width * bmp.height * 3) as usize);
    for px in bmp.rgba.chunks_exact(4) {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let noise = (seed % 9) as f32 - 4.0;
        let g = 0.3 * f32::from(px[0]) + 0.59 * f32::from(px[1]) + 0.11 * f32::from(px[2]);
        // Paper that is not quite white, ink that is not quite black.
        let v = (g * 0.93 + 8.0 + noise).clamp(0.0, 255.0);
        rgb.extend_from_slice(&[v as u8, (v - 1.0).max(0.0) as u8, (v - 4.0).max(0.0) as u8]);
    }
    let mut png_bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png_bytes, bmp.width, bmp.height);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().expect("png header");
        w.write_image_data(&rgb).expect("png data");
    }
    let mut out = PdfDocument::from_image(&png_bytes).expect("from_image");
    std::fs::write(&args[2], out.snapshot_bytes().expect("bytes")).expect("write output");
    println!("wrote {} ({}x{} px)", args[2], bmp.width, bmp.height);
}
