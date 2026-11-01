//! Each kind's number fields and their ranges: what a binding may drive ([`crate::bind`]) and the
//! range its result is clamped to. The Appearance panel and the stencil settings show the same
//! ranges (a ui-egui test holds them together).

/// One number field of a kind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NumberField {
    /// The param, camelCase.
    pub key: &'static str,
    pub min: f64,
    pub max: f64,
    /// Rounded to a whole number.
    pub whole: bool,
}

const fn f(key: &'static str, min: f64, max: f64) -> NumberField {
    NumberField { key, min, max, whole: false }
}

const fn w(key: &'static str, min: f64, max: f64) -> NumberField {
    NumberField { key, min, max, whole: true }
}

const BIG: f64 = crate::MAX_LENGTH;
const COUNT: f64 = crate::MAX_COUNT as f64;
const SEED_MAX: f64 = 4_294_967_295.0;

const LINEAR: &[NumberField] =
    &[w("count", 1.0, COUNT), f("offsetX", -BIG, BIG), f("offsetY", -BIG, BIG), f("rotateEach", -BIG, BIG), f("scaleEach", 0.0, 100.0)];
const CIRCULAR: &[NumberField] = &[
    w("count", 1.0, COUNT),
    f("radius", 0.0, BIG),
    f("startAngle", -BIG, BIG),
    f("sweep", -360.0, 360.0),
    f("centerX", -BIG, BIG),
    f("centerY", -BIG, BIG),
];
const GRID: &[NumberField] = &[w("rows", 1.0, COUNT), w("cols", 1.0, COUNT), f("gapX", -BIG, BIG), f("gapY", -BIG, BIG)];
const MIRROR: &[NumberField] = &[f("offset", -BIG, BIG)];
const WALLPAPER: &[NumberField] = &[
    w("cols", 1.0, 100.0),
    w("rows", 1.0, 100.0),
    f("cellWidth", 0.001, BIG),
    f("cellHeight", 0.001, BIG),
    f("angle", 1.0, 179.0),
    f("originX", -BIG, BIG),
    f("originY", -BIG, BIG),
];
const COPY_TO_SHAPE: &[NumberField] = &[w("count", 1.0, COUNT), f("scale", 0.0, 100.0), w("seed", 0.0, SEED_MAX)];
const TRANSFORM_EACH: &[NumberField] = &[
    f("offsetX", -BIG, BIG),
    f("offsetY", -BIG, BIG),
    f("rotate", -BIG, BIG),
    f("scaleX", 0.0, 100.0),
    f("scaleY", 0.0, 100.0),
    f("jitter", 0.0, 10.0),
    w("jitterSeed", 0.0, SEED_MAX),
];
const RANDOMIZE: &[NumberField] = &[f("position", 0.0, BIG), f("rotation", 0.0, BIG), f("scale", 0.0, 100.0), w("seed", 0.0, SEED_MAX)];
const RESAMPLE: &[NumberField] = &[w("iterations", 1.0, 5.0), w("count", 2.0, 1024.0), f("spacing", 1.0, 5000.0)];
const TRIM: &[NumberField] = &[f("start", 0.0, 1.0), f("end", 0.0, 1.0), f("offset", 0.0, 1.0)];
const TIDY: &[NumberField] = &[f("tolerance", 0.0, 500.0), w("seed", 0.0, 999_999.0)];
const DEFORM: &[NumberField] = &[
    f("startScale", 0.1, 3.0),
    f("endScale", 0.1, 3.0),
    f("angle", -720.0, 720.0),
    f("bendAngle", -360.0, 360.0),
    f("bendPoint", 0.0, 1.0),
    f("shearX", -2.0, 2.0),
    f("shearY", -2.0, 2.0),
    f("strength", -3.0, 3.0),
    f("radius", 0.01, 2.0),
    f("progressiveAmount", 0.0, 1.0),
];
const DISPLACE: &[NumberField] = &[
    f("amplitude", 0.0, 100.0),
    f("scale", 0.01, 1.0),
    w("octaves", 1.0, 6.0),
    w("rippleCount", 2.0, 12.0),
    w("seed", 0.0, 999_999.0),
    f("progressiveAmount", 0.0, 1.0),
];
/// The stencil settings' number fields (`tieAngle`, optional, can't be bound).
const STENCIL: &[NumberField] = &[
    f("threshold", 0.0, 1.0),
    f("widthMm", 1.0, 10_000.0),
    f("marginMm", 0.0, 1000.0),
    f("minCutAreaMm2", 0.0, 10_000.0),
    f("minCutWidthMm", 0.0, 100.0),
    f("minBridgeMm", 0.0, 100.0),
    f("toleranceMm", 0.0, 10.0),
    f("minCornerAngle", 0.0, 179.0),
    f("patternWidthMm", 0.05, 50.0),
    f("patternSpacingMm", 0.5, 500.0),
    f("patternAngle", 0.0, 360.0),
    f("tieWidthMm", 0.05, 50.0),
    w("tiesPerIsland", 1.0, 8.0),
    w("tieSeed", 0.0, 1.0e9),
    f("hoshiDiameterMm", 0.2, 20.0),
];

/// The number fields of kind id `kind`, in panel order (empty for kinds without any).
pub fn number_fields(kind: &str) -> &'static [NumberField] {
    match kind {
        "linearArray" => LINEAR,
        "circularArray" => CIRCULAR,
        "gridArray" => GRID,
        "mirror" => MIRROR,
        "wallpaper" => WALLPAPER,
        "copyToShape" => COPY_TO_SHAPE,
        "transformEach" => TRANSFORM_EACH,
        "randomize" => RANDOMIZE,
        "resample" => RESAMPLE,
        "trim" => TRIM,
        "tidy" => TIDY,
        "deform" => DEFORM,
        "displace" => DISPLACE,
        "stencil" => STENCIL,
        _ => &[],
    }
}

/// Number field `key` of kind id `kind`.
pub fn number_field(kind: &str, key: &str) -> Option<NumberField> {
    number_fields(kind).iter().find(|f| f.key == key).copied()
}

/// Whether field `key` of kind id `kind` is read per copy: an expression binding it that reads
/// `i` or `n` gives each copy its own value. Only `transformEach`'s and `randomize`'s amounts are
/// (they run on every final copy); every other field takes one value per evaluation.
pub fn per_copy(kind: &str, key: &str) -> bool {
    crate::expr_fields(kind).contains(&key)
}

impl NumberField {
    /// `v` within the field's range (rounded when whole); `None` when `v` isn't finite.
    pub fn clamp(&self, v: f64) -> Option<f64> {
        if !v.is_finite() {
            return None;
        }
        let v = v.clamp(self.min, self.max);
        Some(if self.whole { v.round() } else { v })
    }
}
