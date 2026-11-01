//! A single-stroke font for engraving labels on laser sheets, designed for Picabia from scratch on
//! a small grid (original work, no font data from anywhere else).
//!
//! Each glyph is a few open polylines on a grid 6 units tall (y down: 0 = cap top, 6 = baseline),
//! most 4 units wide. Round letters are octagons, which reads well at 2 mm and engraves in a few
//! straight moves. Lowercase letters draw as uppercase; characters the font lacks draw as a small
//! box (check with [`font_has`]).

use vectorcraft_geom::kurbo::{Point, Vec2};

use crate::laser::Polyline;

/// Grid units from the cap top to the baseline.
const CAP_UNITS: f64 = 6.0;
/// Space after every glyph, grid units.
const GAP: f64 = 1.5;
/// The most characters [`text_polylines`] draws (the rest are left out).
pub const MAX_TEXT_CHARS: usize = 512;

/// The glyph table: character, width in grid units, strokes (`;` between strokes, points `x,y`
/// separated by spaces).
const GLYPHS: &[(char, f64, &str)] = &[
    ('A', 4.0, "0,6 2,0 4,6;0.7,4 3.3,4"),
    ('B', 4.0, "0,3 3,3 4,3.8 4,5.2 3,6 0,6 0,0 3,0 4,0.8 4,2.2 3,3"),
    ('C', 4.0, "4,1 3,0 1,0 0,1 0,5 1,6 3,6 4,5"),
    ('D', 4.0, "0,0 0,6 2.5,6 4,4.5 4,1.5 2.5,0 0,0"),
    ('E', 4.0, "4,0 0,0 0,6 4,6;0,3 3,3"),
    ('F', 4.0, "4,0 0,0 0,6;0,3 3,3"),
    ('G', 4.0, "4,1 3,0 1,0 0,1 0,5 1,6 3,6 4,5 4,3.5 2.2,3.5"),
    ('H', 4.0, "0,0 0,6;4,0 4,6;0,3 4,3"),
    ('I', 2.0, "0,0 2,0;1,0 1,6;0,6 2,6"),
    ('J', 4.0, "4,0 4,5 3,6 1,6 0,5"),
    ('K', 4.0, "0,0 0,6;4,0 0,3.6;1.3,2.7 4,6"),
    ('L', 4.0, "0,0 0,6 4,6"),
    ('M', 5.0, "0,6 0,0 2.5,3.6 5,0 5,6"),
    ('N', 4.0, "0,6 0,0 4,6 4,0"),
    ('O', 4.0, "1,0 3,0 4,1 4,5 3,6 1,6 0,5 0,1 1,0"),
    ('P', 4.0, "0,6 0,0 3,0 4,1 4,2 3,3 0,3"),
    ('Q', 4.0, "1,0 3,0 4,1 4,5 3,6 1,6 0,5 0,1 1,0;2.4,4.4 4,6.2"),
    ('R', 4.0, "0,6 0,0 3,0 4,1 4,2 3,3 0,3;2,3 4,6"),
    ('S', 4.0, "4,1 3,0 1,0 0,1 0,2 1,3 3,3 4,4 4,5 3,6 1,6 0,5"),
    ('T', 4.0, "0,0 4,0;2,0 2,6"),
    ('U', 4.0, "0,0 0,5 1,6 3,6 4,5 4,0"),
    ('V', 4.0, "0,0 2,6 4,0"),
    ('W', 5.0, "0,0 1.25,6 2.5,2 3.75,6 5,0"),
    ('X', 4.0, "0,0 4,6;4,0 0,6"),
    ('Y', 4.0, "0,0 2,3 4,0;2,3 2,6"),
    ('Z', 4.0, "0,0 4,0 0,6 4,6"),
    // Zero is narrower than O and slashed.
    ('0', 3.6, "0.9,0 2.7,0 3.6,1 3.6,5 2.7,6 0.9,6 0,5 0,1 0.9,0;3.2,0.8 0.4,5.2"),
    ('1', 3.0, "0.2,1.4 1.6,0 1.6,6;0.2,6 3,6"),
    ('2', 4.0, "0,1 1,0 3,0 4,1 4,2.2 0,6 4,6"),
    ('3', 4.0, "0,1 1,0 3,0 4,1 4,2 3,3 1.6,3;3,3 4,4 4,5 3,6 1,6 0,5"),
    ('4', 4.0, "3,6 3,0 0,4.2 4,4.2"),
    ('5', 4.0, "4,0 0.4,0 0,2.8 1,2.4 3,2.4 4,3.4 4,5 3,6 1,6 0,5"),
    ('6', 4.0, "3.8,0.6 3,0 1,0 0,1 0,5 1,6 3,6 4,5 4,3.8 3,2.8 1,2.8 0,3.8"),
    ('7', 4.0, "0,0 4,0 1.4,6"),
    ('8', 4.0, "1,3 0,2 0,1 1,0 3,0 4,1 4,2 3,3 1,3 0,4 0,5 1,6 3,6 4,5 4,4 3,3"),
    ('9', 4.0, "4,2.2 3,3.2 1,3.2 0,2.2 0,1 1,0 3,0 4,1 4,5 3,6 1,6 0.2,5.2"),
    ('.', 1.0, "0.2,5.4 0.8,5.4 0.8,6 0.2,6 0.2,5.4"),
    (',', 1.0, "0.8,5.2 0.8,6 0.2,7"),
    ('-', 3.0, "0.3,3.5 2.7,3.5"),
    ('_', 4.0, "0,6 4,6"),
    ('/', 3.0, "0,6 3,0"),
    (':', 1.0, "0.2,1.6 0.8,1.6 0.8,2.2 0.2,2.2 0.2,1.6;0.2,5.4 0.8,5.4 0.8,6 0.2,6 0.2,5.4"),
    ('#', 4.0, "1.4,0.6 0.9,5.4;3.1,0.6 2.6,5.4;0,2 4,2;0,4 4,4"),
    ('(', 2.0, "2,0 0.8,1.2 0.4,3 0.8,4.8 2,6"),
    (')', 2.0, "0,0 1.2,1.2 1.6,3 1.2,4.8 0,6"),
    ('×', 3.4, "0.2,2 3.2,5;3.2,2 0.2,5"),
    ('+', 4.0, "0.4,3.4 3.6,3.4;2,1.8 2,5"),
    ('=', 4.0, "0.4,2.6 3.6,2.6;0.4,4.2 3.6,4.2"),
    ('%', 4.0, "4,0 0,6;0,0 1.2,0 1.2,1.4 0,1.4 0,0;2.8,4.6 4,4.6 4,6 2.8,6 2.8,4.6"),
    ('\'', 1.0, "0.5,0 0.5,1.8"),
    ('"', 2.0, "0.3,0 0.3,1.8;1.7,0 1.7,1.8"),
    ('°', 2.0, "0.6,0 1.4,0 2,0.6 2,1.4 1.4,2 0.6,2 0,1.4 0,0.6 0.6,0"),
    ('↑', 4.0, "2,6 2,0;0.4,1.6 2,0 3.6,1.6"),
];

/// What a character the font lacks draws: a small box.
const MISSING: (f64, &str) = (3.0, "0,1.5 3,1.5 3,6 0,6 0,1.5");
/// Width of a space, grid units (plus the gap).
const SPACE: f64 = 2.0;

/// The character the table is looked up with: lowercase ASCII draws uppercase.
fn key(c: char) -> char {
    c.to_ascii_uppercase()
}

fn glyph(c: char) -> Option<(f64, &'static str)> {
    let k = key(c);
    GLYPHS.iter().find(|g| g.0 == k).map(|g| (g.1, g.2))
}

/// Whether the font draws `c` (space included); anything else draws as a small box.
pub fn font_has(c: char) -> bool {
    c == ' ' || glyph(c).is_some()
}

/// A glyph's strokes in grid units.
fn strokes(src: &str) -> Vec<Vec<(f64, f64)>> {
    src.split(';')
        .map(|s| {
            s.split_whitespace()
                .filter_map(|xy| {
                    let (x, y) = xy.split_once(',')?;
                    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
                })
                .collect::<Vec<(f64, f64)>>()
        })
        .filter(|s| s.len() >= 2)
        .collect()
}

/// `text` drawn as single-stroke polylines (mm), the baseline's left end at `origin_mm`, capitals
/// `cap_h_mm` tall; and the text's width (mm, from the first glyph's left to the last one's right).
/// Lowercase draws uppercase, unknown characters draw a small box, at most [`MAX_TEXT_CHARS`]
/// characters. Non-finite or non-positive input gives nothing.
pub fn text_polylines(text: &str, origin_mm: Point, cap_h_mm: f64) -> (Vec<Polyline>, f64) {
    if !(cap_h_mm.is_finite() && cap_h_mm > 0.0 && origin_mm.x.is_finite() && origin_mm.y.is_finite()) {
        return (Vec::new(), 0.0);
    }
    let s = cap_h_mm / CAP_UNITS;
    let mut out = Vec::new();
    let mut x = 0.0;
    let mut width = 0.0;
    for c in text.chars().take(MAX_TEXT_CHARS) {
        if c == ' ' {
            x += SPACE + GAP;
            continue;
        }
        let (w, src) = glyph(c).unwrap_or(MISSING);
        for stroke in strokes(src) {
            out.push(stroke.into_iter().map(|(gx, gy)| origin_mm + Vec2::new((x + gx) * s, (gy - CAP_UNITS) * s)).collect());
        }
        width = (x + w) * s;
        x += w + GAP;
    }
    (out, width)
}

/// How far below the baseline the font reaches (`,` and `Q`), as a fraction of the cap height.
pub(crate) const DESCENT: f64 = 1.0 / CAP_UNITS;

#[cfg(test)]
pub(crate) fn glyph_table() -> impl Iterator<Item = (char, f64)> {
    GLYPHS.iter().map(|g| (g.0, g.1))
}
