//! Generate application icons (PNG, ICO, ICNS) from the logo master `assets/brand/logo-master.png`.

use std::io;
use std::path::Path;

fn master() -> io::Result<&'static brand::Image> {
    static MASTER: std::sync::OnceLock<Option<brand::Image>> = std::sync::OnceLock::new();
    MASTER
        .get_or_init(|| brand::decode_png(include_bytes!("../../assets/brand/logo-master.png")))
        .as_ref()
        .ok_or_else(|| io::Error::other("assets/brand/logo-master.png is not a readable 8-bit PNG"))
}

fn png_bytes(size: u32) -> io::Result<Vec<u8>> {
    let rgba = brand::icon_from_master(master()?, size);
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, size, size);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(io::Error::other)?;
        w.write_image_data(&rgba).map_err(io::Error::other)?;
    }
    Ok(out)
}

/// ICO container with PNG-compressed images (supported since Windows Vista).
pub fn ico(sizes: &[u32]) -> io::Result<Vec<u8>> {
    let images: Vec<(u32, Vec<u8>)> = sizes
        .iter()
        .map(|s| png_bytes(*s).map(|b| (*s, b)))
        .collect::<io::Result<_>>()?;
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len();
    for (s, b) in &images {
        out.push(if *s >= 256 { 0 } else { *s as u8 });
        out.push(if *s >= 256 { 0 } else { *s as u8 });
        out.extend_from_slice(&[0, 0]); // palette, reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(b.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += b.len();
    }
    for (_, b) in images {
        out.extend_from_slice(&b);
    }
    Ok(out)
}

/// ICNS container with PNG payloads (macOS 10.7+).
pub fn icns() -> io::Result<Vec<u8>> {
    // (type, pixel size)
    let table: [(&[u8; 4], u32); 7] = [
        (b"icp4", 16),
        (b"icp5", 32),
        (b"icp6", 64),
        (b"ic07", 128),
        (b"ic08", 256),
        (b"ic09", 512),
        (b"ic10", 1024),
    ];
    let mut body = Vec::new();
    for (ty, size) in table {
        let b = png_bytes(size)?;
        body.extend_from_slice(ty);
        body.extend_from_slice(&((b.len() + 8) as u32).to_be_bytes());
        body.extend_from_slice(&b);
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Write all icons under `dir`.
pub fn write_all(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("bergpdf.png"), png_bytes(512)?)?;
    std::fs::write(dir.join("bergpdf-256.png"), png_bytes(256)?)?;
    std::fs::write(dir.join("bergpdf.ico"), ico(&[16, 32, 48, 64, 128, 256])?)?;
    std::fs::write(dir.join("bergpdf.icns"), icns()?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn containers_have_valid_headers() {
        let i = ico(&[16, 32]).unwrap();
        assert_eq!(&i[..6], &[0, 0, 1, 0, 2, 0]);
        let c = icns().unwrap();
        assert_eq!(&c[..4], b"icns");
        assert_eq!(
            u32::from_be_bytes([c[4], c[5], c[6], c[7]]) as usize,
            c.len()
        );
        assert_eq!(&png_bytes(16).unwrap()[..8], b"\x89PNG\r\n\x1a\n");
    }
}
