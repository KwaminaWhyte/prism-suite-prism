//! Gradient-domain seamless cloning (Poisson image editing, Pérez et al. 2003).
//!
//! Used by the Healing Brush: transplant a source patch's *texture* (its
//! gradients) into a destination region while matching the destination's tone
//! and color at the region boundary, so the repair blends in seamlessly instead
//! of pasting a hard-edged copy. This is app-agnostic image math — generic
//! enough to live in the shared core (PLAN.md §0a "shared-crate" rule).

/// Seamlessly clone `src` into `dest` over `mask`, returning a new RGBA buffer.
///
/// Buffers are straight (non-premultiplied) linear RGBA, length `w*h*4`; `mask`
/// is `w*h` (`true` = heal here). `src` must already be aligned to destination
/// coordinates, i.e. `src[p]` is the source pixel intended to land at `p`.
///
/// For each masked pixel the result is solved so its Laplacian matches the
/// source's, with the destination imposed as a Dirichlet boundary condition (a
/// "membrane" that carries the boundary tone offset smoothly across the region).
/// Solved by `iterations` Gauss–Seidel sweeps over the mask's bounding box. RGB
/// is healed; alpha is taken from `dest` unchanged. Pixels outside the mask are
/// returned as-is from `dest`.
pub fn seamless_clone(
    dest: &[f32],
    src: &[f32],
    mask: &[bool],
    w: usize,
    h: usize,
    iterations: usize,
) -> Vec<f32> {
    let mut out = dest.to_vec();
    if w == 0 || h == 0 {
        return out;
    }
    assert_eq!(dest.len(), w * h * 4, "dest must be w*h*4");
    assert_eq!(src.len(), w * h * 4, "src must be w*h*4");
    assert_eq!(mask.len(), w * h, "mask must be w*h");

    // Bounding box of the mask — everything else stays equal to `dest`.
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    let mut any = false;
    for y in 0..h {
        for x in 0..w {
            if mask[y * w + x] {
                any = true;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if !any {
        return out;
    }

    // Seed masked pixels with the source value (converges faster than from dest).
    for y in y0..=y1 {
        for x in x0..=x1 {
            let i = y * w + x;
            if mask[i] {
                out[i * 4] = src[i * 4];
                out[i * 4 + 1] = src[i * 4 + 1];
                out[i * 4 + 2] = src[i * 4 + 2];
            }
        }
    }

    // Gauss–Seidel: f(p) = ( Σ_q b(q) + Σ_q (src(p) − src(q)) ) / |N(p)|,
    // b(q) = f(q) if q masked else dest(q); N(p) = in-bounds 4-neighborhood.
    for _ in 0..iterations {
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = y * w + x;
                if !mask[i] {
                    continue;
                }
                let neighbors = [
                    if x > 0 { Some(i - 1) } else { None },
                    if x + 1 < w { Some(i + 1) } else { None },
                    if y > 0 { Some(i - w) } else { None },
                    if y + 1 < h { Some(i + w) } else { None },
                ];
                for c in 0..3 {
                    let sp = src[i * 4 + c];
                    let mut sum = 0.0f32;
                    let mut n = 0.0f32;
                    for nb in neighbors.into_iter().flatten() {
                        let bq = if mask[nb] {
                            out[nb * 4 + c]
                        } else {
                            dest[nb * 4 + c]
                        };
                        sum += bq + (sp - src[nb * 4 + c]);
                        n += 1.0;
                    }
                    if n > 0.0 {
                        out[i * 4 + c] = sum / n;
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(w: usize, h: usize, rgba: [f32; 4]) -> Vec<f32> {
        let mut v = Vec::with_capacity(w * h * 4);
        for _ in 0..w * h {
            v.extend_from_slice(&rgba);
        }
        v
    }

    #[test]
    fn identical_source_leaves_image_unchanged() {
        let (w, h) = (8, 8);
        let dest = fill(w, h, [0.4, 0.6, 0.2, 1.0]);
        let src = dest.clone();
        let mut mask = vec![false; w * h];
        for y in 2..6 {
            for x in 2..6 {
                mask[y * w + x] = true;
            }
        }
        let out = seamless_clone(&dest, &src, &mask, w, h, 100);
        for (i, (&o, &d)) in out.iter().zip(dest.iter()).enumerate() {
            assert!((o - d).abs() < 1e-3, "idx {i}: {o} vs {d}");
        }
    }

    #[test]
    fn constant_offset_source_matches_dest_tone() {
        // A uniformly brighter source patch should heal to the destination tone
        // (membrane absorbs the constant offset) — the signature heal property.
        let (w, h) = (10, 10);
        let dest = fill(w, h, [0.5, 0.5, 0.5, 1.0]);
        let src = fill(w, h, [0.85, 0.85, 0.85, 1.0]); // +0.35 everywhere
        let mut mask = vec![false; w * h];
        for y in 3..7 {
            for x in 3..7 {
                mask[y * w + x] = true;
            }
        }
        let out = seamless_clone(&dest, &src, &mask, w, h, 400);
        // Center masked pixel should be pulled back near the dest tone (0.5),
        // NOT the source's 0.85.
        let c = (5 * w + 5) * 4;
        assert!(
            (out[c] - 0.5).abs() < 0.03,
            "healed center should match dest tone 0.5, got {}",
            out[c]
        );
    }

    #[test]
    fn outside_mask_is_untouched() {
        let (w, h) = (8, 8);
        let dest = fill(w, h, [0.3, 0.3, 0.3, 1.0]);
        let src = fill(w, h, [0.9, 0.1, 0.1, 1.0]);
        let mut mask = vec![false; w * h];
        mask[4 * w + 4] = true;
        let out = seamless_clone(&dest, &src, &mask, w, h, 50);
        // A corner well outside the mask must equal dest exactly.
        let p = 0;
        assert_eq!(out[p * 4], dest[p * 4]);
        assert_eq!(out[p * 4 + 1], dest[p * 4 + 1]);
    }

    #[test]
    fn texture_variation_transfers_while_tone_matches() {
        // Flat gray dest; source carries a left-dark/right-bright gradient with
        // mean ~0.5. After heal the interior keeps relative variation (left <
        // right) but the overall tone stays near dest.
        let (w, h) = (12, 6);
        let dest = fill(w, h, [0.5, 0.5, 0.5, 1.0]);
        let mut src = fill(w, h, [0.0, 0.0, 0.0, 1.0]);
        for y in 0..h {
            for x in 0..w {
                let g = x as f32 / (w - 1) as f32; // 0..1 left→right
                let p = (y * w + x) * 4;
                src[p] = g;
                src[p + 1] = g;
                src[p + 2] = g;
            }
        }
        let mut mask = vec![false; w * h];
        for y in 1..5 {
            for x in 3..9 {
                mask[y * w + x] = true;
            }
        }
        let out = seamless_clone(&dest, &src, &mask, w, h, 600);
        let left = out[(3 * w + 3) * 4];
        let right = out[(3 * w + 8) * 4];
        assert!(right > left, "gradient direction preserved: {left} !< {right}");
        let mid = out[(3 * w + 5) * 4];
        assert!((mid - 0.5).abs() < 0.1, "mid tone stays near dest 0.5: {mid}");
    }
}
