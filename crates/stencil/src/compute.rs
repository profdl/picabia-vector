//! The whole pipeline: [`compute`].

use crate::ops::BoolOp;
use crate::ops::split_subpaths;
use vectorcraft_geom::kurbo::{BezPath, Point, Rect, Shape};
use vectorcraft_modifiers::{Stencil, StencilLimitMode};

use std::collections::HashMap;

use vectorcraft_geom::kurbo::PathEl;
use vectorcraft_modifiers::{Repeat, StencilOutput, StencilTieMode};

use crate::clean::{Limits, clean};
use crate::hold::Held;
use crate::islands::Tree;
use crate::problems::{CheckLimits, MAX_ZONES, Problem, ProblemCounts, ProblemKind, Zones, check};
use crate::repeat::{Cell, Lattice, MAX_REPEAT_BARS, moved};
use crate::ties::Bar;
use crate::{ColourInfo, Paint, Selector, colour_infos, mm_per_unit, visible_regions};

/// The most paints [`compute`] takes; past it it makes an empty sheet with a warning.
pub const MAX_PAINTS: usize = 20_000;

/// The flattening tolerance used when `toleranceMm` is 0, mm.
const FINEST_MM: f64 = 0.005;

/// What the stencil cuts and leaves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    /// Separate cut-outs (outer contours of the cut, islands in paper holes included).
    pub cut_pieces: usize,
    /// Cut pieces smaller than `minCutAreaMm2` left as paper.
    pub dropped_specks: usize,
    /// Paper holes in the cut smaller than `minCutAreaMm2` cut away.
    pub filled_holes: usize,
    /// Length of every cut contour, mm (what the laser travels while cutting).
    pub cut_length_mm: f64,
    /// Area cut out, mm².
    pub area_cut_mm2: f64,
    /// Paper islands found (paper pieces not joined to the outer sheet before tying), nested ones
    /// included.
    pub islands: usize,
    /// Tie bars added.
    pub ties: usize,
    /// Islands held by the lattice (`pattern` mode), not counted in `ties`' islands.
    pub pattern_held: usize,
    /// Islands left without a tie (gauze mode, or no bar could be placed).
    pub untied: usize,
    /// What still breaks a limit, per kind (all of them; [`StencilOut::problems`] lists at most
    /// [`MAX_PROBLEMS`](crate::MAX_PROBLEMS)).
    pub problems_by_kind: ProblemCounts,
    /// Islands in one repeat cell when the islands are judged on the repeat (0 otherwise): the
    /// periodic design's paper pieces that don't reach the 3×3-cell block's outline, whose bounds
    /// centre lies in the cell.
    pub repeat_islands: usize,
}

/// The computed stencil, in the layer's units and space.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StencilOut {
    /// The cut-outs: plain polygonal contours (even-odd; non-zero reads the same).
    pub cut: BezPath,
    /// The sheet minus the cut (even-odd; non-zero reads the same).
    pub paper: BezPath,
    /// The sheet: the layer bounds grown by `marginMm`.
    pub sheet: Rect,
    /// Millimetres per layer unit (`widthMm / layer width`): the sheet is
    /// `sheet.width() · mm_per_unit` mm wide.
    pub mm_per_unit: f64,
    pub warnings: Vec<String>,
    pub stats: Stats,
    /// Every colour of the design, largest visible area first.
    pub colours: Vec<ColourInfo>,
    /// The tie bars and, in `pattern` mode, the lattice bars that ended up in the paper (non-zero;
    /// rectangles), already merged into `paper` and removed from `cut`; for the preview overlay.
    /// Empty in gauze mode.
    pub ties: BezPath,
    /// Every paper island found in the cut sheet, outermost first.
    pub islands: Vec<Island>,
    /// What still breaks a cutting limit in [`StencilOut::cut`], at most [`MAX_PROBLEMS`](crate::MAX_PROBLEMS) (taken in
    /// turn from each kind, then grouped by kind); [`Stats::problems_by_kind`] has the counts.
    pub problems: Vec<Problem>,
    /// The registration holes' centres (layer units), `hoshiDiameterMm` across, already in `cut`:
    /// in the margin at the design's corners (a tile's: at the cell's corners). Empty when `hoshi` is off or the margin is narrower
    /// than a hole.
    pub hoshi: Vec<Point>,
    /// `output: tile` with a repeat: the tile (also in `cut`, `paper` and `sheet`, which are then
    /// the tile's); the islands and their counts in [`Stats`] stay the whole sheet's.
    pub tile: Option<Tile>,
    /// The repeat cell the islands were judged in (a parallelogram, layer units), when repeat-aware.
    pub cell: Option<BezPath>,
}

/// One repeat cell cut on its own, for step-and-repeat printing (`output: tile`): its cut is the
/// periodic cut inside the cell, so stamping it at every lattice translate reproduces the design.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tile {
    /// The cell: a parallelogram (layer units) spanned by the lattice vectors, with registration
    /// holes on its corners (the lattice points).
    pub cell: BezPath,
    /// The cut inside the cell, plus the registration holes.
    pub cut: BezPath,
    /// The tile's sheet minus its cut.
    pub paper: BezPath,
    /// The cell's bounds grown by `marginMm`.
    pub sheet: Rect,
}

/// A paper island: a piece of paper the cut separates from the outer sheet (it would fall out
/// without ties).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Island {
    /// Its outer contour's bounds, layer units.
    pub bounds: Rect,
    /// Tie bars holding it (0 in gauze mode, when the lattice holds it, or when no bar could be
    /// placed).
    pub tied: u32,
    /// The `pattern` lattice holds it.
    pub pattern_held: bool,
    /// How deep it sits: 0 = in a cut-out of the sheet, 1 = in a cut-out of a depth-0 island, ...
    pub nested_depth: u32,
}

/// Why [`compute`] made nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StencilError {
    /// The layer bounds aren't finite or have no width, so there is no physical scale.
    BadBounds,
}

impl std::fmt::Display for StencilError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StencilError::BadBounds => write!(f, "the layer has no usable size for a stencil"),
        }
    }
}

impl std::error::Error for StencilError {}

/// Turn `paints` (in the layer's space) into a stencil sheet for `params` (clamped here; its
/// warnings are passed on). `layer_bounds` (layer units) sets the physical scale
/// ([`mm_per_unit`]) and, grown by the margin, the sheet.
///
/// Steps: [`visible_regions`] (curves flattened at `toleranceMm / 4`, every ring simplified to
/// `toleranceMm / 2`), the cut colours ([`Selector`]) merged into one region, clean-up (area
/// filter, opening by `minCutWidthMm / 2` then closing by `minBridgeMm / 2` as three offsets,
/// area filter again, simplify to `toleranceMm / 2`; with `limitMode: flag` only the
/// simplification), the cut clipped to the sheet, the islands found and held (ties, the pattern
/// lattice, or none in gauze mode), the registration holes ([`StencilOut::hoshi`]),
/// `paper = sheet − cut`, and the checks ([`Problem`]s).
///
/// More than [`MAX_PAINTS`] paints: an uncut sheet and a warning.
pub fn compute(paints: &[Paint], params: &Stencil, layer_bounds: Rect) -> Result<StencilOut, StencilError> {
    compute_repeat(paints, params, layer_bounds, None)
}

/// [`compute`] for a design that is a translation `repeat` (the lattice of a wallpaper or grid
/// row above the stencil, in the layer's space; `None`: [`compute`]).
///
/// With a usable lattice (not degenerate, at most 10 000 cells on the sheet)
/// the islands are judged on the periodic design and tied the same in every copy (see
/// [`Stats::repeat_islands`]; ties of the cell nearest the design's centre are repeated over the
/// sheet, then the usual hold ties what is left, such as pieces at the sheet's edge). `pattern`
/// mode isn't repeat-aware (a warning); gauze only counts the cell's islands. With
/// `output: tile` the drawn and exported cut is one repeat cell ([`StencilOut::tile`]); a tile
/// without a repeat is a sheet (the modifier evaluation warns about that).
pub fn compute_repeat(paints: &[Paint], params: &Stencil, layer_bounds: Rect, repeat: Option<Repeat>) -> Result<StencilOut, StencilError> {
    let (p, mut warnings) = params.clamped();
    let mpu = mm_per_unit(p.width_mm, layer_bounds).ok_or(StencilError::BadBounds)?;
    let margin = p.margin_mm / mpu;
    let sheet = layer_bounds.abs().inflate(margin, margin);
    if !sheet.is_finite() {
        return Err(StencilError::BadBounds);
    }
    if paints.len() > MAX_PAINTS {
        warnings.push(format!("the design has {} shapes; a stencil takes at most {MAX_PAINTS}", paints.len()));
        return Ok(StencilOut { paper: sheet.to_path(0.1), sheet, mm_per_unit: mpu, warnings, ..StencilOut::default() });
    }
    let lattice = match repeat {
        None => None,
        Some(_) if p.tie_mode == StencilTieMode::Pattern => {
            warnings.push("the pattern lattice isn't repeat-aware: the islands are judged on the sheet as drawn".into());
            None
        }
        Some(r) => Lattice::new(r, layer_bounds, margin).map_err(|w| warnings.push(w)).ok(),
    };
    let pre = prepare(paints, &p, mpu, sheet, None);
    warnings.extend(pre.warnings);
    let periodic = lattice.and_then(|lat| periodic(paints, &p, mpu, &lat, layer_bounds).map_err(|w| warnings.push(w)).ok().map(|per| (lat, per)));
    let held = match &periodic {
        Some((lat, per)) => hold_repeated(pre.cut, sheet, &p, mpu, lat, per),
        None => crate::hold::hold(pre.cut, sheet, &p, mpu),
    };
    warnings.extend(held.warnings);
    let limits = check_limits(&p, mpu);
    let tile = match (&periodic, p.output) {
        (Some((lat, per)), StencilOutput::Tile) => tile(lat, per, &p, mpu, margin, &mut warnings),
        _ => None,
    };
    let (cut, out_sheet, hoshi, ties, checked, tile) = match tile {
        Some(t) => {
            let untied = Tree::new(&t.tile.cut)
                .map(|tr| {
                    tr.islands()
                        .iter()
                        .filter_map(|&i| tr.bounds.get(i).copied())
                        .map(|bounds| Problem { kind: ProblemKind::Untied, bounds })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let mut changed: Vec<Rect> = edge_zones(&t.tile.cell, limits.min_bridge.max(limits.min_cut_width) + limits.slack);
            if let Zones::Near(z) = zones(&p, &limits, t.tile.sheet, &[&t.ties, &t.holes]) {
                changed.extend(z);
            }
            let z = if p.limit_mode == StencilLimitMode::Flag { Zones::Everywhere } else { Zones::near(changed) };
            let checked = check(&t.tile.cut, t.tile.sheet, &untied, &limits, &z);
            (t.tile.cut.clone(), t.tile.sheet, t.hoshi, t.ties, checked, Some(t.tile))
        }
        None => {
            let (cut, hoshi, holes) =
                crate::hoshi::register(&p, held.cut, crate::hoshi::sheet_points(layer_bounds, margin), margin, mpu, &mut warnings);
            let z = zones(&p, &limits, sheet, &[&tie_bars(&held.ties, &held.lattice), &holes]);
            let mut given: Vec<Problem> = held.untied_bounds.iter().map(|&bounds| Problem { kind: ProblemKind::Untied, bounds }).collect();
            if p.limit_mode == StencilLimitMode::Fix && p.pattern_width_mm < p.min_bridge_mm {
                given.extend(split_subpaths(&held.lattice).iter().map(|s| Problem { kind: ProblemKind::NarrowPaper, bounds: s.bounding_box() }));
            }
            let checked = check(&cut, sheet, &given, &limits, &z);
            (cut, sheet, hoshi, held.ties, checked, None)
        }
    };
    warnings.extend(checked.warnings);
    let mut stats = measure(&cut, mpu);
    stats.dropped_specks = pre.dropped_specks;
    stats.filled_holes = pre.filled_holes;
    stats.islands = held.islands.len();
    stats.ties = held.ties_placed;
    stats.pattern_held = held.pattern_held;
    stats.untied = held.untied;
    stats.repeat_islands = periodic.as_ref().map_or(0, |(_, per)| per.islands);
    stats.problems_by_kind = checked.counts;
    Ok(StencilOut {
        paper: paper(out_sheet, &cut),
        cut,
        sheet: out_sheet,
        mm_per_unit: mpu,
        warnings,
        stats,
        colours: pre.colours,
        ties,
        islands: held.islands,
        problems: checked.problems,
        hoshi,
        tile,
        cell: periodic.map(|(lat, per)| lat.cell_path(per.cell)),
    })
}

/// A hold's tie bars without the lattice (which `ties` starts with): the narrow checks look near these in fix mode (the
/// lattice's own thin bars are reported from the pattern's width; looking near all of them would
/// cost as much as looking everywhere).
fn tie_bars(ties: &BezPath, lattice: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    out.extend(ties.elements().iter().skip(lattice.elements().len()).copied());
    out
}

/// Bands `width` wide around each edge of the polygon `path` (as their bounds).
fn edge_zones(path: &BezPath, width: f64) -> Vec<Rect> {
    let mut out = Vec::new();
    let mut last: Option<Point> = None;
    let mut first: Option<Point> = None;
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                first = Some(p);
                last = Some(p);
            }
            PathEl::LineTo(p) => {
                if let Some(l) = last {
                    out.push(Rect::from_points(l, p).inflate(width, width));
                }
                last = Some(p);
            }
            PathEl::ClosePath => {
                if let (Some(l), Some(f)) = (last, first) {
                    out.push(Rect::from_points(l, f).inflate(width, width));
                }
                last = first;
            }
            PathEl::QuadTo(..) | PathEl::CurveTo(..) => {}
        }
    }
    out
}

/// The periodic design's block around the cell nearest the design's centre: its cut with the
/// cell's ties repeated over the block, the cell's tie bars, its island count.
struct Periodic {
    cell: Cell,
    cut: BezPath,
    bars: Vec<Bar>,
    islands: usize,
}

/// Build the block (see [`crate::repeat`]) and tie it. `Err` with a warning when it can't.
fn periodic(paints: &[Paint], p: &Stencil, mpu: f64, lat: &Lattice, design: Rect) -> Result<Periodic, String> {
    let c = lat.cell_at(design.abs().center());
    let shapes = crate::repeat::block_paints(paints, lat, c)?;
    let block = lat.block_path(c);
    let bb = block.bounding_box();
    let pre = prepare(&shapes, p, mpu, bb, Some(&block));
    let mut tree = Tree::new(&pre.cut)
        .ok_or_else(|| "the repeat is too detailed to look for islands across its edges; the stencil ignores the repeat".to_string())?;
    let islands = tree.islands();
    let in_cell: Vec<bool> = islands.iter().map(|&i| tree.bounds.get(i).is_some_and(|b| lat.in_cell(c, b.center()))).collect();
    let n = in_cell.iter().filter(|&&x| x).count();
    let mut bars = Vec::new();
    if p.tie_mode == StencilTieMode::Ties && n > 0 {
        let tp = crate::hold::tie_params(p, mpu, bb);
        let (all, _, owners) = crate::ties::tie_owned(&mut tree, &islands, &tp);
        bars = all.into_iter().zip(owners).filter(|(_, o)| in_cell.get(*o).copied().unwrap_or(false)).map(|(b, _)| b).collect();
    }
    let mut cut = pre.cut;
    if !bars.is_empty() {
        let around = around(lat, &bars);
        cut = crate::hold::subtract_bars(&cut, &around).map_err(|_| "couldn't tie the repeat's cell; the stencil ignores the repeat".to_string())?;
    }
    Ok(Periodic { cell: c, cut, bars, islands: n })
}

/// `bars` and their translates into the 8 (a strip's 2) neighbouring cells.
fn around(lat: &Lattice, bars: &[Bar]) -> Vec<Bar> {
    let ls: &[i64] = if lat.two_d { &[-1, 0, 1] } else { &[0] };
    let mut out = Vec::with_capacity(bars.len() * 3 * ls.len());
    for k in -1..=1 {
        for &l in ls {
            let v = lat.shift(k, l);
            out.extend(bars.iter().map(|b| moved(b, v)));
        }
    }
    out
}

/// Hold the sheet's `cut` with the periodic ties: the cell's bars repeated over every lattice
/// translate that meets the sheet, kept where a translate lands on an island's outline and on
/// other paper (a copy at the sheet's edge may differ), then the usual hold for what still floats.
/// Islands, counts and ties are the sheet's.
fn hold_repeated(cut: BezPath, sheet: Rect, p: &Stencil, mpu: f64, lat: &Lattice, per: &Periodic) -> Held {
    let plain = |cut: BezPath| crate::hold::hold(cut, sheet, p, mpu);
    if per.bars.is_empty() {
        return plain(cut);
    }
    let Some(mut tree) = Tree::new(&cut) else { return plain(cut) };
    let originals = tree.islands();
    if originals.is_empty() {
        return plain(cut);
    }
    let Some(bb) = per.bars.iter().map(crate::repeat::bar_bounds).reduce(|a, b| a.union(b)) else { return plain(cut) };
    let shifts = lat.shifts(lat.range(bb), lat.range(sheet));
    let mut extra = Vec::new();
    if shifts.len().saturating_mul(per.bars.len()) > MAX_REPEAT_BARS {
        extra.push(format!("the repeat needs too many ties (over {MAX_REPEAT_BARS}); they are placed on the sheet as drawn"));
        let mut held = plain(cut);
        held.warnings.extend(extra);
        return held;
    }
    let odd = |t: &Tree, r: u32| t.depth.get(r as usize).is_some_and(|d| d % 2 == 1);
    let mut count = vec![0u32; tree.rings.len()];
    let mut kept = Vec::new();
    for (k, l) in shifts {
        let v = lat.shift(k, l);
        for bar in &per.bars {
            let q = moved(bar, v);
            if !q.iter().all(|&pt| sheet.contains(pt)) {
                continue;
            }
            let hit = tree.index.rings_hit(&q);
            if hit.len() < 2 || !hit.iter().any(|&r| odd(&tree, r)) {
                continue;
            }
            for &r in hit.iter().filter(|&&r| odd(&tree, r)) {
                if let Some(c) = count.get_mut(r as usize) {
                    *c += 1;
                }
            }
            kept.push(q);
        }
    }
    let cut_r = match crate::hold::subtract_bars(&cut, &kept) {
        Ok(c) => c,
        Err(_) => {
            let mut held = plain(cut);
            held.warnings.push("couldn't merge the repeat's ties into the paper; they are placed on the sheet as drawn".into());
            return held;
        }
    };
    let mut held = plain(cut_r);
    let key = |b: Rect| [b.x0, b.y0, b.x1, b.y1].map(f64::to_bits);
    let fallback: HashMap<[u64; 4], u32> = held.islands.iter().map(|i| (key(i.bounds), i.tied)).collect();
    held.islands = originals
        .iter()
        .map(|&i| {
            let bounds = tree.bounds.get(i).copied().unwrap_or(Rect::ZERO);
            let tied = count.get(i).copied().unwrap_or(0) + fallback.get(&key(bounds)).copied().unwrap_or(0);
            Island { bounds, tied, pattern_held: false, nested_depth: tree.nested_depth(i) }
        })
        .collect();
    let mut ties = crate::hold::bars_path(&kept);
    ties.extend(held.ties.iter());
    held.ties = ties;
    held.ties_placed += kept.len();
    held
}

/// A tile and what goes with it.
struct TileOut {
    tile: Tile,
    hoshi: Vec<Point>,
    /// Tie bars inside the cell.
    ties: BezPath,
    holes: BezPath,
}

/// The tile of the periodic block's cell: its cut ∩ the cell, registration holes on the cell's
/// corners, the cell's bounds plus the margin as its sheet. `None` with a warning when the clip
/// fails.
fn tile(lat: &Lattice, per: &Periodic, p: &Stencil, mpu: f64, margin: f64, warnings: &mut Vec<String>) -> Option<TileOut> {
    let cell = lat.cell_path(per.cell);
    let Ok(cut) = crate::ops::boolean_step(&per.cut, &cell, BoolOp::Intersect) else {
        warnings.push("couldn't cut the repeat tile; the whole sheet is cut instead".into());
        return None;
    };
    let ties = if per.bars.is_empty() {
        BezPath::new()
    } else {
        crate::ops::boolean_step(&crate::hold::bars_path(&around(lat, &per.bars)), &cell, BoolOp::Intersect).unwrap_or_default()
    };
    let o = lat.cell_origin(per.cell);
    let corners = vec![o, o + lat.a, o + lat.a + lat.b, o + lat.b];
    let (cut, hoshi, holes) = crate::hoshi::register(p, cut, corners, f64::INFINITY, mpu, warnings);
    let sheet = cell.bounding_box().inflate(margin, margin);
    Some(TileOut { tile: Tile { paper: paper(sheet, &cut), cut, sheet, cell }, hoshi, ties, holes })
}

/// The checks' limits for the clamped params `p` at `mpu` mm per unit.
pub(crate) fn check_limits(p: &Stencil, mpu: f64) -> CheckLimits {
    CheckLimits {
        min_bridge: p.min_bridge_mm / mpu,
        min_cut_width: p.min_cut_width_mm / mpu,
        min_area: p.min_cut_area_mm2 / (mpu * mpu),
        min_corner_angle: p.min_corner_angle,
        slack: p.tolerance_mm / mpu / 2.0,
    }
}

/// Where the narrow checks look: everywhere in flag mode; in fix mode (the clean-up removed every
/// narrow part) only near what changed the cut after it: the bars in `changed` (one zone per
/// subpath) and a band along the sheet's edge (the clip, a thin margin).
pub(crate) fn zones(p: &Stencil, l: &CheckLimits, sheet: Rect, changed: &[&BezPath]) -> Zones {
    if p.limit_mode == StencilLimitMode::Flag {
        return Zones::Everywhere;
    }
    let mut rects: Vec<Rect> = Zones::frame(sheet, l.min_bridge.max(l.min_cut_width) + l.slack).to_vec();
    for path in changed {
        if path.elements().len() > 8 * MAX_ZONES {
            return Zones::Everywhere;
        }
        rects.extend(split_subpaths(path).iter().map(|s| s.bounding_box()));
    }
    Zones::near(rects)
}

/// The cleaned cut of a design, before islands are held.
#[derive(Debug, Default)]
pub(crate) struct Prepared {
    pub cut: BezPath,
    pub colours: Vec<ColourInfo>,
    pub dropped_specks: usize,
    pub filled_holes: usize,
    pub warnings: Vec<String>,
}

/// Visible regions, colour selection, clean-up and the clip to `sheet` for the clamped params
/// `p` at `mpu` mm per unit (stages 1–4), clipped to `clip` when given (a repeat's block; `sheet`
/// is then its bounds). Nothing cut is an empty `cut` with a warning.
pub(crate) fn prepare(paints: &[Paint], p: &Stencil, mpu: f64, sheet: Rect, clip: Option<&BezPath>) -> Prepared {
    let units = |mm: f64| mm / mpu;
    let mut out = Prepared::default();
    let tolerance = units(p.tolerance_mm);
    let flatten = units(if p.tolerance_mm > 0.0 { p.tolerance_mm / 4.0 } else { FINEST_MM });
    let visible = visible_regions(paints, flatten, tolerance / 2.0);
    out.warnings.extend(visible.warnings);
    let selector = Selector::new(p);
    out.colours = colour_infos(&visible.regions, &selector, mpu);
    let mut chosen = BezPath::new();
    for r in visible.regions.iter().filter(|r| selector.cuts(r.colour)) {
        chosen.extend(r.path.iter());
    }
    if chosen.elements().is_empty() {
        if !visible.regions.is_empty() {
            out.warnings.push("nothing is cut: no visible colour is chosen for cutting".into());
        }
        return out;
    }
    // The visible pieces are disjoint but touch: one sweep merges them into plain contours.
    let Ok(merged) = crate::ops::simplify(&chosen) else {
        out.warnings.push("couldn't merge the cut colours; nothing is cut".into());
        return out;
    };
    let limits = Limits {
        min_area: p.min_cut_area_mm2 / (mpu * mpu),
        open_radius: units(p.min_cut_width_mm) / 2.0,
        close_radius: units(p.min_bridge_mm) / 2.0,
        tolerance: tolerance / 2.0,
        fix: p.limit_mode == StencilLimitMode::Fix,
    };
    let cleaned = clean(merged, limits);
    out.warnings.extend(cleaned.warnings);
    out.dropped_specks = cleaned.dropped_specks;
    out.filled_holes = cleaned.filled_holes;
    let mut cut = cleaned.cut;
    if !cut.elements().is_empty() && (clip.is_some() || !contains(sheet, cut.bounding_box())) {
        let sheet_path = sheet.to_path(0.1);
        match crate::ops::boolean_step(&cut, clip.unwrap_or(&sheet_path), BoolOp::Intersect) {
            Ok(c) => cut = c,
            Err(_) => out.warnings.push("couldn't clip the cut to the sheet".into()),
        }
    }
    if cut.elements().is_empty() {
        out.warnings.push("nothing is left to cut after clean-up".into());
    }
    out.cut = cut;
    out
}

fn contains(outer: Rect, inner: Rect) -> bool {
    inner.x0 >= outer.x0 && inner.y0 >= outer.y0 && inner.x1 <= outer.x1 && inner.y1 <= outer.y1
}

/// The sheet with the cut's contours reversed inside it: sheet − cut, with no sweep (the cut is
/// plain contours within the sheet), winding 1 on paper and 0 in the cut-outs.
pub(crate) fn paper(sheet: Rect, cut: &BezPath) -> BezPath {
    let mut out = sheet.to_path(0.1);
    if out.area() < 0.0 {
        out = out.reverse_subpaths();
    }
    out.extend(cut.reverse_subpaths().iter());
    out
}

/// Pieces, length and area of the (plain-contour) cut at `mpu` mm per unit.
fn measure(cut: &BezPath, mpu: f64) -> Stats {
    let subs = split_subpaths(cut);
    Stats {
        cut_pieces: subs.iter().filter(|s| s.area() > 0.0).count(),
        cut_length_mm: subs.iter().map(|s| s.perimeter(1e-6)).sum::<f64>() * mpu,
        area_cut_mm2: cut.area().max(0.0) * mpu * mpu,
        ..Stats::default()
    }
}
