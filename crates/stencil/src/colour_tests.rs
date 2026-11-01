use vectorcraft_geom::kurbo::{BezPath, Rect};
use vectorcraft_modifiers::{Stencil, StencilColourMode};

use crate::{Region, Selector, colour_infos, luminance, mm_per_unit};

fn region(colour: [u8; 3], area: f64) -> Region {
    Region { colour, path: BezPath::new(), area }
}

#[test]
fn luminance_is_rec709_on_decoded_srgb() {
    assert_eq!(luminance([0, 0, 0]), 0.0);
    assert!((luminance([255, 255, 255]) - 1.0).abs() < 1e-12);
    assert!((luminance([255, 0, 0]) - 0.2126).abs() < 1e-12);
    assert!((luminance([0, 255, 0]) - 0.7152).abs() < 1e-12);
    // Mid grey #808080 decodes to ~0.2159, not 0.5.
    assert!((luminance([128, 128, 128]) - 0.21586).abs() < 1e-4);
}

#[test]
fn threshold_cuts_lighter_and_invert_swaps() {
    let p = Stencil { threshold: 0.5, ..Stencil::default() };
    let s = Selector::new(&p);
    assert!(s.cuts([255, 255, 255]) && s.cuts([200, 200, 200]));
    assert!(!s.cuts([0, 0, 0]) && !s.cuts([128, 128, 128]));
    let inv = Selector::new(&Stencil { invert: true, ..p.clone() });
    for c in [[255, 255, 255], [200, 200, 200], [0, 0, 0], [128, 128, 128]] {
        assert_ne!(s.cuts(c), inv.cuts(c), "{c:?}");
    }
    // At the level exactly: not cut (strictly above), cut when inverted.
    let edge = Selector::new(&Stencil { threshold: 1.0, ..Stencil::default() });
    assert!(!edge.cuts([255, 255, 255]));
    assert!(Selector::new(&Stencil { threshold: 0.0, ..Stencil::default() }).cuts([1, 0, 0]));
    assert!(!Selector::new(&Stencil { threshold: 0.0, ..Stencil::default() }).cuts([0, 0, 0]));
    // `separate` evaluates as threshold.
    assert!(Selector::new(&Stencil { colour_mode: StencilColourMode::Separate, ..p }).cuts([255, 255, 255]));
}

#[test]
fn pick_cuts_listed_colours() {
    let p =
        Stencil { colour_mode: StencilColourMode::Pick, cut_colours: vec!["#e7cb5b".into(), "#28262E".into(), "junk".into()], ..Stencil::default() };
    let s = Selector::new(&p);
    assert!(s.cuts([0xe7, 0xcb, 0x5b]) && s.cuts([0x28, 0x26, 0x2e]));
    assert!(!s.cuts([0xcb, 0xab, 0x9e]) && !s.cuts([0xe7, 0xcb, 0x5c]), "exact match only");
    let inv = Selector::new(&Stencil { invert: true, ..p });
    assert!(!inv.cuts([0xe7, 0xcb, 0x5b]) && inv.cuts([0xcb, 0xab, 0x9e]));
    // Nothing picked: nothing cut (everything, inverted).
    let none = Selector::new(&Stencil { colour_mode: StencilColourMode::Pick, ..Stencil::default() });
    assert!(!none.cuts([255, 255, 255]));
}

#[test]
fn physical_scale_follows_the_width() {
    assert_eq!(mm_per_unit(300.0, Rect::new(0.0, 0.0, 600.0, 100.0)), Some(0.5));
    assert_eq!(mm_per_unit(300.0, Rect::new(600.0, 0.0, 0.0, 100.0)), Some(0.5), "unnormalised bounds");
    assert_eq!(mm_per_unit(300.0, Rect::new(0.0, 0.0, 0.0, 100.0)), None);
    assert_eq!(mm_per_unit(300.0, Rect::new(0.0, 0.0, f64::INFINITY, 100.0)), None);
    assert_eq!(mm_per_unit(300.0, Rect::new(0.0, f64::NAN, 10.0, 100.0)), None);
    assert_eq!(mm_per_unit(300.0, Rect::new(0.0, 0.0, 1e-320, 1.0)), None, "overflowing scale");
}

#[test]
fn colour_list_is_by_area_in_square_millimetres() {
    let regions = [region([0, 0, 0], 10.0), region([255, 255, 255], 40.0), region([10, 10, 10], 0.0), region([0x80, 0, 0], 40.0)];
    let infos = colour_infos(&regions, &Selector::new(&Stencil::default()), 0.5);
    let got: Vec<(&str, bool, f64)> = infos.iter().map(|c| (c.hex.as_str(), c.cut, c.area)).collect();
    assert_eq!(got, [("#800000", false, 10.0), ("#ffffff", true, 10.0), ("#000000", false, 2.5), ("#0a0a0a", false, 0.0)]);
}
