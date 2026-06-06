//! Dodge & burn — local tonal brushing in linear light. `amount[p] > 0` dodges
//! (lightens toward white), `< 0` burns (darkens toward black); magnitude is the
//! per-pixel strength in `[-1, 1]`. App-agnostic pixel math (PLAN.md §6 retouch).

/// Apply a signed per-pixel dodge/burn `amount` to straight-RGBA `image`
/// (`w*h*4`); `amount` is `w*h`. Returns a new buffer; alpha is unchanged. The
/// mapping stays within `[0, 1]` for any `amount ∈ [-1, 1]`: dodge lerps the
/// channel toward 1, burn scales it toward 0.
pub fn dodge_burn(image: &[f32], amount: &[f32], w: usize, h: usize) -> Vec<f32> {
    assert_eq!(image.len(), w * h * 4);
    assert_eq!(amount.len(), w * h);
    let mut out = image.to_vec();
    for p in 0..w * h {
        let a = amount[p].clamp(-1.0, 1.0);
        if a == 0.0 {
            continue;
        }
        for c in 0..3 {
            let v = out[p * 4 + c];
            out[p * 4 + c] = if a > 0.0 {
                v + (1.0 - v) * a // dodge toward white
            } else {
                v + v * a // burn toward black (a < 0)
            };
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(rgba: [f32; 4]) -> Vec<f32> {
        rgba.to_vec()
    }

    #[test]
    fn dodge_lightens() {
        let img = px([0.4, 0.4, 0.4, 1.0]);
        let out = dodge_burn(&img, &[0.5], 1, 1);
        assert!((out[0] - 0.7).abs() < 1e-5, "0.4 dodge 0.5 -> 0.7, got {}", out[0]);
        assert_eq!(out[3], 1.0, "alpha unchanged");
    }

    #[test]
    fn burn_darkens() {
        let img = px([0.4, 0.4, 0.4, 1.0]);
        let out = dodge_burn(&img, &[-0.5], 1, 1);
        assert!((out[0] - 0.2).abs() < 1e-5, "0.4 burn 0.5 -> 0.2, got {}", out[0]);
    }

    #[test]
    fn zero_amount_is_noop() {
        let img = px([0.3, 0.6, 0.9, 0.5]);
        let out = dodge_burn(&img, &[0.0], 1, 1);
        assert_eq!(out, img);
    }

    #[test]
    fn stays_in_unit_range_at_extremes() {
        let img = vec![0.0, 0.5, 1.0, 1.0, 0.2, 0.8, 0.3, 1.0];
        let out = dodge_burn(&img, &[1.0, -1.0], 2, 1);
        for (i, &v) in out.iter().enumerate() {
            assert!((0.0..=1.0).contains(&v), "out[{i}]={v} out of range");
        }
        // full dodge -> white, full burn -> black (rgb).
        assert!((out[0] - 1.0).abs() < 1e-6 && (out[2] - 1.0).abs() < 1e-6);
        assert!(out[4] < 1e-6 && out[6] < 1e-6);
    }
}
