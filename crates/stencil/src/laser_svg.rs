//! A [`LaserSheet`] as an SVG for laser software: the page in real millimetres with a matching
//! viewBox (1 user unit = 1 mm), one layer per operation in the order the laser should run them
//! (engrave, score, then cut, so parts don't shift before they're engraved), hairline strokes in
//! the operation's colour, no fills.

use std::fmt::Write as _;

use vectorcraft_geom::kurbo::Point;

use crate::laser::{LaserParams, LaserSheet};

/// A number with at most 3 decimals, never an exponent, trailing zeros trimmed (`-0` is `0`).
pub(crate) fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let s = format!("{v:.3}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { &s };
    if s == "-0" { "0".into() } else { s.to_string() }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

fn path_d(points: &[Point], close: bool) -> String {
    let mut d = String::new();
    for (i, p) in points.iter().enumerate() {
        let _ = write!(d, "{}{} {}", if i == 0 { "M" } else { "L" }, num(p.x), num(p.y));
    }
    if close {
        d.push('Z');
    }
    d
}

fn group(out: &mut String, label: &str, colour: &str, items: &[Vec<Point>], close: bool) {
    let items: Vec<&Vec<Point>> = items.iter().filter(|c| c.len() >= 2).collect();
    if items.is_empty() {
        return;
    }
    let _ = writeln!(
        out,
        "<g id=\"{label}\" inkscape:groupmode=\"layer\" inkscape:label=\"{label}\" fill=\"none\" stroke=\"{}\" stroke-width=\"0.01\">",
        escape(colour)
    );
    for c in items {
        let _ = writeln!(out, "<path d=\"{}\"/>", path_d(c, close));
    }
    out.push_str("</g>\n");
}

/// `sheet` as an SVG document; colours from `params` (clamped).
pub fn laser_svg(sheet: &LaserSheet, params: &LaserParams) -> String {
    let p = params.clamped();
    let (w, h) = (num(sheet.w_mm), num(sheet.h_mm));
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:inkscape=\"http://www.inkscape.org/namespaces/inkscape\" version=\"1.1\" width=\"{w}mm\" height=\"{h}mm\" viewBox=\"0 0 {w} {h}\">"
    );
    let _ = writeln!(out, "<title>Picabia laser: {}</title>", escape(&sheet.name));
    group(&mut out, "Engrave", &p.engrave, &sheet.engrave, false);
    group(&mut out, "Score", &p.score, &sheet.score, false);
    group(&mut out, "Cut", &p.cut, &sheet.cut, true);
    out.push_str("</svg>\n");
    out
}
