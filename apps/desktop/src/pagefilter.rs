//! Rendering-only page filters. These operate on display bitmaps; the PDF is never touched.

/// Comfortable dark reading: invert lightness while preserving hue ("invert + hue-rotate 180°"),
/// so black-on-white text becomes light-on-dark while coloured symbols keep their colour
/// family instead of turning into their complements. Alpha is left unchanged.
///
/// Limitation: photographs are filtered too (they look like negatives with preserved hue);
/// excluding image regions needs content-level information and is future work.
pub fn dark_reading(rgba: &mut [u8]) {
    for p in rgba.chunks_exact_mut(4) {
        let (r, g, b) = (
            f32::from(p[0]) / 255.0,
            f32::from(p[1]) / 255.0,
            f32::from(p[2]) / 255.0,
        );
        let nr = 1.0 - (-0.574 * r + 1.430 * g + 0.144 * b);
        let ng = 1.0 - (0.426 * r + 0.430 * g + 0.144 * b);
        let nb = 1.0 - (0.426 * r + 1.430 * g - 0.856 * b);
        p[0] = (nr.clamp(0.0, 1.0) * 255.0).round() as u8;
        p[1] = (ng.clamp(0.0, 1.0) * 255.0).round() as u8;
        p[2] = (nb.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_becomes_black_black_becomes_white_and_grays_invert() {
        let mut px = [255, 255, 255, 255, 0, 0, 0, 255, 128, 128, 128, 255];
        dark_reading(&mut px);
        assert_eq!(&px[0..4], &[0, 0, 0, 255]);
        assert_eq!(&px[4..8], &[255, 255, 255, 255]);
        assert!((i32::from(px[8]) - 127).abs() <= 1);
    }

    #[test]
    fn hue_family_is_preserved_for_saturated_colours() {
        // Pure red stays red-dominant (not cyan as a plain inversion would give).
        let mut px = [220, 30, 30, 255];
        dark_reading(&mut px);
        assert!(px[0] > px[1] && px[0] > px[2], "{px:?}");
    }
}
