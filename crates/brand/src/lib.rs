//! BergPDF identity: the product name and the logo — an outline of the Mont Blanc massif
//! (“Berg” is Dutch for mountain). The logo is plain vector data so the UI can draw it with its
//! own painter, and [`render_rgba`] can rasterise it for window/application icons without any
//! binary asset files or image libraries.
//!
//! Coordinates are in a 100 × 100 box with y pointing down.

/// Product name (a working name until it has been cleared).
pub const NAME: &str = "BergPDF";

/// Mountain outline, left to right: the Aiguille du Midi spires, Mont Maudit, the long rounded
/// Mont Blanc summit, the Dôme du Goûter shoulder and the descent to the right.
pub const OUTLINE: &[(f32, f32)] = &[
    (6.0, 82.0),
    (20.0, 60.0),
    (24.0, 62.0),
    (30.0, 44.0),
    (33.0, 50.0),
    (38.0, 40.0),
    (44.0, 52.0),
    (50.0, 38.0),
    (56.0, 44.0),
    (62.0, 26.0),
    (66.0, 21.0),
    (71.0, 20.0),
    (76.0, 24.0),
    (80.0, 32.0),
    (84.0, 38.0),
    (92.0, 56.0),
    (96.0, 82.0),
];

/// Snow line across the summit dome.
pub const SNOW: &[(f32, f32)] = &[
    (62.0, 42.0),
    (65.0, 37.0),
    (68.0, 42.0),
    (71.0, 37.0),
    (74.0, 42.0),
    (77.0, 37.0),
    (80.0, 42.0),
];

/// Ground line under the massif.
pub const GROUND: &[(f32, f32)] = &[(6.0, 82.0), (96.0, 82.0)];

/// Stroke width of the logo in the 100-unit box.
pub const STROKE: f32 = 3.4;

/// All strokes of the logo: `(points, is_accent)`.
pub fn strokes() -> [(&'static [(f32, f32)], bool); 3] {
    [(OUTLINE, false), (GROUND, false), (SNOW, true)]
}

/// Colours (RGBA).
pub mod color {
    /// Icon background (deep slate).
    pub const BACKGROUND: [u8; 4] = [0x14, 0x21, 0x2E, 0xFF];
    /// Mountain outline.
    pub const OUTLINE: [u8; 4] = [0xF2, 0xF5, 0xF8, 0xFF];
    /// Snow line (copper accent).
    pub const ACCENT: [u8; 4] = [0xE8, 0x83, 0x3A, 0xFF];
}

fn dist_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let len2 = abx * abx + aby * aby;
    let t = if len2 <= f32::EPSILON {
        0.0
    } else {
        (((p.0 - a.0) * abx + (p.1 - a.1) * aby) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (a.0 + t * abx, a.1 + t * aby);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

fn polyline_hit(poly: &[(f32, f32)], p: (f32, f32), half: f32) -> bool {
    poly.windows(2)
        .any(|w| dist_to_segment(p, w[0], w[1]) <= half)
}

fn in_rounded_square(x: f32, y: f32, radius: f32) -> bool {
    // Box is 0..100; corner circles of `radius`.
    let cx = x.clamp(radius, 100.0 - radius);
    let cy = y.clamp(radius, 100.0 - radius);
    (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius
}

/// Rasterise the logo to RGBA8 (`size × size`). With `background` the logo sits on a rounded
/// dark square (application icon); without, only the strokes are drawn over transparency.
/// Uses 4 × 4 supersampling for smooth edges.
pub fn render_rgba(size: u32, background: bool) -> Vec<u8> {
    let n = size as usize;
    let mut out = vec![0u8; n * n * 4];
    let ss = 4usize;
    let half = STROKE / 2.0;
    // Inset the artwork a little so it breathes inside the rounded square.
    let (scale, off) = if background { (0.78, 11.0) } else { (1.0, 0.0) };
    for py in 0..n {
        for px in 0..n {
            let mut acc = [0.0f32; 4];
            for sy in 0..ss {
                for sx in 0..ss {
                    let x = (px as f32 + (sx as f32 + 0.5) / ss as f32) / n as f32 * 100.0;
                    let y = (py as f32 + (sy as f32 + 0.5) / ss as f32) / n as f32 * 100.0;
                    let mut col: Option<[u8; 4]> = None;
                    if background && in_rounded_square(x, y, 22.0) {
                        col = Some(color::BACKGROUND);
                    }
                    let (ax, ay) = ((x - off) / scale, (y - off) / scale);
                    let hh = half;
                    for (poly, accent) in strokes() {
                        if polyline_hit(poly, (ax, ay), hh) {
                            col = Some(if accent {
                                color::ACCENT
                            } else {
                                color::OUTLINE
                            });
                        }
                    }
                    if let Some(c) = col {
                        let a = f32::from(c[3]) / 255.0;
                        acc[0] += f32::from(c[0]) * a;
                        acc[1] += f32::from(c[1]) * a;
                        acc[2] += f32::from(c[2]) * a;
                        acc[3] += a;
                    }
                }
            }
            let cnt = (ss * ss) as f32;
            let a = acc[3] / cnt;
            let i = (py * n + px) * 4;
            if a > 0.0 {
                out[i] = (acc[0] / acc[3]).round() as u8;
                out[i + 1] = (acc[1] / acc[3]).round() as u8;
                out[i + 2] = (acc[2] / acc[3]).round() as u8;
                out[i + 3] = (a * 255.0).round() as u8;
            }
        }
    }
    out
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
    fn rendering_has_ink_and_transparent_corners() {
        let px = render_rgba(64, true);
        assert_eq!(px.len(), 64 * 64 * 4);
        // Corner of the rounded square is transparent; the centre-bottom has the background.
        assert_eq!(px[3], 0);
        let mid = ((48 * 64) + 32) * 4;
        assert_eq!(px[mid + 3], 255);
        // Some pixel is the light outline colour.
        assert!(
            px.chunks_exact(4)
                .any(|p| p[0] > 0xE0 && p[1] > 0xE0 && p[3] == 255)
        );
        // Without background the corner is transparent and strokes exist.
        let bare = render_rgba(64, false);
        assert!(bare.chunks_exact(4).any(|p| p[3] == 255));
        assert_eq!(bare[3], 0);
    }
}
