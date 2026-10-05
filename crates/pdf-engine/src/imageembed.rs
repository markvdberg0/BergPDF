//! Embedding raster images as PDF Image XObjects.
//!
//! JPEG data (baseline/progressive, 1 or 3 components) is embedded as-is (`DCTDecode`) so
//! the pixels are not re-compressed. PNG and other JPEG variants are decoded, stored as
//! Flate-compressed `DeviceRGB`/`DeviceGray`, with an `SMask` for transparency.
//! Decoded dimensions are capped to guard against decompression bombs.

use crate::doc::Tx;
use crate::error::{EngineError, Result};
use crate::fontembed::zlib;
use image::{DynamicImage, ImageDecoder, ImageFormat};
use lopdf::{Object, ObjectId, Stream, dictionary};
use std::io::Cursor;

/// Maximum accepted image dimension (pixels per side) and total pixels.
pub const MAX_DIM: u32 = 16_384;
/// Maximum total pixels.
pub const MAX_PIXELS: u64 = 120_000_000;

/// An embedded image.
pub struct EmbeddedImage {
    /// The XObject id.
    pub id: ObjectId,
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
}

/// Decode `data` and add it as an Image XObject.
pub fn add_image_xobject(tx: &mut Tx<'_>, data: &[u8]) -> Result<EmbeddedImage> {
    let fmt = image::guess_format(data).map_err(|_| {
        EngineError::Unsupported("unrecognised image format (PNG and JPEG are supported)".into())
    })?;
    if !matches!(fmt, ImageFormat::Png | ImageFormat::Jpeg) {
        return Err(EngineError::Unsupported(
            "only PNG and JPEG images are supported".into(),
        ));
    }
    // Header-only size check before decoding pixels.
    let (w, h, color) = {
        let dec: Box<dyn ImageDecoder> = match fmt {
            ImageFormat::Png => {
                Box::new(image::codecs::png::PngDecoder::new(Cursor::new(data)).map_err(img_err)?)
            }
            _ => {
                Box::new(image::codecs::jpeg::JpegDecoder::new(Cursor::new(data)).map_err(img_err)?)
            }
        };
        let (w, h) = dec.dimensions();
        (w, h, dec.color_type())
    };
    if w == 0 || h == 0 || w > MAX_DIM || h > MAX_DIM || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(EngineError::LimitExceeded(format!(
            "image {w}×{h} is too large"
        )));
    }
    if fmt == ImageFormat::Jpeg {
        let comps = match color {
            image::ColorType::L8 => Some("DeviceGray"),
            image::ColorType::Rgb8 => Some("DeviceRGB"),
            _ => None,
        };
        if let Some(cs) = comps {
            let mut s = Stream::new(
                dictionary! {
                    "Type" => "XObject", "Subtype" => "Image",
                    "Width" => i64::from(w), "Height" => i64::from(h),
                    "ColorSpace" => cs, "BitsPerComponent" => 8i64, "Filter" => "DCTDecode",
                },
                data.to_vec(),
            );
            s.allows_compression = false;
            let id = tx.add(Object::Stream(s));
            return Ok(EmbeddedImage {
                id,
                width: w,
                height: h,
            });
        }
    }
    let img = image::load_from_memory_with_format(data, fmt).map_err(img_err)?;
    let (rgb, alpha, gray) = split_channels(&img);
    let (cs, pix) = if gray {
        ("DeviceGray", rgb)
    } else {
        ("DeviceRGB", rgb)
    };
    let mut dict = dictionary! {
        "Type" => "XObject", "Subtype" => "Image",
        "Width" => i64::from(w), "Height" => i64::from(h),
        "ColorSpace" => cs, "BitsPerComponent" => 8i64, "Filter" => "FlateDecode",
    };
    if let Some(a) = alpha {
        let mut sm = Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image",
                "Width" => i64::from(w), "Height" => i64::from(h),
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8i64, "Filter" => "FlateDecode",
            },
            zlib(&a),
        );
        sm.allows_compression = false;
        let sid = tx.add(Object::Stream(sm));
        dict.set("SMask", Object::Reference(sid));
    }
    let mut s = Stream::new(dict, zlib(&pix));
    s.allows_compression = false;
    let id = tx.add(Object::Stream(s));
    Ok(EmbeddedImage {
        id,
        width: w,
        height: h,
    })
}

fn img_err(e: image::ImageError) -> EngineError {
    EngineError::Unsupported(format!("image could not be read: {e}"))
}

/// Returns (colour samples, optional alpha samples, is_gray).
fn split_channels(img: &DynamicImage) -> (Vec<u8>, Option<Vec<u8>>, bool) {
    use image::ColorType as C;
    match img.color() {
        C::L8 | C::L16 => (img.to_luma8().into_raw(), None, true),
        C::La8 | C::La16 => {
            let la = img.to_luma_alpha8();
            let (mut g, mut a) = (
                Vec::with_capacity(la.len() / 2),
                Vec::with_capacity(la.len() / 2),
            );
            for p in la.pixels() {
                g.push(p.0[0]);
                a.push(p.0[1]);
            }
            (g, Some(a), true)
        }
        C::Rgba8 | C::Rgba16 | C::Rgba32F => {
            let rgba = img.to_rgba8();
            let (mut c, mut a) = (
                Vec::with_capacity(rgba.len() / 4 * 3),
                Vec::with_capacity(rgba.len() / 4),
            );
            for p in rgba.pixels() {
                c.extend_from_slice(&p.0[..3]);
                a.push(p.0[3]);
            }
            let alpha = if a.iter().all(|v| *v == 255) {
                None
            } else {
                Some(a)
            };
            (c, alpha, false)
        }
        _ => (img.to_rgb8().into_raw(), None, false),
    }
}

/// Pixel dimensions of a PNG/JPEG without decoding the pixels (so limits can be enforced first).
pub fn dimensions(data: &[u8]) -> Result<(u32, u32)> {
    let fmt = image::guess_format(data).map_err(|_| {
        EngineError::Unsupported("unrecognised image format (PNG and JPEG are supported)".into())
    })?;
    let dec: Box<dyn ImageDecoder> = match fmt {
        ImageFormat::Png => {
            Box::new(image::codecs::png::PngDecoder::new(Cursor::new(data)).map_err(img_err)?)
        }
        ImageFormat::Jpeg => {
            Box::new(image::codecs::jpeg::JpegDecoder::new(Cursor::new(data)).map_err(img_err)?)
        }
        _ => {
            return Err(EngineError::Unsupported(
                "only PNG and JPEG images are supported".into(),
            ));
        }
    };
    let (w, h) = dec.dimensions();
    if w == 0 || h == 0 || w > MAX_DIM || h > MAX_DIM || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(EngineError::LimitExceeded(format!(
            "image {w}×{h} is too large"
        )));
    }
    Ok((w, h))
}

/// EXIF orientation (1–8) of a JPEG, 1 when absent or unreadable.
pub fn jpeg_orientation(data: &[u8]) -> u8 {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return 1;
    }
    let mut i = 2;
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            return 1;
        }
        let marker = data[i + 1];
        if marker == 0xDA || marker == 0xD9 {
            return 1; // start of scan / end of image: no EXIF before pixel data
        }
        if (0xD0..=0xD8).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        let len = usize::from(u16::from_be_bytes([data[i + 2], data[i + 3]]));
        if len < 2 || i + 2 + len > data.len() {
            return 1;
        }
        let seg = &data[i + 4..i + 2 + len];
        if marker == 0xE1 && seg.starts_with(b"Exif\0\0") {
            return exif_orientation(&seg[6..]).unwrap_or(1);
        }
        i += 2 + len;
    }
    1
}

fn exif_orientation(tiff: &[u8]) -> Option<u8> {
    let little = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16at = |o: usize| -> Option<u16> {
        let b = tiff.get(o..o + 2)?;
        Some(if little {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    };
    let u32at = |o: usize| -> Option<u32> {
        let b = tiff.get(o..o + 4)?;
        Some(if little {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    };
    let ifd = u32at(4)? as usize;
    let n = usize::from(u16at(ifd)?);
    for k in 0..n.min(64) {
        let e = ifd + 2 + k * 12;
        if u16at(e)? == 0x0112 {
            let v = u16at(e + 8)?;
            return u8::try_from(v).ok().filter(|v| (1..=8).contains(v));
        }
    }
    None
}
