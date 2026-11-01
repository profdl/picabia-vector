//! Stage 2: which colours are cut (colour modes, polarity) and the physical scale.

use vectorcraft_geom::kurbo::Rect;
use vectorcraft_modifiers::{Stencil, StencilColourMode, hex_colour, parse_hex_colour};

use crate::Region;

/// Relative luminance of an 8-bit sRGB colour, 0–1: Rec. 709 weights on the sRGB-decoded
/// (linear) channels.
pub fn luminance(rgb: [u8; 3]) -> f64 {
    let lin = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2])
}

/// Decides, per colour, whether a [`Stencil`] cuts it out.
///
/// - `threshold` (and `separate`, until it exists): cut when [`luminance`] is strictly above
///   `threshold` (lighter is cut).
/// - `pick`: cut when the colour is in `cutColours` (exact match after rounding to 8 bits).
///
/// `invert` negates the decision in both modes (so with `threshold`, colours at or below the level
/// are cut).
#[derive(Clone, Debug, PartialEq)]
pub struct Selector {
    mode: StencilColourMode,
    threshold: f64,
    invert: bool,
    picked: Vec<[u8; 3]>,
}

impl Selector {
    pub fn new(params: &Stencil) -> Self {
        Selector {
            mode: params.colour_mode,
            threshold: params.threshold,
            invert: params.invert,
            picked: params.cut_colours.iter().filter_map(|h| parse_hex_colour(h)).collect(),
        }
    }

    pub fn cuts(&self, rgb: [u8; 3]) -> bool {
        let chosen = match self.mode {
            StencilColourMode::Pick => self.picked.contains(&rgb),
            StencilColourMode::Threshold | StencilColourMode::Separate => luminance(rgb) > self.threshold,
        };
        chosen != self.invert
    }
}

/// One colour of the design, for the panel's swatch list.
#[derive(Clone, Debug, PartialEq)]
pub struct ColourInfo {
    /// `"#rrggbb"`, lowercase.
    pub hex: String,
    /// Whether the stencil cuts this colour out (before clean-up).
    pub cut: bool,
    /// The colour's visible area at physical size, mm² (0 when fully covered).
    pub area: f64,
}

/// The swatch list for `regions`: largest visible area first (ties by hex).
pub fn colour_infos(regions: &[Region], selector: &Selector, mm_per_unit: f64) -> Vec<ColourInfo> {
    let mut out: Vec<ColourInfo> = regions
        .iter()
        .map(|r| ColourInfo { hex: hex_colour(r.colour), cut: selector.cuts(r.colour), area: r.area.max(0.0) * mm_per_unit * mm_per_unit })
        .collect();
    out.sort_by(|a, b| b.area.total_cmp(&a.area).then_with(|| a.hex.cmp(&b.hex)));
    out
}

/// Millimetres per layer unit: `widthMm / layer width` (the height follows the aspect ratio).
/// `None` when the bounds or the width aren't usable (non-finite, or a layer of no width). Bounds
/// with `x1 < x0` count by their absolute width.
pub fn mm_per_unit(width_mm: f64, layer_bounds: Rect) -> Option<f64> {
    let finite = [layer_bounds.x0, layer_bounds.y0, layer_bounds.x1, layer_bounds.y1].iter().all(|v| v.is_finite());
    let w = layer_bounds.abs().width();
    let mpu = width_mm / w;
    (finite && w > 0.0 && mpu.is_finite() && mpu > 0.0).then_some(mpu)
}
