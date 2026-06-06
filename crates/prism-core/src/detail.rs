//! Local blur / sharpen brushing — neighborhood ops weighted per pixel by a
//! coverage `amount`, for Pigment's Blur and Sharpen tools (PLAN.md §6). Blur
//! lerps a pixel toward its 3×3 neighborhood mean; sharpen adds back the
//! high-frequency residual (unsharp-style). App-agnostic image math.

/// Apply a local blur (`sharpen = false`) or sharpen (`true`) to straight RGBA
/// `image` (`w*h*4`), weighted per pixel by `amount` (`w*h`, 0..1 coverage).
/// The 3×3 mean is taken from the original `image` (edge-clamped) so updates
/// don't feed back. Alpha unchanged.
pub fn blur_sharpen(
    image: &[f32],
    amount: &[f32],
    w: usize,
    h: usize,
    sharpen: bool,
) -> Vec<f32> {
    assert_eq!(image.len(), w * h * 4);
    assert_eq!(amount.len(), w * h);
    let mut out = image.to_vec();
    let (wi, hi) = (w as i64, h as i64);
    let mean = |x: i64, y: i64, c: usize| -> f32 {
        let mut s = 0.0;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let nx = (x + dx).clamp(0, wi - 1);
                let ny = (y + dy).clamp(0, hi - 1);
                s += image[((ny * wi + nx) as usize) * 4 + c];
            }
        }
        s / 9.0
    };
    for y in 0..h {
        for x in 0..w {
            let p = y * w + x;
            let amt = amount[p].clamp(0.0, 1.0);
            if amt <= 0.0 {
                continue;
            }
            for c in 0..3 {
                let v = image[p * 4 + c];
                let m = mean(x as i64, y as i64, c);
                out[p * 4 + c] = if sharpen {
                    (v + (v - m) * amt).clamp(0.0, 1.0)
                } else {
                    v + (m - v) * amt
                };
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spike(w: usize, h: usize) -> Vec<f32> {
        // Single bright center pixel on black; alpha 1.
        let mut v = vec![0.0f32; w * h * 4];
        for p in 0..w * h {
            v[p * 4 + 3] = 1.0;
        }
        let c = (h / 2 * w + w / 2) * 4;
        v[c] = 1.0;
        v[c + 1] = 1.0;
        v[c + 2] = 1.0;
        v
    }

    #[test]
    fn blur_lowers_the_spike() {
        let (w, h) = (3, 3);
        let img = spike(w, h);
        let amt = vec![1.0; w * h];
        let out = blur_sharpen(&img, &amt, w, h, false);
        let c = (1 * w + 1) * 4;
        // Center mean over 3x3 = 1/9; full blur pulls center down toward it.
        assert!(out[c] < 0.2, "blur lowers spike, got {}", out[c]);
        // A neighbor rises off zero.
        let nb = (1 * w + 0) * 4;
        assert!(out[nb] > 0.0, "neighbor picks up blur, got {}", out[nb]);
    }

    #[test]
    fn sharpen_does_not_lower_the_spike() {
        let (w, h) = (3, 3);
        let img = spike(w, h);
        let amt = vec![1.0; w * h];
        let out = blur_sharpen(&img, &amt, w, h, true);
        let c = (1 * w + 1) * 4;
        // Center is already a peak; sharpen pushes it further (clamped at 1).
        assert!((out[c] - 1.0).abs() < 1e-6, "sharpen keeps peak high, got {}", out[c]);
    }

    #[test]
    fn zero_amount_noop() {
        let (w, h) = (4, 4);
        let img = spike(w, h);
        let amt = vec![0.0; w * h];
        assert_eq!(blur_sharpen(&img, &amt, w, h, false), img);
        assert_eq!(blur_sharpen(&img, &amt, w, h, true), img);
    }
}
