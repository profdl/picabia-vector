//! Screen: a vector halftone, the computational core (PIC20.5a). A row, panel and command wire it
//! later.
//!
//! Written for Picabia: a spot-function field contoured by our own marching squares
//! ([`crate::contour`]). Nothing here comes from the AGPL `marching-squares` package.
//!
//! # What it makes
//! The art is covered by a screen of cells, `72 / lpi` points apart (lines per inch), turned by
//! `angle` degrees (clockwise on screen). Each cell holds one spot whose size follows the tone
//! (darkness 0 = no ink, 1 = solid ink) at that point: the tone comes from an image's darkness
//! ([`ToneImage`]), the layer's fill, or a fixed percentage — any `Fn(Point) -> f64`.
//!
//! # Spots
//! Every [`SpotShape`] is a function `g(a, b)` of the distance from the cell centre across and
//! down, as fractions of half a cell (`a, b` in 0..1); ink lies where `g <= threshold`. The
//! threshold is chosen per tone from the shape's own table of `g` over a cell, so the inked share
//! of a cell equals the tone for every shape (within about 0.1%).
//!
//! - **Round**: dots that grow, touch at mid-grey (a checkerboard), then turn into round holes that
//!   shrink in the shadows. `g = r_c / (r_c + r_k)`, the distance to the cell centre over the sum of
//!   the distances to the centre and to the nearest cell corner: its level sets are circles round
//!   the centre below 0.5, the line half-way at 0.5, and circles round the corner above it.
//! - **Ellipse**: the same measured with `b` stretched, so dots join into chains along the screen
//!   angle before they close up.
//! - **Line**: lines along the screen angle, as thick as the tone.
//! - **Square**: squares that meet corner to corner at mid-grey, then square holes.
//! - **Diamond**: diamonds that meet at mid-grey, then diamond holes.
//! - **Cross**: crosses whose arms thicken with the tone.
//!
//! Ben-Day dots are Round at a fixed tone; Line Fill is Line at a fixed tone.
//!
//! # Caps
//! The traced grid stays within [`crate::contour::MAX_CELLS`] cells: a finer sampling is coarsened
//! to fit (reported in [`Traced::coarsened`]). Lines per inch, angle and quality are clamped
//! ([`Screen::new`]); a non-finite tone counts as no ink. Images are capped at
//! [`MAX_TONE_PIXELS`].

use vectorcraft_geom::{Point, Rect};

use crate::contour::{self, ContourError, Grid, MAX_CELLS, Ring};

/// Points per inch.
const PT_PER_IN: f64 = 72.0;
/// Clamped ranges.
pub const MIN_LPI: f64 = 1.0;
pub const MAX_LPI: f64 = 300.0;
pub const DEFAULT_LPI: f64 = 20.0;
/// Samples across one screen cell, before any coarsening.
pub const MIN_QUALITY: u32 = 4;
pub const MAX_QUALITY: u32 = 64;
pub const DEFAULT_QUALITY: u32 = 12;
/// The most pixels a [`ToneImage`] holds.
pub const MAX_TONE_PIXELS: usize = 16_000_000;
/// Samples across a quarter cell in a shape's threshold table.
const TABLE_SIDE: usize = 128;
/// How [`SpotShape::Ellipse`] stretches `b`.
const ELLIPSE_STRETCH: f64 = 1.4;

/// The spot drawn in each cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpotShape {
    #[default]
    Round,
    Ellipse,
    Line,
    Square,
    Diamond,
    Cross,
}

impl SpotShape {
    pub const ALL: [SpotShape; 6] = [SpotShape::Round, SpotShape::Ellipse, SpotShape::Line, SpotShape::Square, SpotShape::Diamond, SpotShape::Cross];

    /// The spot function: `a`, `b` in 0..1 (0 at the cell centre, 1 at its edge); smaller values
    /// take ink first. Continuous across cell edges.
    pub fn value(self, a: f64, b: f64) -> f64 {
        match self {
            SpotShape::Round => apollonius(a.hypot(b), (1.0 - a).hypot(1.0 - b)),
            SpotShape::Ellipse => {
                let k = ELLIPSE_STRETCH;
                apollonius(a.hypot(b * k), (1.0 - a).hypot((1.0 - b) * k))
            }
            SpotShape::Line => b,
            SpotShape::Square => apollonius(a.max(b), (1.0 - a).max(1.0 - b)),
            SpotShape::Diamond => (a + b) * 0.5,
            SpotShape::Cross => a.min(b),
        }
    }
}

/// `near / (near + far)`, 0 at the centre, 1 at the corner, ½ half-way.
fn apollonius(near: f64, far: f64) -> f64 {
    let s = near + far;
    if s > 0.0 { near / s } else { 0.0 }
}

/// A screen: shape, ruling and angle, with its threshold table.
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    shape: SpotShape,
    lpi: f64,
    angle_deg: f64,
    quality: u32,
    /// The shape's spot values over a quarter cell, sorted.
    table: Vec<f64>,
    /// cos and sin of `-angle` (into screen space).
    cos: f64,
    sin: f64,
}

/// `v` clamped to `lo..=hi`, or `default` when it isn't finite.
fn clamp_f(v: f64, lo: f64, hi: f64, default: f64) -> f64 {
    if v.is_finite() { v.clamp(lo, hi) } else { default }
}

impl Screen {
    /// A screen, its params clamped: `lpi` to [`MIN_LPI`]..[`MAX_LPI`], the angle to -360..360,
    /// `quality` (samples across a cell) to [`MIN_QUALITY`]..[`MAX_QUALITY`]. Non-finite values take
    /// the defaults.
    pub fn new(shape: SpotShape, lpi: f64, angle_deg: f64, quality: u32) -> Screen {
        let lpi = clamp_f(lpi, MIN_LPI, MAX_LPI, DEFAULT_LPI);
        let angle_deg = clamp_f(angle_deg, -360.0, 360.0, 0.0);
        let quality = quality.clamp(MIN_QUALITY, MAX_QUALITY);
        let mut table = Vec::with_capacity(TABLE_SIDE * TABLE_SIDE);
        for j in 0..TABLE_SIDE {
            for i in 0..TABLE_SIDE {
                let a = (i as f64 + 0.5) / TABLE_SIDE as f64;
                let b = (j as f64 + 0.5) / TABLE_SIDE as f64;
                table.push(shape.value(a, b));
            }
        }
        table.sort_by(f64::total_cmp);
        let r = angle_deg.to_radians();
        Screen { shape, lpi, angle_deg, quality, table, cos: r.cos(), sin: -r.sin() }
    }

    pub fn shape(&self) -> SpotShape {
        self.shape
    }
    pub fn lpi(&self) -> f64 {
        self.lpi
    }
    pub fn angle_deg(&self) -> f64 {
        self.angle_deg
    }
    pub fn quality(&self) -> u32 {
        self.quality
    }

    /// One screen cell's side, in points.
    pub fn period(&self) -> f64 {
        PT_PER_IN / self.lpi
    }

    /// The spot-value threshold that inks `tone` (0..1) of a cell: below every value at 0, above
    /// every value at 1.
    pub fn threshold(&self, tone: f64) -> f64 {
        let (Some(&lo), Some(&hi)) = (self.table.first(), self.table.last()) else { return -1.0 };
        if !tone.is_finite() || tone <= 0.0 {
            return lo - 1.0;
        }
        if tone >= 1.0 {
            return hi + 1.0;
        }
        // The value below which `tone` of the samples lie, interpolated between neighbours.
        let n = self.table.len();
        let x = tone * n as f64 - 0.5;
        if x <= 0.0 {
            return lo;
        }
        let k = x.floor() as usize;
        let f = x - k as f64;
        match (self.table.get(k), self.table.get(k + 1)) {
            (Some(&a), Some(&b)) => a + (b - a) * f,
            (Some(&a), None) => (a + hi.max(a)) * 0.5,
            _ => hi,
        }
    }

    /// The spot value at `p` (document space).
    pub fn spot_at(&self, p: Point) -> f64 {
        let period = self.period();
        let u = (p.x * self.cos - p.y * self.sin) / period;
        let v = (p.x * self.sin + p.y * self.cos) / period;
        // Distance from the nearest cell centre, as a fraction of half a cell.
        let a = ((u - u.round()) * 2.0).abs().min(1.0);
        let b = ((v - v.round()) * 2.0).abs().min(1.0);
        self.shape.value(a, b)
    }

    /// The halftone field at `p` for `tone` there: ink where it is `>= 0`. Non-finite when `p` is.
    pub fn field(&self, p: Point, tone: f64) -> f64 {
        let tone = if tone.is_finite() { tone.clamp(0.0, 1.0) } else { 0.0 };
        self.threshold(tone) - self.spot_at(p)
    }

    /// The sample spacing for `bounds`: [`Screen::period`] / quality, coarsened so the grid fits in
    /// [`MAX_CELLS`]; and whether it was coarsened. `None` for empty or non-finite bounds.
    pub fn step_for(&self, bounds: Rect) -> Option<(f64, bool)> {
        let (w, h) = (bounds.width(), bounds.height());
        if !(bounds.x0.is_finite() && bounds.y0.is_finite() && w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
            return None;
        }
        let want = self.period() / f64::from(self.quality);
        let fits = |s: f64| s.is_finite() && s > 0.0 && grid_cells(w, h, s) <= MAX_CELLS as f64;
        if fits(want) {
            return Some((want, false));
        }
        // The cell count only falls as the step grows; at the longer side the grid is one cell.
        let mut hi = w.max(h).max(want);
        if !fits(hi) {
            return None;
        }
        let mut lo = want;
        for _ in 0..128 {
            let mid = lo + (hi - lo) * 0.5;
            if !(mid > lo && mid < hi) {
                break;
            }
            if fits(mid) { hi = mid } else { lo = mid }
        }
        fits(hi).then_some((hi, true))
    }

    /// The field over `bounds`, tone from `tone`, and whether the sampling was coarsened. The grid
    /// starts at the top-left of `bounds` and covers it, reaching under one step past its right
    /// and bottom edges; spots cut by the grid's edge are closed along it. Clip to the layer's
    /// outline afterwards.
    pub fn grid(&self, bounds: Rect, tone: impl Fn(Point) -> f64) -> Result<(Grid, bool), ContourError> {
        let Some((step, coarsened)) = self.step_for(bounds) else {
            return Err(ContourError::BadBounds);
        };
        let cols = samples_along(bounds.width(), step);
        let rows = samples_along(bounds.height(), step);
        let grid = Grid::sample(cols, rows, Point::new(bounds.x0, bounds.y0), step, |p| self.field(p, tone(p)))?;
        Ok((grid, coarsened))
    }

    /// The halftone over `bounds`: closed rings, holes opposite to outers.
    pub fn trace(&self, bounds: Rect, tone: impl Fn(Point) -> f64) -> Result<Traced, ContourError> {
        let (grid, coarsened) = self.grid(bounds, tone)?;
        let rings = contour::contours(&grid, 0.0)?;
        Ok(Traced { rings, step: grid.step(), coarsened })
    }
}

/// The samples across `len` at `step`: enough to cover it, at least 2.
fn samples_along(len: f64, step: f64) -> usize {
    ((len / step).ceil() as usize).saturating_add(1).max(2)
}

/// The cells of the grid [`Screen::grid`] builds over `w × h` at `step`, as a float (no overflow).
fn grid_cells(w: f64, h: f64, step: f64) -> f64 {
    (w / step).ceil().max(1.0) * (h / step).ceil().max(1.0)
}

/// What [`Screen::trace`] made.
#[derive(Clone, Debug, PartialEq)]
pub struct Traced {
    pub rings: Vec<Ring>,
    /// The sample spacing used.
    pub step: f64,
    /// Whether the sampling was coarsened to fit the cell cap (the dots are less exact).
    pub coarsened: bool,
}

/// An image's darkness (0 white … 1 black), placed on a rectangle of the document, sampled
/// bilinearly; no ink outside it.
#[derive(Clone, Debug, PartialEq)]
pub struct ToneImage {
    width: usize,
    height: usize,
    darkness: Vec<f32>,
    place: Rect,
}

impl ToneImage {
    /// From 8-bit grey levels (0 black, 255 white), row-major. `None` when the size is zero, over
    /// [`MAX_TONE_PIXELS`], doesn't match the data, or the placement isn't finite and non-empty.
    pub fn from_luma8(width: usize, height: usize, luma: &[u8], place: Rect) -> Option<ToneImage> {
        let n = Self::checked(width, height, place)?;
        if luma.len() != n {
            return None;
        }
        let darkness = luma.iter().map(|&l| 1.0 - f32::from(l) / 255.0).collect();
        Some(ToneImage { width, height, darkness, place })
    }

    /// From 8-bit RGBA with straight (not premultiplied) alpha, row-major: darkness from Rec. 709
    /// luminance, scaled by alpha, so transparent pixels count as white (no ink).
    pub fn from_rgba8(width: usize, height: usize, rgba: &[u8], place: Rect) -> Option<ToneImage> {
        let n = Self::checked(width, height, place)?;
        if rgba.len() != n.checked_mul(4)? {
            return None;
        }
        let darkness = rgba
            .chunks_exact(4)
            .map(|px| match *px {
                [r, g, b, a] => {
                    let lum = (0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b)) / 255.0;
                    (1.0 - lum) * (f32::from(a) / 255.0)
                }
                _ => 0.0,
            })
            .collect();
        Some(ToneImage { width, height, darkness, place })
    }

    fn checked(width: usize, height: usize, place: Rect) -> Option<usize> {
        let n = width.checked_mul(height)?;
        let ok = n > 0
            && n <= MAX_TONE_PIXELS
            && [place.x0, place.y0, place.x1, place.y1, place.width(), place.height()].iter().all(|v| v.is_finite())
            && place.width() > 0.0
            && place.height() > 0.0;
        ok.then_some(n)
    }

    fn px(&self, i: usize, j: usize) -> f64 {
        let i = i.min(self.width.saturating_sub(1));
        let j = j.min(self.height.saturating_sub(1));
        self.darkness.get(j * self.width + i).map_or(0.0, |&d| f64::from(d))
    }

    /// The darkness at `p` (0 outside the placement or when `p` isn't finite).
    pub fn darkness_at(&self, p: Point) -> f64 {
        if !(p.x.is_finite() && p.y.is_finite()) {
            return 0.0;
        }
        let r = self.place;
        if p.x < r.x0 || p.x > r.x1 || p.y < r.y0 || p.y > r.y1 {
            return 0.0;
        }
        // Pixel centres sit at half steps.
        let fx = ((p.x - r.x0) / r.width() * self.width as f64 - 0.5).max(0.0);
        let fy = ((p.y - r.y0) / r.height() * self.height as f64 - 0.5).max(0.0);
        let (i, j) = (fx.floor() as usize, fy.floor() as usize);
        let (tx, ty) = (fx - i as f64, fy - j as f64);
        let top = self.px(i, j) * (1.0 - tx) + self.px(i + 1, j) * tx;
        let bottom = self.px(i, j + 1) * (1.0 - tx) + self.px(i + 1, j + 1) * tx;
        (top * (1.0 - ty) + bottom * ty).clamp(0.0, 1.0)
    }
}
