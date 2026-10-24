//! Shared filtering primitives: separable Gaussian blur and integral images.
//!
//! Deterministic: kernel weights come from `math::exp`, integral images hold exact integers, and
//! every float sum runs in a fixed order.

use crate::math;

/// Kernels are truncated at this many sigmas (numerical choice: it drops 0.27% of the mass).
const KERNEL_RADIUS_SIGMAS: f64 = 3.0;

/// Normalised 1-D Gaussian taps for offsets `-r..=r`, `r = ceil(3 sigma)` and at least 1.
pub fn gaussian_kernel(sigma: f64) -> Vec<f64> {
    let radius = ((KERNEL_RADIUS_SIGMAS * sigma).ceil() as usize).max(1);
    let mut taps: Vec<f64> = (0..=2 * radius)
        .map(|i| {
            let x = i as f64 - radius as f64;
            math::exp(-(x * x) / (2.0 * sigma * sigma))
        })
        .collect();
    let sum = math::pairwise_sum(&taps);
    for t in &mut taps {
        *t /= sum;
    }
    taps
}

/// What a filter sees beyond the image edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Border {
    /// Repeat the edge pixel.
    Replicate,
    /// Use only the taps inside the image and divide by their total weight.
    Renormalise,
}

/// Separable Gaussian blur of a `width * height` row-major image.
pub fn gaussian_blur(
    src: &[f64],
    width: usize,
    height: usize,
    sigma: f64,
    border: Border,
) -> Vec<f64> {
    debug_assert_eq!(src.len(), width * height);
    let taps = gaussian_kernel(sigma);
    let mut tmp = vec![0.0; src.len()];
    for y in 0..height {
        let row = &src[y * width..(y + 1) * width];
        let out = &mut tmp[y * width..(y + 1) * width];
        convolve_line(row, out, &taps, border);
    }
    let mut out = vec![0.0; src.len()];
    let mut column = vec![0.0; height];
    let mut filtered = vec![0.0; height];
    for x in 0..width {
        for y in 0..height {
            column[y] = tmp[y * width + x];
        }
        convolve_line(&column, &mut filtered, &taps, border);
        for y in 0..height {
            out[y * width + x] = filtered[y];
        }
    }
    out
}

fn convolve_line(line: &[f64], out: &mut [f64], taps: &[f64], border: Border) {
    let n = line.len();
    let r = taps.len() / 2;
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = if i >= r && i + r < n {
            let mut acc = 0.0;
            for (t, w) in taps.iter().enumerate() {
                acc += w * line[i + t - r];
            }
            if border == Border::Renormalise {
                // Interior taps all lie inside the image, so the weight total is the kernel's.
                acc / taps_total(taps)
            } else {
                acc
            }
        } else {
            let mut acc = 0.0;
            let mut total = 0.0;
            for (t, w) in taps.iter().enumerate() {
                let pos = i as isize + t as isize - r as isize;
                match border {
                    Border::Replicate => {
                        acc += w * line[pos.clamp(0, n as isize - 1) as usize];
                    }
                    Border::Renormalise => {
                        if pos >= 0 && (pos as usize) < n {
                            acc += w * line[pos as usize];
                            total += w;
                        }
                    }
                }
            }
            if border == Border::Renormalise {
                acc / total
            } else {
                acc
            }
        };
    }
}

fn taps_total(taps: &[f64]) -> f64 {
    // The kernel is normalised to 1 up to rounding; summing it the same way every time keeps the
    // interior and edge paths consistent.
    math::pairwise_sum(taps)
}

/// Summed-area tables of the gray values and their squares, exact in `u64`.
pub struct Integral {
    stride: usize,
    sum: Vec<u64>,
    sq: Vec<u64>,
}

impl Integral {
    pub fn new(gray: &[u8], width: usize, height: usize) -> Self {
        let stride = width + 1;
        let mut sum = vec![0u64; stride * (height + 1)];
        let mut sq = vec![0u64; stride * (height + 1)];
        for y in 0..height {
            let mut row_sum = 0u64;
            let mut row_sq = 0u64;
            for x in 0..width {
                let v = u64::from(gray[y * width + x]);
                row_sum += v;
                row_sq += v * v;
                sum[(y + 1) * stride + x + 1] = sum[y * stride + x + 1] + row_sum;
                sq[(y + 1) * stride + x + 1] = sq[y * stride + x + 1] + row_sq;
            }
        }
        Self { stride, sum, sq }
    }

    /// `(pixel count, sum, sum of squares)` over `[x0, x1) x [y0, y1)`.
    pub fn window(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> (u64, u64, u64) {
        let s = self.stride;
        let at = |t: &[u64], x: usize, y: usize| t[y * s + x];
        let sum = at(&self.sum, x1, y1) + at(&self.sum, x0, y0)
            - at(&self.sum, x1, y0)
            - at(&self.sum, x0, y1);
        let sq = at(&self.sq, x1, y1) + at(&self.sq, x0, y0)
            - at(&self.sq, x1, y0)
            - at(&self.sq, x0, y1);
        (((x1 - x0) * (y1 - y0)) as u64, sum, sq)
    }
}

/// Mean and standard deviation from exact window sums.
pub fn mean_std(count: u64, sum: u64, sq: u64) -> (f64, f64) {
    let n = count as f64;
    let mean = sum as f64 / n;
    // n * sq - sum^2 is a non-negative integer (Cauchy-Schwarz), so the variance is exact up to one
    // final division.
    let numerator = u128::from(count) * u128::from(sq) - u128::from(sum) * u128::from(sum);
    let variance = numerator as f64 / (n * n);
    (mean, variance.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_is_symmetric_and_sums_to_one() {
        for sigma in [0.6, 1.0, 20.0] {
            let k = gaussian_kernel(sigma);
            assert_eq!(k.len() % 2, 1);
            for i in 0..k.len() / 2 {
                assert_eq!(k[i], k[k.len() - 1 - i]);
            }
            assert!((k.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn blur_keeps_a_constant_image_constant_under_both_borders() {
        let src = vec![7.5; 9 * 6];
        for border in [Border::Replicate, Border::Renormalise] {
            for v in gaussian_blur(&src, 9, 6, 2.0, border) {
                assert!((v - 7.5).abs() < 1e-12, "{border:?}: {v}");
            }
        }
    }

    #[test]
    fn blur_of_an_impulse_spreads_symmetrically() {
        let mut src = vec![0.0; 11 * 11];
        src[5 * 11 + 5] = 1.0;
        let out = gaussian_blur(&src, 11, 11, 1.0, Border::Replicate);
        assert!((out.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        assert_eq!(out[5 * 11 + 4], out[5 * 11 + 6]);
        assert_eq!(out[4 * 11 + 5], out[6 * 11 + 5]);
        assert!(out[5 * 11 + 5] > out[5 * 11 + 4]);
    }

    #[test]
    fn integral_windows_match_brute_force() {
        let (w, h) = (13usize, 9usize);
        let gray: Vec<u8> = (0..w * h).map(|i| ((i * 37 + 11) % 256) as u8).collect();
        let integral = Integral::new(&gray, w, h);
        for (x0, y0, x1, y1) in [(0, 0, w, h), (2, 1, 7, 5), (12, 8, 13, 9), (0, 3, 4, 4)] {
            let (mut n, mut s, mut q) = (0u64, 0u64, 0u64);
            for y in y0..y1 {
                for x in x0..x1 {
                    let v = u64::from(gray[y * w + x]);
                    n += 1;
                    s += v;
                    q += v * v;
                }
            }
            assert_eq!(integral.window(x0, y0, x1, y1), (n, s, q));
        }
    }

    #[test]
    fn mean_std_of_known_values() {
        // Values 2, 4, 4, 4, 5, 5, 7, 9: mean 5, population std 2.
        let v = [2u64, 4, 4, 4, 5, 5, 7, 9];
        let (m, s) = mean_std(8, v.iter().sum(), v.iter().map(|x| x * x).sum());
        assert_eq!((m, s), (5.0, 2.0));
        assert_eq!(mean_std(4, 400, 40_000), (100.0, 0.0));
    }
}
