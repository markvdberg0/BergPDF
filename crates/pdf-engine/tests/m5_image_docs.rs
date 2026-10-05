//! Opening pictures as PDF documents.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use image::{ImageEncoder, codecs::jpeg::JpegEncoder};
use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::geom::Rotation;
use pdf_engine::imagedoc;
use test_support::*;

fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().unwrap();
    wr.write_image_data(&(0..w * h).flat_map(|_| rgba).collect::<Vec<u8>>())
        .unwrap();
    drop(wr);
    out
}

/// A JPEG whose left half is red and right half blue, optionally with an EXIF orientation tag.
fn jpeg_with_orientation(w: u32, h: u32, orientation: Option<u16>) -> Vec<u8> {
    let mut px = Vec::new();
    for _y in 0..h {
        for x in 0..w {
            px.extend_from_slice(if x < w / 2 {
                &[255, 0, 0]
            } else {
                &[0, 0, 255]
            });
        }
    }
    let mut jpg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpg, 95)
        .write_image(&px, w, h, image::ExtendedColorType::Rgb8)
        .unwrap();
    let Some(o) = orientation else { return jpg };
    // Little-endian TIFF with one IFD entry: Orientation (0x0112), SHORT, count 1.
    let mut tiff = Vec::new();
    tiff.extend_from_slice(b"II*\0");
    tiff.extend_from_slice(&8u32.to_le_bytes());
    tiff.extend_from_slice(&1u16.to_le_bytes());
    tiff.extend_from_slice(&0x0112u16.to_le_bytes());
    tiff.extend_from_slice(&3u16.to_le_bytes());
    tiff.extend_from_slice(&1u32.to_le_bytes());
    tiff.extend_from_slice(&o.to_le_bytes());
    tiff.extend_from_slice(&[0, 0]);
    tiff.extend_from_slice(&0u32.to_le_bytes());
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend_from_slice(&tiff);
    let mut seg = vec![0xFF, 0xE1];
    seg.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
    seg.extend_from_slice(&app1);
    let mut out = jpg[..2].to_vec();
    out.extend_from_slice(&seg);
    out.extend_from_slice(&jpg[2..]);
    out
}

#[test]
fn png_becomes_a_single_page_with_a4_long_side_and_keeps_aspect() {
    let doc = PdfDocument::from_image(&png(400, 200, [0, 200, 0, 255])).unwrap();
    assert_eq!(doc.page_count(), 1);
    let g = doc.pages().unwrap()[0].geometry;
    let s = g.view_size(Rotation::R0);
    assert!(
        (s.width - 842.0).abs() < 0.01 && (s.height - 421.0).abs() < 0.01,
        "{s:?}"
    );
    // Independent renderer shows the picture filling the page.
    let mut d = doc;
    let bytes = d.snapshot_bytes().unwrap();
    if let Some((w, h, px)) = poppler_render(&bytes, 1, 36) {
        let i = (((h / 2) * w + w / 2) * 3) as usize;
        assert!(
            px[i + 1] > 150 && px[i] < 80,
            "green expected, got {:?}",
            &px[i..i + 3]
        );
    } else {
        assert!(!oracles_required(), "poppler required");
    }
}

#[test]
fn jpeg_is_embedded_without_recompression() {
    let jpg = jpeg_with_orientation(120, 80, None);
    let doc = PdfDocument::from_image(&jpg).unwrap();
    let bytes = doc.original_bytes().as_ref().clone();
    assert!(
        bytes.windows(jpg.len()).any(|w| w == jpg.as_slice()),
        "the original JPEG bytes must appear verbatim in the PDF"
    );
}

#[test]
fn exif_orientation_is_applied_via_page_rotate() {
    // Stored landscape 200×100 with orientation 6 (needs a 90° clockwise turn) → displays portrait.
    let doc = PdfDocument::from_image(&jpeg_with_orientation(200, 100, Some(6))).unwrap();
    let g = doc.pages().unwrap()[0].geometry;
    let shown = g.view_size(Rotation::R0);
    assert!(shown.height > shown.width, "portrait expected: {shown:?}");
    let mut d = doc;
    let bytes = d.snapshot_bytes().unwrap();
    if let Some((w, h, px)) = poppler_render(&bytes, 1, 36) {
        assert!(h > w);
        // Rotating a red-left/blue-right picture 90° clockwise puts red at the TOP.
        let top = ((h / 8 * w + w / 2) * 3) as usize;
        let bottom = (((h - h / 8) * w + w / 2) * 3) as usize;
        assert!(
            px[top] > 150 && px[top + 2] < 100,
            "top should be red: {:?}",
            &px[top..top + 3]
        );
        assert!(
            px[bottom + 2] > 150 && px[bottom] < 100,
            "bottom should be blue"
        );
    } else {
        assert!(!oracles_required(), "poppler required");
    }
    assert_eq!(
        pdf_engine::imageembed::jpeg_orientation(&jpeg_with_orientation(8, 8, Some(3))),
        3
    );
    assert_eq!(
        pdf_engine::imageembed::jpeg_orientation(&jpeg_with_orientation(8, 8, None)),
        1
    );
}

#[test]
fn unsupported_and_oversized_inputs_are_refused() {
    assert!(!imagedoc::is_supported_image(b"GIF89a....."));
    assert!(PdfDocument::from_image(b"not an image").is_err());
    assert!(PdfDocument::from_image(b"GIF89a\x01\0\x01\0").is_err());
    // Header claims a huge PNG: refused before any pixel allocation.
    let mut huge = png(2, 2, [0, 0, 0, 255]);
    // IHDR width/height live at bytes 16..24 of the file.
    huge[16..20].copy_from_slice(&40_000u32.to_be_bytes());
    huge[20..24].copy_from_slice(&40_000u32.to_be_bytes());
    assert!(PdfDocument::from_image(&huge).is_err());
}

#[test]
fn image_can_be_inserted_as_a_page_of_an_existing_document() {
    let mut doc = PdfDocument::open(
        test_support::fixtures::helvetica_lines(),
        &OpenOptions::default(),
    )
    .unwrap();
    doc.transact(|tx| imagedoc::insert_image_page(tx, 1, &png(100, 300, [255, 0, 0, 255])))
        .unwrap();
    assert_eq!(doc.page_count(), 2);
    let s = doc.pages().unwrap()[1].geometry.view_size(Rotation::R0);
    assert!((s.height - 842.0).abs() < 0.01 && s.width < s.height);
}
