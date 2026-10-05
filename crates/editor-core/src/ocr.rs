//! Running OCR over document pages: render at 300 dpi, recognise, and map word boxes back to PDF
//! user space so the engine can add them as an invisible text layer.

use pdf_engine::doc::PageId;
use pdf_engine::geom::{PageGeometry, Point, Rect, Rotation};
use pdf_engine::pagecontent::OcrWord;
use pdf_engine::render::{export_scale, with_session};
use std::sync::Arc;

/// Resolution used for recognition.
pub const OCR_DPI: f64 = 300.0;

/// One page to recognise.
#[derive(Clone, Copy, Debug)]
pub struct OcrPageSpec {
    /// Page id.
    pub page: PageId,
    /// Zero-based index in the document.
    pub index: usize,
    /// Visible geometry.
    pub geometry: PageGeometry,
}

/// Recognise `pages` of the document in `bytes`. `progress(done, total)` is called before each
/// page and after the last; returning `false` cancels (the pages finished so far are returned).
pub fn recognize_pages(
    engine: &pdf_ocr::Engine,
    bytes: Arc<Vec<u8>>,
    pages: &[OcrPageSpec],
    progress: &mut dyn FnMut(usize, usize) -> bool,
) -> Result<Vec<(PageId, Vec<OcrWord>)>, pdf_engine::EngineError> {
    let total = pages.len();
    let mut out = Vec::new();
    with_session(bytes, |s| {
        for (n, spec) in pages.iter().enumerate() {
            if !progress(n, total) {
                return Ok(());
            }
            let size = spec.geometry.view_size(Rotation::R0);
            let scale = export_scale(size.width, size.height, OCR_DPI);
            let bmp = s.render_page(spec.index, spec.geometry, Rotation::R0, scale)?;
            let rgb: Vec<u8> = bmp
                .rgba
                .chunks_exact(4)
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect();
            let words = engine
                .recognize_rgb(&rgb, bmp.width, bmp.height)
                .map_err(|e| pdf_engine::EngineError::Unsupported(e.to_string()))?;
            let to_pdf = spec.geometry.view_to_pdf(Rotation::R0);
            let mapped: Vec<OcrWord> = words
                .into_iter()
                .map(|w| {
                    let corners =
                        [(w.x0, w.y0), (w.x1, w.y0), (w.x1, w.y1), (w.x0, w.y1)].map(|(x, y)| {
                            to_pdf * Point::new(f64::from(x) / scale, f64::from(y) / scale)
                        });
                    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                    for c in corners {
                        x0 = x0.min(c.x);
                        y0 = y0.min(c.y);
                        x1 = x1.max(c.x);
                        y1 = y1.max(c.y);
                    }
                    OcrWord {
                        text: w.text,
                        rect: Rect::new(x0, y0, x1, y1),
                    }
                })
                .collect();
            out.push((spec.page, mapped));
        }
        progress(total, total);
        Ok(())
    })?;
    Ok(out)
}
