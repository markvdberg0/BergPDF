//! Turning pictures into PDF pages: "open an image" creates a one-page document, and an image can
//! be inserted as a page of an existing document.
//!
//! A page is sized so the picture's longer side is A4-length (842 pt) with the picture's own
//! aspect ratio; the full-resolution pixels are embedded unchanged (JPEG data is passed through,
//! never re-compressed). JPEG EXIF orientation 3/6/8 is honoured with the page `/Rotate`; the
//! mirrored orientations (2, 4, 5, 7) are not and the picture is shown as stored.

use crate::doc::{OpenOptions, PageId, PdfDocument, Tx};
use crate::error::Result;
use crate::geom::Rect;
use crate::imageembed::{dimensions, jpeg_orientation};
use crate::pagecontent::add_image;
use crate::pageops::insert_blank_page;
use image::ImageFormat;

/// Length of the longer page side in points (A4 long edge).
pub const LONG_SIDE_PT: f64 = 842.0;

/// Whether `data` starts like a PNG or JPEG file.
pub fn is_supported_image(data: &[u8]) -> bool {
    matches!(
        image::guess_format(data),
        Ok(ImageFormat::Png | ImageFormat::Jpeg)
    )
}

/// Page size in points for a picture of `w × h` pixels.
pub fn page_size_for(w: u32, h: u32) -> (f64, f64) {
    let s = LONG_SIDE_PT / f64::from(w.max(h));
    (f64::from(w) * s, f64::from(h) * s)
}

/// Insert `data` (PNG/JPEG) as a new page at `index`. Returns the new page.
pub fn insert_image_page(tx: &mut Tx<'_>, index: usize, data: &[u8]) -> Result<PageId> {
    let (w, h) = dimensions(data)?;
    let (pw, ph) = page_size_for(w, h);
    let page = insert_blank_page(tx, index, pw, ph)?;
    add_image(tx, page, Rect::new(0.0, 0.0, pw, ph), data)?;
    let rotate = match jpeg_orientation(data) {
        3 => 180,
        6 => 90,
        8 => 270,
        _ => 0,
    };
    if rotate != 0 {
        tx.dict_mut(page.0)?.set("Rotate", i64::from(rotate));
    }
    Ok(page)
}

impl PdfDocument {
    /// A new one-page document showing `data` (PNG or JPEG).
    pub fn from_image(data: &[u8]) -> Result<PdfDocument> {
        let mut doc = PdfDocument::new_empty()?;
        doc.transact(|tx| insert_image_page(tx, 0, data))?;
        // Re-open the serialised bytes so the result is an ordinary, fully consistent document.
        let bytes = doc.rewrite_bytes()?;
        PdfDocument::open(bytes, &OpenOptions::default())
    }
}
