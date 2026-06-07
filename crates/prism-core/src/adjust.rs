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
    /// Per-tonal-range RGB push (Photoshop "Color Balance"). Each of `shadows`,
    /// `midtones`, `highlights` is a `[cyan↔red, magenta↔green, yellow↔blue]`
    /// shift in `−1..1`. Because each output channel depends only on that same
    /// input channel, this rasterizes to a per-channel transfer LUT (like
    /// Curves/Gradient Map) — shader kind `13`. `preserve_luminosity` keeps the
    /// pixel's luma constant after the shift.
    ColorBalance {
        shadows: [f32; 3],
        midtones: [f32; 3],
        highlights: [f32; 3],
        preserve_luminosity: bool,
    },
    /// Per-output-channel linear mix of the input RGB plus a constant (Photoshop
    /// "Channel Mixer"). `r`/`g`/`b` are `[from_r, from_g, from_b, constant]`
    /// in source-channel weights (typ. −2..2) and a `−1..1` offset. `monochrome`
    /// collapses to a single mix written to all three outputs. Output channel
    /// mixes *all* inputs, so this needs a matrix (not a 1-D LUT) — shader kind
    /// `14`, fed via dedicated compositor params.
    ChannelMixer {
        r: [f32; 4],
        g: [f32; 4],
        b: [f32; 4],
        monochrome: bool,
    },
}

impl ColorBalanceLuts {
    /// Build the three per-channel transfer LUTs (each `n` entries, input/output
    /// in `[0,1]`) for a [`Adjustment::ColorBalance`]. Range weighting follows
    /// Photoshop: shadows act on dark input, highlights on light input, midtones
    /// on a bell centred at 0.5. A `+1` shift on a channel lifts it by ~0.25 at
    /// the range's peak. Pure function — unit-tested, no GPU.
    pub fn build(
        shadows: [f32; 3],
        midtones: [f32; 3],
        highlights: [f32; 3],
        n: usize,
    ) -> Self {
        let mut out = [Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n)];
        for c in 0..3 {
            for i in 0..n {
                let x = i as f32 / (n - 1).max(1) as f32;
                // Range weights (sum stays ~1 across the tonal range).
                let w_shadow = (1.0 - x).powi(2); // strongest at 0
                let w_high = x * x; // strongest at 1
                let w_mid = 1.0 - (2.0 * x - 1.0).powi(2); // bell peaking at 0.5
                let shift = 0.25
                    * (shadows[c] * w_shadow + midtones[c] * w_mid + highlights[c] * w_high);
                out[c].push((x + shift).clamp(0.0, 1.0));
            }
        }
        let [r, g, b] = out;
        Self { r, g, b }
    }
}

/// The three per-channel transfer LUTs produced by [`ColorBalanceLuts::build`].
#[derive(Clone, Debug, PartialEq)]
pub struct ColorBalanceLuts {
    pub r: Vec<f32>,
    pub g: Vec<f32>,
    pub b: Vec<f32>,
}

/// Channel-mixer matrix encoded for the compositor: rows `r`/`g`/`b` are
/// `[from_r, from_g, from_b, constant]`. [`monochrome`](Self::monochrome) means
/// every output row equals the `r` row (a single weighted gray).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelMixerMatrix {
    pub r: [f32; 4],
    pub g: [f32; 4],
    pub b: [f32; 4],
    pub monochrome: bool,
}

impl ChannelMixerMatrix {
    /// Apply the mix to a straight-sRGB pixel (clamped). Pure CPU reference used
    /// by tests; the compositor runs the identical math in WGSL (kind 14).
    pub fn apply(&self, c: [f32; 3]) -> [f32; 3] {
        let mix = |row: &[f32; 4]| {
            (row[0] * c[0] + row[1] * c[1] + row[2] * c[2] + row[3]).clamp(0.0, 1.0)
        };
        if self.monochrome {
            let v = mix(&self.r);
            [v, v, v]
        } else {
            [mix(&self.r), mix(&self.g), mix(&self.b)]
        }
    }
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
            // Color Balance carries a per-channel LUT (like Curves/GradientMap);
            // p.x flags luminosity preservation.
            Adjustment::ColorBalance {
                preserve_luminosity,
                ..
            } => (13, [if *preserve_luminosity { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0]),
            // Channel Mixer needs a full matrix — fed via dedicated compositor
            // params, not these 4 floats; p.x flags monochrome.
            Adjustment::ChannelMixer { monochrome, .. } => {
                (14, [if *monochrome { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0])
            }
        }
    }

    /// The channel-mixer matrix for [`Adjustment::ChannelMixer`], else `None`.
    /// The compositor uploads this to its mixer params for shader kind 14.
    pub fn channel_mixer_matrix(&self) -> Option<ChannelMixerMatrix> {
        match self {
            Adjustment::ChannelMixer {
                r,
                g,
                b,
                monochrome,
            } => Some(ChannelMixerMatrix {
                r: *r,
                g: *g,
                b: *b,
                monochrome: *monochrome,
            }),
            _ => None,
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
            Adjustment::ColorBalance { .. } => "Color Balance",
            Adjustment::ChannelMixer { .. } => "Channel Mixer",
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
            Adjustment::ColorBalance {
                shadows: [0.0; 3],
                midtones: [0.0; 3],
                highlights: [0.0; 3],
                preserve_luminosity: true,
            },
            Adjustment::ChannelMixer {
                // Identity: each output = its own input channel, no constant.
                r: [1.0, 0.0, 0.0, 0.0],
                g: [0.0, 1.0, 0.0, 0.0],
                b: [0.0, 0.0, 1.0, 0.0],
                monochrome: false,
            },
            Adjustment::Invert,
            Adjustment::Threshold { level: 0.5 },
            Adjustment::BlackWhite,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_balance_identity_is_passthrough() {
        let l = ColorBalanceLuts::build([0.0; 3], [0.0; 3], [0.0; 3], 256);
        for i in 0..256 {
            let x = i as f32 / 255.0;
            assert!((l.r[i] - x).abs() < 1e-6, "r[{i}] = {} != {x}", l.r[i]);
            assert!((l.g[i] - x).abs() < 1e-6);
            assert!((l.b[i] - x).abs() < 1e-6);
        }
    }

    #[test]
    fn color_balance_shadows_lift_only_darks() {
        // +1 red shift in shadows: lifts dark reds, leaves brights ~untouched.
        let l = ColorBalanceLuts::build([1.0, 0.0, 0.0], [0.0; 3], [0.0; 3], 256);
        let dark = 16usize; // x ≈ 0.063
        let bright = 240usize; // x ≈ 0.94
        assert!(
            l.r[dark] > dark as f32 / 255.0 + 0.1,
            "dark red lifted: {} vs {}",
            l.r[dark],
            dark as f32 / 255.0
        );
        assert!(
            (l.r[bright] - bright as f32 / 255.0).abs() < 0.02,
            "bright red barely moved: {}",
            l.r[bright]
        );
        // Green/blue channels untouched by a red-only shift.
        assert!((l.g[dark] - dark as f32 / 255.0).abs() < 1e-6);
        assert!((l.b[bright] - bright as f32 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn color_balance_highlights_lift_only_brights() {
        let l = ColorBalanceLuts::build([0.0; 3], [0.0; 3], [0.0, 0.0, 1.0], 256);
        let dark = 16usize; // x ≈ 0.063
        let bright = 180usize; // x ≈ 0.71 (visible lift, below the clamp ceiling)
        assert!(
            l.b[bright] > bright as f32 / 255.0 + 0.05,
            "bright blue lifted: {}",
            l.b[bright]
        );
        assert!(
            (l.b[dark] - dark as f32 / 255.0).abs() < 0.02,
            "dark blue barely moved: {}",
            l.b[dark]
        );
    }

    #[test]
    fn channel_mixer_swap_channels() {
        // Output R = input B, output B = input R (a red/blue swap).
        let m = ChannelMixerMatrix {
            r: [0.0, 0.0, 1.0, 0.0],
            g: [0.0, 1.0, 0.0, 0.0],
            b: [1.0, 0.0, 0.0, 0.0],
            monochrome: false,
        };
        let out = m.apply([0.8, 0.3, 0.1]);
        assert!((out[0] - 0.1).abs() < 1e-6, "R<-B: {}", out[0]);
        assert!((out[1] - 0.3).abs() < 1e-6, "G unchanged: {}", out[1]);
        assert!((out[2] - 0.8).abs() < 1e-6, "B<-R: {}", out[2]);
    }

    #[test]
    fn channel_mixer_monochrome_and_constant() {
        // Classic mono weights, plus a +0.1 constant on the gray output.
        let m = ChannelMixerMatrix {
            r: [0.4, 0.4, 0.2, 0.1],
            g: [0.0, 1.0, 0.0, 0.0],
            b: [0.0, 0.0, 1.0, 0.0],
            monochrome: true,
        };
        let out = m.apply([1.0, 0.0, 0.0]);
        // 0.4*1 + 0.1 = 0.5, written to all three.
        assert!((out[0] - 0.5).abs() < 1e-6, "{out:?}");
        assert_eq!(out[0], out[1]);
        assert_eq!(out[1], out[2]);
    }

    #[test]
    fn channel_mixer_clamps() {
        let m = ChannelMixerMatrix {
            r: [2.0, 0.0, 0.0, 0.5],
            g: [0.0, 1.0, 0.0, 0.0],
            b: [0.0, 0.0, 1.0, -2.0],
            monochrome: false,
        };
        let out = m.apply([1.0, 1.0, 1.0]);
        assert_eq!(out[0], 1.0); // 2.5 clamped
        assert_eq!(out[2], 0.0); // -1.0 clamped
    }

    #[test]
    fn encode_kinds_and_names_stable() {
        let cb = Adjustment::ColorBalance {
            shadows: [0.0; 3],
            midtones: [0.0; 3],
            highlights: [0.0; 3],
            preserve_luminosity: true,
        };
        assert_eq!(cb.encode().0, 13);
        assert_eq!(cb.encode().1[0], 1.0); // preserve-luminosity flag
        assert_eq!(cb.name(), "Color Balance");
        assert!(cb.channel_mixer_matrix().is_none());

        let cm = Adjustment::ChannelMixer {
            r: [1.0, 0.0, 0.0, 0.0],
            g: [0.0, 1.0, 0.0, 0.0],
            b: [0.0, 0.0, 1.0, 0.0],
            monochrome: false,
        };
        assert_eq!(cm.encode().0, 14);
        assert_eq!(cm.name(), "Channel Mixer");
        let mat = cm.channel_mixer_matrix().expect("mixer matrix");
        assert_eq!(mat.r, [1.0, 0.0, 0.0, 0.0]);
    }
}
