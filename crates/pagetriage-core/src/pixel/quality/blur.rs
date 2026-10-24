//! Blur as the variance of the Laplacian over the text mask.
//!
//! Variance of the Laplacian as a focus measure: J. L. Pech-Pacheco, G. Cristobal, J. Chamorro-
//! Martinez, J. Fernandez-Valdivia, "Diatom autofocusing in brightfield microscopy: a comparative
//! study", ICPR 2000 (recalled from memory, unverified). The 4-neighbour Laplacian of an 8-bit
//! image is an integer in [-1020, 1020], so its sums are exact and the result is identical on every
//! target. The value depends on scan resolution and on contrast (it scales with contrast squared),
//! so the grade uses a contrast-normalised form.

/// Focus statistics over the interior pixels where a mask is set.
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
    pub sharpness: f64,
}

/// Laplacian variance and edge sharpness over the interior pixels where `mask` is set.
pub fn focus_stats(gray: &[u8], width: usize, height: usize, mask: &[u8]) -> BlurStats {
    debug_assert_eq!(gray.len(), width * height);
    let empty = BlurStats {
        pixels: 0,
        variance: 0.0,
        sharpness: 0.0,
    };
    if width < 3 || height < 3 {
        return empty;
    }
    let (mut n, mut sum, mut sq, mut grad_sq) = (0u64, 0i64, 0u64, 0u64);
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let i = y * width + x;
            if mask[i] == 0 {
                continue;
            }
            let (w, e) = (i32::from(gray[i - 1]), i32::from(gray[i + 1]));
            let (nn, ss) = (i32::from(gray[i - width]), i32::from(gray[i + width]));
            let lap = w + e + nn + ss - 4 * i32::from(gray[i]);
            n += 1;
            sum += i64::from(lap);
            sq += (i64::from(lap) * i64::from(lap)) as u64;
            let (gx, gy) = (i64::from(e - w), i64::from(ss - nn));
            grad_sq += (gx * gx + gy * gy) as u64;
        }
    }
    if n == 0 {
        return empty;
    }
    let numerator = i128::from(n) * i128::from(sq) - i128::from(sum) * i128::from(sum);
    BlurStats {
        pixels: n,
        variance: numerator as f64 / (n as f64 * n as f64),
        sharpness: if grad_sq == 0 {
            0.0
        } else {
            2.0 * (sq as f64 / grad_sq as f64).sqrt()
        },
    }
}

/// Mark every pixel within `radius` (Chebyshev distance) of a set pixel of `src`.
pub fn dilate(src: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    let mut out = vec![0u8; src.len()];
    for y in 0..height {
        for x in 0..width {
            if src[y * width + x] == 0 {
                continue;
            }
            let (y0, y1) = (y.saturating_sub(radius), (y + radius).min(height - 1));
            let (x0, x1) = (x.saturating_sub(radius), (x + radius).min(width - 1));
            for yy in y0..=y1 {
                for v in &mut out[yy * width + x0..=yy * width + x1] {
                    *v = 1;
                }
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
        let sharp = focus_stats(&step_image(w, h, false), w, h, &mask);
        let soft = focus_stats(&step_image(w, h, true), w, h, &mask);
        assert!(
            sharp.variance > soft.variance * 2.0,
            "{sharp:?} vs {soft:?}"
        );
        assert!(sharp.sharpness > soft.sharpness, "{sharp:?} vs {soft:?}");
    }

    #[test]
    fn flat_and_empty_masks_give_zero() {
        let (w, h) = (8, 8);
        let flat = vec![99u8; w * h];
        let on = focus_stats(&flat, w, h, &vec![1u8; w * h]);
        assert_eq!((on.variance, on.pixels, on.sharpness), (0.0, 36, 0.0));
        assert_eq!(focus_stats(&flat, w, h, &vec![0u8; w * h]).pixels, 0);
        assert_eq!(focus_stats(&[1, 2, 3, 4], 2, 2, &[1, 1, 1, 1]).pixels, 0);
    }

    #[test]
    fn variance_matches_hand_computation_on_one_impulse() {
        // A single bright pixel in a 3x3 image: only the centre is interior, laplacian = -4 * 100.
        let mut g = vec![0u8; 9];
        g[4] = 100;
        let one = focus_stats(&g, 3, 3, &[1u8; 9]);
        assert_eq!(one.pixels, 1);
        assert_eq!(one.variance, 0.0, "one sample has zero variance");
        // Two samples -400 and 0: mean -200, variance 40000.
        let mut g = vec![0u8; 5 * 3];
        g[5 + 1] = 100;
        let two = focus_stats(&g, 5, 3, &[1u8; 15]);
        let (v, n) = (two.variance, two.pixels);
        assert_eq!(n, 3);
        // Laplacian: centre-left -400, its right neighbour +100, the far one 0.
        let mean = (-400.0 + 100.0 + 0.0) / 3.0;
        let sq = |v: f64| v * v;
        let expect = (sq(-400.0 - mean) + sq(100.0 - mean) + sq(0.0 - mean)) / 3.0;
        assert!((v - expect).abs() < 1e-9, "{v} vs {expect}");
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
