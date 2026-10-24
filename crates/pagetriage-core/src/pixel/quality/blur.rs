//! Blur as the variance of the Laplacian over the text mask.
//!
//! Variance of the Laplacian as a focus measure: J. L. Pech-Pacheco, G. Cristobal, J. Chamorro-
//! Martinez, J. Fernandez-Valdivia, "Diatom autofocusing in brightfield microscopy: a comparative
//! study", ICPR 2000 (recalled from memory, unverified). The 4-neighbour Laplacian of an 8-bit
//! image is an integer in [-1020, 1020], so its sums are exact and the result is identical on every
//! target. The value depends on scan resolution and on contrast (it scales with contrast squared),
//! so the grade uses a contrast-normalised form.

/// Variance of the 4-neighbour Laplacian over the interior pixels where `mask` is set.
/// Returns `(variance, pixels counted)`; `(0.0, 0)` when the mask is empty.
pub fn laplacian_variance(gray: &[u8], width: usize, height: usize, mask: &[u8]) -> (f64, u64) {
    debug_assert_eq!(gray.len(), width * height);
    if width < 3 || height < 3 {
        return (0.0, 0);
    }
    let (mut n, mut sum, mut sq) = (0u64, 0i64, 0u64);
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let i = y * width + x;
            if mask[i] == 0 {
                continue;
            }
            let lap = i32::from(gray[i - 1])
                + i32::from(gray[i + 1])
                + i32::from(gray[i - width])
                + i32::from(gray[i + width])
                - 4 * i32::from(gray[i]);
            n += 1;
            sum += i64::from(lap);
            sq += (i64::from(lap) * i64::from(lap)) as u64;
        }
    }
    if n == 0 {
        return (0.0, 0);
    }
    let numerator = i128::from(n) * i128::from(sq) - i128::from(sum) * i128::from(sum);
    (numerator as f64 / (n as f64 * n as f64), n)
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
        let (sharp, _) = laplacian_variance(&step_image(w, h, false), w, h, &mask);
        let (soft, _) = laplacian_variance(&step_image(w, h, true), w, h, &mask);
        assert!(sharp > soft * 2.0, "sharp {sharp} vs soft {soft}");
    }

    #[test]
    fn flat_and_empty_masks_give_zero() {
        let (w, h) = (8, 8);
        assert_eq!(
            laplacian_variance(&vec![99u8; w * h], w, h, &vec![1u8; w * h]),
            (0.0, 36)
        );
        assert_eq!(
            laplacian_variance(&vec![99u8; w * h], w, h, &vec![0u8; w * h]),
            (0.0, 0)
        );
        assert_eq!(
            laplacian_variance(&[1, 2, 3, 4], 2, 2, &[1, 1, 1, 1]),
            (0.0, 0)
        );
    }

    #[test]
    fn variance_matches_hand_computation_on_one_impulse() {
        // A single bright pixel in a 3x3 image: only the centre is interior, laplacian = -4 * 100.
        let mut g = vec![0u8; 9];
        g[4] = 100;
        let (v, n) = laplacian_variance(&g, 3, 3, &[1u8; 9]);
        assert_eq!(n, 1);
        assert_eq!(v, 0.0, "one sample has zero variance");
        // Two samples -400 and 0: mean -200, variance 40000.
        let mut g = vec![0u8; 5 * 3];
        g[5 + 1] = 100;
        let (v, n) = laplacian_variance(&g, 5, 3, &[1u8; 15]);
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
