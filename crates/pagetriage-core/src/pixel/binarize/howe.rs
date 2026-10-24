//! Howe's Laplacian-energy binariser with automatic parameter tuning.
//!
//! Source: N. R. Howe, "Document binarization with automatic parameter tuning", IJDAR
//! 16(3):247-258, 2013 (author's manuscript); the base method is Howe, "A Laplacian energy for
//! document binarization", ICDAR 2011. Implemented from the paper's text alone: the author's
//! MATLAB reference code was not consulted.
//!
//! The binarisation minimises, over labellings `B` (1 = ink),
//! `E(B) = sum_p [L0(p) (1 - B_p) + L1(p) B_p] + sum_{pq} C_pq [B_p != B_q]` where `L0 = lap(I)`
//! and `L1 = -lap(I)` are the Laplacian label costs, bright outliers are forced to paper, and
//! neighbour penalty `C_pq` is `c` everywhere except between a Canny edge pixel and its brighter
//! neighbour, where it is 0. The graph cut is [`super::maxflow`]. `c` and the high Canny
//! threshold are then tuned per image by the stability criterion of Algorithms 1 and 3.
//!
//! Where the printed text cannot be right, the stated intent is followed and noted inline.

use super::canny::{Nms, hysteresis, non_max_suppressed_gradient};
use super::filters::{Border, gaussian_blur};
use super::maxflow::min_cut;
use crate::math;
use crate::params::{HoweMode, HoweParams};

/// A binarisation together with the parameters that produced it.
pub struct HoweResult {
    /// `1` for ink, `0` for paper, row-major.
    pub ink: Vec<u8>,
    /// Neighbour penalty used for the final cut, in the paper's cost units.
    pub c: f64,
    /// High Canny threshold used for the final cut.
    pub t_hi: f64,
}

pub fn binarize(gray: &[u8], width: usize, height: usize, p: &HoweParams) -> HoweResult {
    let prepared = Prepared::new(gray, width, height, p);
    match p.mode {
        HoweMode::Static => {
            let pairs = prepared.pair_free(p.static_t_hi);
            HoweResult {
                ink: prepared.solve(p.static_c, &pairs),
                c: p.static_c,
                t_hi: p.static_t_hi,
            }
        }
        HoweMode::TuneC => {
            let pairs = prepared.pair_free(p.static_t_hi);
            let c = prepared.tune_c(&pairs);
            HoweResult {
                ink: prepared.solve(c, &pairs),
                c,
                t_hi: p.static_t_hi,
            }
        }
        HoweMode::Dual => prepared.dual(),
    }
}

/// Which neighbour pairs are free to differ because a Canny edge separates them.
struct PairFree {
    right: Vec<bool>,
    down: Vec<bool>,
}

/// Everything that does not depend on `c` or the high Canny threshold, computed once per page.
struct Prepared<'a> {
    gray: &'a [u8],
    width: usize,
    height: usize,
    /// `L0 - L1` per pixel in solver units: positive favours ink.
    terminal: Vec<i32>,
    nms: Nms,
    p: &'a HoweParams,
}

impl<'a> Prepared<'a> {
    fn new(gray: &'a [u8], width: usize, height: usize, p: &'a HoweParams) -> Self {
        let at = |x: isize, y: isize| -> i32 {
            let xc = x.clamp(0, width as isize - 1) as usize;
            let yc = y.clamp(0, height as isize - 1) as usize;
            i32::from(gray[yc * width + xc])
        };
        // Local mean and standard deviation, Gaussian-weighted with sigma = radius, to find the
        // pixels that are surely paper.
        let intensity: Vec<f64> = gray.iter().map(|&g| f64::from(g)).collect();
        let squared: Vec<f64> = intensity.iter().map(|v| v * v).collect();
        let mean = gaussian_blur(&intensity, width, height, p.radius, Border::Renormalise);
        let mean_sq = gaussian_blur(&squared, width, height, p.radius, Border::Renormalise);

        // The solver's unit is the integer 5-point Laplacian S (4x MATLAB's `del2`). Label costs
        // are D0 = S for paper and D1 = -S for ink, so a pixel's terminal capacity is D0 - D1 = 2S.
        // A bright outlier keeps D0 and gets the ink cost -phi (large and positive): Equation 4
        // prints phi (negative) as the ink cost, but the text says the purpose is to force
        // background, so the sign is flipped to do that.
        let phi_solver = (p.phi * p.cost_scale).round() as i32;
        let mut terminal = vec![0i32; gray.len()];
        for y in 0..height {
            for x in 0..width {
                let (xi, yi) = (x as isize, y as isize);
                let i = y * width + x;
                let s = at(xi + 1, yi) + at(xi - 1, yi) + at(xi, yi + 1) + at(xi, yi - 1)
                    - 4 * at(xi, yi);
                let m = mean[i];
                let sd = (mean_sq[i] - m * m).max(0.0).sqrt();
                let outlier = intensity[i] > m + p.outlier_sigmas * sd;
                terminal[i] = if outlier { s + phi_solver } else { 2 * s };
            }
        }
        Self {
            gray,
            width,
            height,
            terminal,
            nms: non_max_suppressed_gradient(gray, width, height, p.sigma_e),
            p,
        }
    }

    fn pair_free(&self, t_hi: f64) -> PairFree {
        let (w, h) = (self.width, self.height);
        let edge = hysteresis(&self.nms, self.p.t_lo, t_hi);
        // Equations 5 and 6: the penalty between two neighbours is 0 when the darker one (the
        // second, on a tie) is a Canny edge pixel, which puts edge pixels on the ink side.
        let free = |a: usize, b: usize| {
            (edge[a] && self.gray[a] < self.gray[b]) || (edge[b] && self.gray[a] >= self.gray[b])
        };
        let mut right = vec![false; w * h];
        let mut down = vec![false; w * h];
        for y in 0..h {
            for x in 0..w {
                let a = y * w + x;
                if x + 1 < w {
                    right[a] = free(a, a + 1);
                }
                if y + 1 < h {
                    down[a] = free(a, a + w);
                }
            }
        }
        PairFree { right, down }
    }

    /// One graph cut at neighbour penalty `c` (paper units).
    fn solve(&self, c: f64, pairs: &PairFree) -> Vec<u8> {
        let c_solver = ((c * self.p.cost_scale).round() as i32).max(1);
        let cost = |free: &[bool]| -> Vec<i32> {
            free.iter().map(|&f| if f { 0 } else { c_solver }).collect()
        };
        let cut = min_cut(
            self.width,
            self.height,
            &self.terminal,
            &cost(&pairs.right),
            &cost(&pairs.down),
        );
        cut.source_side.iter().map(|&s| u8::from(s)).collect()
    }

    /// Algorithm 1: pick `c` at the most stable point between the two peaks of the instability
    /// curve (the fraction of pixels that change label between neighbouring values of `c`).
    fn tune_c(&self, pairs: &PairFree) -> f64 {
        let p = self.p;
        let n = p.c_steps + 1;
        let candidates: Vec<f64> = (0..n)
            .map(|i| p.c_start * math::pow(2.0, i as f64 / f64::from(p.steps_per_octave)))
            .collect();

        let mut changes = vec![0.0f64; n - 1];
        let mut previous = self.solve(candidates[0], pairs);
        for i in 0..n - 1 {
            let next = self.solve(candidates[i + 1], pairs);
            changes[i] = changed_pixels(&previous, &next) as f64;
            previous = next;
        }

        // Smooth the curve with the Gaussian taps, zero beyond both ends.
        let taps = &p.curve_smoothing;
        let half = taps.len() / 2;
        let smoothed: Vec<f64> = (0..n)
            .map(|i| {
                let mut acc = 0.0;
                for (j, w) in taps.iter().enumerate() {
                    let k = i as isize + j as isize - half as isize;
                    if k >= 0 && (k as usize) < changes.len() {
                        acc += changes[k as usize] * w;
                    }
                }
                acc
            })
            .collect();

        // Algorithm 1 prints `arg min D'_q - 2 D'_r + D'_s` over q < r < s. The text says c is
        // chosen at the minimum between two peaks, which is the maximum of this second difference
        // (high peaks at q and s, low valley at r), so the maximum is taken. The first of equal
        // maxima wins, in (q, r, s) order.
        let mut best = (f64::NEG_INFINITY, n / 2);
        for q in 0..n {
            for r in q + 1..n {
                for s in r + 1..n {
                    let score = smoothed[q] - 2.0 * smoothed[r] + smoothed[s];
                    if score > best.0 {
                        best = (score, r);
                    }
                }
            }
        }
        candidates[best.1]
    }

    /// Algorithm 3: tune `c` at three high Canny thresholds, then keep whichever of the two
    /// candidate thresholds yields the binarisation closer to the one at their midpoint.
    fn dual(&self) -> HoweResult {
        let p = self.p;
        let mid = (p.tau1 + p.tau2) / 2.0;
        let run = |t_hi: f64| {
            let pairs = self.pair_free(t_hi);
            let c = self.tune_c(&pairs);
            (self.solve(c, &pairs), c)
        };
        let (mid_ink, _) = run(mid);
        let (low_ink, low_c) = run(p.tau1);
        let (high_ink, high_c) = run(p.tau2);
        let d1 = changed_pixels(&mid_ink, &low_ink);
        let d2 = changed_pixels(&mid_ink, &high_ink);
        if d1 < d2 {
            HoweResult {
                ink: low_ink,
                c: low_c,
                t_hi: p.tau1,
            }
        } else {
            HoweResult {
                ink: high_ink,
                c: high_c,
                t_hi: p.tau2,
            }
        }
    }
}

/// Delta(B, B'): the number of pixels whose label differs.
fn changed_pixels(a: &[u8], b: &[u8]) -> u64 {
    a.iter().zip(b).filter(|(x, y)| x != y).count() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::Params;

    fn params(mode: HoweMode) -> HoweParams {
        let mut p = Params::defaults().unwrap().howe;
        p.mode = mode;
        p
    }

    /// Dark strokes on a light page, blurred like a scan (a perfectly sharp step has no meaningful
    /// edge side) with mild deterministic noise.
    fn stroke_page(w: usize, h: usize) -> (Vec<u8>, Vec<u8>) {
        let mut truth = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                truth[y * w + x] = u8::from(
                    (x % 16 < 3 && y > 6 && y + 6 < h) || (y % 20 < 3 && x > 6 && x + 6 < w),
                );
            }
        }
        let ideal: Vec<f64> = truth
            .iter()
            .map(|&t| if t == 1 { 60.0 } else { 190.0 })
            .collect();
        let soft = gaussian_blur(&ideal, w, h, 0.8, Border::Replicate);
        let mut state = 12345u32;
        let gray = soft
            .iter()
            .map(|v| {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let noise = ((state >> 16) % 9) as f64 - 4.0;
                (v + noise).round().clamp(0.0, 255.0) as u8
            })
            .collect();
        (gray, truth)
    }

    fn recall(pred: &[u8], truth: &[u8]) -> f64 {
        let hit = pred
            .iter()
            .zip(truth)
            .filter(|(a, b)| **a == 1 && **b == 1)
            .count();
        hit as f64 / truth.iter().filter(|&&t| t == 1).count() as f64
    }

    fn f_measure(pred: &[u8], truth: &[u8]) -> f64 {
        let (mut tp, mut fp, mut fn_) = (0f64, 0f64, 0f64);
        for (a, b) in pred.iter().zip(truth) {
            match (a, b) {
                (1, 1) => tp += 1.0,
                (1, 0) => fp += 1.0,
                (0, 1) => fn_ += 1.0,
                _ => {}
            }
        }
        2.0 * tp / (2.0 * tp + fp + fn_)
    }

    #[test]
    fn static_mode_recovers_strokes_on_a_clean_page() {
        let (w, h) = (96, 64);
        let (gray, truth) = stroke_page(w, h);
        let out = binarize(&gray, w, h, &params(HoweMode::Static));
        assert_eq!(out.c, 160.0);
        // The method puts Canny edge pixels on the ink side, so strokes come out up to a pixel
        // fatter at their edges: recall is near-perfect and precision pays for the extra pixel.
        let recall = recall(&out.ink, &truth);
        let f = f_measure(&out.ink, &truth);
        assert!(recall > 0.97, "recall = {recall}");
        assert!(f > 0.78, "F = {f}");
    }

    #[test]
    fn a_blank_page_stays_blank() {
        let out = binarize(&[200u8; 40 * 30], 40, 30, &params(HoweMode::Static));
        assert!(out.ink.iter().all(|&v| v == 0));
    }

    #[test]
    fn output_does_not_depend_on_page_brightness_or_contrast_offset() {
        // The Laplacian cost is invariant to adding a constant to the page.
        let (w, h) = (64, 48);
        let (gray, _) = stroke_page(w, h);
        let brighter: Vec<u8> = gray
            .iter()
            .map(|&g| g.saturating_add(25).min(250))
            .collect();
        // Skip pixels that clipped, which would change the shape; the page maximum is 194 here.
        assert!(gray.iter().all(|&g| g <= 194));
        let a = binarize(&gray, w, h, &params(HoweMode::Static)).ink;
        let b = binarize(&brighter, w, h, &params(HoweMode::Static)).ink;
        let differing = a.iter().zip(&b).filter(|(x, y)| x != y).count();
        assert!(differing * 100 < a.len(), "{differing} pixels differ");
    }

    #[test]
    fn tuned_modes_run_and_report_what_they_chose() {
        let (w, h) = (48, 32);
        let (gray, truth) = stroke_page(w, h);
        let mut p = params(HoweMode::TuneC);
        let tuned = binarize(&gray, w, h, &p);
        assert!(tuned.c >= p.c_start && tuned.c <= p.c_start * 128.0);
        assert!(f_measure(&tuned.ink, &truth) > 0.85);
        p.mode = HoweMode::Dual;
        let dual = binarize(&gray, w, h, &p);
        assert!(dual.t_hi == p.tau1 || dual.t_hi == p.tau2);
        assert!(f_measure(&dual.ink, &truth) > 0.85);
    }

    #[test]
    fn repeated_runs_are_identical() {
        let (w, h) = (40, 30);
        let (gray, _) = stroke_page(w, h);
        let p = params(HoweMode::TuneC);
        let a = binarize(&gray, w, h, &p);
        let b = binarize(&gray, w, h, &p);
        assert_eq!((a.ink, a.c.to_bits()), (b.ink, b.c.to_bits()));
    }

    #[test]
    fn one_pixel_and_tiny_pages_do_not_panic() {
        for (w, h) in [(1, 1), (1, 9), (9, 1), (2, 2), (3, 3)] {
            let gray: Vec<u8> = (0..w * h).map(|i| (i * 37 % 256) as u8).collect();
            assert_eq!(
                binarize(&gray, w, h, &params(HoweMode::Dual)).ink.len(),
                w * h
            );
        }
    }
}
