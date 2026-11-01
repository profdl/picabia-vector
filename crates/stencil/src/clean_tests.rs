use crate::ops::BoolOp;
use vectorcraft_geom::kurbo::{BezPath, Shape};
use vectorcraft_modifiers::Stencil;

use crate::clean::open_close;
use crate::fixture::{self, BOUNDS};
use crate::{compute, mm_per_unit};

fn xor_area(a: &BezPath, b: &BezPath) -> f64 {
    crate::ops::boolean_step(a, b, BoolOp::Exclude).unwrap().area().abs()
}

/// Three offsets (grows composed) give what an opening then a closing give.
#[test]
fn composed_open_close_matches_open_then_close() {
    let (paints, _) = fixture::design(90, 21);
    let raw = Stencil { min_cut_area_mm2: 0.0, min_cut_width_mm: 0.0, min_bridge_mm: 0.0, ..Stencil::default() };
    let cut = compute(&paints, &raw, BOUNDS).unwrap().cut;
    let mpu = mm_per_unit(raw.width_mm, BOUNDS).unwrap();
    let (r1, r2) = (0.15 / mpu, 0.25 / mpu);
    let reference = crate::offset::close(&crate::offset::open(&cut, r1).unwrap(), r2).unwrap();
    for (tolerance, slack) in [(0.0, 1e-3), (0.025 / mpu, 0.05)] {
        let ours = open_close(&cut, r1, r2, tolerance).unwrap();
        let diff = xor_area(&ours, &reference);
        // The differing area stays within a thin band (`slack` units wide) along the outline.
        let band = cut.perimeter(1e-3) * slack;
        assert!(diff < band, "tolerance {tolerance}: {diff} ≥ {band}");
    }
    // One side only, or none.
    let opened = open_close(&cut, r1, 0.0, 0.0).unwrap();
    assert!(xor_area(&opened, &crate::offset::open(&cut, r1).unwrap()) < cut.perimeter(1e-3) * 1e-3);
    let closed = open_close(&cut, 0.0, r2, 0.0).unwrap();
    assert!(xor_area(&closed, &crate::offset::close(&cut, r2).unwrap()) < cut.perimeter(1e-3) * 1e-3);
    assert_eq!(open_close(&cut, 0.0, 0.0, 0.0).unwrap(), cut);
}
