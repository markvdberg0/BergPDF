//! Windows: pages go to the printer through GDI.
//!
//! This is the only module of the workspace that uses `unsafe`: the GDI printing functions are raw C APIs.
//! The unsafe blocks are small, each has the reason it is sound next to it, and every handle is released on
//! every path (`PrinterDc` closes the device context when it goes out of scope).
#![allow(unsafe_code)]

use crate::{PrintError, PrintRequest, Printer, place};
use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::geom::Rotation;
use pdf_engine::render::with_session;
use std::sync::Arc;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDCW, DIB_RGB_COLORS, DeleteDC, GetDeviceCaps,
    HALFTONE, HDC, HORZRES, LOGPIXELSX, LOGPIXELSY, SRCCOPY, SetStretchBltMode, StretchDIBits,
    VERTRES,
};
use windows::Win32::Graphics::Printing::{
    EnumPrintersW, GetDefaultPrinterW, PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL,
    PRINTER_INFO_4W,
};
use windows::Win32::Storage::Xps::{AbortDoc, DOCINFOW, EndDoc, EndPage, StartDocW, StartPage};
use windows::core::{PCWSTR, PWSTR};

/// Highest resolution pages are drawn at; the driver scales up for printers with more dots.
const MAX_RENDER_DPI: f64 = 300.0;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The printers of this computer (local and connected network printers).
pub fn printers() -> Vec<Printer> {
    let default = default_printer();
    let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
    let mut needed = 0u32;
    let mut returned = 0u32;
    // SAFETY: the first call only asks how many bytes are needed (it fails with "buffer too small" on
    // purpose); no buffer is given.
    let _ = unsafe { EnumPrintersW(flags, PCWSTR::null(), 4, None, &mut needed, &mut returned) };
    if needed == 0 {
        return Vec::new();
    }
    // A buffer of u64 so that the PRINTER_INFO_4W structures in it are aligned.
    let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
    // SAFETY: `buf` is at least `needed` bytes long, viewed as bytes for the call.
    let bytes =
        unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>(), buf.len() * 8) };
    // SAFETY: `bytes` is a writable buffer of the size the system asked for.
    let ok = unsafe {
        EnumPrintersW(
            flags,
            PCWSTR::null(),
            4,
            Some(bytes),
            &mut needed,
            &mut returned,
        )
    };
    if ok.is_err() {
        return Vec::new();
    }
    // SAFETY: on success the buffer starts with `returned` aligned PRINTER_INFO_4W structures whose strings
    // point into the same buffer, which outlives this loop.
    let infos = unsafe {
        std::slice::from_raw_parts(buf.as_ptr().cast::<PRINTER_INFO_4W>(), returned as usize)
    };
    infos
        .iter()
        .filter_map(|i| {
            // SAFETY: the name is a NUL-terminated string inside `buf` (or null, which `to_string` rejects).
            let name = unsafe { i.pPrinterName.to_string() }.ok()?;
            Some(Printer {
                is_default: name == default,
                name,
            })
        })
        .collect()
}

fn default_printer() -> String {
    let mut len = 0u32;
    // SAFETY: asking for the needed length with no buffer.
    let _ = unsafe { GetDefaultPrinterW(None, &mut len) };
    if len == 0 {
        return String::new();
    }
    let mut buf = vec![0u16; len as usize];
    // SAFETY: `buf` holds `len` UTF-16 units, the size the system asked for.
    let ok = unsafe { GetDefaultPrinterW(Some(PWSTR(buf.as_mut_ptr())), &mut len) };
    if !ok.as_bool() {
        return String::new();
    }
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// A device context of a printer, deleted when dropped.
struct PrinterDc(HDC);

impl PrinterDc {
    fn open(printer: &str) -> Result<Self, PrintError> {
        let name = wide(printer);
        // SAFETY: both strings are NUL-terminated and live for the call; no DEVMODE means the printer's defaults.
        let hdc = unsafe {
            CreateDCW(
                windows::core::w!("WINSPOOL"),
                PCWSTR(name.as_ptr()),
                PCWSTR::null(),
                None,
            )
        };
        if hdc.is_invalid() {
            return Err(PrintError::System(format!(
                "the printer “{printer}” could not be opened"
            )));
        }
        Ok(PrinterDc(hdc))
    }

    fn caps(&self, index: windows::Win32::Graphics::Gdi::GET_DEVICE_CAPS_INDEX) -> i32 {
        // SAFETY: `self.0` is a valid device context for the life of `self`.
        unsafe { GetDeviceCaps(Some(self.0), index) }
    }
}

impl Drop for PrinterDc {
    fn drop(&mut self) {
        // SAFETY: the context was created by `CreateDCW` and is deleted exactly once, here.
        let _ = unsafe { DeleteDC(self.0) };
    }
}

pub fn print(
    bytes: Arc<Vec<u8>>,
    req: &PrintRequest,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<(), PrintError> {
    let doc = PdfDocument::open(bytes.as_ref().clone(), &OpenOptions::default())
        .map_err(|e| PrintError::Invalid(e.to_string()))?;
    let page_ids = doc
        .page_ids()
        .map_err(|e| PrintError::Invalid(e.to_string()))?;
    if let Some(bad) = req.pages.iter().find(|p| **p >= page_ids.len()) {
        return Err(PrintError::Invalid(format!(
            "page {} does not exist",
            bad + 1
        )));
    }
    let dc = PrinterDc::open(&req.printer)?;
    let (area_w, area_h) = (dc.caps(HORZRES), dc.caps(VERTRES));
    let dpi = (
        f64::from(dc.caps(LOGPIXELSX).max(72)),
        f64::from(dc.caps(LOGPIXELSY).max(72)),
    );
    if area_w <= 0 || area_h <= 0 {
        return Err(PrintError::System(
            "the printer reports no printable area".into(),
        ));
    }

    let title = wide(&req.title);
    let output = req.output_file.as_ref().map(|p| wide(&p.to_string_lossy()));
    let info = DOCINFOW {
        cbSize: size_of::<DOCINFOW>() as i32,
        lpszDocName: PCWSTR(title.as_ptr()),
        lpszOutput: output
            .as_ref()
            .map_or(PCWSTR::null(), |o| PCWSTR(o.as_ptr())),
        lpszDatatype: PCWSTR::null(),
        fwType: 0,
    };
    // SAFETY: `info` and the strings it points to outlive the call.
    if unsafe { StartDocW(dc.0, &info) } <= 0 {
        return Err(PrintError::System(
            "the printer did not accept the job".into(),
        ));
    }
    let total = req.pages.len() * req.copies as usize;
    let result = with_session(bytes, |s| {
        let mut done = 0usize;
        for _copy in 0..req.copies {
            for &index in &req.pages {
                let geom = doc.page_geometry(page_ids[index])?;
                let size = geom.view_size(Rotation::R0);
                let at = place((size.width, size.height), (area_w, area_h), dpi, req.scale);
                let view_rotation = if at.rotate {
                    Rotation::R90
                } else {
                    Rotation::R0
                };
                // Draw at the printer's resolution, but not finer than 300 dpi (the driver scales up).
                let shown = geom.view_size(view_rotation);
                let scale = (f64::from(at.width) / shown.width.max(1.0)).min(MAX_RENDER_DPI / 72.0);
                // The whole page in one picture (drawn in tiles), as BGRX for the DIB.
                let (pw, ph) = {
                    let size = geom.view_size(view_rotation);
                    (
                        (size.width * scale).ceil().max(1.0) as u32,
                        (size.height * scale).ceil().max(1.0) as u32,
                    )
                };
                let mut bgrx = vec![0u8; pw as usize * ph as usize * 4];
                s.render_tiled(index, geom, view_rotation, scale, true, &mut |x, y, t| {
                    for row in 0..t.height {
                        let src = &t.rgba
                            [(row * t.width) as usize * 4..((row + 1) * t.width) as usize * 4];
                        let dst = ((y + row) as usize * pw as usize + x as usize) * 4;
                        for (i, px) in src.chunks_exact(4).enumerate() {
                            bgrx[dst + i * 4] = px[2];
                            bgrx[dst + i * 4 + 1] = px[1];
                            bgrx[dst + i * 4 + 2] = px[0];
                        }
                    }
                    Ok(())
                })?;
                send_page(&dc, &bgrx, pw, ph, at.x, at.y, at.width, at.height)
                    .map_err(pdf_engine::EngineError::Render)?;
                done += 1;
                progress(done, total);
            }
        }
        Ok(())
    });
    match result {
        Ok(()) => {
            // SAFETY: the document was started on this context.
            if unsafe { EndDoc(dc.0) } <= 0 {
                return Err(PrintError::System(
                    "the printer reported an error at the end of the job".into(),
                ));
            }
            Ok(())
        }
        Err(e) => {
            // SAFETY: the document was started on this context; this cancels the job.
            let _ = unsafe { AbortDoc(dc.0) };
            Err(PrintError::Render(0, e.to_string()))
        }
    }
}

/// One page: start it, copy the bitmap onto it scaled to the destination box, end it.
#[allow(clippy::too_many_arguments)]
fn send_page(
    dc: &PrinterDc,
    bgrx: &[u8],
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> Result<(), String> {
    let header = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width as i32,
        // Negative: the first row is the top row.
        biHeight: -(height as i32),
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    let info = BITMAPINFO {
        bmiHeader: header,
        ..Default::default()
    };
    // SAFETY: the context is valid; the page is started here and ended below on every path that started it.
    if unsafe { StartPage(dc.0) } <= 0 {
        return Err("the printer did not accept a new page".into());
    }
    // SAFETY: valid context.
    let _ = unsafe { SetStretchBltMode(dc.0, HALFTONE) };
    // SAFETY: `bgrx` holds `width × height` 32-bit pixels, as the header says, and lives for the call.
    let copied = unsafe {
        StretchDIBits(
            dc.0,
            x,
            y,
            w,
            h,
            0,
            0,
            width as i32,
            height as i32,
            Some(bgrx.as_ptr().cast()),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        )
    };
    // SAFETY: ends the page started above.
    let ended = unsafe { EndPage(dc.0) };
    if copied <= 0 {
        return Err("a page could not be drawn on the printer".into());
    }
    if ended <= 0 {
        return Err("the printer could not finish a page".into());
    }
    Ok(())
}
