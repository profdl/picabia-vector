//! The stack model: a [`Stack`] of [`Modifier`]s, each with an id, an enabled flag and a
//! [`ModifierKind`] carrying its parameters.
//!
//! Every parameter struct reads with `#[serde(default)]` (a missing field takes its default) and
//! writes every field, camelCase. Fields a struct doesn't know are kept in [`Modifier::extra`];
//! kinds this build doesn't know (or whose params don't read) become [`ModifierKind::Other`] with
//! all their params in `extra`, so nothing is lost on a round trip.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub use crate::model_deform::{Deform, DeformAxis, DeformMode, Falloff, ShearOrigin};
pub use crate::model_displace::{Displace, DisplaceMode};
pub use crate::model_shape::{CloseMethod, Resample, ResampleMode, SortBy, SubdivideMethod, Tidy, Trim};
use crate::{MAX_STACK_LEN, ModError, STACK_VERSION, lenient};

/// The kinds this build can add, as `(kind id, label)`, in menu order.
pub const KINDS: &[(&str, &str)] = &[
    ("linearArray", "Linear array"),
    ("circularArray", "Circular array"),
    ("gridArray", "Grid array"),
    ("mirror", "Mirror"),
    ("transformEach", "Transform each"),
    ("randomize", "Randomize"),
    ("wallpaper", "Wallpaper"),
    ("copyToShape", "Copy to shape"),
    ("stencil", "Stencil"),
    ("resample", "Resample"),
    ("trim", "Trim"),
    ("tidy", "Tidy"),
    ("deform", "Deform"),
    ("displace", "Displace"),
];

/// The label of kind id `kind` ("Linear array"), or `None` for a kind this build doesn't know.
pub fn kind_label(kind: &str) -> Option<&'static str> {
    KINDS.iter().find(|(k, _)| *k == kind).map(|(_, l)| *l)
}

/// One layer's modifier stack. `modifiers[0]` runs first.
#[derive(Clone, Debug, PartialEq)]
pub struct Stack {
    /// The format version read from `"v"` ([`STACK_VERSION`] when missing).
    pub version: u64,
    pub modifiers: Vec<Modifier>,
    /// Top-level fields other than `v` and `stack`, kept on a round trip.
    pub extra: Map<String, Value>,
}

impl Default for Stack {
    fn default() -> Self {
        Stack { version: STACK_VERSION, modifiers: Vec::new(), extra: Map::new() }
    }
}

impl Stack {
    pub fn is_empty(&self) -> bool {
        self.modifiers.is_empty()
    }

    pub fn len(&self) -> usize {
        self.modifiers.len()
    }

    /// The position of the modifier with id `id`.
    pub fn position(&self, id: &str) -> Option<usize> {
        self.modifiers.iter().position(|m| m.id == id)
    }

    /// Whether any modifier is enabled and of a kind this build evaluates.
    pub fn is_active(&self) -> bool {
        self.modifiers.iter().any(|m| m.enabled && !matches!(m.kind, ModifierKind::Other { .. }))
    }

    /// A fresh modifier id: `m<n>` with `n` one past the highest `m<number>` in the stack.
    pub fn next_id(&self) -> String {
        let max = self.modifiers.iter().filter_map(|m| m.id.strip_prefix('m')?.parse::<u64>().ok()).max().unwrap_or(0);
        format!("m{}", max.saturating_add(1))
    }

    /// Insert `m` at `at` (clamped to the end), giving it a fresh id when its id is empty or
    /// already taken. Refused when the stack is full ([`MAX_STACK_LEN`]).
    pub fn insert(&mut self, at: usize, mut m: Modifier) -> Result<&Modifier, ModError> {
        if self.modifiers.len() >= MAX_STACK_LEN {
            return Err(ModError::TooManyModifiers { count: self.modifiers.len() + 1 });
        }
        if m.id.is_empty() || self.position(&m.id).is_some() {
            m.id = self.next_id();
        }
        let at = at.min(self.modifiers.len());
        self.modifiers.insert(at, m);
        self.modifiers.get(at).ok_or_else(|| ModError::Invalid("insert failed".into()))
    }

    /// Give every modifier with an empty or repeated id a fresh one.
    pub(crate) fn fix_ids(&mut self) {
        let mut seen = std::collections::BTreeSet::new();
        for i in 0..self.modifiers.len() {
            let taken = self.modifiers.get(i).is_none_or(|m| m.id.is_empty() || seen.contains(&m.id));
            if taken {
                let id = self.next_id();
                if let Some(m) = self.modifiers.get_mut(i) {
                    m.id = id;
                }
            }
            if let Some(m) = self.modifiers.get(i) {
                seen.insert(m.id.clone());
            }
        }
    }
}

/// One modifier row.
#[derive(Clone, Debug, PartialEq)]
pub struct Modifier {
    /// Unique within its stack (`m1`, `m2`…).
    pub id: String,
    pub enabled: bool,
    pub kind: ModifierKind,
    /// Fields the kind's parameter struct doesn't know (all params for [`ModifierKind::Other`]).
    pub extra: Map<String, Value>,
}

/// What a modifier does, with its parameters.
#[derive(Clone, Debug, PartialEq)]
pub enum ModifierKind {
    LinearArray(LinearArray),
    CircularArray(CircularArray),
    GridArray(GridArray),
    Mirror(Mirror),
    TransformEach(TransformEach),
    Randomize(Randomize),
    Wallpaper(Wallpaper),
    CopyToShape(CopyToShape),
    /// Terminal: the stack ends here (see [`Stencil`]).
    Stencil(Stencil),
    /// Geometry rows: they change the object's paths instead of placing copies (see
    /// [`crate::shape`]).
    Resample(Resample),
    Trim(Trim),
    Tidy(Tidy),
    Deform(Deform),
    Displace(Displace),
    /// A kind this build doesn't know, or a known kind whose params don't read (`error` says
    /// why). Kept on a round trip, ignored when evaluating.
    Other {
        kind: String,
        error: Option<String>,
    },
}

/// `linearArray`: `count` copies in a row, each `offsetX`/`offsetY` points further from the one
/// before, turned `rotateEach` degrees more and scaled by `scaleEach` once more (around its own
/// centre).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LinearArray {
    /// Copies including the original (clamped to `1..=MAX_COUNT` when evaluating).
    #[serde(deserialize_with = "lenient::count")]
    pub count: u32,
    /// Points between neighbouring copies.
    pub offset_x: f64,
    pub offset_y: f64,
    /// Degrees each copy turns more than the one before.
    pub rotate_each: f64,
    /// Factor each copy is scaled by relative to the one before (1 = same size).
    pub scale_each: f64,
}

impl Default for LinearArray {
    fn default() -> Self {
        LinearArray { count: 3, offset_x: 100.0, offset_y: 0.0, rotate_each: 0.0, scale_each: 1.0 }
    }
}

/// `circularArray`: `count` copies on a circle of `radius` around the pivot offset by
/// `centerX`/`centerY`, from `startAngle` over `sweep` degrees; `align` turns each copy to follow
/// the circle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CircularArray {
    #[serde(deserialize_with = "lenient::count")]
    pub count: u32,
    pub radius: f64,
    pub start_angle: f64,
    pub sweep: f64,
    pub center_x: f64,
    pub center_y: f64,
    pub align: bool,
}

impl Default for CircularArray {
    fn default() -> Self {
        CircularArray { count: 6, radius: 100.0, start_angle: 0.0, sweep: 360.0, center_x: 0.0, center_y: 0.0, align: true }
    }
}

/// `gridArray`: `rows` × `cols` copies, `gapX`/`gapY` points apart.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GridArray {
    #[serde(deserialize_with = "lenient::count")]
    pub rows: u32,
    #[serde(deserialize_with = "lenient::count")]
    pub cols: u32,
    pub gap_x: f64,
    pub gap_y: f64,
}

impl Default for GridArray {
    fn default() -> Self {
        GridArray { rows: 3, cols: 3, gap_x: 100.0, gap_y: 100.0 }
    }
}

/// Which axis a [`Mirror`] reflects across.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MirrorAxis {
    /// A vertical line through the pivot: left and right swap.
    #[default]
    Vertical,
    /// A horizontal line through the pivot: top and bottom swap.
    Horizontal,
    /// Both lines: four quadrants.
    Both,
}

/// `mirror`: reflect across `axis` through the pivot moved `offset` points along the reflection
/// direction; with `keepOriginal` the previous set stays too.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Mirror {
    pub axis: MirrorAxis,
    pub offset: f64,
    pub keep_original: bool,
}

impl Default for Mirror {
    fn default() -> Self {
        Mirror { axis: MirrorAxis::Vertical, offset: 0.0, keep_original: true }
    }
}

/// The 17 plane crystallographic groups, by their short (Hermann-Mauguin) names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WallpaperGroup {
    #[serde(rename = "p1")]
    P1,
    #[serde(rename = "p2")]
    P2,
    #[serde(rename = "pm")]
    Pm,
    #[serde(rename = "pg")]
    Pg,
    #[serde(rename = "cm")]
    Cm,
    #[serde(rename = "pmm")]
    Pmm,
    #[serde(rename = "pmg")]
    Pmg,
    #[serde(rename = "pgg")]
    Pgg,
    #[serde(rename = "cmm")]
    Cmm,
    #[serde(rename = "p4")]
    P4,
    #[default]
    #[serde(rename = "p4m")]
    P4m,
    #[serde(rename = "p4g")]
    P4g,
    #[serde(rename = "p3")]
    P3,
    #[serde(rename = "p3m1")]
    P3m1,
    #[serde(rename = "p31m")]
    P31m,
    #[serde(rename = "p6")]
    P6,
    #[serde(rename = "p6m")]
    P6m,
}

impl WallpaperGroup {
    /// All 17 groups, in the usual order.
    pub const ALL: [WallpaperGroup; 17] = [
        WallpaperGroup::P1,
        WallpaperGroup::P2,
        WallpaperGroup::Pm,
        WallpaperGroup::Pg,
        WallpaperGroup::Cm,
        WallpaperGroup::Pmm,
        WallpaperGroup::Pmg,
        WallpaperGroup::Pgg,
        WallpaperGroup::Cmm,
        WallpaperGroup::P4,
        WallpaperGroup::P4m,
        WallpaperGroup::P4g,
        WallpaperGroup::P3,
        WallpaperGroup::P3m1,
        WallpaperGroup::P31m,
        WallpaperGroup::P6,
        WallpaperGroup::P6m,
    ];

    /// Whether `cellHeight` shapes the lattice: the oblique and rectangular groups (including the
    /// centred ones) do; the square and hexagonal lattices are fixed by `cellWidth` alone.
    pub fn uses_cell_height(self) -> bool {
        !matches!(self, Self::P4 | Self::P4m | Self::P4g | Self::P3 | Self::P3m1 | Self::P31m | Self::P6 | Self::P6m)
    }

    /// Whether `angle` shapes the lattice: only the oblique groups, whose lattice vectors can meet
    /// at any angle; every other lattice has its angle fixed by the group.
    pub fn uses_angle(self) -> bool {
        matches!(self, Self::P1 | Self::P2)
    }

    /// Whether `originX`/`originY` change the copies: p1 has no symmetry element to move (its only
    /// copy of a cell is a translation), every other group's rotation centres and mirror or glide
    /// axes pass through the origin.
    pub fn uses_origin(self) -> bool {
        self != Self::P1
    }
}

/// `wallpaper`: one of the 17 plane groups as a lattice of `cols` x `rows` cells, each holding the
/// group's motif copies. The origin of the symmetry is the layer's top-left corner plus
/// `originX`/`originY`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Wallpaper {
    pub group: WallpaperGroup,
    #[serde(deserialize_with = "lenient::count")]
    pub cols: u32,
    #[serde(deserialize_with = "lenient::count")]
    pub rows: u32,
    pub cell_width: f64,
    pub cell_height: f64,
    pub angle: f64,
    pub origin_x: f64,
    pub origin_y: f64,
}

impl Default for Wallpaper {
    fn default() -> Self {
        Wallpaper { group: WallpaperGroup::P4m, cols: 3, rows: 3, cell_width: 100.0, cell_height: 100.0, angle: 90.0, origin_x: 0.0, origin_y: 0.0 }
    }
}

/// How a [`CopyToShape`] places its copies on the target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CopyMode {
    /// One copy at each anchor point of the target's outline.
    Points,
    /// `count` copies spaced evenly by arc length along the target's outline.
    #[default]
    AlongPath,
    /// `count` copies at seeded pseudo-random points inside the target's fill area.
    Populate,
}

/// `copyToShape`: copies of the previous set placed on another layer, the `target` (a layer id, as
/// text), whose outline is read in this layer's parent space. Each copy moves the layer's pivot to
/// its point, scaled by `scale` around it; with `align` (points and alongPath) it turns to follow
/// the outline's direction there. The original isn't kept: every copy is on the target. A
/// missing target, the layer itself, an ancestor or descendant of it, or a target whose own stack
/// points back at it places no copies (with a warning).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CopyToShape {
    /// The target layer's id (`"12"`); empty = not chosen yet.
    #[serde(deserialize_with = "lenient::target")]
    pub target: String,
    pub mode: CopyMode,
    /// Copies for `alongPath` and `populate` (clamped to `1..=MAX_COUNT`); `points` makes one per
    /// anchor.
    #[serde(deserialize_with = "lenient::count")]
    pub count: u32,
    /// Factor each copy is scaled by around its pivot.
    pub scale: f64,
    /// Turn each copy to the outline's direction (points and alongPath).
    pub align: bool,
    /// Seed for `populate`.
    #[serde(deserialize_with = "lenient::seed")]
    pub seed: u32,
}

impl Default for CopyToShape {
    fn default() -> Self {
        CopyToShape { target: String::new(), mode: CopyMode::AlongPath, count: 10, scale: 1.0, align: true, seed: 1 }
    }
}

/// How a [`Stencil`] chooses the colours it cuts out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StencilColourMode {
    /// Colours whose luminance is above `threshold` are cut (below with `invert`).
    #[default]
    Threshold,
    /// Colours listed in `cutColours` are cut (the others with `invert`).
    Pick,
    /// One stencil per colour (P12): reads and writes, evaluates as `threshold` with a warning.
    Separate,
}

/// How the canvas previews a [`Stencil`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StencilPreview {
    /// The sheet in `paperColour` with the cut-outs open.
    #[default]
    Paper,
    /// The dyed cloth: `pasteColour` where the paste goes through, `clothColour` elsewhere.
    Cloth,
}

/// What a [`Stencil`] does with paper islands (paper pieces the cut separates from the sheet).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StencilTieMode {
    /// Every island is held by `tiesPerIsland` tie bars to the paper around it.
    #[default]
    Ties,
    /// No ties: the islands are kept (held by a gauze or silk mesh laid on the stencil) and flagged.
    Gauze,
    /// A lattice of thin paper bars across the cut (`pattern*` fields) holds the islands it
    /// crosses; the ones it misses get automatic ties.
    Pattern,
}

/// The lattice of a [`Stencil`] in `pattern` tie mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StencilPatternType {
    /// Horizontal and vertical bars.
    #[default]
    Grid,
    /// Horizontal bars only.
    Horizontal,
    /// Vertical bars only.
    Vertical,
}

/// Where a [`Stencil`]'s lattice goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StencilPatternWhere {
    /// Across every cut-out.
    #[default]
    Everywhere,
    /// Only in the cut-outs that border an island.
    Islands,
}

/// The sheet a [`Stencil`] is cut from: a preset of the four cutting limits (see
/// [`StencilMaterial::limits`]) or `custom` (the fields as set).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StencilMaterial {
    /// Kakishibu (persimmon-tannin) paper: the stencil's defaults.
    #[default]
    Paper,
    /// Mylar / PET film: finer cuts and bridges hold.
    Film,
    /// Card: coarser cuts, wider bridges and ties.
    Card,
    /// No preset: the limit fields as set.
    Custom,
}

/// The cutting limits of a [`StencilMaterial`] preset: the four [`Stencil`] fields it fills, plus
/// the laser kerf (for export; not a field).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialLimits {
    /// [`Stencil::min_cut_width_mm`].
    pub min_cut_width_mm: f64,
    /// [`Stencil::min_bridge_mm`].
    pub min_bridge_mm: f64,
    /// [`Stencil::tie_width_mm`].
    pub tie_width_mm: f64,
    /// [`Stencil::min_corner_angle`], degrees.
    pub min_corner_angle: f64,
    /// The laser's cut width in this material, mm.
    pub kerf_mm: f64,
}

impl StencilMaterial {
    /// Every material, presets first, in menu order.
    pub const ALL: [StencilMaterial; 4] = [StencilMaterial::Paper, StencilMaterial::Film, StencilMaterial::Card, StencilMaterial::Custom];

    /// The preset's limits; `None` for `custom`. `paper` is exactly the [`Stencil`] defaults.
    pub fn limits(self) -> Option<MaterialLimits> {
        let l = |min_cut_width_mm, min_bridge_mm, tie_width_mm, min_corner_angle, kerf_mm| MaterialLimits {
            min_cut_width_mm,
            min_bridge_mm,
            tie_width_mm,
            min_corner_angle,
            kerf_mm,
        };
        match self {
            StencilMaterial::Paper => Some(l(0.3, 0.5, 1.0, 30.0, 0.10)),
            StencilMaterial::Film => Some(l(0.2, 0.4, 0.6, 20.0, 0.08)),
            StencilMaterial::Card => Some(l(0.5, 1.0, 1.5, 35.0, 0.15)),
            StencilMaterial::Custom => None,
        }
    }

    /// The id written in the file (`"paper"`, `"film"`, `"card"`, `"custom"`).
    pub fn id(self) -> &'static str {
        match self {
            StencilMaterial::Paper => "paper",
            StencilMaterial::Film => "film",
            StencilMaterial::Card => "card",
            StencilMaterial::Custom => "custom",
        }
    }
}

/// What a [`Stencil`]'s clean-up does with cuts and bridges that break its limits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StencilLimitMode {
    /// Clean-up fixes them: narrow slivers and bridges are opened/closed away, small pieces dropped.
    #[default]
    Fix,
    /// Clean-up only simplifies: every violation stays and is reported.
    Flag,
}

/// Whether a [`Stencil`] uses the repeat of a `wallpaper` or `gridArray` row above it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StencilRepeat {
    /// Repeat-aware when the last lattice row above is a `wallpaper` or `gridArray` (and nothing
    /// after it breaks the lattice): see [`crate::Output::repeat`].
    #[default]
    Auto,
    /// Treat the design as one picture.
    Off,
}

/// What a [`Stencil`] cuts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StencilOutput {
    /// The whole design on one sheet.
    #[default]
    Sheet,
    /// One repeat cell plus margin and registration holes, for step-and-repeat printing. Without
    /// a repeat it evaluates as `sheet` with a warning.
    Tile,
}

/// `stencil`: turn the layer (everything the rows above it drew) into a laser-cut katagami stencil.
/// **Terminal**: it adds no copies (the copies of the rows above stand), and enabled rows below it
/// are ignored with a warning; the evaluated [`crate::Output::stencil`] carries its params (clamped)
/// for the engine and renderer, which compute and draw the sheet (`vectorcraft-stencil`).
///
/// Lengths are in **millimetres** at the stencil's physical size: `mm_per_unit = widthMm / layer
/// width`. Colours are `"#rrggbb"` texts (lowercase when written). Ranges are applied by
/// [`Stencil::clamped`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Stencil {
    /// Unknown values read as the default.
    #[serde(deserialize_with = "lenient::choice")]
    pub colour_mode: StencilColourMode,
    /// `pick`: the colours cut out. At most [`MAX_CUT_COLOURS`]; entries that aren't colours are
    /// dropped, repeats kept once.
    #[serde(deserialize_with = "lenient::colours")]
    pub cut_colours: Vec<String>,
    /// `threshold`: the luminance level, 0–1 (Rec. 709 on sRGB-decoded values).
    pub threshold: f64,
    /// Swap what is cut and what stays paper (both modes).
    pub invert: bool,
    /// The design's physical width (the height follows the aspect ratio), 1–10000 mm.
    pub width_mm: f64,
    /// The paper border around the design, 0–1000 mm.
    pub margin_mm: f64,
    /// Smaller cut pieces and paper holes are dropped, 0–10000 mm².
    pub min_cut_area_mm2: f64,
    /// Narrower cut slivers are removed, 0–100 mm.
    pub min_cut_width_mm: f64,
    /// Narrower paper bridges are removed, 0–100 mm.
    pub min_bridge_mm: f64,
    /// Simplification tolerance, 0–10 mm.
    pub tolerance_mm: f64,
    /// Unknown values read as the default.
    #[serde(deserialize_with = "lenient::choice")]
    pub preview: StencilPreview,
    #[serde(deserialize_with = "lenient::paper_colour")]
    pub paper_colour: String,
    #[serde(deserialize_with = "lenient::paste_colour")]
    pub paste_colour: String,
    #[serde(deserialize_with = "lenient::cloth_colour")]
    pub cloth_colour: String,
    /// Ties or gauze for the paper islands. Unknown values read as the default.
    #[serde(deserialize_with = "lenient::choice")]
    pub tie_mode: StencilTieMode,
    /// Width of each tie bar, 0.05–50 mm.
    pub tie_width_mm: f64,
    /// Ties holding each island, 1–8.
    #[serde(deserialize_with = "lenient::count")]
    pub ties_per_island: u32,
    /// Keep an island's ties apart (spread around it) rather than all on its shortest gaps.
    pub tie_spread: bool,
    /// Preferred tie direction in degrees (0 = right, 90 = up on the canvas; the opposite
    /// direction counts too), wrapped to `0..360`; `None` (`null`) = no preference.
    #[serde(deserialize_with = "lenient::optional_number")]
    pub tie_angle: Option<f64>,
    /// Breaks ties between equally short gaps.
    #[serde(deserialize_with = "lenient::seed")]
    pub tie_seed: u32,
    /// `pattern` mode: the lattice's bars. Unknown values read as the default.
    #[serde(deserialize_with = "lenient::choice")]
    pub pattern_type: StencilPatternType,
    /// `pattern` mode: bar width (the tie size), 0.05–50 mm.
    pub pattern_width_mm: f64,
    /// `pattern` mode: centre-to-centre bar spacing, 0.5–500 mm and at least twice the width.
    pub pattern_spacing_mm: f64,
    /// `pattern` mode: the lattice's rotation in degrees (counter-clockwise on the canvas), wrapped
    /// to `0..360`. The lattice is anchored at the sheet's top-left corner.
    pub pattern_angle: f64,
    /// `pattern` mode: every cut-out, or only those bordering an island. Unknown values read as the
    /// default.
    #[serde(deserialize_with = "lenient::choice")]
    pub pattern_where: StencilPatternWhere,
    /// The sheet material: a preset fills `minCutWidthMm`, `minBridgeMm`, `tieWidthMm` and
    /// `minCornerAngle` ([`Modifier::set_props`]); editing one of those away from the preset makes
    /// it `custom`. Unknown values read as the default.
    #[serde(deserialize_with = "lenient::choice")]
    pub material: StencilMaterial,
    /// Fix or only flag cuts and bridges that break the limits. Unknown values read as the default.
    #[serde(deserialize_with = "lenient::choice")]
    pub limit_mode: StencilLimitMode,
    /// Paper corners sharper than this many degrees are flagged, 0–179.
    pub min_corner_angle: f64,
    /// Registration holes (hoshi) at the sheet's or tile's corners.
    pub hoshi: bool,
    /// The registration holes' diameter, 0.2–20 mm.
    pub hoshi_diameter_mm: f64,
    /// Use the repeat of a lattice row above. Unknown values read as the default.
    #[serde(deserialize_with = "lenient::choice")]
    pub repeat: StencilRepeat,
    /// The whole sheet or one repeat tile. Unknown values read as the default.
    #[serde(deserialize_with = "lenient::choice")]
    pub output: StencilOutput,
}

/// The most entries [`Stencil::cut_colours`] keeps.
pub const MAX_CUT_COLOURS: usize = 256;
/// [`Stencil::ties_per_island`]'s range is `1..=MAX_TIES_PER_ISLAND`.
pub const MAX_TIES_PER_ISLAND: u32 = 8;
/// [`Stencil::tie_width_mm`]'s range, mm.
pub const MIN_TIE_WIDTH_MM: f64 = 0.05;
/// [`Stencil::tie_width_mm`]'s range, mm.
pub const MAX_TIE_WIDTH_MM: f64 = 50.0;
/// [`Stencil::pattern_spacing_mm`]'s range, mm (it is also at least twice the width).
pub const MIN_PATTERN_SPACING_MM: f64 = 0.5;
/// [`Stencil::pattern_spacing_mm`]'s range, mm.
pub const MAX_PATTERN_SPACING_MM: f64 = 500.0;
/// [`Stencil::min_corner_angle`]'s range, degrees.
pub const MAX_CORNER_ANGLE: f64 = 179.0;
/// [`Stencil::hoshi_diameter_mm`]'s range, mm.
pub const MIN_HOSHI_DIAMETER_MM: f64 = 0.2;
/// [`Stencil::hoshi_diameter_mm`]'s range, mm.
pub const MAX_HOSHI_DIAMETER_MM: f64 = 20.0;
/// [`Stencil::paper_colour`]'s default: kakishibu (persimmon-tannin) paper brown.
pub const STENCIL_PAPER: &str = "#c8a165";
/// [`Stencil::paste_colour`]'s default: indigo.
pub const STENCIL_PASTE: &str = "#1f3a8a";
/// [`Stencil::cloth_colour`]'s default: undyed cloth.
pub const STENCIL_CLOTH: &str = "#f4efe6";

impl Default for Stencil {
    fn default() -> Self {
        Stencil {
            colour_mode: StencilColourMode::Threshold,
            cut_colours: Vec::new(),
            threshold: 0.5,
            invert: false,
            width_mm: 300.0,
            margin_mm: 15.0,
            min_cut_area_mm2: 0.5,
            min_cut_width_mm: 0.3,
            min_bridge_mm: 0.5,
            tolerance_mm: 0.05,
            preview: StencilPreview::Paper,
            paper_colour: STENCIL_PAPER.into(),
            paste_colour: STENCIL_PASTE.into(),
            cloth_colour: STENCIL_CLOTH.into(),
            tie_mode: StencilTieMode::Ties,
            tie_width_mm: 1.0,
            ties_per_island: 2,
            tie_spread: true,
            tie_angle: None,
            tie_seed: 1,
            pattern_type: StencilPatternType::Grid,
            pattern_width_mm: 0.6,
            pattern_spacing_mm: 8.0,
            pattern_angle: 0.0,
            pattern_where: StencilPatternWhere::Everywhere,
            material: StencilMaterial::Paper,
            limit_mode: StencilLimitMode::Fix,
            min_corner_angle: 30.0,
            hoshi: true,
            hoshi_diameter_mm: 1.5,
            repeat: StencilRepeat::Auto,
            output: StencilOutput::Sheet,
        }
    }
}

impl Stencil {
    /// These params with every number in its range (a non-finite one takes its default), `separate`
    /// evaluated as `threshold`, and the colours checked; with a warning for each change. What
    /// [`crate::evaluate`] puts in [`crate::Output::stencil`].
    pub fn clamped(&self) -> (Stencil, Vec<String>) {
        crate::kinds::stencil_clamped(self)
    }
}

/// `"#rrggbb"` (also `#rgb`, any case, `#` optional) as 8-bit sRGB channels.
pub fn parse_hex_colour(s: &str) -> Option<[u8; 3]> {
    let h = s.trim();
    let h = h.strip_prefix('#').unwrap_or(h);
    if !h.is_ascii() {
        return None;
    }
    let nib = |i: usize| h.get(i..=i).and_then(|c| u8::from_str_radix(c, 16).ok());
    let byte = |i: usize| h.get(i..i + 2).and_then(|c| u8::from_str_radix(c, 16).ok());
    match h.len() {
        3 => Some([nib(0)? * 17, nib(1)? * 17, nib(2)? * 17]),
        6 => Some([byte(0)?, byte(2)?, byte(4)?]),
        _ => None,
    }
}

/// 8-bit sRGB channels as `"#rrggbb"` (lowercase).
pub fn hex_colour(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// `transformEach`: move, turn and scale every instance around its own centre; with
/// `progressive` the amounts ramp from none on the first instance to the full amount on the last.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TransformEach {
    pub offset_x: f64,
    pub offset_y: f64,
    pub rotate: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub progressive: bool,
    /// Seeded per-copy variation: each amount above is multiplied by `1 + jitter · r`, `r` in
    /// -1..=1 (0 = off).
    pub jitter: f64,
    #[serde(deserialize_with = "lenient::seed")]
    pub jitter_seed: u32,
    /// Per-copy expressions ([`crate::expr`]) giving a field's amount for each instance instead of
    /// the field (which they read as `value`); an expression-driven field doesn't ramp with
    /// `progressive`, jitter still applies. Written only when set.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient::expr")]
    pub offset_x_expr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient::expr")]
    pub offset_y_expr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient::expr")]
    pub rotate_expr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient::expr")]
    pub scale_x_expr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient::expr")]
    pub scale_y_expr: Option<String>,
}

impl Default for TransformEach {
    fn default() -> Self {
        TransformEach {
            offset_x: 0.0,
            offset_y: 0.0,
            rotate: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            progressive: false,
            jitter: 0.0,
            jitter_seed: 1,
            offset_x_expr: None,
            offset_y_expr: None,
            rotate_expr: None,
            scale_x_expr: None,
            scale_y_expr: None,
        }
    }
}

/// `randomize`: jitter every instance's position (points), rotation (degrees) and scale (a
/// fraction: 0.2 = up to ±20%), deterministically from `seed` and the instance index.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Randomize {
    pub position: f64,
    pub rotation: f64,
    pub scale: f64,
    #[serde(deserialize_with = "lenient::seed")]
    pub seed: u32,
    /// Per-copy expressions ([`crate::expr`]) for each amount (`value` = the field). Written only
    /// when set.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient::expr")]
    pub position_expr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient::expr")]
    pub rotation_expr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient::expr")]
    pub scale_expr: Option<String>,
}

/// The fields of kind id `kind` that take a per-copy expression (stored as `<field>Expr`).
pub fn expr_fields(kind: &str) -> &'static [&'static str] {
    match kind {
        "transformEach" => &["offsetX", "offsetY", "rotate", "scaleX", "scaleY"],
        "randomize" => &["position", "rotation", "scale"],
        _ => &[],
    }
}

/// The param holding `field`'s expression (`rotate` → `rotateExpr`).
pub fn expr_key(field: &str) -> String {
    format!("{field}Expr")
}

/// The field whose expression param `key` is, when kind id `kind` takes one (`rotateExpr` →
/// `rotate`).
pub fn expr_field<'a>(kind: &str, key: &'a str) -> Option<&'a str> {
    let field = key.strip_suffix("Expr")?;
    expr_fields(kind).contains(&field).then_some(field)
}

/// The fields every modifier has besides its params.
const COMMON: [&str; 3] = ["id", "enabled", "kind"];

fn params_of<T: Serialize>(p: &T) -> Map<String, Value> {
    match serde_json::to_value(p) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

/// Read params `rest` as `T`: numeric strings count as numbers. On success, the fields `T` doesn't
/// know are returned as extras.
fn read<T: for<'de> Deserialize<'de> + Serialize>(rest: &Map<String, Value>) -> Result<(T, Map<String, Value>), String> {
    let coerced: Map<String, Value> = rest
        .iter()
        .map(|(k, v)| {
            let v = match v {
                // Expression texts stay text (`"rotateExpr": "15"`).
                Value::String(_) if !k.ends_with("Expr") => {
                    lenient::finite(v).and_then(serde_json::Number::from_f64).map_or_else(|| v.clone(), Value::Number)
                }
                _ => v.clone(),
            };
            (k.clone(), v)
        })
        .collect();
    let p: T = serde_json::from_value(Value::Object(coerced)).map_err(|e| e.to_string())?;
    let known = params_of(&p);
    let extra = rest.iter().filter(|(k, _)| !known.contains_key(*k)).map(|(k, v)| (k.clone(), v.clone())).collect();
    Ok((p, extra))
}

impl ModifierKind {
    /// Whether this is a geometry row (it changes the object's paths; [`crate::shape`]).
    pub fn is_geometry(&self) -> bool {
        matches!(
            self,
            ModifierKind::Resample(_) | ModifierKind::Trim(_) | ModifierKind::Tidy(_) | ModifierKind::Deform(_) | ModifierKind::Displace(_)
        )
    }

    /// The kind id written in `"kind"` (`"linearArray"`…).
    pub fn id(&self) -> &str {
        match self {
            ModifierKind::LinearArray(_) => "linearArray",
            ModifierKind::CircularArray(_) => "circularArray",
            ModifierKind::GridArray(_) => "gridArray",
            ModifierKind::Mirror(_) => "mirror",
            ModifierKind::TransformEach(_) => "transformEach",
            ModifierKind::Randomize(_) => "randomize",
            ModifierKind::Wallpaper(_) => "wallpaper",
            ModifierKind::CopyToShape(_) => "copyToShape",
            ModifierKind::Stencil(_) => "stencil",
            ModifierKind::Resample(_) => "resample",
            ModifierKind::Trim(_) => "trim",
            ModifierKind::Tidy(_) => "tidy",
            ModifierKind::Deform(_) => "deform",
            ModifierKind::Displace(_) => "displace",
            ModifierKind::Other { kind, .. } => kind,
        }
    }

    /// The default parameters of kind id `kind`, or `None` for a kind this build doesn't know.
    pub fn default_for(kind: &str) -> Option<ModifierKind> {
        Some(match kind {
            "linearArray" => ModifierKind::LinearArray(LinearArray::default()),
            "circularArray" => ModifierKind::CircularArray(CircularArray::default()),
            "gridArray" => ModifierKind::GridArray(GridArray::default()),
            "mirror" => ModifierKind::Mirror(Mirror::default()),
            "transformEach" => ModifierKind::TransformEach(TransformEach::default()),
            "randomize" => ModifierKind::Randomize(Randomize::default()),
            "wallpaper" => ModifierKind::Wallpaper(Wallpaper::default()),
            "copyToShape" => ModifierKind::CopyToShape(CopyToShape::default()),
            "stencil" => ModifierKind::Stencil(Stencil::default()),
            "resample" => ModifierKind::Resample(Resample::default()),
            "trim" => ModifierKind::Trim(Trim::default()),
            "tidy" => ModifierKind::Tidy(Tidy::default()),
            "deform" => ModifierKind::Deform(Deform::default()),
            "displace" => ModifierKind::Displace(Displace::default()),
            _ => return None,
        })
    }

    /// The parameters as a JSON object (empty for [`ModifierKind::Other`]).
    pub fn params(&self) -> Map<String, Value> {
        match self {
            ModifierKind::LinearArray(p) => params_of(p),
            ModifierKind::CircularArray(p) => params_of(p),
            ModifierKind::GridArray(p) => params_of(p),
            ModifierKind::Mirror(p) => params_of(p),
            ModifierKind::TransformEach(p) => params_of(p),
            ModifierKind::Randomize(p) => params_of(p),
            ModifierKind::Wallpaper(p) => params_of(p),
            ModifierKind::CopyToShape(p) => params_of(p),
            ModifierKind::Stencil(p) => params_of(p),
            ModifierKind::Resample(p) => params_of(p),
            ModifierKind::Trim(p) => params_of(p),
            ModifierKind::Tidy(p) => params_of(p),
            ModifierKind::Deform(p) => params_of(p),
            ModifierKind::Displace(p) => params_of(p),
            ModifierKind::Other { .. } => Map::new(),
        }
    }

    /// Read kind `kind` from its params `rest`: `(kind, extras)`. Unknown kinds and params that
    /// don't read become [`ModifierKind::Other`] with everything in the extras.
    fn read(kind: &str, rest: &Map<String, Value>) -> (ModifierKind, Map<String, Value>) {
        fn wrap<T>(r: Result<(T, Map<String, Value>), String>, f: fn(T) -> ModifierKind) -> Result<(ModifierKind, Map<String, Value>), String> {
            r.map(|(p, extra)| (f(p), extra))
        }
        let r = match kind {
            "linearArray" => wrap(read(rest), ModifierKind::LinearArray),
            "circularArray" => wrap(read(rest), ModifierKind::CircularArray),
            "gridArray" => wrap(read(rest), ModifierKind::GridArray),
            "mirror" => wrap(read(rest), ModifierKind::Mirror),
            "transformEach" => wrap(read(rest), ModifierKind::TransformEach),
            "randomize" => wrap(read(rest), ModifierKind::Randomize),
            "wallpaper" => wrap(read(rest), ModifierKind::Wallpaper),
            "copyToShape" => wrap(read(rest), ModifierKind::CopyToShape),
            "stencil" => wrap(read(rest), ModifierKind::Stencil),
            "resample" => wrap(read(rest), ModifierKind::Resample),
            "trim" => wrap(read(rest), ModifierKind::Trim),
            "tidy" => wrap(read(rest), ModifierKind::Tidy),
            "deform" => wrap(read(rest), ModifierKind::Deform),
            "displace" => wrap(read(rest), ModifierKind::Displace),
            _ => return (ModifierKind::Other { kind: kind.to_string(), error: None }, rest.clone()),
        };
        r.unwrap_or_else(|e| (ModifierKind::Other { kind: kind.to_string(), error: Some(e) }, rest.clone()))
    }
}

impl Stack {
    /// The stack's terminal stencil row: the first enabled `stencil` modifier, as `(modifier id,
    /// params as written)`. Rows below it are ignored when evaluating. [`crate::Output::stencil`]
    /// has the same row with its params clamped.
    pub fn stencil(&self) -> Option<(&str, &Stencil)> {
        self.modifiers.iter().filter(|m| m.enabled).find_map(|m| match &m.kind {
            ModifierKind::Stencil(p) => Some((m.id.as_str(), p)),
            _ => None,
        })
    }

    /// The targets of the enabled `copyToShape` modifiers, as `(modifier id, target)`, in stack
    /// order (empty targets left out).
    pub fn targets(&self) -> impl Iterator<Item = (&str, &str)> + '_ {
        self.modifiers.iter().filter(|m| m.enabled).filter_map(|m| match &m.kind {
            ModifierKind::CopyToShape(p) if !p.target.is_empty() => Some((m.id.as_str(), p.target.as_str())),
            _ => None,
        })
    }
}

impl Modifier {
    /// A new enabled modifier of kind id `kind` with default params, or `None` for a kind this
    /// build doesn't know. An empty `id` is replaced by [`Stack::insert`].
    pub fn new(id: impl Into<String>, kind: &str) -> Option<Modifier> {
        Some(Modifier { id: id.into(), enabled: true, kind: ModifierKind::default_for(kind)?, extra: Map::new() })
    }

    /// The label of this modifier's kind ("Linear array"; the kind id for unknown kinds).
    pub fn label(&self) -> &str {
        kind_label(self.kind.id()).unwrap_or_else(|| self.kind.id())
    }

    /// Read one modifier object. Missing `id` reads as empty, missing `enabled` as true; anything
    /// but an object is refused.
    pub fn from_value(v: &Value) -> Result<Modifier, ModError> {
        let Value::Object(o) = v else {
            return Err(ModError::Invalid(format!("a modifier must be an object, got {}", short(v))));
        };
        let id = match o.get("id") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Number(n)) => n.to_string(),
            _ => String::new(),
        };
        let enabled = !matches!(o.get("enabled"), Some(Value::Bool(false)));
        let kind = o.get("kind").and_then(Value::as_str).unwrap_or_default();
        let rest: Map<String, Value> = o.iter().filter(|(k, _)| !COMMON.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
        let (kind, extra) = ModifierKind::read(kind, &rest);
        Ok(Modifier { id, enabled, kind, extra })
    }

    /// The modifier as one flat JSON object: `id`, `enabled`, `kind`, its params and its extras.
    pub fn to_value(&self) -> Value {
        let mut o = self.extra.clone();
        o.extend(self.kind.params());
        o.insert("id".into(), Value::String(self.id.clone()));
        o.insert("enabled".into(), Value::Bool(self.enabled));
        o.insert("kind".into(), Value::String(self.kind.id().to_string()));
        Value::Object(o)
    }

    /// Change params from `props` (camelCase names, as written in the file; `enabled` too).
    /// Refused, leaving the modifier unchanged, when a name isn't one of the kind's params, a
    /// value has the wrong type or a number isn't finite, or the kind is unknown.
    ///
    /// A stencil's `material` preset fills its four limit fields unless `props` sets them too, and
    /// setting one of those fields (without `material`) off the current preset makes the material
    /// `custom` ([`StencilMaterial::limits`]), so each edit is one undo step.
    pub fn set_props(&mut self, props: &Map<String, Value>) -> Result<(), ModError> {
        if let ModifierKind::Other { kind, .. } = &self.kind {
            return Err(ModError::Invalid(format!("can't edit a `{kind}` modifier (unknown kind)")));
        }
        let mut params = self.kind.params();
        let mut enabled = self.enabled;
        for (k, v) in props {
            match k.as_str() {
                "enabled" => enabled = v.as_bool().ok_or_else(|| ModError::Invalid(format!("`enabled` must be true or false, got {}", short(v))))?,
                "id" | "kind" => return Err(ModError::Invalid(format!("`{k}` can't be changed"))),
                _ if expr_field(self.kind.id(), k).is_some() => set_expr(&mut params, k, v)?,
                _ => {
                    let Some(cur) = params.get(k) else {
                        let names: Vec<&str> = params.keys().map(String::as_str).chain(["enabled"]).collect();
                        return Err(ModError::Invalid(format!("`{}` has no param `{k}` (it takes {})", self.kind.id(), names.join(", "))));
                    };
                    // `null` passes through: optional numbers (`tieAngle`) take it, the rest refuse it.
                    let v = if cur.is_number() && !v.is_null() {
                        let f = lenient::finite(v).ok_or_else(|| ModError::Invalid(format!("`{k}` must be a finite number, got {}", short(v))))?;
                        serde_json::Number::from_f64(f).map(Value::Number).ok_or_else(|| ModError::Invalid(format!("`{k}` must be finite")))?
                    } else {
                        v.clone()
                    };
                    params.insert(k.clone(), v);
                }
            }
        }
        match ModifierKind::read(self.kind.id(), &params) {
            (ModifierKind::Other { error, .. }, _) => Err(ModError::Invalid(error.unwrap_or_else(|| "bad params".into()))),
            (mut kind, _) => {
                if let (ModifierKind::Stencil(old), ModifierKind::Stencil(new)) = (&self.kind, &mut kind) {
                    crate::kinds::stencil_apply_material(old, new, props);
                }
                self.kind = kind;
                self.enabled = enabled;
                Ok(())
            }
        }
    }
}

/// Set (or with `null`/an empty text, clear) expression param `key` in `params`: a text of at most
/// [`crate::expr::MAX_EXPR_LEN`] bytes that parses (a number is taken as its text).
fn set_expr(params: &mut Map<String, Value>, key: &str, v: &Value) -> Result<(), ModError> {
    let text = match v {
        Value::Null => String::new(),
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return Err(ModError::Invalid(format!("`{key}` must be an expression text such as \"i * 15\", got {}", short(v)))),
    };
    if text.is_empty() {
        params.remove(key);
        return Ok(());
    }
    crate::expr::check(&text).map_err(|e| ModError::Invalid(format!("`{key}`: {e}")))?;
    params.insert(key.to_string(), Value::String(text));
    Ok(())
}

/// A value for an error message, cut short.
pub(crate) fn short(v: &Value) -> String {
    let s = v.to_string();
    match s.char_indices().nth(40) {
        Some((i, _)) => format!("{}…", s.get(..i).unwrap_or_default()),
        None => s,
    }
}
