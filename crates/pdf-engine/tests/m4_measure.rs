//! Milestone 4: measurement annotations, scale registry, rescale and CSV export.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::annot;
use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::geom::Point;
use pdf_engine::measure::{self, MeasureKind, Scale, ScaleRegion, ScaleSet, ScaleSource, Unit};
use test_support::fixtures::*;
use test_support::*;

fn open(bytes: Vec<u8>) -> PdfDocument {
    PdfDocument::open(bytes, &OpenOptions::default()).unwrap()
}

fn reopen(doc: &mut PdfDocument) -> PdfDocument {
    open(doc.snapshot_bytes().unwrap())
}

fn p(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

#[test]
fn measurement_annotations_roundtrip_with_standard_measure_dictionary() {
    let mut doc = open(helvetica_lines());
    let page = doc.page_ids().unwrap()[0];
    let scale = Scale::from_ratio(1.0, Unit::In, 10.0, Unit::Ft).unwrap();
    let dist = measure::spec_for(
        MeasureKind::Distance,
        &[p(100.0, 500.0), p(172.0, 500.0)],
        &scale,
        "",
    )
    .unwrap();
    let area = measure::spec_for(
        MeasureKind::Area,
        &[
            p(100.0, 300.0),
            p(244.0, 300.0),
            p(244.0, 372.0),
            p(100.0, 372.0),
        ],
        &scale,
        "",
    )
    .unwrap();
    let count = measure::spec_for(MeasureKind::Count, &[p(300.0, 300.0)], &scale, "Doors").unwrap();
    doc.transact(|tx| {
        annot::add_annotation(tx, page, &dist)?;
        annot::add_annotation(tx, page, &area)?;
        annot::add_annotation(tx, page, &count)?;
        Ok(())
    })
    .unwrap();
    let mut doc = reopen(&mut doc);
    let rows = measure::collect_measurements(&doc);
    assert_eq!(rows.len(), 3);
    let d = rows
        .iter()
        .find(|r| r.kind == MeasureKind::Distance)
        .unwrap();
    assert!((d.value - 10.0).abs() < 1e-6, "{d:?}");
    assert_eq!(d.unit, "ft");
    let a = rows.iter().find(|r| r.kind == MeasureKind::Area).unwrap();
    assert!(
        (a.value - 200.0).abs() < 1e-6,
        "2in x 1in at 10ft/in = 20x10 ft: {a:?}"
    );
    assert_eq!(a.unit, "ft²");
    // The standard /Measure dictionary is present for other viewers.
    let id = annot::annotation_ids(&doc, page)[0];
    let ad = doc.lopdf().get_dictionary(id).unwrap();
    let m = ad.get(b"Measure").unwrap().as_dict().unwrap();
    assert_eq!(m.get(b"Subtype").unwrap().as_name().unwrap(), b"RL");
    assert_eq!(ad.get(b"IT").unwrap().as_name().unwrap(), b"LineDimension");
    // The label is part of the annotation's Rect so it is not clipped.
    let info = annot::read_annotation(doc.lopdf(), id).unwrap();
    assert!(
        info.rect.width() > 72.0,
        "rect must include the label: {:?}",
        info.rect
    );
    // Poppler (independent renderer) must render the file, and the label box must show ink.
    let bytes = doc.snapshot_bytes().unwrap();
    if let Some((w, _h, px)) = poppler_render(&bytes, 1, 100) {
        let sc = 100.0 / 72.0;
        let (cx, cy) = (136.0 * sc, (842.0 - 500.0) * sc);
        let mut dark = 0;
        for y in (cy as u32 - 10)..(cy as u32 + 10) {
            for x in (cx as u32 - 25)..(cx as u32 + 25) {
                if px[((y * w + x) * 3) as usize] < 80 {
                    dark += 1;
                }
            }
        }
        assert!(dark > 20, "label ink missing ({dark})");
    } else {
        assert!(
            !oracles_required(),
            "poppler is required for this check (BERG_REQUIRE_ORACLES=1)"
        );
    }
}

#[test]
fn counts_summarise_by_category_and_page() {
    let mut doc = open(form_rich());
    let pages = doc.page_ids().unwrap();
    let s = Scale::uncalibrated();
    doc.transact(|tx| {
        for (pg, cat, x) in [
            (0, "Doors", 10.0),
            (0, "Doors", 20.0),
            (1, "Doors", 30.0),
            (0, "Windows", 40.0),
        ] {
            let spec = measure::spec_for(MeasureKind::Count, &[p(x, 100.0)], &s, cat)?;
            annot::add_annotation(tx, pages[pg], &spec)?;
        }
        Ok(())
    })
    .unwrap();
    let rows = measure::collect_measurements(&doc);
    let sum = measure::count_summary(&rows);
    let doors = sum.iter().find(|c| c.category == "Doors").unwrap();
    assert_eq!(doors.total, 3);
    assert_eq!(doors.per_page, vec![(1, 2), (2, 1)]);
    assert_eq!(
        sum.iter().find(|c| c.category == "Windows").unwrap().total,
        1
    );
    let csv = measure::to_csv(&rows);
    assert!(csv.contains("Count summary"));
    assert!(csv.contains("Doors,3,p1:2; p2:1"), "{csv}");
}

#[test]
fn scale_registry_roundtrips_and_resolves_precedence() {
    let mut doc = open(form_rich());
    let pages = doc.page_ids().unwrap();
    let doc_s = Scale::from_one_to(100.0, Unit::M).unwrap();
    let page_s = Scale::from_ratio(1.0, Unit::In, 8.0, Unit::Ft).unwrap();
    let reg_s = Scale::from_calibration(100.0, 2.0, Unit::M).unwrap();
    let mut set = ScaleSet {
        document: Some(doc_s.clone()),
        ..Default::default()
    };
    set.pages.insert(pages[1], page_s.clone());
    set.regions.push(ScaleRegion {
        page: pages[0],
        rect: pdf_engine::geom::Rect::new(0.0, 0.0, 200.0, 100.0),
        scale: reg_s.clone(),
    });
    doc.transact(|tx| measure::write_scales(tx, &set)).unwrap();
    let doc = reopen(&mut doc);
    let got = measure::read_scales(&doc);
    assert_eq!(got, set);
    assert_eq!(
        got.resolve(pages[0], Some(p(10.0, 10.0))).1,
        ScaleSource::Region
    );
    assert_eq!(
        got.resolve(pages[0], Some(p(300.0, 10.0))).1,
        ScaleSource::Document
    );
    assert_eq!(got.resolve(pages[1], None).1, ScaleSource::Page);
}

#[test]
fn rescale_page_updates_existing_measurements_and_keeps_counts() {
    let mut doc = open(helvetica_lines());
    let page = doc.page_ids().unwrap()[0];
    let old = Scale::from_calibration(100.0, 10.0, Unit::M).unwrap();
    let new = Scale::from_calibration(100.0, 20.0, Unit::M).unwrap();
    doc.transact(|tx| {
        let d = measure::spec_for(
            MeasureKind::Distance,
            &[p(0.0, 100.0), p(100.0, 100.0)],
            &old,
            "",
        )?;
        annot::add_annotation(tx, page, &d)?;
        let c = measure::spec_for(MeasureKind::Count, &[p(5.0, 5.0)], &old, "X")?;
        annot::add_annotation(tx, page, &c)?;
        Ok(())
    })
    .unwrap();
    let (n, _) = doc
        .transact(|tx| measure::rescale_page(tx, page, &new))
        .unwrap();
    assert_eq!(n, 1, "only the distance is scale dependent");
    let rows = measure::collect_measurements(&doc);
    let d = rows
        .iter()
        .find(|r| r.kind == MeasureKind::Distance)
        .unwrap();
    assert!((d.value - 20.0).abs() < 1e-9);
    let info = annot::read_annotations(&doc, page);
    let spec = info
        .iter()
        .find_map(|a| {
            a.spec.as_ref().filter(|s| {
                s.measure
                    .as_ref()
                    .is_some_and(|m| m.kind == MeasureKind::Distance)
            })
        })
        .unwrap();
    assert_eq!(spec.contents, "20.00 m", "label text follows the scale");
}

#[test]
fn moving_a_measurement_keeps_its_value() {
    let mut doc = open(helvetica_lines());
    let page = doc.page_ids().unwrap()[0];
    let s = Scale::from_calibration(100.0, 10.0, Unit::M).unwrap();
    let spec = measure::spec_for(
        MeasureKind::Perimeter,
        &[p(0.0, 100.0), p(30.0, 140.0), p(30.0, 240.0)],
        &s,
        "",
    )
    .unwrap();
    let (id, _) = doc
        .transact(|tx| annot::add_annotation(tx, page, &spec))
        .unwrap();
    let before = measure::collect_measurements(&doc)[0].value;
    doc.transact(|tx| annot::move_annotation(tx, id, 57.0, -33.0))
        .unwrap();
    let after = measure::collect_measurements(&doc)[0].value;
    assert!((before - after).abs() < 1e-9, "{before} vs {after}");
}

#[test]
fn csv_export_is_safe_for_spreadsheets() {
    let mut doc = open(helvetica_lines());
    let page = doc.page_ids().unwrap()[0];
    let s = Scale::uncalibrated();
    let mut spec =
        measure::spec_for(MeasureKind::Distance, &[p(0.0, 0.0), p(10.0, 0.0)], &s, "").unwrap();
    spec.contents = "=HYPERLINK(\"http://evil\",\"x\")".into();
    spec.author = "Eve, \"the\" tester".into();
    doc.transact(|tx| annot::add_annotation(tx, page, &spec))
        .unwrap();
    let csv = measure::to_csv(&measure::collect_measurements(&doc));
    assert!(csv.contains("\"'=HYPERLINK("), "{csv}");
    assert!(csv.contains("\"Eve, \"\"the\"\" tester\""), "{csv}");
    assert!(csv.contains(",no,"), "uncalibrated flag must be exported");
}
