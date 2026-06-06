//! Liquify-style mesh warping via a per-pixel displacement field. `disp[p]` is a
//! *sample offset*: the warped result at `p` reads the source at `p + disp[p]`,
//! so brushes accumulate into `disp` and the image is resampled once from the
//! original (no compounding blur). App-agnostic image math (PLAN.md §6 Liquify).

/// Smooth radial falloff: 1 at the brush center, 0 at radius `r` (smoothstep).
fn falloff(d: f32, r: f32) -> f32 {
    if r <= 0.0 || d >= r {
        return 0.0;
    }
    let t = 1.0 - d / r;
    t * t * (3.0 - 2.0 * t)
}

/// Resample `image` (straight RGBA, `w*h*4`) through the sample-offset field
/// `disp` (`w*h` of `[dx, dy]`): `out[p] = bilinear(image, p + disp[p])`, clamped
/// at the edges. Returns a new buffer.
pub fn apply_displacement(image: &[f32], disp: &[[f32; 2]], w: usize, h: usize) -> Vec<f32> {
    assert_eq!(image.len(), w * h * 4);
    assert_eq!(disp.len(), w * h);
    let mut out = vec![0.0f32; w * h * 4];
    let sample = |x: f32, y: f32, c: usize| -> f32 {
        let xf = x.clamp(0.0, (w - 1) as f32);
        let yf = y.clamp(0.0, (h - 1) as f32);
        let x0 = xf.floor() as usize;
        let y0 = yf.floor() as usize;
        let x1 = (x0 + 1).min(w - 1);
        let y1 = (y0 + 1).min(h - 1);
        let tx = xf - x0 as f32;
        let ty = yf - y0 as f32;
        let a = image[(y0 * w + x0) * 4 + c];
        let b = image[(y0 * w + x1) * 4 + c];
        let cc = image[(y1 * w + x0) * 4 + c];
        let dd = image[(y1 * w + x1) * 4 + c];
        let top = a + (b - a) * tx;
        let bot = cc + (dd - cc) * tx;
        top + (bot - top) * ty
    };
    for y in 0..h {
        for x in 0..w {
            let p = y * w + x;
            let sx = x as f32 + disp[p][0];
            let sy = y as f32 + disp[p][1];
            for c in 0..4 {
                out[p * 4 + c] = sample(sx, sy, c);
            }
        }
    }
    out
}

/// Iterate the integer pixels of the brush disk `(cx,cy,r)`, calling `f(p, fall)`
/// with the falloff weight at each in-bounds pixel.
fn brush<F: FnMut(usize, f32, f32, f32)>(
    w: usize,
    h: usize,
    cx: f32,
    cy: f32,
    r: f32,
    mut f: F,
) {
    let x0 = (cx - r).floor().max(0.0) as usize;
    let y0 = (cy - r).floor().max(0.0) as usize;
    let x1 = ((cx + r).ceil() as usize).min(w.saturating_sub(1));
    let y1 = ((cy + r).ceil() as usize).min(h.saturating_sub(1));
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            let fall = falloff(d, r);
            if fall > 0.0 {
                f(y * w + x, fall, dx, dy);
            }
        }
    }
}

/// Push (forward-warp): drag pixels in the brush by `(mvx,mvy)`. Pixels show what
/// was *behind* them, so the sample offset accumulates `-mv * fall * strength`.
pub fn stamp_push(
    disp: &mut [[f32; 2]],
    w: usize,
    h: usize,
    cx: f32,
    cy: f32,
    r: f32,
    mvx: f32,
    mvy: f32,
    strength: f32,
) {
    brush(w, h, cx, cy, r, |p, fall, _dx, _dy| {
        disp[p][0] -= mvx * fall * strength;
        disp[p][1] -= mvy * fall * strength;
    });
}

/// Twirl: rotate pixels about the brush center by `angle` rad (scaled by falloff).
pub fn stamp_twirl(
    disp: &mut [[f32; 2]],
    w: usize,
    h: usize,
    cx: f32,
    cy: f32,
    r: f32,
    angle: f32,
    strength: f32,
) {
    brush(w, h, cx, cy, r, |p, fall, dx, dy| {
        // Sample from the pre-rotated position (rotate source by -a).
        let a = -angle * fall * strength;
        let (s, c) = a.sin_cos();
        let rx = c * dx - s * dy;
        let ry = s * dx + c * dy;
        disp[p][0] += rx - dx;
        disp[p][1] += ry - dy;
    });
}

/// Pucker (pull toward center) for `sign > 0`, Bloat (push outward) for `sign < 0`.
pub fn stamp_pinch(
    disp: &mut [[f32; 2]],
    w: usize,
    h: usize,
    cx: f32,
    cy: f32,
    r: f32,
    sign: f32,
    strength: f32,
) {
    brush(w, h, cx, cy, r, |p, fall, dx, dy| {
        // Pucker: pixels move inward, so sample further out (offset along +radial).
        let k = sign * fall * strength;
        disp[p][0] += dx * k;
        disp[p][1] += dy * k;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(w: usize, h: usize) -> Vec<f32> {
        // Red channel = x/(w-1) horizontal ramp; alpha 1.
        let mut v = vec![0.0f32; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let p = (y * w + x) * 4;
                v[p] = x as f32 / (w - 1) as f32;
                v[p + 3] = 1.0;
            }
        }
        v
    }

    #[test]
    fn identity_displacement_unchanged() {
        let (w, h) = (8, 8);
        let img = ramp(w, h);
        let disp = vec![[0.0, 0.0]; w * h];
        let out = apply_displacement(&img, &disp, w, h);
        for (i, (&o, &v)) in out.iter().zip(img.iter()).enumerate() {
            assert!((o - v).abs() < 1e-6, "idx {i}");
        }
    }

    #[test]
    fn constant_offset_shifts_sampling() {
        // disp +2 in x means out[x] samples img[x+2] -> ramp value shifts left.
        let (w, h) = (10, 4);
        let img = ramp(w, h);
        let disp = vec![[2.0, 0.0]; w * h];
        let out = apply_displacement(&img, &disp, w, h);
        let at = |x: usize, y: usize| out[(y * w + x) * 4];
        assert!((at(3, 1) - 5.0 / 9.0).abs() < 1e-5, "got {}", at(3, 1));
    }

    #[test]
    fn push_offsets_against_motion() {
        let (w, h) = (16, 16);
        let mut disp = vec![[0.0, 0.0]; w * h];
        stamp_push(&mut disp, w, h, 8.0, 8.0, 5.0, 4.0, 0.0, 1.0);
        // Center sample offset points opposite the motion (so pixels appear pushed).
        let c = 8 * w + 8;
        assert!(disp[c][0] < 0.0, "push +x -> sample offset -x, got {}", disp[c][0]);
        // Outside the brush radius: untouched.
        assert_eq!(disp[0], [0.0, 0.0]);
    }

    #[test]
    fn pinch_pucker_vs_bloat_signs() {
        let (w, h) = (16, 16);
        let mut pucker = vec![[0.0, 0.0]; w * h];
        let mut bloat = vec![[0.0, 0.0]; w * h];
        stamp_pinch(&mut pucker, w, h, 8.0, 8.0, 6.0, 1.0, 1.0);
        stamp_pinch(&mut bloat, w, h, 8.0, 8.0, 6.0, -1.0, 1.0);
        // A pixel to the right of center (dx>0): pucker samples further out (+x),
        // bloat samples inward (−x).
        let p = 8 * w + 11;
        assert!(pucker[p][0] > 0.0, "pucker right-of-center offset +x");
        assert!(bloat[p][0] < 0.0, "bloat right-of-center offset −x");
    }
}
