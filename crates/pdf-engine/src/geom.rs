//! Geometry primitives and the coordinate-space contract.
//!
//! Spaces used throughout the application:
//!
//! * **PDF user space** – y-up, units of 1/72 inch × `UserUnit`, origin wherever the
//!   page's boxes put it (may be non-zero or negative).
//! * **Page view space** – y-down, origin at the top-left of the *visible* (crop ∩ media)
//!   box after page `/Rotate` and any temporary view rotation, measured in points
//!   (`UserUnit` already applied).
//! * **Logical UI pixels** – page view space × zoom (+ canvas offset); UI-owned.
//! * **Physical pixels** – logical × device pixel ratio; renderer-owned.
//!
//! This module owns the PDF-user-space ⇄ page-view-space part. Zoom and DPI are plain
//! scalings applied by the caller.

pub use hayro::kurbo::{Affine, Point, Rect, Size, Vec2};

/// A quadrilateral with four corners in a consistent winding order
/// (`[bottom-left, bottom-right, top-right, top-left]` for upright text in PDF space).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad(pub [Point; 4]);

impl Quad {
    /// Build a quad from an axis-aligned rectangle (PDF space, y-up).
    pub fn from_rect(r: Rect) -> Self {
        let r = r.abs();
        Quad([
            Point::new(r.x0, r.y0),
            Point::new(r.x1, r.y0),
            Point::new(r.x1, r.y1),
            Point::new(r.x0, r.y1),
        ])
    }

    /// Transform every corner.
    pub fn transform(&self, t: Affine) -> Quad {
        Quad([t * self.0[0], t * self.0[1], t * self.0[2], t * self.0[3]])
    }

    /// Axis-aligned bounding box.
    pub fn bounds(&self) -> Rect {
        let mut r = Rect::new(self.0[0].x, self.0[0].y, self.0[0].x, self.0[0].y);
        for p in &self.0[1..] {
            r = r.union_pt(*p);
        }
        r
    }

    /// Point-in-quad test (convex quads, either winding).
    pub fn contains(&self, p: Point) -> bool {
        let mut pos = false;
        let mut neg = false;
        for i in 0..4 {
            let a = self.0[i];
            let b = self.0[(i + 1) % 4];
            let cross = (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
            if cross > 1e-9 {
                pos = true;
            } else if cross < -1e-9 {
                neg = true;
            }
        }
        !(pos && neg)
    }
}

/// Page rotation in clockwise quarter turns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Rotation {
    /// 0°
    #[default]
    R0,
    /// 90° clockwise
    R90,
    /// 180°
    R180,
    /// 270° clockwise
    R270,
}

impl Rotation {
    /// Normalise arbitrary degrees. Values that are not multiples of 90 are rejected
    /// (the spec requires multiples of 90; viewers ignore bad values).
    pub fn from_degrees(d: i64) -> Option<Self> {
        match d.rem_euclid(360) {
            0 => Some(Rotation::R0),
            90 => Some(Rotation::R90),
            180 => Some(Rotation::R180),
            270 => Some(Rotation::R270),
            _ => None,
        }
    }

    /// Degrees clockwise (0, 90, 180, 270).
    pub fn degrees(self) -> i64 {
        match self {
            Rotation::R0 => 0,
            Rotation::R90 => 90,
            Rotation::R180 => 180,
            Rotation::R270 => 270,
        }
    }

    /// Compose two rotations.
    pub fn plus(self, other: Rotation) -> Rotation {
        Rotation::from_degrees(self.degrees() + other.degrees()).unwrap_or(Rotation::R0)
    }

    /// Rotate by a number of clockwise quarter turns (may be negative).
    pub fn rotated_by(self, quarter_turns_cw: i64) -> Rotation {
        Rotation::from_degrees(self.degrees() + 90 * quarter_turns_cw).unwrap_or(Rotation::R0)
    }

    /// Whether width and height are swapped.
    pub fn swaps_axes(self) -> bool {
        matches!(self, Rotation::R90 | Rotation::R270)
    }
}

/// Normalise a rectangle so `x0 <= x1` and `y0 <= y1`.
pub fn normalize_rect(r: Rect) -> Rect {
    r.abs()
}

/// Visible-area description of one page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageGeometry {
    /// `MediaBox`, normalised, in user space.
    pub media_box: Rect,
    /// `CropBox ∩ MediaBox` (the visible area), in user space.
    pub crop_box: Rect,
    /// Inherited `/Rotate`.
    pub rotate: Rotation,
    /// `/UserUnit` (default 1.0).
    pub user_unit: f64,
}

impl PageGeometry {
    /// Letter-size default used when a page has no usable `MediaBox`.
    pub const DEFAULT_MEDIA: Rect = Rect {
        x0: 0.0,
        y0: 0.0,
        x1: 612.0,
        y1: 792.0,
    };

    /// Build from raw boxes, applying fallbacks and intersection rules.
    pub fn new(media: Option<Rect>, crop: Option<Rect>, rotate: Rotation, user_unit: f64) -> Self {
        let media = media
            .map(normalize_rect)
            .filter(|r| r.width() > 0.0 && r.height() > 0.0 && r.is_finite())
            .unwrap_or(Self::DEFAULT_MEDIA);
        let crop = crop
            .map(normalize_rect)
            .map(|c| c.intersect(media))
            .filter(|r| r.width() > 0.0 && r.height() > 0.0)
            .unwrap_or(media);
        let user_unit = if user_unit.is_finite() && user_unit > 0.0 {
            user_unit
        } else {
            1.0
        };
        Self {
            media_box: media,
            crop_box: crop,
            rotate,
            user_unit,
        }
    }

    fn unrotated_size(&self) -> (f64, f64) {
        (
            self.crop_box.width() * self.user_unit,
            self.crop_box.height() * self.user_unit,
        )
    }

    /// Total clockwise rotation: page `/Rotate` plus a temporary view rotation.
    pub fn total_rotation(&self, view_rotation: Rotation) -> Rotation {
        self.rotate.plus(view_rotation)
    }

    /// Size in points of the visible page as displayed (rotation applied).
    pub fn view_size(&self, view_rotation: Rotation) -> Size {
        let (w, h) = self.unrotated_size();
        if self.total_rotation(view_rotation).swaps_axes() {
            Size::new(h, w)
        } else {
            Size::new(w, h)
        }
    }

    /// Physical size in inches (`UserUnit` aware, rotation-independent orientation of the
    /// *unrotated* page).
    pub fn physical_size_inches(&self) -> (f64, f64) {
        let (w, h) = self.unrotated_size();
        (w / 72.0, h / 72.0)
    }

    /// Transform from PDF user space to page view space (points, y-down).
    pub fn pdf_to_view(&self, view_rotation: Rotation) -> Affine {
        let u = self.user_unit;
        let (w, h) = self.unrotated_size();
        let base = Affine::new([u, 0.0, 0.0, -u, -self.crop_box.x0 * u, self.crop_box.y1 * u]);
        let rot = match self.total_rotation(view_rotation) {
            Rotation::R0 => Affine::IDENTITY,
            Rotation::R90 => Affine::new([0.0, 1.0, -1.0, 0.0, h, 0.0]),
            Rotation::R180 => Affine::new([-1.0, 0.0, 0.0, -1.0, w, h]),
            Rotation::R270 => Affine::new([0.0, -1.0, 1.0, 0.0, 0.0, w]),
        };
        rot * base
    }

    /// Inverse of [`Self::pdf_to_view`] — used for hit-testing and placing new objects.
    pub fn view_to_pdf(&self, view_rotation: Rotation) -> Affine {
        self.pdf_to_view(view_rotation).inverse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn geom(rot: Rotation) -> PageGeometry {
        // Deliberately awkward: offset origin, crop inside media, UserUnit 2.
        PageGeometry::new(
            Some(Rect::new(-50.0, -20.0, 550.0, 780.0)),
            Some(Rect::new(10.0, 30.0, 310.0, 530.0)),
            rot,
            2.0,
        )
    }

    fn close(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9
    }

    #[test]
    fn corners_map_correctly_for_all_rotations() {
        // crop 300x500 pts, UserUnit 2 => 600x1000 unrotated view size.
        let tl = Point::new(10.0, 530.0); // user-space top-left of crop box
        let br = Point::new(310.0, 30.0);
        let g = geom(Rotation::R0);
        assert!(close(
            g.pdf_to_view(Rotation::R0) * tl,
            Point::new(0.0, 0.0)
        ));
        assert!(close(
            g.pdf_to_view(Rotation::R0) * br,
            Point::new(600.0, 1000.0)
        ));
        // 90° cw: unrotated top-left lands at top-right of a 1000x600 view.
        let g = geom(Rotation::R90);
        assert_eq!(g.view_size(Rotation::R0), Size::new(1000.0, 600.0));
        assert!(close(
            g.pdf_to_view(Rotation::R0) * tl,
            Point::new(1000.0, 0.0)
        ));
        assert!(close(
            g.pdf_to_view(Rotation::R0) * br,
            Point::new(0.0, 600.0)
        ));
        // 180°
        let g = geom(Rotation::R180);
        assert!(close(
            g.pdf_to_view(Rotation::R0) * tl,
            Point::new(600.0, 1000.0)
        ));
        assert!(close(
            g.pdf_to_view(Rotation::R0) * br,
            Point::new(0.0, 0.0)
        ));
        // 270° cw: unrotated top-left lands at bottom-left.
        let g = geom(Rotation::R270);
        assert!(close(
            g.pdf_to_view(Rotation::R0) * tl,
            Point::new(0.0, 600.0)
        ));
        assert!(close(
            g.pdf_to_view(Rotation::R0) * br,
            Point::new(1000.0, 0.0)
        ));
    }

    #[test]
    fn view_rotation_is_distinct_from_page_rotation() {
        let g = geom(Rotation::R90);
        // page /Rotate 90 + view 90 == 180 total, but g.rotate is untouched.
        assert_eq!(g.total_rotation(Rotation::R90), Rotation::R180);
        assert_eq!(g.rotate, Rotation::R90);
    }

    #[test]
    fn crop_is_clamped_to_media_and_defaults_apply() {
        let g = PageGeometry::new(
            Some(Rect::new(0.0, 0.0, 100.0, 100.0)),
            Some(Rect::new(-10.0, 50.0, 500.0, 500.0)),
            Rotation::R0,
            1.0,
        );
        assert_eq!(g.crop_box, Rect::new(0.0, 50.0, 100.0, 100.0));
        let d = PageGeometry::new(None, None, Rotation::R0, f64::NAN);
        assert_eq!(d.media_box, PageGeometry::DEFAULT_MEDIA);
        assert_eq!(d.user_unit, 1.0);
        // Inverted (non-normalised) boxes are normalised.
        let n = PageGeometry::new(
            Some(Rect::new(100.0, 100.0, 0.0, 0.0)),
            None,
            Rotation::R0,
            1.0,
        );
        assert_eq!(n.media_box, Rect::new(0.0, 0.0, 100.0, 100.0));
    }

    #[test]
    fn rotation_parsing() {
        assert_eq!(Rotation::from_degrees(-90), Some(Rotation::R270));
        assert_eq!(Rotation::from_degrees(450), Some(Rotation::R90));
        assert_eq!(Rotation::from_degrees(45), None);
        assert_eq!(Rotation::R270.rotated_by(1), Rotation::R0);
    }

    #[test]
    fn quad_contains() {
        let q = Quad::from_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(q.contains(Point::new(5.0, 5.0)));
        assert!(!q.contains(Point::new(11.0, 5.0)));
    }

    proptest! {
        #[test]
        fn roundtrip_user_view_user(
            mx0 in -500.0f64..500.0, my0 in -500.0f64..500.0,
            mw in 10.0f64..3000.0, mh in 10.0f64..3000.0,
            cx in 0.0f64..0.5, cy in 0.0f64..0.5,
            rot in 0i64..4, vrot in 0i64..4, uu in 0.5f64..4.0,
            px in -1000.0f64..4000.0, py in -1000.0f64..4000.0,
        ) {
            let media = Rect::new(mx0, my0, mx0 + mw, my0 + mh);
            let crop = Rect::new(mx0 + cx * mw, my0 + cy * mh, mx0 + mw, my0 + mh);
            let r = Rotation::from_degrees(rot * 90).unwrap();
            let v = Rotation::from_degrees(vrot * 90).unwrap();
            let g = PageGeometry::new(Some(media), Some(crop), r, uu);
            let p = Point::new(px, py);
            let back = g.view_to_pdf(v) * (g.pdf_to_view(v) * p);
            prop_assert!((back.x - p.x).abs() < 1e-6 && (back.y - p.y).abs() < 1e-6);
        }

        #[test]
        fn crop_box_always_fills_view_rect(
            mx0 in -500.0f64..500.0, my0 in -500.0f64..500.0,
            mw in 10.0f64..3000.0, mh in 10.0f64..3000.0,
            rot in 0i64..4, vrot in 0i64..4, uu in 0.5f64..4.0,
        ) {
            let media = Rect::new(mx0, my0, mx0 + mw, my0 + mh);
            let r = Rotation::from_degrees(rot * 90).unwrap();
            let v = Rotation::from_degrees(vrot * 90).unwrap();
            let g = PageGeometry::new(Some(media), None, r, uu);
            let t = g.pdf_to_view(v);
            let b = (Quad::from_rect(g.crop_box).transform(t)).bounds();
            let s = g.view_size(v);
            prop_assert!(b.x0.abs() < 1e-6 && b.y0.abs() < 1e-6);
            prop_assert!((b.x1 - s.width).abs() < 1e-6 && (b.y1 - s.height).abs() < 1e-6);
        }
    }
}
