//! A saved handwritten signature: a few strokes drawn once and stored locally.
//!
//! This is only a *picture* of a signature (it becomes an ink annotation). It is not a digital
//! signature and carries no cryptographic or legal weight by itself.

use serde::{Deserialize, Serialize};

/// Hard limits on what is stored/loaded (the file is user-writable, so treat it as untrusted).
pub const MAX_STROKES: usize = 64;
/// Maximum points in one stroke.
pub const MAX_POINTS_PER_STROKE: usize = 4_000;

/// Strokes normalised so the drawing's width is 1.0 and y grows downward with the same scale.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HandwrittenSignature {
    /// Height divided by width of the drawing.
    pub aspect: f32,
    /// Strokes of `[x, y]` points (x in 0..=1, y in 0..=aspect).
    pub strokes: Vec<Vec<[f32; 2]>>,
}

impl HandwrittenSignature {
    /// Normalise raw canvas strokes (any coordinate system, y down). `None` when empty/degenerate.
    pub fn from_canvas(raw: &[Vec<[f32; 2]>]) -> Option<Self> {
        let pts: Vec<[f32; 2]> = raw.iter().flatten().copied().collect();
        if pts.len() < 2 {
            return None;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in &pts {
            if !p[0].is_finite() || !p[1].is_finite() {
                return None;
            }
            x0 = x0.min(p[0]);
            y0 = y0.min(p[1]);
            x1 = x1.max(p[0]);
            y1 = y1.max(p[1]);
        }
        let (w, h) = (x1 - x0, y1 - y0);
        if w < 4.0 && h < 4.0 {
            return None; // a dot, not a signature
        }
        let scale = w.max(h * 0.2).max(1.0); // width drives scale; avoid division by ~0 for a vertical line
        let strokes: Vec<Vec<[f32; 2]>> = raw
            .iter()
            .filter(|s| !s.is_empty())
            .take(MAX_STROKES)
            .map(|s| {
                s.iter()
                    .take(MAX_POINTS_PER_STROKE)
                    .map(|p| [(p[0] - x0) / scale, (p[1] - y0) / scale])
                    .collect()
            })
            .collect();
        Some(Self {
            aspect: h / scale,
            strokes,
        })
    }

    /// Whether anything is stored.
    pub fn is_empty(&self) -> bool {
        self.strokes.is_empty()
    }

    /// Strokes in PDF user space: `width_pt` wide, centred on `center` (y up).
    pub fn to_user_space(&self, center: (f64, f64), width_pt: f64) -> Vec<Vec<(f64, f64)>> {
        let h = f64::from(self.aspect) * width_pt;
        self.strokes
            .iter()
            .map(|s| {
                s.iter()
                    .map(|p| {
                        (
                            center.0 - width_pt / 2.0 + f64::from(p[0]) * width_pt,
                            center.1 + h / 2.0 - f64::from(p[1]) * width_pt,
                        )
                    })
                    .collect()
            })
            .collect()
    }

    /// Serialise to TOML.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string(self)
    }

    /// Parse from TOML, sanitising anything out of bounds.
    pub fn from_toml(s: &str) -> Option<Self> {
        let mut v: Self = toml::from_str(s).ok()?;
        v.strokes.truncate(MAX_STROKES);
        for st in &mut v.strokes {
            st.truncate(MAX_POINTS_PER_STROKE);
            st.retain(|p| {
                p[0].is_finite()
                    && p[1].is_finite()
                    && (-0.01..=1.01).contains(&p[0])
                    && (-0.01..=20.0).contains(&p[1])
            });
        }
        v.strokes.retain(|s| !s.is_empty());
        if !v.aspect.is_finite() || v.aspect <= 0.0 || v.aspect > 20.0 || v.strokes.is_empty() {
            return None;
        }
        Some(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_and_places_a_signature() {
        let raw = vec![
            vec![[100.0, 50.0], [200.0, 80.0]],
            vec![[120.0, 60.0], [180.0, 55.0]],
        ];
        let s = HandwrittenSignature::from_canvas(&raw).unwrap();
        assert!((s.aspect - 0.3).abs() < 1e-5, "{}", s.aspect);
        assert!(s.strokes[0][0] == [0.0, 0.0] && (s.strokes[0][1][0] - 1.0).abs() < 1e-6);
        let placed = s.to_user_space((300.0, 400.0), 150.0);
        let xs: Vec<f64> = placed.iter().flatten().map(|p| p.0).collect();
        assert!((xs.iter().cloned().fold(f64::MAX, f64::min) - 225.0).abs() < 1e-3);
        assert!((xs.iter().cloned().fold(f64::MIN, f64::max) - 375.0).abs() < 1e-3);
        // y is flipped: the first (top-left) point is the highest on the page.
        let y_first = placed[0][0].1;
        let y_last = placed[0][1].1;
        assert!(y_first > y_last);
    }

    #[test]
    fn rejects_dots_empty_and_non_finite_input() {
        assert!(HandwrittenSignature::from_canvas(&[]).is_none());
        assert!(HandwrittenSignature::from_canvas(&[vec![[1.0, 1.0], [2.0, 2.0]]]).is_none());
        assert!(
            HandwrittenSignature::from_canvas(&[vec![[f32::NAN, 0.0], [50.0, 50.0]]]).is_none()
        );
    }

    #[test]
    fn toml_roundtrip_and_hostile_files_are_sanitised() {
        let s = HandwrittenSignature::from_canvas(&[vec![[0.0, 0.0], [100.0, 30.0], [50.0, 10.0]]])
            .unwrap();
        let t = s.to_toml().unwrap();
        assert_eq!(HandwrittenSignature::from_toml(&t).unwrap(), s);
        assert!(HandwrittenSignature::from_toml("aspect = 1e9\nstrokes = [[[0.1,0.1]]]").is_none());
        assert!(HandwrittenSignature::from_toml("garbage {").is_none());
        let huge = format!(
            "aspect = 0.5\nstrokes = [[{}]]",
            vec!["[0.5, 0.2]"; 10_000].join(",")
        );
        let v = HandwrittenSignature::from_toml(&huge).unwrap();
        assert_eq!(v.strokes[0].len(), MAX_POINTS_PER_STROKE);
    }
}
