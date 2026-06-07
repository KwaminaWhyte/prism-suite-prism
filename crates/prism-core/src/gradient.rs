//! Multi-stop gradients (color stops + opacity stops), the five Photoshop
//! gradient geometries, and ordered dithering to suppress banding.
//!
//! App-agnostic and GPU-agnostic: this is pure math over interleaved RGBA
//! buffers, reusable by any compositing app in the suite (Pigment's gradient
//! tool/fill, Contour's gradient meshes, Pulse's ramp generators).
//!
//! ## Color space
//! Stop colors are **straight (non-premultiplied)** values in the caller's
//! *working* space — the suite works in linear light, so pass linear RGB. Color
//! and opacity stops are independent lists (Photoshop's two-rail gradient
//! editor): a color stop carries RGB, an opacity stop carries alpha, each keyed
//! by a position in `0..=1`. They are interpolated independently and combined.
//!
//! [`Gradient::sample`] returns a straight RGBA color. [`Gradient::render`]
//! rasterizes the gradient over a buffer and returns **premultiplied** RGBA
//! (matching `shape.rs`), so the result drops straight into the linear-premul
//! pixel pipeline.
//!
//! ## Geometry
//! A gradient is parameterized by a drag from `start` to `end` (pixel coords).
//! Each [`GradientType`] maps a pixel to a scalar `t` differently (see
//! [`GradientType::param`]); `t` is then looked up in the stops. `start→end`
//! defines the axis/radius/centre/angle so a single drag drives every type.
//!
//! ## Dithering
//! 8-bit (and even f16) ramps band visibly. [`Gradient::render`] applies a
//! deterministic ordered dither: a per-pixel offset from a Bayer matrix nudges
//! the lookup position by a sub-stop amount, breaking up the flat steps without
//! noise. It is fully reproducible (no RNG) and toggleable.

use serde::{Deserialize, Serialize};

/// One color stop: RGB at a position along the gradient.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorStop {
    /// Position along the gradient, `0..=1`.
    pub pos: f32,
    /// Straight RGB in the working space.
    pub color: [f32; 3],
}

impl ColorStop {
    pub fn new(pos: f32, color: [f32; 3]) -> Self {
        Self { pos, color }
    }
}

/// One opacity stop: alpha at a position along the gradient (independent of the
/// color rail, à la Photoshop).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpacityStop {
    /// Position along the gradient, `0..=1`.
    pub pos: f32,
    /// Alpha, `0..=1`.
    pub alpha: f32,
}

impl OpacityStop {
    pub fn new(pos: f32, alpha: f32) -> Self {
        Self { pos, alpha }
    }
}

/// The five Photoshop gradient geometries. Each maps a pixel to the scalar `t`
/// fed into the stop lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GradientType {
    /// Projection onto the `start→end` axis, clamped to `0..1`.
    Linear,
    /// Distance from `start`, normalized by the `start→end` length (a disc).
    Radial,
    /// Angle around `start` measured from the `start→end` direction, `0..1`.
    Angle,
    /// Like linear but mirrored about `start` — `|projection|`, so the ramp
    /// reflects back past the centre.
    Reflected,
    /// Chebyshev (max of |u|, |v|) distance in the `start→end` frame — concentric
    /// diamonds.
    Diamond,
}

impl GradientType {
    /// Stable discriminant for serialization / a GPU `kind` uniform if needed.
    pub fn id(self) -> u32 {
        match self {
            GradientType::Linear => 0,
            GradientType::Radial => 1,
            GradientType::Angle => 2,
            GradientType::Reflected => 3,
            GradientType::Diamond => 4,
        }
    }

    /// Inverse of [`id`](Self::id); unknown ids fall back to `Linear`.
    pub fn from_id(id: u32) -> Self {
        match id {
            1 => GradientType::Radial,
            2 => GradientType::Angle,
            3 => GradientType::Reflected,
            4 => GradientType::Diamond,
            _ => GradientType::Linear,
        }
    }

    /// Map a pixel `(px, py)` to the gradient parameter `t in 0..=1` given the
    /// drag `start→end` (pixel coords). The mapping is the only thing that
    /// differs between geometries; everything else (stops, dither) is shared.
    pub fn param(self, px: f32, py: f32, start: (f32, f32), end: (f32, f32)) -> f32 {
        let (ax, ay) = start;
        let dx = end.0 - ax;
        let dy = end.1 - ay;
        let len2 = dx * dx + dy * dy;
        let qx = px - ax;
        let qy = py - ay;
        let t = match self {
            GradientType::Linear => {
                if len2 <= 0.0 {
                    0.0
                } else {
                    (qx * dx + qy * dy) / len2
                }
            }
            GradientType::Reflected => {
                if len2 <= 0.0 {
                    0.0
                } else {
                    ((qx * dx + qy * dy) / len2).abs()
                }
            }
            GradientType::Radial => {
                if len2 <= 0.0 {
                    0.0
                } else {
                    ((qx * qx + qy * qy) / len2).sqrt()
                }
            }
            GradientType::Angle => {
                // Angle of the pixel relative to the axis direction, wrapped to
                // 0..1 going counter-clockwise (atan2 with y down → negate).
                let a = ((-qy).atan2(qx) - (-dy).atan2(dx)).rem_euclid(std::f32::consts::TAU);
                a / std::f32::consts::TAU
            }
            GradientType::Diamond => {
                if len2 <= 0.0 {
                    0.0
                } else {
                    // Project into the (axis, perpendicular) frame, both
                    // normalized by the axis length; Chebyshev distance.
                    let inv = 1.0 / len2.sqrt();
                    let ux = dx * inv;
                    let uy = dy * inv;
                    let along = (qx * ux + qy * uy) * inv;
                    let perp = (qx * (-uy) + qy * ux) * inv;
                    along.abs().max(perp.abs())
                }
            }
        };
        t.clamp(0.0, 1.0)
    }
}

/// A multi-stop gradient: a color rail, an opacity rail, a geometry, and a
/// dither toggle. Stops need not be pre-sorted — lookups sort internally.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gradient {
    pub color_stops: Vec<ColorStop>,
    pub opacity_stops: Vec<OpacityStop>,
    pub kind: GradientType,
    /// Apply ordered dithering on [`render`](Self::render) to kill banding.
    pub dither: bool,
}

impl Default for Gradient {
    /// Black→white, fully opaque, linear, dithered — Photoshop's default ramp.
    fn default() -> Self {
        Self {
            color_stops: vec![
                ColorStop::new(0.0, [0.0, 0.0, 0.0]),
                ColorStop::new(1.0, [1.0, 1.0, 1.0]),
            ],
            opacity_stops: vec![OpacityStop::new(0.0, 1.0), OpacityStop::new(1.0, 1.0)],
            kind: GradientType::Linear,
            dither: true,
        }
    }
}

/// Generic 2-point lerp helper used by both rails.
#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

impl Gradient {
    /// A two-color, fully-opaque gradient of the given geometry — the common
    /// "foreground→background" case.
    pub fn two_color(c0: [f32; 3], c1: [f32; 3], kind: GradientType) -> Self {
        Self {
            color_stops: vec![ColorStop::new(0.0, c0), ColorStop::new(1.0, c1)],
            opacity_stops: vec![OpacityStop::new(0.0, 1.0), OpacityStop::new(1.0, 1.0)],
            kind,
            dither: true,
        }
    }

    /// A foreground→transparent gradient (opacity ramps 1→0), matching the old
    /// gradient-tool behavior.
    pub fn foreground_to_transparent(rgb: [f32; 3], kind: GradientType) -> Self {
        Self {
            color_stops: vec![ColorStop::new(0.0, rgb), ColorStop::new(1.0, rgb)],
            opacity_stops: vec![OpacityStop::new(0.0, 1.0), OpacityStop::new(1.0, 0.0)],
            kind,
            dither: true,
        }
    }

    /// Look up the straight RGB color at parameter `t` (clamped to the stop
    /// range; the rail is treated as constant outside its first/last stop).
    pub fn color_at(&self, t: f32) -> [f32; 3] {
        sample_rail(
            &self.color_stops,
            t,
            |s| s.pos,
            |a, b, f| {
                [
                    lerp(a.color[0], b.color[0], f),
                    lerp(a.color[1], b.color[1], f),
                    lerp(a.color[2], b.color[2], f),
                ]
            },
            |s| s.color,
            [0.0, 0.0, 0.0],
        )
    }

    /// Look up the alpha at parameter `t` (clamped to the stop range).
    pub fn alpha_at(&self, t: f32) -> f32 {
        sample_rail(
            &self.opacity_stops,
            t,
            |s| s.pos,
            |a, b, f| lerp(a.alpha, b.alpha, f),
            |s| s.alpha,
            1.0,
        )
    }

    /// Straight RGBA at parameter `t`: color rail + opacity rail combined.
    pub fn sample(&self, t: f32) -> [f32; 4] {
        let c = self.color_at(t);
        let a = self.alpha_at(t);
        [c[0], c[1], c[2], a]
    }

    /// Rasterize the gradient over a `width × height` buffer driven by a
    /// `start→end` drag (pixel coords). Returns interleaved **premultiplied**
    /// linear RGBA f32 of length `width*height*4`, matching `shape.rs`.
    ///
    /// When [`dither`](Self::dither) is set, a deterministic ordered (Bayer 8×8)
    /// dither perturbs each pixel's lookup position by up to one quantization
    /// step, breaking 8-bit banding without introducing noise (no RNG; identical
    /// output for identical inputs).
    pub fn render(&self, start: (f32, f32), end: (f32, f32), width: u32, height: u32) -> Vec<f32> {
        let mut out = vec![0.0f32; (width as usize) * (height as usize) * 4];
        if width == 0 || height == 0 {
            return out;
        }
        // One quantization step at 8-bit; the dither amplitude. Scaled small so
        // adjacent steps overlap by ~1 level without smearing the ramp.
        let amp = if self.dither { 1.0 / 255.0 } else { 0.0 };
        for y in 0..height {
            let py = y as f32 + 0.5;
            for x in 0..width {
                let px = x as f32 + 0.5;
                let mut t = self.kind.param(px, py, start, end);
                if amp > 0.0 {
                    // Bayer offset in (-0.5, 0.5], times the step amplitude.
                    t = (t + (bayer8(x, y) - 0.5) * amp).clamp(0.0, 1.0);
                }
                let s = self.sample(t);
                let a = s[3];
                let i = ((y * width + x) as usize) * 4;
                out[i] = s[0] * a;
                out[i + 1] = s[1] * a;
                out[i + 2] = s[2] * a;
                out[i + 3] = a;
            }
        }
        out
    }
}

/// Sample a rail of stops at `t`: sort by position, clamp outside the range to
/// the end stops, otherwise linearly interpolate the bracketing pair.
fn sample_rail<S: Copy, V>(
    stops: &[S],
    t: f32,
    pos: impl Fn(&S) -> f32,
    interp: impl Fn(&S, &S, f32) -> V,
    value: impl Fn(&S) -> V,
    default: V,
) -> V {
    if stops.is_empty() {
        return default;
    }
    if stops.len() == 1 {
        return value(&stops[0]);
    }
    // Sort indices by position (stops are usually few; this is cheap).
    let mut idx: Vec<usize> = (0..stops.len()).collect();
    idx.sort_by(|&a, &b| {
        pos(&stops[a])
            .partial_cmp(&pos(&stops[b]))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let first = stops[idx[0]];
    let last = stops[idx[idx.len() - 1]];
    if t <= pos(&first) {
        return value(&first);
    }
    if t >= pos(&last) {
        return value(&last);
    }
    for w in idx.windows(2) {
        let a = stops[w[0]];
        let b = stops[w[1]];
        let (pa, pb) = (pos(&a), pos(&b));
        if t >= pa && t <= pb {
            let span = pb - pa;
            let f = if span > 1e-9 { (t - pa) / span } else { 0.0 };
            return interp(&a, &b, f);
        }
    }
    value(&last)
}

/// Normalized Bayer 8×8 ordered-dither value in `[0, 1)` for pixel `(x, y)`.
/// Deterministic; the classic recursive matrix.
fn bayer8(x: u32, y: u32) -> f32 {
    // Standard 8×8 Bayer matrix (values 0..63).
    const M: [[u8; 8]; 8] = [
        [0, 32, 8, 40, 2, 34, 10, 42],
        [48, 16, 56, 24, 50, 18, 58, 26],
        [12, 44, 4, 36, 14, 46, 6, 38],
        [60, 28, 52, 20, 62, 30, 54, 22],
        [3, 35, 11, 43, 1, 33, 9, 41],
        [51, 19, 59, 27, 49, 17, 57, 25],
        [15, 47, 7, 39, 13, 45, 5, 37],
        [63, 31, 55, 23, 61, 29, 53, 21],
    ];
    let v = M[(y & 7) as usize][(x & 7) as usize];
    (v as f32 + 0.5) / 64.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    // ---- stop interpolation -------------------------------------------------

    #[test]
    fn color_rail_interpolates_in_working_space() {
        // Black→white: midpoint is exactly 0.5 in the working (linear) space —
        // no gamma applied here, the caller owns the color space.
        let g = Gradient::two_color([0.0; 3], [1.0; 3], GradientType::Linear);
        assert!(approx(g.color_at(0.0)[0], 0.0));
        assert!(approx(g.color_at(0.5)[0], 0.5));
        assert!(approx(g.color_at(1.0)[0], 1.0));
        // Quarter point is a clean linear blend.
        assert!(approx(g.color_at(0.25)[0], 0.25));
    }

    #[test]
    fn multi_stop_color_brackets_correctly() {
        // 0:red, 0.5:green, 1:blue. Each segment interpolates independently.
        let g = Gradient {
            color_stops: vec![
                ColorStop::new(0.0, [1.0, 0.0, 0.0]),
                ColorStop::new(0.5, [0.0, 1.0, 0.0]),
                ColorStop::new(1.0, [0.0, 0.0, 1.0]),
            ],
            opacity_stops: vec![OpacityStop::new(0.0, 1.0)],
            kind: GradientType::Linear,
            dither: false,
        };
        // 0.25 sits halfway between red and green.
        let c = g.color_at(0.25);
        assert!(approx(c[0], 0.5) && approx(c[1], 0.5) && approx(c[2], 0.0));
        // 0.75 sits halfway between green and blue.
        let c = g.color_at(0.75);
        assert!(approx(c[0], 0.0) && approx(c[1], 0.5) && approx(c[2], 0.5));
        // Exactly on the middle stop returns that stop's color.
        let c = g.color_at(0.5);
        assert!(approx(c[1], 1.0));
    }

    #[test]
    fn unsorted_stops_are_handled() {
        // Same gradient as above but stops given out of order.
        let g = Gradient {
            color_stops: vec![
                ColorStop::new(1.0, [0.0, 0.0, 1.0]),
                ColorStop::new(0.0, [1.0, 0.0, 0.0]),
                ColorStop::new(0.5, [0.0, 1.0, 0.0]),
            ],
            opacity_stops: vec![OpacityStop::new(0.0, 1.0)],
            kind: GradientType::Linear,
            dither: false,
        };
        let c = g.color_at(0.25);
        assert!(approx(c[0], 0.5) && approx(c[1], 0.5));
    }

    #[test]
    fn out_of_range_clamps_to_end_stops() {
        let g = Gradient {
            color_stops: vec![
                ColorStop::new(0.2, [1.0, 0.0, 0.0]),
                ColorStop::new(0.8, [0.0, 0.0, 1.0]),
            ],
            opacity_stops: vec![OpacityStop::new(0.0, 1.0)],
            kind: GradientType::Linear,
            dither: false,
        };
        // Below the first stop → first color; above the last → last color.
        assert!(approx(g.color_at(0.0)[0], 1.0));
        assert!(approx(g.color_at(1.0)[2], 1.0));
    }

    // ---- opacity stops ------------------------------------------------------

    #[test]
    fn opacity_rail_is_independent_of_color() {
        // Solid red color, alpha ramps 1→0.
        let g = Gradient {
            color_stops: vec![ColorStop::new(0.0, [1.0, 0.0, 0.0])],
            opacity_stops: vec![OpacityStop::new(0.0, 1.0), OpacityStop::new(1.0, 0.0)],
            kind: GradientType::Linear,
            dither: false,
        };
        // Color constant; alpha tracks the opacity rail.
        assert!(approx(g.alpha_at(0.0), 1.0));
        assert!(approx(g.alpha_at(0.5), 0.5));
        assert!(approx(g.alpha_at(1.0), 0.0));
        // Color is the single red stop throughout.
        assert!(approx(g.color_at(0.7)[0], 1.0));
        let s = g.sample(0.25);
        assert!(approx(s[0], 1.0) && approx(s[3], 0.75));
    }

    #[test]
    fn foreground_to_transparent_helper() {
        let g = Gradient::foreground_to_transparent([0.2, 0.4, 0.6], GradientType::Linear);
        assert!(approx(g.alpha_at(0.0), 1.0) && approx(g.alpha_at(1.0), 0.0));
        // Color stays the foreground.
        let c = g.color_at(0.5);
        assert!(approx(c[0], 0.2) && approx(c[2], 0.6));
    }

    #[test]
    fn render_is_premultiplied() {
        // Opaque→transparent red across a row; premultiplied rgb scales by alpha.
        let g = Gradient {
            color_stops: vec![ColorStop::new(0.0, [1.0, 0.0, 0.0])],
            opacity_stops: vec![OpacityStop::new(0.0, 1.0), OpacityStop::new(1.0, 0.0)],
            kind: GradientType::Linear,
            dither: false,
        };
        let w = 9;
        let buf = g.render((0.0, 0.0), (w as f32, 0.0), w, 1);
        // Left: a≈1, premul r≈1. Right: a≈0, premul r≈0.
        assert!(buf[3] > 0.9 && buf[0] > 0.9, "left {:?}", &buf[0..4]);
        let r = ((w - 1) * 4) as usize;
        assert!(
            buf[r + 3] < 0.1 && buf[r] < 0.1,
            "right {:?}",
            &buf[r..r + 4]
        );
        // Premultiplied invariant at every pixel: rgb == straight*alpha == alpha
        // (since straight r is constant 1) — so r channel ≈ alpha channel.
        for i in 0..w as usize {
            let b = i * 4;
            assert!(
                (buf[b] - buf[b + 3]).abs() < EPS,
                "px {i}: premul r {} should equal a {}",
                buf[b],
                buf[b + 3]
            );
        }
    }

    // ---- geometry / parameterization ---------------------------------------

    #[test]
    fn linear_param_projects_onto_axis() {
        let s = (0.0, 0.0);
        let e = (10.0, 0.0);
        assert!(approx(GradientType::Linear.param(0.0, 0.0, s, e), 0.0));
        assert!(approx(GradientType::Linear.param(5.0, 0.0, s, e), 0.5));
        assert!(approx(GradientType::Linear.param(10.0, 0.0, s, e), 1.0));
        // Off-axis: only the projection onto the axis matters.
        assert!(approx(GradientType::Linear.param(5.0, 7.0, s, e), 0.5));
        // Before start / after end clamps.
        assert!(approx(GradientType::Linear.param(-3.0, 0.0, s, e), 0.0));
        assert!(approx(GradientType::Linear.param(20.0, 0.0, s, e), 1.0));
    }

    #[test]
    fn radial_param_is_distance_normalized() {
        let s = (0.0, 0.0);
        let e = (10.0, 0.0); // radius 10
        assert!(approx(GradientType::Radial.param(0.0, 0.0, s, e), 0.0));
        assert!(approx(GradientType::Radial.param(5.0, 0.0, s, e), 0.5));
        // Distance is isotropic: (0,5) is the same t as (5,0).
        assert!(approx(GradientType::Radial.param(0.0, 5.0, s, e), 0.5));
        assert!(approx(GradientType::Radial.param(10.0, 0.0, s, e), 1.0));
        // Beyond the radius clamps to 1.
        assert!(approx(GradientType::Radial.param(20.0, 0.0, s, e), 1.0));
    }

    #[test]
    fn reflected_param_mirrors_about_start() {
        let s = (0.0, 0.0);
        let e = (10.0, 0.0);
        // Symmetric: -5 and +5 both map to 0.5.
        assert!(approx(GradientType::Reflected.param(5.0, 0.0, s, e), 0.5));
        assert!(approx(GradientType::Reflected.param(-5.0, 0.0, s, e), 0.5));
        assert!(approx(GradientType::Reflected.param(0.0, 0.0, s, e), 0.0));
    }

    #[test]
    fn diamond_param_is_chebyshev() {
        let s = (0.0, 0.0);
        let e = (10.0, 0.0); // unit = 10 along x, perp along y
                             // On-axis distance 5 → 0.5.
        assert!(approx(GradientType::Diamond.param(5.0, 0.0, s, e), 0.5));
        // Perp distance 5 → 0.5 (Chebyshev uses the max of the two).
        assert!(approx(GradientType::Diamond.param(0.0, 5.0, s, e), 0.5));
        // A corner (5,5) is still 0.5 (max(0.5,0.5)).
        assert!(approx(GradientType::Diamond.param(5.0, 5.0, s, e), 0.5));
        // (5,3) → max(0.5,0.3) = 0.5; (3,5) → 0.5.
        assert!(approx(GradientType::Diamond.param(3.0, 5.0, s, e), 0.5));
    }

    #[test]
    fn angle_param_wraps_around_start() {
        let s = (0.0, 0.0);
        let e = (10.0, 0.0); // axis points +x
                             // Along the axis → t≈0.
        assert!(
            GradientType::Angle.param(10.0, 0.0, s, e) < 0.02
                || GradientType::Angle.param(10.0, 0.0, s, e) > 0.98
        );
        // Quarter turn counter-clockwise (screen y-down → up is -y) → ≈0.25.
        let q = GradientType::Angle.param(0.0, -10.0, s, e);
        assert!(approx(q, 0.25), "quarter turn t={q}");
        // Opposite the axis → ≈0.5.
        let half = GradientType::Angle.param(-10.0, 0.0, s, e);
        assert!(approx(half, 0.5), "half turn t={half}");
    }

    #[test]
    fn zero_length_drag_is_safe() {
        let s = (3.0, 3.0);
        for kind in [
            GradientType::Linear,
            GradientType::Radial,
            GradientType::Angle,
            GradientType::Reflected,
            GradientType::Diamond,
        ] {
            // No panic; param stays in range.
            let t = kind.param(7.0, 1.0, s, s);
            assert!((0.0..=1.0).contains(&t), "{kind:?} produced {t}");
        }
    }

    // ---- dithering ----------------------------------------------------------

    #[test]
    fn dither_is_deterministic_and_seeded() {
        let g = Gradient::two_color([0.0; 3], [1.0; 3], GradientType::Linear);
        assert!(g.dither);
        let a = g.render((0.0, 0.0), (64.0, 0.0), 64, 8);
        let b = g.render((0.0, 0.0), (64.0, 0.0), 64, 8);
        // Bit-for-bit reproducible (ordered dither, no RNG).
        assert_eq!(a, b);
    }

    #[test]
    fn dither_present_when_enabled_absent_when_off() {
        // A near-flat ramp: with dither ON, adjacent pixels in a Bayer cell
        // differ; with it OFF they are identical (banded). Use a long gentle
        // ramp so the underlying gradient barely changes pixel-to-pixel.
        let on = Gradient {
            dither: true,
            ..Gradient::two_color([0.0; 3], [1.0; 3], GradientType::Linear)
        };
        let off = Gradient {
            dither: false,
            ..on.clone()
        };
        let w = 256;
        let bon = on.render((0.0, 0.0), (w as f32, 0.0), w, 8);
        let boff = off.render((0.0, 0.0), (w as f32, 0.0), w, 8);
        // Without dither, two pixels in the same column (same x → same t) are
        // identical across rows.
        let col = 100usize;
        let r0 = col * 4;
        let r1 = (w as usize + col) * 4;
        assert_eq!(
            boff[r0], boff[r1],
            "no-dither: same column must be identical across rows"
        );
        // With dither, the Bayer matrix differs between row 0 and row 1 at the
        // same x, so at least one of these columns differs across the two rows.
        let mut differs = false;
        for col in 0..w as usize {
            let a = (col) * 4;
            let b = (w as usize + col) * 4;
            if bon[a] != bon[b] {
                differs = true;
                break;
            }
        }
        assert!(differs, "dither: rows should differ at some column");
    }

    #[test]
    fn dither_preserves_average() {
        // Dither must not shift the mean brightness materially: average of a
        // dithered ramp ≈ average of the undithered ramp.
        let on = Gradient::two_color([0.0; 3], [1.0; 3], GradientType::Linear);
        let off = Gradient {
            dither: false,
            ..on.clone()
        };
        let w = 256;
        let bon = on.render((0.0, 0.0), (w as f32, 0.0), w, 8);
        let boff = off.render((0.0, 0.0), (w as f32, 0.0), w, 8);
        let mean = |b: &[f32]| b.iter().step_by(4).sum::<f32>() / (b.len() / 4) as f32;
        assert!((mean(&bon) - mean(&boff)).abs() < 0.01);
    }

    // ---- ids / serde --------------------------------------------------------

    #[test]
    fn type_ids_round_trip() {
        for kind in [
            GradientType::Linear,
            GradientType::Radial,
            GradientType::Angle,
            GradientType::Reflected,
            GradientType::Diamond,
        ] {
            assert_eq!(GradientType::from_id(kind.id()), kind);
        }
        // Unknown ids fall back to Linear.
        assert_eq!(GradientType::from_id(99), GradientType::Linear);
    }

    // (A serde JSON round-trip is covered in prism-io's persistence tests, which
    // own the `serde_json` dependency; here we keep prism-core dependency-light.)

    #[test]
    fn render_length_and_zero_dims() {
        let g = Gradient::default();
        assert_eq!(g.render((0.0, 0.0), (4.0, 0.0), 4, 3).len(), 4 * 3 * 4);
        assert!(g.render((0.0, 0.0), (4.0, 0.0), 0, 3).is_empty());
        assert!(g.render((0.0, 0.0), (4.0, 0.0), 4, 0).is_empty());
    }
}
