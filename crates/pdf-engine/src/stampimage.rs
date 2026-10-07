//! Pictures used as stamps and signatures: a decoded RGB picture with optional transparency.
//!
//! A scanned or photographed signature is dark ink on white paper, so [`StampImage::from_bytes`] can
//! turn the paper transparent and trim the empty margins, which makes it sit on the page like a real
//! signature. A logo or stamp that already has transparency keeps it.

use crate::doc::Tx;
use crate::error::{EngineError, Result};
use crate::fontembed::zlib;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};

/// Longest side kept, in pixels. Larger pictures are scaled down so a placed signature stays small
/// in the file.
pub const MAX_SIDE: u32 = 1600;

/// A decoded picture: 8-bit RGB and, when it has transparency, an 8-bit alpha channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StampImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width × height × 3` colour samples.
    pub rgb: Vec<u8>,
    /// `width × height` opacity samples (`None` = fully opaque).
    pub alpha: Option<Vec<u8>>,
}

impl StampImage {
    /// Height divided by width.
    pub fn aspect(&self) -> f64 {
        f64::from(self.height) / f64::from(self.width.max(1))
    }

    /// Decode a PNG or JPEG. With `ink_on_paper` a picture without transparency of its own has its
    /// paper (the light background) made transparent and its empty margins cut away.
    pub fn from_bytes(data: &[u8], ink_on_paper: bool) -> Result<Self> {
        // Size limits first, before any pixel is decoded.
        crate::imageembed::dimensions(data)?;
        let mut img = image::load_from_memory(data)
            .map_err(|e| EngineError::Unsupported(format!("image could not be read: {e}")))?;
        if image::guess_format(data).ok() == Some(image::ImageFormat::Jpeg) {
            let o = crate::imageembed::jpeg_orientation(data);
            if let Some(orientation) = image::metadata::Orientation::from_exif(o) {
                img.apply_orientation(orientation);
            }
        }
        if img.width().max(img.height()) > MAX_SIDE {
            img = img.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Lanczos3);
        }
        let rgba = img.to_rgba8();
        let (w, h) = (rgba.width(), rgba.height());
        let mut pixels = rgba.into_raw();
        let has_alpha = pixels.chunks_exact(4).any(|p| p[3] != 255);
        if ink_on_paper && !has_alpha {
            key_out_paper(&mut pixels);
        }
        let mut out = Self::from_rgba(w, h, &pixels)?;
        if ink_on_paper {
            out = out.trimmed()?;
        }
        Ok(out)
    }

    /// From `width × height` RGBA samples.
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Self> {
        if width == 0 || height == 0 || rgba.len() != (width as usize) * (height as usize) * 4 {
            return Err(EngineError::Unsupported("the picture is empty".into()));
        }
        let n = (width as usize) * (height as usize);
        let (mut rgb, mut alpha) = (Vec::with_capacity(n * 3), Vec::with_capacity(n));
        for p in rgba.chunks_exact(4) {
            rgb.extend_from_slice(&p[..3]);
            alpha.push(p[3]);
        }
        let alpha = (!alpha.iter().all(|a| *a == 255)).then_some(alpha);
        Ok(Self {
            width,
            height,
            rgb,
            alpha,
        })
    }

    /// RGBA samples (opaque where there is no alpha).
    pub fn to_rgba(&self) -> Vec<u8> {
        let n = (self.width as usize) * (self.height as usize);
        let mut out = Vec::with_capacity(n * 4);
        for i in 0..n {
            out.extend_from_slice(&self.rgb[i * 3..i * 3 + 3]);
            out.push(self.alpha.as_ref().map_or(255, |a| a[i]));
        }
        out
    }

    /// As a PNG file (for keeping a copy on disk).
    pub fn to_png(&self) -> Result<Vec<u8>> {
        use image::ImageEncoder;
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(
                &self.to_rgba(),
                self.width,
                self.height,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| EngineError::Unsupported(format!("png: {e}")))?;
        Ok(out)
    }

    /// Cut away the margins that are (nearly) transparent, keeping a few pixels around the rest.
    fn trimmed(&self) -> Result<Self> {
        let Some(alpha) = &self.alpha else {
            return Ok(self.clone());
        };
        let (w, h) = (self.width as usize, self.height as usize);
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
        for y in 0..h {
            for x in 0..w {
                if alpha[y * w + x] > 24 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        if x1 < x0 || y1 < y0 {
            return Err(EngineError::Unsupported(
                "the picture has nothing visible in it".into(),
            ));
        }
        let pad = 4usize;
        let (x0, y0) = (x0.saturating_sub(pad), y0.saturating_sub(pad));
        let (x1, y1) = ((x1 + pad).min(w - 1), (y1 + pad).min(h - 1));
        let (nw, nh) = (x1 - x0 + 1, y1 - y0 + 1);
        let (mut rgb, mut a) = (Vec::with_capacity(nw * nh * 3), Vec::with_capacity(nw * nh));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = y * w + x;
                rgb.extend_from_slice(&self.rgb[i * 3..i * 3 + 3]);
                a.push(alpha[i]);
            }
        }
        Ok(Self {
            width: nw as u32,
            height: nh as u32,
            rgb,
            alpha: Some(a),
        })
    }

    /// Write the picture (and its mask) into the document as Image XObjects.
    pub(crate) fn add_to(&self, tx: &mut Tx<'_>) -> ObjectId {
        let mut dict = dictionary! {
            "Type" => "XObject", "Subtype" => "Image",
            "Width" => i64::from(self.width), "Height" => i64::from(self.height),
            "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8i64, "Filter" => "FlateDecode",
        };
        if let Some(a) = &self.alpha {
            let mut sm = Stream::new(
                dictionary! {
                    "Type" => "XObject", "Subtype" => "Image",
                    "Width" => i64::from(self.width), "Height" => i64::from(self.height),
                    "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8i64,
                    "Filter" => "FlateDecode",
                },
                zlib(a),
            );
            sm.allows_compression = false;
            let sid = tx.add(Object::Stream(sm));
            dict.set("SMask", Object::Reference(sid));
        }
        let mut s = Stream::new(dict, zlib(&self.rgb));
        s.allows_compression = false;
        tx.add(Object::Stream(s))
    }

    /// Read back a picture written by [`add_to`](Self::add_to).
    pub(crate) fn read_from(doc: &Document, image: &Dictionary, stream: &Stream) -> Option<Self> {
        let num = |d: &Dictionary, k: &[u8]| {
            crate::objutil::dict_num(doc, d, k)
                .filter(|v| *v >= 1.0 && *v <= f64::from(crate::imageembed::MAX_DIM))
                .map(|v| v as u32)
        };
        let (w, h) = (num(image, b"Width")?, num(image, b"Height")?);
        if u64::from(w) * u64::from(h) > crate::imageembed::MAX_PIXELS {
            return None;
        }
        let n = (w as usize) * (h as usize);
        let rgb = stream.decompressed_content().ok()?;
        if rgb.len() != n * 3 {
            return None;
        }
        let alpha = match image
            .get(b"SMask")
            .ok()
            .and_then(|o| crate::objutil::deref(doc, o))
        {
            Some(Object::Stream(sm)) => {
                let a = sm.decompressed_content().ok()?;
                if a.len() != n {
                    return None;
                }
                Some(a)
            }
            _ => None,
        };
        Some(Self {
            width: w,
            height: h,
            rgb,
            alpha,
        })
    }
}

/// Make the paper transparent: the light background level is found from the picture itself, pixels
/// at that level become fully transparent, dark ink stays opaque, in between it fades.
fn key_out_paper(rgba: &mut [u8]) {
    // Brightness of a pixel is its darkest channel: coloured ink is dark in at least one.
    let mut hist = [0usize; 256];
    for p in rgba.chunks_exact(4) {
        hist[usize::from(p[0].min(p[1]).min(p[2]))] += 1;
    }
    let total: usize = hist.iter().sum();
    // The paper is the bright end of the picture: the level that 90 % of the pixels are darker than.
    let (mut seen, mut paper) = (0usize, 255usize);
    for (level, count) in hist.iter().enumerate() {
        seen += count;
        if seen * 10 >= total * 9 {
            paper = level;
            break;
        }
    }
    let paper = paper.max(96) as f32;
    let full_ink = paper * 0.45;
    for p in rgba.chunks_exact_mut(4) {
        let m = f32::from(p[0].min(p[1]).min(p[2]));
        let a = ((paper - m) / (paper - full_ink)).clamp(0.0, 1.0);
        p[3] = (a * 255.0).round() as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_of(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut raw = Vec::new();
        for y in 0..h {
            for x in 0..w {
                raw.extend_from_slice(&f(x, y));
            }
        }
        StampImage::from_rgba(w, h, &raw).unwrap().to_png().unwrap()
    }

    #[test]
    fn ink_on_paper_becomes_a_trimmed_transparent_picture() {
        // White paper, a dark stroke in the middle.
        let data = png_of(200, 100, |x, y| {
            if (80..120).contains(&x) && (40..44).contains(&y) {
                [10, 20, 90, 255]
            } else {
                [250, 250, 250, 255]
            }
        });
        let s = StampImage::from_bytes(&data, true).unwrap();
        // Cut down to the stroke plus a small margin.
        assert!(s.width < 60 && s.height < 20, "{}x{}", s.width, s.height);
        let a = s.alpha.as_ref().unwrap();
        assert!(a.contains(&255), "the ink stays opaque");
        assert!(a.contains(&0), "the paper is transparent");
    }

    #[test]
    fn a_picture_with_its_own_transparency_is_kept_as_it_is() {
        let data = png_of(30, 20, |x, _| {
            if x < 15 {
                [200, 0, 0, 255]
            } else {
                [0, 0, 0, 0]
            }
        });
        let s = StampImage::from_bytes(&data, false).unwrap();
        assert_eq!((s.width, s.height), (30, 20));
        let a = s.alpha.unwrap();
        assert_eq!((a[0], a[29]), (255, 0));
    }

    #[test]
    fn an_empty_page_is_refused_and_big_pictures_are_scaled_down() {
        let blank = png_of(50, 50, |_, _| [255, 255, 255, 255]);
        assert!(StampImage::from_bytes(&blank, true).is_err());
        let big = png_of(3200, 800, |_, _| [30, 30, 30, 255]);
        let s = StampImage::from_bytes(&big, false).unwrap();
        assert_eq!(s.width, MAX_SIDE);
        assert_eq!(s.height, 400);
    }
}
