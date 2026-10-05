//! Snap targets from real page geometry, including Form XObjects.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::geom::Point;
use pdf_engine::snap::{SnapIndex, SnapKind};
use test_support::*;

fn index_of(bytes: Vec<u8>) -> SnapIndex {
    let doc = PdfDocument::open(bytes, &OpenOptions::default()).unwrap();
    let page = doc.page_ids().unwrap()[0];
    SnapIndex::build(doc.lopdf(), page.0)
}

#[test]
fn corners_of_a_reused_form_xobject_are_found_at_both_placements() {
    let ix = index_of(fixtures::reused_form_and_transparency());
    // Form is a 50x50 square painted at (20,20) and (120,20); the page also has a rectangle
    // (40,40)-(160,120).
    for (query, expect) in [
        ((21.0, 19.0), (20.0, 20.0)),
        ((69.0, 71.0), (70.0, 70.0)),
        ((121.0, 21.0), (120.0, 20.0)),
        ((169.0, 69.0), (170.0, 70.0)),
        ((159.0, 121.0), (160.0, 120.0)),
    ] {
        let h = ix.query(Point::new(query.0, query.1), 4.0).unwrap();
        assert_eq!(h.kind, SnapKind::Endpoint, "{query:?}");
        assert!((h.point.x - expect.0).abs() < 1e-6 && (h.point.y - expect.1).abs() < 1e-6);
    }
}

#[test]
fn edges_of_overlapping_shapes_intersect() {
    let ix = index_of(fixtures::reused_form_and_transparency());
    // First form's right edge x=70 (y 20..70) meets the page rectangle's bottom edge y=40.
    let h = ix.query(Point::new(71.0, 41.0), 4.0).unwrap();
    assert_eq!(h.kind, SnapKind::Intersection);
    assert!((h.point.x - 70.0).abs() < 1e-6 && (h.point.y - 40.0).abs() < 1e-6);
}

#[test]
fn a_large_vector_drawing_indexes_quickly_and_answers_queries() {
    let ix = index_of(fixtures::vector_a0(20_000));
    assert!(ix.segment_count() >= 20_000, "{}", ix.segment_count());
    let t = std::time::Instant::now();
    let mut hits = 0;
    for k in 0..2000 {
        let p = Point::new(
            10.0 + (k % 100) as f64 * 7.3,
            10.0 + (k / 100) as f64 * 41.1,
        );
        if ix.query(p, 6.0).is_some() {
            hits += 1;
        }
    }
    assert!(
        t.elapsed().as_secs_f64() < 2.0,
        "queries too slow: {:?}",
        t.elapsed()
    );
    eprintln!("2000 queries, {hits} hits, {:?}", t.elapsed());
}

#[test]
fn text_only_and_image_only_pages_have_nothing_to_snap_to() {
    assert!(index_of(fixtures::helvetica_lines()).is_empty());
    assert!(index_of(fixtures::image_page()).is_empty());
}

#[test]
fn floor_plan_snaps_to_corners_junctions_and_crossings() {
    let ix = index_of(fixtures::floor_plan());
    let at = |x: f64, y: f64| ix.query(Point::new(x, y), 5.0).unwrap();
    // Outer corner.
    let h = at(102.0, 98.0);
    assert_eq!(
        (h.kind, h.point.x, h.point.y),
        (SnapKind::Endpoint, 100.0, 100.0)
    );
    // T-junction end of the horizontal inner wall.
    let h = at(399.0, 301.0);
    assert_eq!(
        (h.kind, h.point.x, h.point.y),
        (SnapKind::Endpoint, 400.0, 300.0)
    );
    // The diagonal crosses the vertical inner wall at (400, 250).
    let h = at(401.0, 251.0);
    assert_eq!(h.kind, SnapKind::Intersection);
    assert!((h.point.x - 400.0).abs() < 1e-9 && (h.point.y - 250.0).abs() < 1e-9);
    // Middle of the bottom outer wall.
    let h = at(399.0, 98.0);
    assert!(matches!(
        h.kind,
        SnapKind::Midpoint | SnapKind::Intersection | SnapKind::Endpoint
    ));
    // The door arc's end is an end point, its middle is only "on line".
    let h = at(560.0, 101.0);
    assert_eq!(
        (h.kind, h.point.x, h.point.y),
        (SnapKind::Endpoint, 560.0, 100.0)
    );
    let h = at(541.0, 141.0);
    assert_eq!(h.kind, SnapKind::Edge);
}
