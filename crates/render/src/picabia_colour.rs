//! PICABIA: the Color modifier row's recolouring of one copy (`docs/picabia/colour.md` › Which
//! colours change).
//!
//! Each copy goes through `vectorcraft_doc::recolor::recolor_node` once per `color` row: the fill
//! shift on fills (solid colours, gradient stops, freeform points, mesh points, a colorized
//! image's colour, text runs' fills) and the stroke shift on strokes. Images' pixels, effects'
//! colours, pattern tiles, None and Registration are left alone. A colour that changes loses its
//! swatch link; RGB, CMYK, Gray and Lab colours keep their model (a Gray that gains chroma, and a
//! spot colour, become a process colour of the document's mode). A row whose shift is zero leaves
//! the copy as it was.

use std::cell::RefCell;
use std::collections::HashMap;

use vectorcraft_color::cms::Model;
use vectorcraft_color::oklch_shift::{OklchShift, shift_color};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::recolor::{ColorVisitor, Reach, map_links, recolor_node};
use vectorcraft_doc::{AppearanceItem, Document, Node, NodeKind};
use vectorcraft_modifiers::ColourShift;

/// What [`recolour`] notes when an object holds a pattern fill or stroke.
pub const PATTERN_NOTE: &str = "pattern fills keep their colours";

/// The most shifted colours one art keeps (the cache is cleared past it).
const MAX_CACHED: usize = 65_536;

/// Shifted colours by `(colour, shift, spot)`.
type Cache = HashMap<([u32; 5], [u64; 3], bool), Color>;

/// Shifted colours kept while one art is built.
pub struct Recolourer {
    /// The document's colour model (what spot colours and Grays that gain chroma become).
    doc_model: Model,
    /// The document's spot swatches' names.
    spots: Vec<String>,
    cache: RefCell<Cache>,
}

impl Recolourer {
    pub fn new(doc: &Document) -> Recolourer {
        let spots = doc.swatches_iter().filter(|w| w.spot).map(|w| w.name.clone()).collect();
        Recolourer { doc_model: doc.color_mode.model(), spots, cache: RefCell::new(HashMap::new()) }
    }

    /// `c` (linked to swatch `link`) shifted by `s`, in the model it keeps.
    fn shifted(&self, c: Color, link: Option<&str>, s: OklchShift) -> Color {
        let spot = link.is_some_and(|l| self.spots.iter().any(|n| n == l));
        let key = (c.key(), s.key(), spot);
        if let Some(hit) = self.cache.borrow().get(&key) {
            return *hit;
        }
        let rgb = shift_color(c, s);
        let model = match c.model() {
            _ if spot => self.doc_model,
            Model::Gray => {
                let [r, g, b] = rgb.to_rgb();
                if (r - g).abs() > 1e-4 || (g - b).abs() > 1e-4 { self.doc_model } else { Model::Gray }
            }
            m => m,
        };
        let out = rgb.in_model(model);
        let mut cache = self.cache.borrow_mut();
        if cache.len() >= MAX_CACHED {
            cache.clear();
        }
        cache.insert(key, out);
        out
    }

    /// `p`'s colours shifted by `s` (None, patterns and Registration untouched); true when it
    /// changed.
    fn paint(&self, p: &mut Paint, s: OklchShift) -> bool {
        if s.is_zero() || p.is_registration() {
            return false;
        }
        map_links(p, &|c, link, _tint| {
            if link.as_deref() == Some(vectorcraft_color::swatch::REGISTRATION) {
                return false;
            }
            let n = self.shifted(*c, link.as_deref(), s);
            if n == *c {
                return false;
            }
            *c = n;
            *link = None;
            true
        })
    }
}

/// One pass: every paint `reach` reaches shifted by `s`.
struct Pass<'a> {
    r: &'a Recolourer,
    s: OklchShift,
}

impl ColorVisitor for Pass<'_> {
    fn paint(&mut self, p: &mut Paint) -> bool {
        self.r.paint(p, self.s)
    }
    fn color(&mut self, c: Color) -> Color {
        self.r.shifted(c, None, self.s)
    }
    fn image(&mut self, _key: &str) -> Option<String> {
        // Images' pixels keep their colours.
        None
    }
}

/// Recolour copy `c` by `shifts` (one per `color` row, in order).
pub fn recolour(r: &Recolourer, c: &mut Node, shifts: &[ColourShift]) {
    for s in shifts {
        match s.stroke {
            Some(st) if st == s.fill => {
                if !st.is_zero() {
                    recolor_node(c, Reach { fill: true, stroke: true, images: false }, &mut Pass { r, s: st });
                }
            }
            stroke => {
                if !s.fill.is_zero() {
                    recolor_node(c, Reach { fill: true, stroke: false, images: false }, &mut Pass { r, s: s.fill });
                }
                if let Some(st) = stroke.filter(|st| !st.is_zero()) {
                    recolor_node(c, Reach { fill: false, stroke: true, images: false }, &mut Pass { r, s: st });
                }
            }
        }
    }
}

/// Whether `n` or something in it fills or strokes with a pattern (its tiles keep their colours).
pub fn has_pattern(n: &Node) -> bool {
    let pattern = |p: &Paint| matches!(p, Paint::Pattern { .. });
    let mut found = false;
    n.walk(&mut |m: &Node| {
        if found {
            return;
        }
        found = m.appearance.items.iter().any(|it| match it {
            AppearanceItem::Fill(l) => pattern(&l.paint),
            AppearanceItem::Stroke(l) => pattern(&l.paint),
        });
        if let NodeKind::Text(t) = &m.kind {
            found |= t.runs.iter().any(|r| pattern(&r.style.fill) || pattern(&r.style.stroke));
        }
    });
    found
}
