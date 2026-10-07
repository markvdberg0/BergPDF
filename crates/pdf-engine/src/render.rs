//! hayro adapter: revision-tagged byte snapshots in, RGBA tiles and page text out.
//!
//! hayro objects hold `Rc` caches and are not `Send`, so a [`Session`] must be created and
//! used on a single worker thread; it is created from an immutable byte snapshot and never
//! shares state with the editable `lopdf` document.

use crate::error::{EngineError, Result};
use crate::geom::{Affine, PageGeometry, Rect, Rotation};
use crate::text::{self, TextPage};
use hayro::hayro_interpret::hayro_syntax::Pdf;
use hayro::hayro_interpret::{InterpreterCache, InterpreterSettings};
use hayro::vello_cpu::RenderContext;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};
use std::sync::Arc;

/// Maximum pixels in a single rendered tile (guards memory use).
pub const MAX_TILE_PIXELS: u64 = 4096 * 4096;

/// An immutable RGBA8 bitmap with an opaque background (straight == premultiplied).
#[derive(Clone, Debug)]
pub struct Bitmap {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA8 pixels, row-major.
    pub rgba: Vec<u8>,
}

/// A request for one rectangular region of a page at a given scale.
#[derive(Clone, Copy, Debug)]
pub struct TileRequest {
    /// Zero-based page index.
    pub page_index: usize,
    /// Page geometry as known to the editor (single source of truth for coordinates).
    pub geometry: PageGeometry,
    /// Temporary view rotation (not stored in the document).
    pub view_rotation: Rotation,
    /// Device pixels per point.
    pub scale: f64,
    /// Region origin in device pixels within the full-page bitmap.
    pub x: u32,
    /// Region origin in device pixels within the full-page bitmap.
    pub y: u32,
    /// Region width in device pixels.
    pub width: u32,
    /// Region height in device pixels.
    pub height: u32,
}

/// A rendering/extraction session over one byte snapshot.
pub struct Session<'a> {
    pdf: &'a Pdf,
    render_cache: RenderCache<'a>,
    text_cache: InterpreterCache<'a>,
}

/// Parse `bytes` and run `f` with a [`Session`] bound to them.
pub fn with_session<R>(
    bytes: Arc<Vec<u8>>,
    f: impl FnOnce(&Session<'_>) -> Result<R>,
) -> Result<R> {
    let pdf = Pdf::new(bytes).map_err(|e| EngineError::Parse(format!("{e:?}")))?;
    let session = Session {
        pdf: &pdf,
        render_cache: RenderCache::new(),
        text_cache: InterpreterCache::new(),
    };
    f(&session)
}

impl Session<'_> {
    /// Number of pages hayro sees.
    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    /// Render a region of a page.
    pub fn render_tile(&self, req: &TileRequest) -> Result<Bitmap> {
        self.render_tile_with(req, true)
    }

    /// Render a region of a page, optionally without the annotations (their appearance streams, form fields,
    /// comments): what a viewer shows of the page content alone.
    pub fn render_tile_with(&self, req: &TileRequest, annotations: bool) -> Result<Bitmap> {
        if req.width == 0 || req.height == 0 {
            return Err(EngineError::InvalidArgument("empty tile".into()));
        }
        if u64::from(req.width) * u64::from(req.height) > MAX_TILE_PIXELS
            || req.width > u32::from(u16::MAX)
            || req.height > u32::from(u16::MAX)
        {
            return Err(EngineError::LimitExceeded(format!(
                "tile {}x{} exceeds the per-tile pixel budget",
                req.width, req.height
            )));
        }
        let page = self
            .pdf
            .pages()
            .get(req.page_index)
            .ok_or(EngineError::NoSuchPage(req.page_index))?;
        let transform = Affine::translate((-f64::from(req.x), -f64::from(req.y)))
            * Affine::scale(req.scale)
            * req.geometry.pdf_to_view(req.view_rotation);
        let mut ctx = RenderContext::new(req.width as u16, req.height as u16);
        hayro::render_into(
            page,
            &self.render_cache,
            &InterpreterSettings {
                render_annotations: annotations,
                ..InterpreterSettings::default()
            },
            &RenderSettings::default(),
            &mut ctx,
            transform,
        );
        ctx.flush();
        let mut pixmap = hayro::vello_cpu::Pixmap::new(ctx.width(), ctx.height());
        ctx.render_with(
            &mut pixmap,
            &mut hayro::vello_cpu::Resources::default(),
            hayro::vello_cpu::RasterizerSettings {
                target_init: hayro::vello_cpu::TargetInit::Clear(WHITE),
                ..Default::default()
            },
        );
        let _ = PixmapSettings::default();
        Ok(Bitmap {
            width: req.width,
            height: req.height,
            rgba: pixmap.data_as_u8_slice().to_vec(),
        })
    }

    /// Render a whole page at `scale` device pixels per point (convenience for thumbnails
    /// and tests; subject to the tile pixel budget).
    pub fn render_page(
        &self,
        page_index: usize,
        geometry: PageGeometry,
        view_rotation: Rotation,
        scale: f64,
    ) -> Result<Bitmap> {
        let size = geometry.view_size(view_rotation);
        let w = (size.width * scale).ceil().max(1.0) as u32;
        let h = (size.height * scale).ceil().max(1.0) as u32;
        self.render_tile(&TileRequest {
            page_index,
            geometry,
            view_rotation,
            scale,
            x: 0,
            y: 0,
            width: w,
            height: h,
        })
    }

    /// Extract positioned text for a page (PDF user-space quads).
    pub fn extract_text(&self, page_index: usize, geometry: &PageGeometry) -> Result<TextPage> {
        let page = self
            .pdf
            .pages()
            .get(page_index)
            .ok_or(EngineError::NoSuchPage(page_index))?;
        Ok(text::collect(
            page,
            &self.text_cache,
            self.pdf,
            geometry.crop_box,
        ))
    }

    /// Visible crop box in user space.
    pub fn crop_box(&self, page_index: usize) -> Option<Rect> {
        let p = self.pdf.pages().get(page_index)?;
        let b = p.intersected_crop_box();
        Some(Rect::new(b.x0, b.y0, b.x1, b.y1))
    }
}

impl Bitmap {
    /// Encode as a PNG file.
    pub fn to_png(&self) -> Result<Vec<u8>> {
        use image::{ExtendedColorType, ImageEncoder};
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(
                &self.rgba,
                self.width,
                self.height,
                ExtendedColorType::Rgba8,
            )
            .map_err(|e| EngineError::Unsupported(format!("PNG encoding failed: {e}")))?;
        Ok(out)
    }
}

/// Scale (device pixels per point) for exporting a `width × height` pt page at `dpi`, reduced
/// when needed so the bitmap stays within the single-render pixel budget.
pub fn export_scale(width_pt: f64, height_pt: f64, dpi: f64) -> f64 {
    let want = dpi / 72.0;
    let area = (width_pt * want).ceil().max(1.0) * (height_pt * want).ceil().max(1.0);
    let budget = MAX_TILE_PIXELS as f64 * 0.95;
    if area <= budget {
        want
    } else {
        want * (budget / area).sqrt()
    }
}
