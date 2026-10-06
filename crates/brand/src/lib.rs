//! BergPDF identity: the product name and the logo — two white mountain outlines on a red square
//! (“Berg” is Dutch for mountain).
//!
//! The artwork is a raster master (`assets/brand/logo-master.png`) from which `cargo xtask icons`
//! generates every icon file. The program embeds the 512 px result for its window icon
//! ([`render_rgba`]), and the UI draws a vector trace of the mountains ([`strokes`]) in its own
//! colours. Trace coordinates are in a 100 × 100 box with y pointing down.

use std::io::Cursor;

/// Product name (not yet checked against existing trademarks).
pub const NAME: &str = "BergPDF";

/// Silhouette: left foot, the small step, the high peak, the saddle, the second peak and the right foot.
pub const OUTLINE: &[(f32, f32)] = &[
    (5.2, 74.5),
    (22.6, 50.6),
    (25.3, 53.4),
    (44.3, 24.8),
    (62.5, 51.8),
    (71.4, 41.7),
    (94.8, 74.5),
];

/// Ground line under both mountains.
pub const GROUND: &[(f32, f32)] = &[(5.2, 74.5), (94.8, 74.5)];

/// Crack running down the high peak.
pub const RIDGE_HIGH: &[(f32, f32)] = &[
    (43.2, 30.2),
    (43.4, 40.1),
    (48.4, 44.3),
    (50.5, 53.2),
    (67.2, 66.7),
    (71.4, 72.8),
];

/// Crack running down the second peak.
pub const RIDGE_LOW: &[(f32, f32)] = &[
    (72.2, 46.9),
    (72.6, 52.3),
    (76.5, 55.4),
    (78.6, 62.5),
    (87.4, 67.2),
];

/// Stroke width of the logo in the 100-unit box.
pub const STROKE: f32 = 2.6;

/// All strokes of the logo: `(points, is_accent)`.
pub fn strokes() -> [(&'static [(f32, f32)], bool); 4] {
    [
        (OUTLINE, false),
        (GROUND, false),
        (RIDGE_HIGH, true),
        (RIDGE_LOW, true),
    ]
}

/// A decoded picture, RGBA8.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Decode an 8-bit PNG (grey, RGB or RGBA; palettes are expanded).
pub fn decode_png(bytes: &[u8]) -> Option<Image> {
    let mut dec = png::Decoder::new(Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let px = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px
            .chunks_exact(3)
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => px
            .chunks_exact(2)
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        png::ColorType::Indexed => return None,
    };
    Some(Image {
        width: info.width,
        height: info.height,
        rgba,
    })
}

/// Scale `src` to `size × size` by averaging the source pixels under each target pixel
/// (weighted by alpha, so transparent corners do not bleed colour).
pub fn resize(src: &Image, size: u32) -> Vec<u8> {
    let n = size as usize;
    let (sw, sh) = (src.width as usize, src.height as usize);
    let mut out = vec![0u8; n * n * 4];
    for py in 0..n {
        let (y0, y1) = (
            py as f32 * sh as f32 / n as f32,
            (py + 1) as f32 * sh as f32 / n as f32,
        );
        for px in 0..n {
            let (x0, x1) = (
                px as f32 * sw as f32 / n as f32,
                (px + 1) as f32 * sw as f32 / n as f32,
            );
            let mut acc = [0.0f32; 4];
            let mut area = 0.0f32;
            for sy in (y0.floor() as usize)..(y1.ceil() as usize).min(sh) {
                let wy = (y1.min(sy as f32 + 1.0) - y0.max(sy as f32)).max(0.0);
                for sx in (x0.floor() as usize)..(x1.ceil() as usize).min(sw) {
                    let w = wy * (x1.min(sx as f32 + 1.0) - x0.max(sx as f32)).max(0.0);
                    let i = (sy * sw + sx) * 4;
                    let a = f32::from(src.rgba[i + 3]) / 255.0;
                    acc[0] += f32::from(src.rgba[i]) * a * w;
                    acc[1] += f32::from(src.rgba[i + 1]) * a * w;
                    acc[2] += f32::from(src.rgba[i + 2]) * a * w;
                    acc[3] += a * w;
                    area += w;
                }
            }
            if area > 0.0 && acc[3] > 0.0 {
                let o = (py * n + px) * 4;
                out[o] = (acc[0] / acc[3]).round() as u8;
                out[o + 1] = (acc[1] / acc[3]).round() as u8;
                out[o + 2] = (acc[2] / acc[3]).round() as u8;
                out[o + 3] = (acc[3] / area * 255.0).round() as u8;
            }
        }
    }
    out
}

/// Turn the square logo master into an application icon: scaled to `size`, corners rounded.
pub fn icon_from_master(master: &Image, size: u32) -> Vec<u8> {
    let mut px = resize(master, size);
    let n = size as f32;
    let radius = n * 0.22;
    let half = n / 2.0;
    for y in 0..size {
        for x in 0..size {
            // Signed distance to a rounded square, in pixels (negative inside).
            let (dx, dy) = (
                ((x as f32 + 0.5) - half).abs() - (half - radius),
                ((y as f32 + 0.5) - half).abs() - (half - radius),
            );
            let d = dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - radius;
            let cover = (0.5 - d).clamp(0.0, 1.0);
            let i = ((y * size + x) * 4 + 3) as usize;
            px[i] = (f32::from(px[i]) * cover).round() as u8;
        }
    }
    px
}

/// The application icon (`size × size` RGBA8), from the embedded 512 px icon.
pub fn render_rgba(size: u32) -> Vec<u8> {
    match decode_png(include_bytes!("../../../assets/icons/bergpdf.png")) {
        Some(img) => resize(&img, size),
        None => vec![0; (size * size * 4) as usize],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_stays_inside_the_box() {
        for (poly, _) in strokes() {
            for (x, y) in poly {
                assert!((0.0..=100.0).contains(x) && (0.0..=100.0).contains(y));
            }
        }
    }

    #[test]
    fn the_embedded_icon_is_red_with_white_mountains_and_round_corners() {
        let px = render_rgba(64);
        assert_eq!(px.len(), 64 * 64 * 4);
        assert_eq!(px[3], 0, "corner is transparent");
        let mid = ((10 * 64) + 32) * 4;
        assert!(
            px[mid] > 0xC0 && px[mid + 1] < 0x60 && px[mid + 3] == 255,
            "red field"
        );
        assert!(
            px.chunks_exact(4)
                .any(|p| p[0] > 0xE0 && p[1] > 0xE0 && p[2] > 0xE0 && p[3] == 255),
            "white strokes"
        );
    }

    #[test]
    fn resizing_keeps_a_flat_colour() {
        let img = Image {
            width: 8,
            height: 8,
            rgba: [10, 20, 30, 255].repeat(64),
        };
        assert!(
            resize(&img, 3)
                .chunks_exact(4)
                .all(|p| p == [10, 20, 30, 255])
        );
    }
}
