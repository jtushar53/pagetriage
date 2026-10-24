//! Blur as the variance of the Laplacian over the text mask.
//!
//! Variance of the Laplacian as a focus measure: J. L. Pech-Pacheco, G. Cristobal, J. Chamorro-
//! Martinez, J. Fernandez-Valdivia, "Diatom autofocusing in brightfield microscopy: a comparative
//! study", ICPR 2000 (recalled from memory, unverified). The 4-neighbour Laplacian of an 8-bit
//! image is an integer in [-1020, 1020], so its sums are exact and the result is identical on every
//! target. The value depends on scan resolution and on contrast (it scales with contrast squared),
//! so the grade uses a contrast-normalised form.

/// Sums of the 4-neighbour Laplacian and of the squared central-difference gradient over the
/// interior pixels where a mask is set. Exact integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Moments {
    pub pixels: u64,
    pub lap_sum: i64,
    pub lap_sq: u64,
    /// Sum of `(E - W)^2 + (S - N)^2`, which is four times the squared gradient.
    pub grad_sq4: u64,
}

pub fn moments(gray: &[u8], width: usize, height: usize, mask: &[u8]) -> Moments {
    debug_assert_eq!(gray.len(), width * height);
    let mut m = Moments::default();
    if width < 3 || height < 3 {
        return m;
    }
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let i = y * width + x;
            if mask[i] == 0 {
                continue;
            }
            let (w, e) = (i32::from(gray[i - 1]), i32::from(gray[i + 1]));
            let (nn, ss) = (i32::from(gray[i - width]), i32::from(gray[i + width]));
            let lap = i64::from(w + e + nn + ss - 4 * i32::from(gray[i]));
            let (gx, gy) = (i64::from(e - w), i64::from(ss - nn));
            m.pixels += 1;
            m.lap_sum += lap;
            m.lap_sq += (lap * lap) as u64;
            m.grad_sq4 += (gx * gx + gy * gy) as u64;
        }
    }
    m
}

/// Focus statistics of the text mask.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlurStats {
    /// Pixels counted.
    pub pixels: u64,
    /// Variance of the 4-neighbour Laplacian (0 for an empty mask).
    pub variance: f64,
    /// Edge sharpness in 1/pixel: `2 sqrt(sum L^2 / sum ((E-W)^2 + (S-N)^2))`, the ratio of the
    /// Laplacian to the gradient. For a step edge blurred by a Gaussian of standard deviation
    /// `s` pixels it equals `1 / (sqrt(2) s)`, and it does not depend on contrast or on how many
    /// edge pixels there are. 0 when there is no gradient at all.
    pub sharpness_raw: f64,
    /// The same with the noise floor removed: the mean `L^2` and mean gradient power measured on
    /// clear paper are subtracted from the text mask's before the ratio is taken. Sensor noise
    /// otherwise makes every blurred page look sharp. `None` when the text's Laplacian power does
    /// not exceed the noise power by at least `min_snr` times it (the edges are no stronger than
    /// the noise), which is read as "very blurred".
    pub sharpness: Option<f64>,
    /// Text Laplacian power above the noise floor, over the noise floor (0 when there is none).
    pub snr: f64,
}

/// Laplacian variance and edge sharpness over the text mask, with the noise floor taken from the
/// pixels of `paper_mask` (clear paper, away from any ink).
pub fn focus_stats(
    gray: &[u8],
    width: usize,
    height: usize,
    mask: &[u8],
    paper_mask: &[u8],
    min_snr: f64,
) -> BlurStats {
    let t = moments(gray, width, height, mask);
    if t.pixels == 0 {
        return BlurStats {
            pixels: 0,
            variance: 0.0,
            sharpness_raw: 0.0,
            sharpness: Some(0.0),
            snr: 0.0,
        };
    }
    let n = t.pixels as f64;
    let numerator =
        i128::from(t.pixels) * i128::from(t.lap_sq) - i128::from(t.lap_sum) * i128::from(t.lap_sum);
    let variance = numerator as f64 / (n * n);
    let ratio = |lap: f64, grad4: f64| {
        if grad4 > 0.0 {
            2.0 * (lap / grad4).sqrt()
        } else {
            0.0
        }
    };
    let sharpness_raw = ratio(t.lap_sq as f64, t.grad_sq4 as f64);

    let q = moments(gray, width, height, paper_mask);
    let (lap_t, grad_t) = (t.lap_sq as f64 / n, t.grad_sq4 as f64 / n);
    let (lap_q, grad_q) = if q.pixels > 0 {
        (
            q.lap_sq as f64 / q.pixels as f64,
            q.grad_sq4 as f64 / q.pixels as f64,
        )
    } else {
        (0.0, 0.0)
    };
    let (lap, grad) = (lap_t - lap_q, grad_t - grad_q);
    let snr = if lap_q > 0.0 {
        (lap / lap_q).max(0.0)
    } else if lap > 0.0 {
        f64::MAX
    } else {
        0.0
    };
    let sharpness = if lap > 0.0 && grad > 0.0 && snr >= min_snr {
        Some(ratio(lap, grad))
    } else {
        None
    };
    BlurStats {
        pixels: t.pixels,
        variance,
        sharpness_raw,
        sharpness,
        snr,
    }
}

/// Mark every pixel within `radius` (Chebyshev distance) of a set pixel of `src`. Two linear
/// passes (rows, then columns) of "distance to the nearest set pixel", so the cost does not
/// depend on the radius.
pub fn dilate(src: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    if radius == 0 {
        return src.iter().map(|&v| u8::from(v != 0)).collect();
    }
    // Horizontal: out[y][x] = 1 when some set pixel of row y lies within `radius` columns.
    let mut rows = vec![0u8; src.len()];
    for y in 0..height {
        let row = &src[y * width..(y + 1) * width];
        let out = &mut rows[y * width..(y + 1) * width];
        let mut last: Option<usize> = None;
        for (x, &v) in row.iter().enumerate() {
            if v != 0 {
                last = Some(x);
            }
            if last.is_some_and(|l| x - l <= radius) {
                out[x] = 1;
            }
        }
        let mut next: Option<usize> = None;
        for x in (0..width).rev() {
            if row[x] != 0 {
                next = Some(x);
            }
            if next.is_some_and(|n| n - x <= radius) {
                out[x] = 1;
            }
        }
    }
    // Vertical: the same on columns of `rows`.
    let mut out = vec![0u8; src.len()];
    let mut last = vec![usize::MAX; width];
    for y in 0..height {
        for x in 0..width {
            if rows[y * width + x] != 0 {
                last[x] = y;
            }
            if last[x] != usize::MAX && y - last[x] <= radius {
                out[y * width + x] = 1;
            }
        }
    }
    let mut next = vec![usize::MAX; width];
    for y in (0..height).rev() {
        for x in 0..width {
            if rows[y * width + x] != 0 {
                next[x] = y;
            }
            if next[x] != usize::MAX && next[x] - y <= radius {
                out[y * width + x] = 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step_image(w: usize, h: usize, soft: bool) -> Vec<u8> {
        let mut g = vec![200u8; w * h];
        for y in 0..h {
            for x in 0..w {
                g[y * w + x] = if x < w / 2 - 1 {
                    200
                } else if x > w / 2 {
                    40
                } else if soft {
                    // Two intermediate columns: a blurred edge.
                    if x == w / 2 - 1 { 140 } else { 100 }
                } else {
                    // Hard edge: both middle columns jump at once.
                    if x == w / 2 - 1 { 200 } else { 40 }
                };
            }
        }
        g
    }

    #[test]
    fn sharp_edge_scores_higher_than_a_blurred_one() {
        let (w, h) = (20, 10);
        let mask = vec![1u8; w * h];
        let sharp = focus_stats(
            &step_image(w, h, false),
            w,
            h,
            &mask,
            &vec![0u8; w * h],
            1.0,
        );
        let soft = focus_stats(&step_image(w, h, true), w, h, &mask, &vec![0u8; w * h], 1.0);
        assert!(
            sharp.variance > soft.variance * 2.0,
            "{sharp:?} vs {soft:?}"
        );
        assert!(
            sharp.sharpness_raw > soft.sharpness_raw,
            "{sharp:?} vs {soft:?}"
        );
    }

    #[test]
    fn flat_and_empty_masks_give_zero() {
        let (w, h) = (8, 8);
        let flat = vec![99u8; w * h];
        let none = vec![0u8; w * h];
        let on = focus_stats(&flat, w, h, &vec![1u8; w * h], &none, 1.0);
        assert_eq!((on.variance, on.pixels, on.sharpness_raw), (0.0, 36, 0.0));
        assert_eq!(on.sharpness, None, "no gradient left after the noise floor");
        assert_eq!(focus_stats(&flat, w, h, &none, &none, 1.0).pixels, 0);
        assert_eq!(
            focus_stats(&[1, 2, 3, 4], 2, 2, &[1, 1, 1, 1], &[0; 4], 1.0).pixels,
            0
        );
    }

    #[test]
    fn sensor_noise_is_subtracted_from_the_sharpness() {
        // A soft edge (sigma about 1.7 px) buried in white noise. Raw sharpness is dominated by the
        // noise; with the noise floor taken from a clean-paper strip it recovers the softness.
        let (w, h) = (80usize, 40usize);
        let mut state = 99u32;
        let mut noise = || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            f64::from((state >> 16) % 3) - 1.0
        };
        let mut g = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                // Logistic ramp across x = 40 (20..39 is paper strip, 40..79 holds the edge).
                let t = (x as f64 - 60.0) / 1.0;
                let v = 60.0 + 140.0 / (1.0 + crate::math::exp(-t));
                g[y * w + x] = (v + noise()).clamp(0.0, 255.0) as u8;
            }
        }
        let mask: Vec<u8> = (0..w * h).map(|i| u8::from(i % w >= 50)).collect();
        let paper: Vec<u8> = (0..w * h).map(|i| u8::from(i % w < 30)).collect();
        let s = focus_stats(&g, w, h, &mask, &paper, 1.0);
        let corrected = s.sharpness.expect("edge is stronger than the noise");
        assert!(corrected < s.sharpness_raw, "{s:?}");
        // The logistic with scale 1 has a Gaussian-equivalent sigma near 1.7 px: S about 0.4.
        assert!(corrected > 0.2 && corrected < 0.8, "{s:?}");
    }

    #[test]
    fn variance_matches_hand_computation_on_one_impulse() {
        // A single bright pixel in a 3x3 image: only the centre is interior, laplacian = -4 * 100.
        let mut g = vec![0u8; 9];
        g[4] = 100;
        let one = focus_stats(&g, 3, 3, &[1u8; 9], &[0u8; 9], 1.0);
        assert_eq!(one.pixels, 1);
        assert_eq!(one.variance, 0.0, "one sample has zero variance");
        // Two samples -400 and 0: mean -200, variance 40000.
        let mut g = vec![0u8; 5 * 3];
        g[5 + 1] = 100;
        let two = focus_stats(&g, 5, 3, &[1u8; 15], &[0u8; 15], 1.0);
        let (v, n) = (two.variance, two.pixels);
        assert_eq!(n, 3);
        // Laplacian: centre-left -400, its right neighbour +100, the far one 0.
        let mean = (-400.0 + 100.0 + 0.0) / 3.0;
        let sq = |v: f64| v * v;
        let expect = (sq(-400.0 - mean) + sq(100.0 - mean) + sq(0.0 - mean)) / 3.0;
        assert!((v - expect).abs() < 1e-9, "{v} vs {expect}");
    }

    #[test]
    fn dilation_matches_a_brute_force_square_on_random_masks() {
        let (w, h) = (23usize, 17usize);
        let mut state = 7u32;
        let mask: Vec<u8> = (0..w * h)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                u8::from((state >> 16).is_multiple_of(29))
            })
            .collect();
        for radius in [0usize, 1, 2, 4, 9] {
            let fast = dilate(&mask, w, h, radius);
            for y in 0..h {
                for x in 0..w {
                    let mut expect = 0u8;
                    for yy in 0..h {
                        for xx in 0..w {
                            if mask[yy * w + xx] != 0
                                && yy.abs_diff(y) <= radius
                                && xx.abs_diff(x) <= radius
                            {
                                expect = 1;
                            }
                        }
                    }
                    assert_eq!(fast[y * w + x], expect, "r={radius} at ({x},{y})");
                }
            }
        }
    }

    #[test]
    fn dilation_grows_by_the_radius_and_clips_at_the_edge() {
        let mut m = vec![0u8; 7 * 7];
        m[3 * 7 + 3] = 1;
        let d = dilate(&m, 7, 7, 1);
        assert_eq!(d.iter().map(|&v| u32::from(v)).sum::<u32>(), 9);
        let mut corner = vec![0u8; 7 * 7];
        corner[0] = 1;
        assert_eq!(
            dilate(&corner, 7, 7, 2)
                .iter()
                .map(|&v| u32::from(v))
                .sum::<u32>(),
            9
        );
    }
}
