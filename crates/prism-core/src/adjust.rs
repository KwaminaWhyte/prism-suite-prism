//! Non-destructive adjustment descriptors. An adjustment layer reads the
//! composited backdrop below it and transforms it (PLAN.md §4 Phase 3). The
//! `(kind, params)` encoding is passed straight to the compositor shader.

use serde::{Deserialize, Serialize};

/// Per-channel control points for a Curves adjustment. Each `Vec` is a set of
/// `(input, output)` knots in `[0, 1]`; identity is `[(0,0), (1,1)]`. `rgb` is
/// the composite (master) curve applied to every channel first, then the
/// per-channel `r`/`g`/`b` curves. Rasterized to a 256-entry LUT via
/// [`crate::curve::build_lut`] and sampled in the compositor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurvePoints {
    pub rgb: Vec<(f32, f32)>,
    pub r: Vec<(f32, f32)>,
    pub g: Vec<(f32, f32)>,
    pub b: Vec<(f32, f32)>,
}

impl Default for CurvePoints {
    fn default() -> Self {
        let id = || vec![(0.0, 0.0), (1.0, 1.0)];
        Self {
            rgb: id(),
            r: id(),
            g: id(),
            b: id(),
        }
    }
}

// Not `Copy`: `Curves` carries variable-length control points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Adjustment {
    BrightnessContrast {
        brightness: f32,
        contrast: f32,
    },
    Levels {
        in_black: f32,
        in_white: f32,
        gamma: f32,
    },
    HueSaturation {
        hue: f32,
        saturation: f32,
        lightness: f32,
    },
    Exposure {
        stops: f32,
    },
    Invert,
    Threshold {
        level: f32,
    },
    BlackWhite,
    /// Tone curves (composite + per-channel). Params are not float-encodable —
    /// the compositor uploads a LUT texture (shader kind `8`) built from these.
    Curves(CurvePoints),
    /// Saturation boost weighted toward less-saturated pixels. `amount` −1..1.
    Vibrance { amount: f32 },
    /// Warming/cooling color filter; `color` is straight sRGB, `density` 0..1.
    PhotoFilter { color: [f32; 3], density: f32 },
    /// Quantize each channel to `levels` steps (2..=255).
    Posterize { levels: u32 },
    /// Map luminance through a two-color gradient (`low` = shadows, `high` =
    /// highlights), straight sRGB. The compositor builds + samples a LUT texture.
    GradientMap { low: [f32; 3], high: [f32; 3] },
}

impl Adjustment {
    /// Shader kind id (stable; written to disk) + up to four float params.
    /// `Curves` (kind 8) carries no float params — its LUT is uploaded separately.
    pub fn encode(&self) -> (u32, [f32; 4]) {
        match self {
            Adjustment::BrightnessContrast {
                brightness,
                contrast,
            } => (1, [*brightness, *contrast, 0.0, 0.0]),
            Adjustment::Levels {
                in_black,
                in_white,
                gamma,
            } => (2, [*in_black, *in_white, *gamma, 0.0]),
            Adjustment::HueSaturation {
                hue,
                saturation,
                lightness,
            } => (3, [*hue, *saturation, *lightness, 0.0]),
            Adjustment::Invert => (4, [0.0; 4]),
            Adjustment::Exposure { stops } => (5, [*stops, 0.0, 0.0, 0.0]),
            Adjustment::Threshold { level } => (6, [*level, 0.0, 0.0, 0.0]),
            Adjustment::BlackWhite => (7, [0.0; 4]),
            Adjustment::Curves(_) => (8, [0.0; 4]),
            Adjustment::Vibrance { amount } => (9, [*amount, 0.0, 0.0, 0.0]),
            Adjustment::PhotoFilter { color, density } => {
                (10, [color[0], color[1], color[2], *density])
            }
            Adjustment::Posterize { levels } => (11, [*levels as f32, 0.0, 0.0, 0.0]),
            Adjustment::GradientMap { .. } => (12, [0.0; 4]),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Adjustment::BrightnessContrast { .. } => "Brightness/Contrast",
            Adjustment::Levels { .. } => "Levels",
            Adjustment::HueSaturation { .. } => "Hue/Saturation",
            Adjustment::Exposure { .. } => "Exposure",
            Adjustment::Invert => "Invert",
            Adjustment::Threshold { .. } => "Threshold",
            Adjustment::BlackWhite => "Black & White",
            Adjustment::Curves(_) => "Curves",
            Adjustment::Vibrance { .. } => "Vibrance",
            Adjustment::PhotoFilter { .. } => "Photo Filter",
            Adjustment::Posterize { .. } => "Posterize",
            Adjustment::GradientMap { .. } => "Gradient Map",
        }
    }

    /// Sensible defaults for each kind (identity-ish where applicable). A `fn`
    /// rather than a `const` because `Curves` holds owned control points.
    pub fn defaults() -> Vec<Adjustment> {
        vec![
            Adjustment::BrightnessContrast {
                brightness: 0.0,
                contrast: 0.0,
            },
            Adjustment::Levels {
                in_black: 0.0,
                in_white: 1.0,
                gamma: 1.0,
            },
            Adjustment::Curves(CurvePoints::default()),
            Adjustment::HueSaturation {
                hue: 0.0,
                saturation: 0.0,
                lightness: 0.0,
            },
            Adjustment::Exposure { stops: 0.0 },
            Adjustment::Vibrance { amount: 0.0 },
            Adjustment::PhotoFilter {
                color: [1.0, 0.64, 0.0], // warming (85)
                density: 0.25,
            },
            Adjustment::Posterize { levels: 4 },
            Adjustment::GradientMap {
                low: [0.05, 0.0, 0.2],  // deep indigo shadows
                high: [1.0, 0.85, 0.4], // warm highlights
            },
            Adjustment::Invert,
            Adjustment::Threshold { level: 0.5 },
            Adjustment::BlackWhite,
        ]
    }
}
