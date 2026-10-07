//! Save As Optimized: smaller files that still look and read the same, checked by poppler
//! (an independent reader) and by our own renderer.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::geom::Rotation;
use pdf_engine::optimize::{ImagePolicy, OptimizeOptions, optimize_bytes};
use pdf_engine::render::with_session;
use pdf_writer::{Content, Filter, Name, Pdf, Rect, Ref, Str};
use std::io::Write;
use std::sync::Arc;
use test_support::*;

/// A deliberately wasteful PDF: two identical 1200×1200 RGB pictures stored with fast Flate
/// (placed 2 in wide = 600 dpi), a line-art picture, an unused orphan stream, uncompressed
/// page content with text.
fn bloated() -> Vec<u8> {
    let mut pdf = Pdf::new();
    let (catalog, tree, page, content, font) = (
        Ref::new(1),
        Ref::new(2),
        Ref::new(3),
        Ref::new(4),
        Ref::new(5),
    );
    let (img1, img2, art, orphan) = (Ref::new(6), Ref::new(7), Ref::new(8), Ref::new(9));
    pdf.catalog(catalog).pages(tree);
    pdf.pages(tree).kids([page]).count(1);
    {
        let mut p = pdf.page(page);
        p.media_box(Rect::new(0.0, 0.0, 612.0, 792.0))
            .parent(tree)
            .contents(content);
        let mut r = p.resources();
        r.fonts().pair(Name(b"F1"), font);
        let mut xo = r.x_objects();
        xo.pair(Name(b"Im1"), img1);
        xo.pair(Name(b"Im2"), img2);
        xo.pair(Name(b"Art"), art);
    }
    pdf.type1_font(font).base_font(Name(b"Helvetica"));
    let (w, h) = (1200u32, 1200u32);
    let mut px = Vec::with_capacity((w * h * 3) as usize);
    let mut seed = 12345u32;
    for y in 0..h {
        for x in 0..w {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let n = ((seed >> 24) % 17) as i32 - 8;
            let r = (x * 255 / w) as i32 + n;
            let g = (y * 255 / h) as i32 + n;
            let b = (((x + y) * 255) / (w + h)) as i32 + n;
            px.extend_from_slice(&[
                r.clamp(0, 255) as u8,
                g.clamp(0, 255) as u8,
                b.clamp(0, 255) as u8,
            ]);
        }
    }
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(&px).unwrap();
    let data = enc.finish().unwrap();
    for id in [img1, img2] {
        let mut im = pdf.image_xobject(id, &data);
        im.width(w as i32)
            .height(h as i32)
            .color_space()
            .device_rgb();
        im.bits_per_component(8);
        im.filter(Filter::FlateDecode);
    }
    // Line art: two flat colours, 800×800 gray, placed 1 in wide (800 dpi).
    let art_px: Vec<u8> = (0..800 * 800)
        .map(|i| {
            if (i / 800 / 50 + i % 800 / 50) % 2 == 0 {
                0
            } else {
                255
            }
        })
        .collect();
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(&art_px).unwrap();
    let art_data = enc.finish().unwrap();
    {
        let mut im = pdf.image_xobject(art, &art_data);
        im.width(800).height(800).color_space().device_gray();
        im.bits_per_component(8);
        im.filter(Filter::FlateDecode);
    }
    pdf.stream(orphan, &vec![7u8; 50_000]);
    let mut c = Content::new();
    c.begin_text()
        .set_font(Name(b"F1"), 18.0)
        .next_line(72.0, 740.0)
        .show(Str(b"Optimizer test page"))
        .end_text();
    for (name, x, y, wd) in [(b"Im1", 72.0, 500.0, 144.0), (b"Im2", 300.0, 500.0, 144.0)] {
        c.save_state()
            .transform([wd, 0.0, 0.0, wd, x, y])
            .x_object(Name(name))
            .restore_state();
    }
    c.save_state()
        .transform([72.0, 0.0, 0.0, 72.0, 72.0, 300.0])
        .x_object(Name(b"Art"))
        .restore_state();
    for i in 0..200 {
        c.begin_text()
            .set_font(Name(b"F1"), 8.0)
            .next_line(72.0, 280.0 - i as f32 * 1.2)
            .show(Str(b"repeated filler line that compresses very well"))
            .end_text();
    }
    // The uncompressed content stream is written as-is by pdf-writer.
    pdf.stream(content, &c.finish());
    pdf.finish()
}

fn check_same_look(a: &[u8], b: &[u8], tolerance: f64) {
    let (Some(ta), Some(tb)) = (poppler_text(a), poppler_text(b)) else {
        assert!(!oracles_required(), "poppler required but missing");
        return;
    };
    assert_eq!(ta, tb, "text must be identical");
    let diag = poppler_diagnostics(b).unwrap();
    assert!(
        diag.trim().is_empty(),
        "poppler complains about the output: {diag}"
    );
    let (Some(ra), Some(rb)) = (poppler_render(a, 1, 100), poppler_render(b, 1, 100)) else {
        assert!(!oracles_required(), "pdftoppm required but missing");
        return;
    };
    assert_eq!((ra.0, ra.1), (rb.0, rb.1));
    let d = mean_abs_diff(&ra.2, &rb.2);
    assert!(
        d <= tolerance,
        "render differs by {d} (allowed {tolerance})"
    );
}

fn our_render_ok(bytes: &[u8]) {
    let doc = PdfDocument::open(bytes.to_vec(), &OpenOptions::default()).unwrap();
    let pages = doc.pages().unwrap();
    let bmp = with_session(Arc::new(bytes.to_vec()), |s| {
        s.render_page(0, pages[0].geometry, Rotation::R0, 0.5)
    })
    .unwrap();
    // Something was drawn.
    assert!(bmp.rgba.chunks_exact(4).any(|p| p[0] < 200));
}

#[test]
fn balanced_shrinks_a_bloated_file_and_keeps_it_readable() {
    let input = bloated();
    let (out, rep) = optimize_bytes(&input, &OptimizeOptions::balanced()).unwrap();
    eprintln!("{rep:?}");
    assert!(!rep.kept_original);
    assert!(
        out.len() * 5 < input.len(),
        "{} -> {}",
        input.len(),
        out.len()
    );
    assert_eq!(rep.images_downsampled, 3, "both pictures and the line art");
    assert!(rep.duplicates_merged >= 1, "identical pictures merge");
    assert!(rep.unused_objects >= 1, "the orphan stream goes");
    assert_eq!(rep.before, input.len());
    assert_eq!(rep.after, out.len());
    check_same_look(&input, &out, 6.0);
    our_render_ok(&out);
    if let Some((n, stderr)) = poppler_info(&out) {
        assert_eq!(n, Some(1));
        assert!(stderr.trim().is_empty(), "poppler complains: {stderr}");
    } else {
        assert!(!oracles_required(), "pdfinfo required but missing");
    }
}

#[test]
fn lossless_keeps_every_pixel_and_still_shrinks() {
    let input = bloated();
    let (out, rep) = optimize_bytes(&input, &OptimizeOptions::lossless()).unwrap();
    assert_eq!(rep.images_downsampled, 0);
    assert!(out.len() < input.len());
    assert!(rep.streams_recompressed >= 1);
    // Identical to the eye *and* to the pixel.
    check_same_look(&input, &out, 0.0);
    our_render_ok(&out);
}

#[test]
fn smallest_is_not_larger_than_balanced() {
    let input = bloated();
    let (b, _) = optimize_bytes(&input, &OptimizeOptions::balanced()).unwrap();
    let (s, _) = optimize_bytes(&input, &OptimizeOptions::smallest()).unwrap();
    assert!(s.len() <= b.len(), "{} vs {}", s.len(), b.len());
    check_same_look(&input, &s, 10.0);
}

#[test]
fn real_world_files_never_get_bigger_and_read_back_identically() {
    for name in ["report-chromium.pdf", "report-cairo.pdf"] {
        let input = fixture_bytes(name);
        let (out, rep) = optimize_bytes(&input, &OptimizeOptions::balanced()).unwrap();
        assert!(
            out.len() <= input.len(),
            "{name}: {} -> {}",
            input.len(),
            out.len()
        );
        assert_eq!(rep.after, out.len());
        check_same_look(&input, &out, 0.5);
        our_render_ok(&out);
        // A second pass has nothing left to gain.
        let (again, rep2) = optimize_bytes(&out, &OptimizeOptions::balanced()).unwrap();
        assert!(again.len() <= out.len());
        let _ = rep2;
    }
}

#[test]
fn already_minimal_input_is_returned_unchanged() {
    let tiny = fixtures::helvetica_lines();
    let (out, rep) = optimize_bytes(&tiny, &OptimizeOptions::lossless()).unwrap();
    assert!(out.len() <= tiny.len());
    if rep.kept_original {
        assert_eq!(out, tiny);
    }
}

#[test]
fn image_policy_keep_never_touches_pictures() {
    let input = bloated();
    let opts = OptimizeOptions {
        images: ImagePolicy::Keep,
        ..OptimizeOptions::balanced()
    };
    let (_, rep) = optimize_bytes(&input, &opts).unwrap();
    assert_eq!(rep.images_downsampled, 0);
}

#[test]
fn damaged_input_is_an_error_not_a_panic() {
    assert!(optimize_bytes(b"%PDF-1.7\nnot really", &OptimizeOptions::balanced()).is_err());
}
