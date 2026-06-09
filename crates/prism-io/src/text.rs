//! Text rasterization into a linear-light premultiplied RGBA f32 layer.
//!
//! Uses `cosmic-text` for shaping + glyph coverage, then converts the supplied
//! straight sRGB color into linear light and premultiplies by glyph coverage so
//! the result can be uploaded directly as a layer.

use cosmic_text::{Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache};

/// Horizontal text alignment within the target buffer.
#[derive(Clone, Copy)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

impl TextAlign {
    fn to_cosmic(self) -> Align {
        match self {
            TextAlign::Left => Align::Left,
            TextAlign::Center => Align::Center,
            TextAlign::Right => Align::Right,
        }
    }
}

/// Standard sRGB transfer function (gamma-decode): straight sRGB -> linear.
#[inline]
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Rasterize `text` into a `width`x`height` RGBA buffer of **linear-light
/// premultiplied f32** (len == width*height*4), suitable to upload as a layer.
/// - `font_px`: font size in pixels.
/// - `color`: straight sRGB RGBA in 0..=1 (alpha multiplies glyph coverage).
/// - `family`: optional font-family name (e.g. `"Arial"`); `None` uses
///   cosmic-text's default sans-serif face. An empty/unknown name falls back to
///   cosmic-text's own substitution, so this never fails to render.
/// - Text is laid out starting near the top-left with the given alignment and
///   wraps to `width`. Glyph coverage (0..1) becomes alpha; rgb = color.rgb in
///   linear light, premultiplied by (coverage*color.a).
/// - Empty text or zero dims => all-zero buffer.
pub fn render_text(
    text: &str,
    font_px: f32,
    color: [f32; 4],
    width: u32,
    height: u32,
    align: TextAlign,
    family: Option<&str>,
) -> Vec<f32> {
    let px_count = (width as usize) * (height as usize);
    let mut out = vec![0.0f32; px_count * 4];

    if text.is_empty() || width == 0 || height == 0 {
        return out;
    }

    // Pre-decode the destination color to linear light. The glyph coverage and
    // the caller's alpha both modulate the final premultiplied result.
    let lin_r = srgb_to_linear(color[0].clamp(0.0, 1.0));
    let lin_g = srgb_to_linear(color[1].clamp(0.0, 1.0));
    let lin_b = srgb_to_linear(color[2].clamp(0.0, 1.0));
    let dst_a = color[3].clamp(0.0, 1.0);

    // A `FontSystem` is not cheap; created per call for simplicity per spec.
    let mut font_system = FontSystem::new();
    let mut swash_cache = SwashCache::new();

    let metrics = Metrics::new(font_px, font_px * 1.2);
    let mut buffer = Buffer::new(&mut font_system, metrics);

    buffer.set_size(Some(width as f32), Some(height as f32));

    // Default attrs use cosmic-text's sans-serif; when a (non-empty) family is
    // supplied, request it by name. Unknown names degrade gracefully via
    // cosmic-text's font matching rather than erroring.
    let mut attrs = Attrs::new();
    if let Some(name) = family.filter(|n| !n.is_empty()) {
        attrs = attrs.family(Family::Name(name));
    }
    buffer.set_text(text, &attrs, Shaping::Advanced, Some(align.to_cosmic()));
    buffer.shape_until_scroll(&mut font_system, false);

    let w = width as i32;
    let h = height as i32;

    // The draw callback hands us a rectangle (px,py,rw,rh) filled with `c`,
    // where `c.a()` is glyph coverage in 0..=255. We override rgb with the
    // caller's color and premultiply by coverage * dst_a in linear light.
    buffer.draw(
        &mut font_system,
        &mut swash_cache,
        Color::rgba(255, 255, 255, 255),
        |px, py, rw, rh, c| {
            let coverage = c.a() as f32 / 255.0;
            if coverage <= 0.0 {
                return;
            }
            let final_a = coverage * dst_a;
            if final_a <= 0.0 {
                return;
            }
            let pr = lin_r * final_a;
            let pg = lin_g * final_a;
            let pb = lin_b * final_a;

            for dy in 0..rh as i32 {
                let y = py + dy;
                if y < 0 || y >= h {
                    continue;
                }
                for dx in 0..rw as i32 {
                    let x = px + dx;
                    if x < 0 || x >= w {
                        continue;
                    }
                    let idx = ((y as usize) * (width as usize) + (x as usize)) * 4;
                    // Source-over accumulate (handles overlapping glyph rects).
                    let inv = 1.0 - final_a;
                    out[idx] = pr + out[idx] * inv;
                    out[idx + 1] = pg + out[idx + 1] * inv;
                    out[idx + 2] = pb + out[idx + 2] * inv;
                    out[idx + 3] = final_a + out[idx + 3] * inv;
                }
            }
        },
    );

    out
}

/// Enumerate the font families available to [`render_text`], as a sorted,
/// de-duplicated list of family names suitable for populating a chooser.
///
/// Names come from the same `FontSystem` (system font database) that
/// `render_text` builds, so any name returned here can be passed back as the
/// `family` argument. Building the `FontSystem` scans the system font database,
/// so callers that need this repeatedly should cache the result.
pub fn available_families() -> Vec<String> {
    let font_system = FontSystem::new();
    let mut names: Vec<String> = font_system
        .db()
        .faces()
        // Each face lists its family names as (name, language); the first is the
        // English/primary family name (see fontdb::FaceInfo::families).
        .filter_map(|face| face.families.first().map(|(name, _lang)| name.clone()))
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_glyph_alpha() {
        let buf = render_text("A", 48.0, [1.0, 1.0, 1.0, 1.0], 64, 64, TextAlign::Left, None);
        assert_eq!(buf.len(), 64 * 64 * 4);

        let lit = buf.iter().skip(3).step_by(4).filter(|&&a| a > 0.1).count();
        assert!(lit > 0, "expected some pixels with alpha > 0.1, got {lit}");
    }

    #[test]
    fn empty_text_all_zero() {
        let buf = render_text("", 48.0, [1.0, 1.0, 1.0, 1.0], 64, 64, TextAlign::Left, None);
        assert_eq!(buf.len(), 64 * 64 * 4);
        assert!(buf.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn zero_dims_empty_buffer() {
        let buf = render_text("A", 48.0, [1.0, 1.0, 1.0, 1.0], 0, 64, TextAlign::Left, None);
        assert_eq!(buf.len(), 0);
        assert!(buf.iter().all(|&v| v == 0.0));

        let buf2 = render_text("A", 48.0, [1.0, 1.0, 1.0, 1.0], 64, 0, TextAlign::Center, None);
        assert_eq!(buf2.len(), 0);
    }

    #[test]
    fn available_families_returns_sorted_unique_names() {
        let families = available_families();
        // The system font DB is environment-dependent, but cosmic-text always
        // bundles fallback faces, so the list is never empty.
        assert!(
            !families.is_empty(),
            "expected at least one available font family"
        );
        // Sorted and de-duplicated.
        assert!(
            families.windows(2).all(|w| w[0] < w[1]),
            "families must be strictly sorted and unique"
        );
    }

    #[test]
    fn honors_set_family_vs_default() {
        // Render once with the default face and once requesting a specific
        // family. Whatever the environment offers, both must produce a valid
        // buffer of the right size and actually draw glyphs.
        let default = render_text("Ag", 48.0, [1.0, 1.0, 1.0, 1.0], 96, 64, TextAlign::Left, None);
        assert_eq!(default.len(), 96 * 64 * 4);

        // Pick a real family from the database so the request is honored rather
        // than substituted; fall back to a plausible name if none is reported.
        let families = available_families();
        let chosen = families.first().cloned().unwrap_or_else(|| "Serif".into());
        let with_family = render_text(
            "Ag",
            48.0,
            [1.0, 1.0, 1.0, 1.0],
            96,
            64,
            TextAlign::Left,
            Some(&chosen),
        );
        assert_eq!(with_family.len(), 96 * 64 * 4);
        let lit = with_family
            .iter()
            .skip(3)
            .step_by(4)
            .filter(|&&a| a > 0.1)
            .count();
        assert!(lit > 0, "expected glyphs to render with family {chosen:?}");
    }

    #[test]
    fn empty_family_falls_back_to_default() {
        // An empty family name must behave exactly like `None` (default face)
        // rather than panicking or producing an empty buffer.
        let buf = render_text(
            "A",
            48.0,
            [1.0, 1.0, 1.0, 1.0],
            64,
            64,
            TextAlign::Left,
            Some(""),
        );
        assert_eq!(buf.len(), 64 * 64 * 4);
        let lit = buf.iter().skip(3).step_by(4).filter(|&&a| a > 0.1).count();
        assert!(lit > 0, "empty family should fall back and still render");
    }
}
