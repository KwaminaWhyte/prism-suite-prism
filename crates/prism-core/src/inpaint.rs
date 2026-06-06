//! Content-aware fill by PatchMatch (Barnes et al. 2009) — fill a masked hole
//! with texture synthesized from the surrounding image, so removed objects /
//! blemishes are replaced by plausible surroundings rather than a flat patch.
//!
//! Single-resolution inpainting variant: an approximate nearest-neighbor field
//! (NNF) maps each hole pixel to a source patch in the *known* region, refined by
//! propagation + random search, with the hole reconstructed by patch voting each
//! iteration (EM-style). App-agnostic image math, shared via `prism-core`.

/// Deterministic xorshift32 PRNG (no external rng dep; reproducible fills/tests).
struct Rng(u32);
impl Rng {
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    /// Uniform in `[0, n)`.
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u32() as usize) % n
        }
    }
}

/// Content-aware fill of the `mask` region (`true` = fill) using PatchMatch.
///
/// `image` is straight linear RGBA (`w*h*4`). `patch_radius` sets the patch size
/// (`2r+1` square; 3–4 is typical). `iterations` is the number of PatchMatch
/// EM sweeps (5–8 typical). Returns a new buffer with the hole synthesized from
/// surrounding known texture; non-hole pixels are unchanged. Fully deterministic.
pub fn content_aware_fill(
    image: &[f32],
    mask: &[bool],
    w: usize,
    h: usize,
    patch_radius: usize,
    iterations: usize,
) -> Vec<f32> {
    assert_eq!(image.len(), w * h * 4);
    assert_eq!(mask.len(), w * h);
    let mut out = image.to_vec();
    if w == 0 || h == 0 {
        return out;
    }

    // Hole pixels + the known-pixel pool (valid source patch centers).
    let holes: Vec<usize> = (0..w * h).filter(|&i| mask[i]).collect();
    let known: Vec<usize> = (0..w * h).filter(|&i| !mask[i]).collect();
    if holes.is_empty() || known.is_empty() {
        return out;
    }

    let r = patch_radius as i64;
    let (wi, hi) = (w as i64, h as i64);
    let mut rng = Rng(0x9E3779B9);

    // Seed the hole with random known colors so patch distances are meaningful.
    for &q in &holes {
        let s = known[rng.below(known.len())];
        for c in 0..3 {
            out[q * 4 + c] = image[s * 4 + c];
        }
    }

    // NNF: for each hole pixel, a source *center* in the known region.
    let mut ann = vec![0usize; w * h];
    for &q in &holes {
        ann[q] = known[rng.below(known.len())];
    }

    // Patch distance between target center `t` (uses current `out`) and source
    // center `s` (uses original `image`), summed over the patch; source samples
    // must be in-bounds and known. Returns f32::MAX if no overlap.
    let patch_dist = |out: &[f32], t: usize, s: usize| -> f32 {
        let (tx, ty) = ((t % w) as i64, (t / w) as i64);
        let (sx, sy) = ((s % w) as i64, (s / w) as i64);
        let mut sum = 0.0f32;
        let mut n = 0.0f32;
        for dy in -r..=r {
            for dx in -r..=r {
                let (tnx, tny) = (tx + dx, ty + dy);
                let (snx, sny) = (sx + dx, sy + dy);
                if tnx < 0 || tny < 0 || tnx >= wi || tny >= hi {
                    continue;
                }
                if snx < 0 || sny < 0 || snx >= wi || sny >= hi {
                    continue;
                }
                let sp = (sny * wi + snx) as usize;
                if mask[sp] {
                    continue; // source must be known texture
                }
                let tp = (tny * wi + tnx) as usize;
                for c in 0..3 {
                    let d = out[tp * 4 + c] - image[sp * 4 + c];
                    sum += d * d;
                }
                n += 1.0;
            }
        }
        if n < 1.0 {
            f32::MAX
        } else {
            sum / n
        }
    };

    for it in 0..iterations.max(1) {
        // Propagation + random search. Alternate scan direction by iteration.
        let forward = it % 2 == 0;
        let order: Vec<usize> = if forward {
            holes.clone()
        } else {
            holes.iter().rev().copied().collect()
        };
        for &q in &order {
            let (qx, qy) = ((q % w) as i64, (q / w) as i64);
            let mut best = ann[q];
            let mut best_d = patch_dist(&out, q, best);

            // Propagate: try the source implied by an already-scanned neighbor.
            let step = if forward { -1 } else { 1 };
            for &(nx, ny) in &[(qx + step, qy), (qx, qy + step)] {
                if nx < 0 || ny < 0 || nx >= wi || ny >= hi {
                    continue;
                }
                let np = (ny * wi + nx) as usize;
                if !mask[np] {
                    continue;
                }
                // Neighbor's source shifted back toward q.
                let ns = ann[np] as i64;
                let cand = ns + (q as i64 - np as i64);
                if cand < 0 || cand >= (w * h) as i64 {
                    continue;
                }
                let cand = cand as usize;
                if mask[cand] {
                    continue;
                }
                let d = patch_dist(&out, q, cand);
                if d < best_d {
                    best_d = d;
                    best = cand;
                }
            }

            // Random search: shrinking windows around the current best.
            let (mut bx, mut by) = ((best % w) as i64, (best / w) as i64);
            let mut radius = wi.max(hi);
            while radius >= 1 {
                let rx = bx + (rng.below((2 * radius + 1) as usize) as i64 - radius);
                let ry = by + (rng.below((2 * radius + 1) as usize) as i64 - radius);
                if rx >= 0 && ry >= 0 && rx < wi && ry < hi {
                    let cand = (ry * wi + rx) as usize;
                    if !mask[cand] {
                        let d = patch_dist(&out, q, cand);
                        if d < best_d {
                            best_d = d;
                            best = cand;
                            bx = rx;
                            by = ry;
                        }
                    }
                }
                radius /= 2;
            }
            ann[q] = best;
        }

        // Vote: each hole pixel = average of overlapping source-patch predictions.
        let mut acc = vec![0.0f32; w * h * 3];
        let mut cnt = vec![0.0f32; w * h];
        for &p in &holes {
            let (px, py) = ((p % w) as i64, (p / w) as i64);
            let (sx, sy) = ((ann[p] % w) as i64, (ann[p] / w) as i64);
            for dy in -r..=r {
                for dx in -r..=r {
                    let (qx, qy) = (px + dx, py + dy);
                    if qx < 0 || qy < 0 || qx >= wi || qy >= hi {
                        continue;
                    }
                    let q = (qy * wi + qx) as usize;
                    if !mask[q] {
                        continue; // only rebuild hole pixels
                    }
                    let (snx, sny) = (sx + dx, sy + dy);
                    if snx < 0 || sny < 0 || snx >= wi || sny >= hi {
                        continue;
                    }
                    let sp = (sny * wi + snx) as usize;
                    if mask[sp] {
                        continue;
                    }
                    for c in 0..3 {
                        acc[q * 3 + c] += image[sp * 4 + c];
                    }
                    cnt[q] += 1.0;
                }
            }
        }
        for &q in &holes {
            if cnt[q] > 0.0 {
                for c in 0..3 {
                    out[q * 4 + c] = acc[q * 3 + c] / cnt[q];
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

    fn hole_rect(mask: &mut [bool], w: usize, x0: usize, y0: usize, x1: usize, y1: usize) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                mask[y * w + x] = true;
            }
        }
    }

    #[test]
    fn uniform_region_fills_with_that_color() {
        let (w, h) = (20, 20);
        let img = fill(w, h, [0.42, 0.42, 0.42, 1.0]);
        let mut mask = vec![false; w * h];
        hole_rect(&mut mask, w, 8, 8, 11, 11);
        let out = content_aware_fill(&img, &mask, w, h, 3, 6);
        for y in 8..=11 {
            for x in 8..=11 {
                let p = (y * w + x) * 4;
                assert!(
                    (out[p] - 0.42).abs() < 1e-3,
                    "hole ({x},{y}) should fill 0.42, got {}",
                    out[p]
                );
            }
        }
    }

    #[test]
    fn fill_picks_surrounding_texture_not_far_region() {
        // Left 2/3 = color A (0.8,0.2,0.2), right 1/3 = B (0.1,0.1,0.7). A hole
        // well inside the A region should fill with A (surrounded by A texture),
        // not B.
        let (w, h) = (30, 16);
        let mut img = fill(w, h, [0.0, 0.0, 0.0, 1.0]);
        for y in 0..h {
            for x in 0..w {
                let p = (y * w + x) * 4;
                let col = if x < 20 {
                    [0.8, 0.2, 0.2]
                } else {
                    [0.1, 0.1, 0.7]
                };
                img[p] = col[0];
                img[p + 1] = col[1];
                img[p + 2] = col[2];
            }
        }
        let mut mask = vec![false; w * h];
        hole_rect(&mut mask, w, 8, 6, 11, 9);
        let out = content_aware_fill(&img, &mask, w, h, 3, 8);
        let c = (7 * w + 9) * 4; // a hole pixel
        assert!(
            out[c] > 0.6 && out[c + 2] < 0.4,
            "hole inside region A should fill reddish A, got [{},{},{}]",
            out[c],
            out[c + 1],
            out[c + 2]
        );
    }

    #[test]
    fn deterministic_across_runs() {
        let (w, h) = (24, 24);
        let mut img = fill(w, h, [0.3, 0.5, 0.7, 1.0]);
        // Add some variation so the NNF isn't trivial.
        for y in 0..h {
            for x in 0..w {
                let p = (y * w + x) * 4;
                img[p] = ((x ^ y) & 7) as f32 / 7.0;
            }
        }
        let mut mask = vec![false; w * h];
        hole_rect(&mut mask, w, 10, 10, 13, 13);
        let a = content_aware_fill(&img, &mask, w, h, 3, 5);
        let b = content_aware_fill(&img, &mask, w, h, 3, 5);
        assert_eq!(a, b, "fill must be deterministic for a fixed seed");
    }

    #[test]
    fn empty_or_full_mask_is_noop() {
        let (w, h) = (8, 8);
        let img = fill(w, h, [0.2, 0.4, 0.6, 1.0]);
        let none = vec![false; w * h];
        assert_eq!(content_aware_fill(&img, &none, w, h, 2, 3), img);
        let all = vec![true; w * h]; // no known source -> unchanged
        assert_eq!(content_aware_fill(&img, &all, w, h, 2, 3), img);
    }
}
