//! `.pigment` document container: metadata JSON + per-layer lz4-compressed
//! linear-premultiplied RGBA16F pixel blobs.
//!
//! Container layout:
//!   magic b"PIGMENT1" (8 bytes)
//!   u32 LE metadata-json-length
//!   metadata JSON (serde_json)
//!   then for each layer in `meta.layers` order:
//!     u64 LE id
//!     u32 LE compressed-len
//!     lz4-compressed rgba16f bytes (length-prepended via lz4_flex)

use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use prism_core::Adjustment;
use serde::{Deserialize, Serialize};

/// 8-byte container magic / version tag.
const MAGIC: &[u8; 8] = b"PIGMENT1";

/// Bytes per pixel of the RGBA16F payload (4 channels * 2 bytes).
const BYTES_PER_PIXEL: usize = 4 * 2;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayerMeta {
    pub id: u64,
    pub name: String,
    pub blend: u32, // BlendMode shader id
    pub opacity: f32,
    pub visible: bool,
    /// Optional non-destructive layer styles (stroke, shadows, glows, overlays,
    /// bevel). Absent in old documents and in documents whose layer has no
    /// styles; `skip_serializing_if` keeps such files byte-compatible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub styles: Option<LayerStyles>,
    /// Optional non-destructive adjustment descriptor. Present iff the layer is
    /// an adjustment layer (Curves, Levels, Color Balance, Channel Mixer, …);
    /// stores the shared, app-agnostic [`prism_core::Adjustment`] (kind + every
    /// param) verbatim so the adjustment round-trips losslessly. Absent in old
    /// documents and on non-adjustment layers; `skip_serializing_if` keeps such
    /// files byte-compatible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjustment: Option<Adjustment>,
    /// Optional non-destructive **smart-filter stack** applied to a raster layer
    /// on top of its stored (un-filtered) source pixels. Each entry is a filter
    /// kind + its parameters + an enabled flag; the displayed layer is the source
    /// with the enabled filters applied in order, re-applied whenever the stack
    /// changes (so any filter stays editable / re-orderable / toggleable).
    /// Absent in old documents and on layers with no smart filters;
    /// `skip_serializing_if` keeps such files byte-compatible.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub smart_filters: Vec<SmartFilterMeta>,
}

/// One entry in a layer's serialized smart-filter stack. Pure data — no app or
/// GPU types. `kind` is a stable app-agnostic filter id (matching the app's
/// `SmartFilterKind` discriminant), `params` carries that filter's numeric
/// parameters (radius / amount / levels, etc.; unused slots are 0), and
/// `enabled` toggles the filter without removing it from the stack.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SmartFilterMeta {
    pub kind: u32,
    pub params: [f32; 4],
    pub enabled: bool,
}

/// Serializable bundle of a layer's non-destructive styles. Pure data — colors
/// are straight (non-premultiplied) RGBA `[r,g,b,a]` or RGB `[r,g,b]` in 0..1
/// linear-agnostic units matching the app's runtime style maps; pixel offsets /
/// sizes / blur are in document pixels; angles in degrees. No app or GPU types.
///
/// Every field is optional and skipped when `None`, so a layer that carries only
/// (say) a stroke serializes to `{"stroke": {...}}` with no empty keys, and a
/// layer with no styles at all is represented as `LayerMeta.styles == None`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LayerStyles {
    /// Outer stroke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<StrokeStyle>,
    /// Drop shadow (cast outside the layer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drop_shadow: Option<ShadowStyle>,
    /// Color overlay (flat fill; `color[3]` is strength).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_overlay: Option<ColorOverlayStyle>,
    /// Inner shadow (cast inside the layer edges).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_shadow: Option<ShadowStyle>,
    /// Outer glow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outer_glow: Option<GlowStyle>,
    /// Inner glow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_glow: Option<GlowStyle>,
    /// Gradient overlay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient_overlay: Option<GradientOverlayStyle>,
    /// Bevel & Emboss (Inner Bevel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bevel: Option<BevelStyle>,
}

/// Outer stroke style: straight RGBA color and stroke width in document pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrokeStyle {
    pub color: [f32; 4],
    /// Stroke width, document pixels.
    pub width_px: f32,
}

/// Drop / inner shadow style: straight RGBA color, `[dx, dy]` offset in document
/// pixels, and blur radius in document pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShadowStyle {
    pub color: [f32; 4],
    /// Offset `[dx, dy]`, document pixels.
    pub offset_px: [f32; 2],
    /// Blur radius, document pixels.
    pub blur_px: f32,
}

/// Color overlay style: straight RGBA where `color[3]` is the overlay strength.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorOverlayStyle {
    pub color: [f32; 4],
}

/// Outer / inner glow style: straight RGBA color and glow size in document pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GlowStyle {
    pub color: [f32; 4],
    /// Glow size, document pixels.
    pub size_px: f32,
}

/// Gradient overlay style: two RGBA stops, angle in degrees, and opacity (0..1).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientOverlayStyle {
    /// Start color (RGBA).
    pub color0: [f32; 4],
    /// End color (RGBA).
    pub color1: [f32; 4],
    /// Gradient angle, degrees.
    pub angle_deg: f32,
    /// Overlay opacity, 0..1.
    pub opacity: f32,
}

/// Bevel & Emboss (Inner Bevel) style: highlight and shadow RGBA colors, bevel
/// size and soften in document pixels, light angle and altitude in degrees.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BevelStyle {
    pub highlight: [f32; 4],
    pub shadow: [f32; 4],
    /// Bevel size, document pixels.
    pub size_px: f32,
    /// Soften radius, document pixels.
    pub soften_px: f32,
    /// Light angle, degrees.
    pub angle_deg: f32,
    /// Light altitude, degrees.
    pub altitude_deg: f32,
}

/// One layer's snapshotted appearance attributes within a layer comp.
/// `blend` is the BlendMode shader id (matching [`LayerMeta::blend`]); the entry
/// is keyed by the stable layer `id` so restore matches by id and tolerates
/// layers being reordered, added, or removed since the comp was captured.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerCompEntry {
    pub id: u64,
    pub blend: u32,
    pub opacity: f32,
    pub visible: bool,
}

/// A named layer comp: a snapshot of per-layer appearance (visibility, opacity,
/// blend mode) the user can restore. Pure data — no app/GPU types.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerCompMeta {
    pub name: String,
    pub entries: Vec<LayerCompEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DocMeta {
    pub width: u32,
    pub height: u32,
    pub layers: Vec<LayerMeta>, // bottom-to-top order
    /// Named layer comps (snapshots of per-layer visibility/opacity/blend).
    /// Absent in documents written before layer comps existed; `#[serde(default)]`
    /// loads such documents as an empty list, and `skip_serializing_if` keeps
    /// comp-free documents byte-compatible with the previous format.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comps: Vec<LayerCompMeta>,
}

/// Raw pixel payload for one layer: linear-premultiplied RGBA16F as little-endian
/// bytes, length == width*height*4*2.
pub struct LayerPixels {
    pub id: u64,
    pub rgba16f: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum DocError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("metadata json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("bad file magic: expected b\"PIGMENT1\"")]
    BadMagic,

    #[error("unsupported format version")]
    Version,

    #[error("lz4 decompress failed: {0}")]
    Decompress(#[from] lz4_flex::block::DecompressError),

    #[error("pixel length mismatch for layer {id}: expected {expected} bytes, got {actual}")]
    LengthMismatch {
        id: u64,
        expected: usize,
        actual: usize,
    },
}

/// Serialize the document to `path`.
pub fn save_document(path: &Path, meta: &DocMeta, pixels: &[LayerPixels]) -> Result<(), DocError> {
    let file = std::fs::File::create(path)?;
    let mut w = BufWriter::new(file);

    // magic
    w.write_all(MAGIC)?;

    // metadata JSON, length-prefixed
    let json = serde_json::to_vec(meta)?;
    w.write_all(&(json.len() as u32).to_le_bytes())?;
    w.write_all(&json)?;

    // layers in meta order; look pixels up by id
    for layer in &meta.layers {
        let blob = pixels
            .iter()
            .find(|p| p.id == layer.id)
            .map(|p| p.rgba16f.as_slice())
            .unwrap_or(&[]);

        let compressed = lz4_flex::compress_prepend_size(blob);

        w.write_all(&layer.id.to_le_bytes())?;
        w.write_all(&(compressed.len() as u32).to_le_bytes())?;
        w.write_all(&compressed)?;
    }

    w.flush()?;
    Ok(())
}

/// Inverse of `save_document`. Decompress each layer blob and validate magic plus
/// that each decompressed blob length == width*height*4*2.
pub fn load_document(path: &Path) -> Result<(DocMeta, Vec<LayerPixels>), DocError> {
    let file = std::fs::File::open(path)?;
    let mut r = BufReader::new(file);

    // magic
    let mut magic = [0u8; 8];
    r.read_exact(&mut magic)?;
    if &magic != MAGIC {
        // distinguish a same-length-but-wrong tag (version) from arbitrary bytes.
        if magic.starts_with(b"PIGMENT") {
            return Err(DocError::Version);
        }
        return Err(DocError::BadMagic);
    }

    // metadata JSON
    let json_len = read_u32(&mut r)? as usize;
    let mut json = vec![0u8; json_len];
    r.read_exact(&mut json)?;
    let meta: DocMeta = serde_json::from_slice(&json)?;

    let expected = meta.width as usize * meta.height as usize * BYTES_PER_PIXEL;

    let mut pixels = Vec::with_capacity(meta.layers.len());
    for _ in 0..meta.layers.len() {
        let id = read_u64(&mut r)?;
        let comp_len = read_u32(&mut r)? as usize;

        let mut compressed = vec![0u8; comp_len];
        r.read_exact(&mut compressed)?;

        let rgba16f = lz4_flex::decompress_size_prepended(&compressed)?;

        if rgba16f.len() != expected {
            return Err(DocError::LengthMismatch {
                id,
                expected,
                actual: rgba16f.len(),
            });
        }

        pixels.push(LayerPixels { id, rgba16f });
    }

    Ok((meta, pixels))
}

fn read_u32<R: Read>(r: &mut R) -> Result<u32, DocError> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u64<R: Read>(r: &mut R) -> Result<u64, DocError> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let (w, h) = (4u32, 4u32);
        let len = w as usize * h as usize * BYTES_PER_PIXEL;

        let meta = DocMeta {
            width: w,
            height: h,
            layers: vec![
                LayerMeta {
                    id: 1,
                    name: "background".to_string(),
                    blend: 0,
                    opacity: 1.0,
                    visible: true,
                    styles: None,
                    adjustment: None,
                smart_filters: Vec::new(),
                },
                LayerMeta {
                    id: 2,
                    name: "paint".to_string(),
                    blend: 3,
                    opacity: 0.5,
                    visible: false,
                    styles: None,
                    adjustment: None,
                smart_filters: Vec::new(),
                },
            ],
            comps: vec![],
        };

        let px0: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
        let px1: Vec<u8> = (0..len).map(|i| (i.wrapping_mul(7) % 253) as u8).collect();
        let pixels = vec![
            LayerPixels {
                id: 1,
                rgba16f: px0.clone(),
            },
            LayerPixels {
                id: 2,
                rgba16f: px1.clone(),
            },
        ];

        let mut path = std::env::temp_dir();
        path.push("pigment_doc_round_trip.pigment");

        save_document(&path, &meta, &pixels).expect("save");
        let (rmeta, rpix) = load_document(&path).expect("load");

        // metadata
        assert_eq!(rmeta.width, w);
        assert_eq!(rmeta.height, h);
        assert_eq!(rmeta.layers.len(), 2);
        assert_eq!(rmeta.layers[0].id, 1);
        assert_eq!(rmeta.layers[0].name, "background");
        assert_eq!(rmeta.layers[0].blend, 0);
        assert_eq!(rmeta.layers[0].opacity, 1.0);
        assert!(rmeta.layers[0].visible);
        assert_eq!(rmeta.layers[1].id, 2);
        assert_eq!(rmeta.layers[1].name, "paint");
        assert_eq!(rmeta.layers[1].blend, 3);
        assert_eq!(rmeta.layers[1].opacity, 0.5);
        assert!(!rmeta.layers[1].visible);

        // pixels
        assert_eq!(rpix.len(), 2);
        assert_eq!(rpix[0].id, 1);
        assert_eq!(rpix[0].rgba16f, px0);
        assert_eq!(rpix[1].id, 2);
        assert_eq!(rpix[1].rgba16f, px1);

        let _ = std::fs::remove_file(&path);
    }

    /// A `LayerMeta` carrying a fully populated `LayerStyles` payload survives a
    /// serde_json round-trip with every field intact.
    #[test]
    fn layer_styles_round_trip() {
        let styles = LayerStyles {
            stroke: Some(StrokeStyle {
                color: [0.1, 0.2, 0.3, 1.0],
                width_px: 4.5,
            }),
            drop_shadow: Some(ShadowStyle {
                color: [0.0, 0.0, 0.0, 0.75],
                offset_px: [5.0, -3.0],
                blur_px: 8.0,
            }),
            color_overlay: Some(ColorOverlayStyle {
                color: [0.8, 0.1, 0.1, 0.6],
            }),
            inner_shadow: Some(ShadowStyle {
                color: [0.05, 0.05, 0.05, 0.5],
                offset_px: [-2.0, 2.0],
                blur_px: 3.0,
            }),
            outer_glow: Some(GlowStyle {
                color: [1.0, 0.9, 0.2, 0.8],
                size_px: 12.0,
            }),
            inner_glow: Some(GlowStyle {
                color: [0.2, 0.9, 1.0, 0.7],
                size_px: 6.0,
            }),
            gradient_overlay: Some(GradientOverlayStyle {
                color0: [0.0, 0.0, 0.0, 1.0],
                color1: [1.0, 1.0, 1.0, 1.0],
                angle_deg: 45.0,
                opacity: 0.9,
            }),
            bevel: Some(BevelStyle {
                highlight: [1.0, 1.0, 1.0, 0.75],
                shadow: [0.0, 0.0, 0.0, 0.75],
                size_px: 5.0,
                soften_px: 2.0,
                angle_deg: 120.0,
                altitude_deg: 30.0,
            }),
        };

        let meta = LayerMeta {
            id: 7,
            name: "styled".to_string(),
            blend: 2,
            opacity: 0.5,
            visible: true,
            styles: Some(styles.clone()),
            adjustment: None,
        smart_filters: Vec::new(),
        };

        let json = serde_json::to_string(&meta).expect("serialize");
        let back: LayerMeta = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.id, 7);
        assert_eq!(back.name, "styled");
        assert_eq!(back.styles, Some(styles));
    }

    /// An old document JSON without the `styles`/`adjustment` keys deserializes
    /// with both `None`, and a plain layer serializes without either key at all
    /// (compact back-compat in both directions).
    #[test]
    fn old_doc_without_styles_key_loads() {
        // Old-format LayerMeta JSON: no `styles` / `adjustment` fields present.
        let old = r#"{"id":1,"name":"bg","blend":0,"opacity":1.0,"visible":true}"#;
        let meta: LayerMeta = serde_json::from_str(old).expect("deserialize old");
        assert!(meta.styles.is_none());
        assert!(meta.adjustment.is_none());

        // A plain layer must not emit a `styles` or `adjustment` key.
        let json = serde_json::to_string(&meta).expect("serialize");
        assert!(
            !json.contains("styles"),
            "style-less layer should omit the styles key, got: {json}"
        );
        assert!(
            !json.contains("adjustment"),
            "non-adjustment layer should omit the adjustment key, got: {json}"
        );
    }

    /// The shared multi-stop `Gradient` (color + opacity stops, geometry, dither)
    /// survives a serde_json round-trip losslessly — it serializes through the
    /// same serde path the `.pigment` doc model uses, so gradient fills/presets
    /// can be embedded in the container alongside styles/adjustments. Covers the
    /// non-trivial case: 3 color stops, 3 opacity stops, a non-linear geometry.
    #[test]
    fn gradient_serde_round_trip() {
        use prism_core::gradient::{ColorStop, Gradient, GradientType, OpacityStop};
        let g = Gradient {
            color_stops: vec![
                ColorStop::new(0.0, [1.0, 0.0, 0.0]),
                ColorStop::new(0.5, [0.0, 1.0, 0.0]),
                ColorStop::new(1.0, [0.0, 0.0, 1.0]),
            ],
            opacity_stops: vec![
                OpacityStop::new(0.0, 1.0),
                OpacityStop::new(0.7, 0.3),
                OpacityStop::new(1.0, 0.0),
            ],
            kind: GradientType::Radial,
            dither: true,
        };
        let json = serde_json::to_string(&g).expect("serialize");
        let back: Gradient = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, g);
    }

    /// A `LayerMeta` carrying a fully populated `Adjustment` payload survives a
    /// serde_json round-trip with the kind + every param intact. Covers a
    /// multi-param kind (Color Balance) — the `Adjustment` enum's own
    /// Serialize/Deserialize is reused verbatim, so all kinds round-trip.
    #[test]
    fn layer_adjustment_round_trip() {
        let adj = Adjustment::ColorBalance {
            shadows: [0.2, -0.1, 0.3],
            midtones: [-0.4, 0.5, 0.0],
            highlights: [0.1, 0.1, -0.6],
            preserve_luminosity: false,
        };
        let meta = LayerMeta {
            id: 9,
            name: "Color Balance".to_string(),
            blend: 0,
            opacity: 0.8,
            visible: true,
            styles: None,
            adjustment: Some(adj.clone()),
        smart_filters: Vec::new(),
        };

        let json = serde_json::to_string(&meta).expect("serialize");
        let back: LayerMeta = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.id, 9);
        assert_eq!(back.name, "Color Balance");
        assert_eq!(back.adjustment, Some(adj));
    }

    /// A `DocMeta` carrying layer comps survives a serde_json round-trip with
    /// every comp entry intact.
    #[test]
    fn layer_comps_round_trip() {
        let meta = DocMeta {
            width: 8,
            height: 8,
            layers: vec![LayerMeta {
                id: 1,
                name: "bg".to_string(),
                blend: 0,
                opacity: 1.0,
                visible: true,
                styles: None,
                adjustment: None,
            smart_filters: Vec::new(),
            }],
            comps: vec![
                LayerCompMeta {
                    name: "Variant A".to_string(),
                    entries: vec![LayerCompEntry {
                        id: 1,
                        blend: 3,
                        opacity: 0.5,
                        visible: false,
                    }],
                },
                LayerCompMeta {
                    name: "Variant B".to_string(),
                    entries: vec![LayerCompEntry {
                        id: 1,
                        blend: 0,
                        opacity: 1.0,
                        visible: true,
                    }],
                },
            ],
        };

        let json = serde_json::to_string(&meta).expect("serialize");
        let back: DocMeta = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.comps, meta.comps);
    }

    /// An old document JSON without the `comps` key deserializes with an empty
    /// comps list, and a comp-free document serializes without the `comps` key
    /// at all (compact back-compat in both directions).
    #[test]
    fn old_doc_without_comps_key_loads() {
        let old = r#"{"width":4,"height":4,"layers":[]}"#;
        let meta: DocMeta = serde_json::from_str(old).expect("deserialize old");
        assert!(meta.comps.is_empty());

        let json = serde_json::to_string(&meta).expect("serialize");
        assert!(
            !json.contains("comps"),
            "comp-free document should omit the comps key, got: {json}"
        );
    }
}
