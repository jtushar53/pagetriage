//! Canny edge detection, split into the parts that depend on the smoothing scale (gradient and
//! non-maximum suppression, computed once per page) and the part that depends on the thresholds
//! (hysteresis, recomputed for each candidate threshold).
//!
//! Source: J. Canny, "A computational approach to edge detection", IEEE TPAMI 8(6):679-698, 1986.
//! Thresholds are fractions of the largest gradient magnitude on the page.
//! The gradient is the derivative of a Gaussian of scale sigma (separable: smooth across, differentiate
//! along). Difference from MATLAB's `edge(..., 'canny')`, which Howe's code presumably used: the
//! gradient direction is quantised to four sectors instead of interpolated, which moves an edge by at
//! most a pixel.

use super::filters::{Border, gaussian_derivative_kernel, gaussian_kernel, separable_filter};

/// A page whose strongest gradient is below this (gray levels per pixel) is flat: thresholds are
/// relative to the strongest gradient, so without this guard rounding residue on a constant image
/// would be scaled up to 1.0 and read as edges everywhere. Numerical guard, not a tuned value.
const FLAT_GRADIENT: f64 = 1e-6;

/// tan(22.5 degrees) = sqrt(2) - 1: the boundary between a horizontal and a diagonal gradient.
const TAN_22_5: f64 = 0.414_213_562_373_095_05;
/// tan(67.5 degrees) = sqrt(2) + 1: the boundary between a diagonal and a vertical gradient.
const TAN_67_5: f64 = 2.414_213_562_373_095;

/// Gradient magnitude after non-maximum suppression, scaled so the largest value is 1.
pub struct Nms {
    pub width: usize,
    pub height: usize,
    pub magnitude: Vec<f64>,
}

pub fn non_max_suppressed_gradient(gray: &[u8], width: usize, height: usize, sigma: f64) -> Nms {
    let image: Vec<f64> = gray.iter().map(|&g| f64::from(g)).collect();
    // Gradient as the derivative of a Gaussian of scale sigma: smooth across, differentiate along.
    let smooth_taps = gaussian_kernel(sigma);
    let deriv_taps = gaussian_derivative_kernel(sigma);
    let gx = separable_filter(
        &image,
        width,
        height,
        &deriv_taps,
        &smooth_taps,
        Border::Replicate,
    );
    let gy = separable_filter(
        &image,
        width,
        height,
        &smooth_taps,
        &deriv_taps,
        Border::Replicate,
    );
    let magnitude: Vec<f64> = gx
        .iter()
        .zip(&gy)
        .map(|(dx, dy)| (dx * dx + dy * dy).sqrt())
        .collect();
    let mag_at = |x: isize, y: isize| {
        if x < 0 || y < 0 || x >= width as isize || y >= height as isize {
            0.0
        } else {
            magnitude[y as usize * width + x as usize]
        }
    };
    let mut kept = vec![0.0; gray.len()];
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            let m = magnitude[i];
            if m <= 0.0 {
                continue;
            }
            let (ax, ay) = (gx[i].abs(), gy[i].abs());
            // Step along the gradient direction, to the "before" and "after" neighbours.
            let (sx, sy): (isize, isize) = if ay <= ax * TAN_22_5 {
                (1, 0)
            } else if ay >= ax * TAN_67_5 {
                (0, 1)
            } else if (gx[i] > 0.0) == (gy[i] > 0.0) {
                (1, 1)
            } else {
                (1, -1)
            };
            let (xi, yi) = (x as isize, y as isize);
            // Strict on one side and non-strict on the other, so a two-pixel plateau keeps one edge.
            if m > mag_at(xi - sx, yi - sy) && m >= mag_at(xi + sx, yi + sy) {
                kept[i] = m;
            }
        }
    }
    let peak = kept.iter().copied().fold(0.0f64, f64::max);
    if peak > FLAT_GRADIENT {
        for v in &mut kept {
            *v /= peak;
        }
    } else {
        kept.fill(0.0);
    }
    Nms {
        width,
        height,
        magnitude: kept,
    }
}

/// Edge map: suppressed pixels above `t_hi` plus those above `t_lo` that connect to one (8-way).
pub fn hysteresis(nms: &Nms, t_lo: f64, t_hi: f64) -> Vec<bool> {
    let (w, h) = (nms.width, nms.height);
    let mut edge = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    for (i, &m) in nms.magnitude.iter().enumerate() {
        if m > t_hi {
            edge[i] = true;
            stack.push(i);
        }
    }
    while let Some(i) = stack.pop() {
        let (x, y) = ((i % w) as isize, (i / w) as isize);
        for dy in -1isize..=1 {
            for dx in -1isize..=1 {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                if !edge[j] && nms.magnitude[j] > t_lo {
                    edge[j] = true;
                    stack.push(j);
                }
            }
        }
    }
    edge
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step_image(w: usize, h: usize, edge_x: usize) -> Vec<u8> {
        (0..w * h)
            .map(|i| if i % w < edge_x { 200 } else { 50 })
            .collect()
    }

    #[test]
    fn a_vertical_step_gives_one_thin_edge_line() {
        let (w, h) = (24, 16);
        let nms = non_max_suppressed_gradient(&step_image(w, h, 12), w, h, 0.6);
        let edges = hysteresis(&nms, 0.1, 0.4);
        for y in 0..h {
            let row: Vec<usize> = (0..w).filter(|&x| edges[y * w + x]).collect();
            assert_eq!(row.len(), 1, "row {y}: {row:?}");
            assert!((11..=12).contains(&row[0]), "row {y}: {row:?}");
        }
    }

    #[test]
    fn magnitudes_are_normalised_to_one() {
        let (w, h) = (24, 16);
        let nms = non_max_suppressed_gradient(&step_image(w, h, 12), w, h, 0.6);
        let peak = nms.magnitude.iter().copied().fold(0.0, f64::max);
        assert_eq!(peak, 1.0);
    }

    #[test]
    fn flat_image_has_no_edges() {
        let nms = non_max_suppressed_gradient(&[90u8; 100], 10, 10, 0.6);
        assert!(hysteresis(&nms, 0.0, 0.0).iter().all(|&e| !e));
    }

    #[test]
    fn hysteresis_keeps_weak_edges_only_when_connected_to_strong_ones() {
        // Two vertical steps of different contrast, far apart: the weak one has no strong pixel.
        let (w, h) = (40, 12);
        let gray: Vec<u8> = (0..w * h)
            .map(|i| match i % w {
                0..=9 => 40,
                10..=29 => 200,
                _ => 170,
            })
            .collect();
        let nms = non_max_suppressed_gradient(&gray, w, h, 0.6);
        // The 160-level step is strong (1.0); the 30-level step is about 0.19 of it.
        let both = hysteresis(&nms, 0.05, 0.1);
        let only_strong = hysteresis(&nms, 0.05, 0.5);
        let weak_pixels = |e: &[bool]| (0..h).filter(|&y| (28..=31).any(|x| e[y * w + x])).count();
        assert_eq!(weak_pixels(&both), h);
        assert_eq!(weak_pixels(&only_strong), 0);
    }

    #[test]
    fn a_diagonal_step_is_found() {
        let (w, h) = (30, 30);
        let gray: Vec<u8> = (0..w * h)
            .map(|i| if (i % w) + (i / w) < w { 220 } else { 40 })
            .collect();
        let nms = non_max_suppressed_gradient(&gray, w, h, 0.8);
        let n = hysteresis(&nms, 0.1, 0.4).iter().filter(|&&e| e).count();
        assert!((20..=100).contains(&n), "{n} edge pixels");
    }
}
