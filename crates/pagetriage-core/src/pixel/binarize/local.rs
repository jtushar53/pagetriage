//! Global (Otsu) and local (Sauvola, Wolf-Jolion) thresholding. Output is `1` for ink, `0` for
//! paper. Window statistics come from exact integer integral images, so the only float work is a
//! handful of IEEE operations per pixel and the result is identical on every target.

use super::filters::{Integral, mean_std};
use crate::params::{SauvolaParams, WolfParams};

/// Otsu's global threshold: the gray level that maximises the between-class variance. Ties go to
/// the lower level. Source: N. Otsu, "A threshold selection method from gray-level histograms",
/// IEEE Trans. SMC 9(1):62-66, 1979.
pub fn otsu_threshold(gray: &[u8]) -> u8 {
    let mut histogram = [0u64; 256];
    for &g in gray {
        histogram[usize::from(g)] += 1;
    }
    let total: u64 = gray.len() as u64;
    let total_sum: u64 = histogram
        .iter()
        .enumerate()
        .map(|(level, &n)| level as u64 * n)
        .sum();
    let (mut w0, mut sum0) = (0u64, 0u64);
    let mut best = (0u8, -1.0f64);
    for (level, &count) in histogram.iter().enumerate() {
        w0 += count;
        sum0 += level as u64 * count;
        let w1 = total - w0;
        if w0 == 0 || w1 == 0 {
            continue;
        }
        // omega0 * omega1 * (mu0 - mu1)^2 = (sum0 * w1 - sum1 * w0)^2 / (w0 * w1 * N^2); the common
        // N^2 factor does not change the argmax.
        let sum1 = total_sum - sum0;
        let cross = i128::from(sum0) * i128::from(w1) - i128::from(sum1) * i128::from(w0);
        let between = (cross * cross) as f64 / (w0 as f64 * w1 as f64);
        if between > best.1 {
            best = (level as u8, between);
        }
    }
    best.0
}

/// Ink where `gray <= otsu_threshold`.
pub fn otsu(gray: &[u8]) -> Vec<u8> {
    let t = otsu_threshold(gray);
    gray.iter().map(|&g| u8::from(g <= t)).collect()
}

fn window_bounds(i: usize, half: usize, len: usize) -> (usize, usize) {
    (i.saturating_sub(half), (i + half + 1).min(len))
}

/// Sauvola: ink where `gray <= m * (1 + k * (s / R - 1))`, window clipped at the page edge.
pub fn sauvola(gray: &[u8], width: usize, height: usize, p: &SauvolaParams) -> Vec<u8> {
    let integral = Integral::new(gray, width, height);
    let half = p.window / 2;
    let mut out = vec![0u8; gray.len()];
    for y in 0..height {
        let (y0, y1) = window_bounds(y, half, height);
        for x in 0..width {
            let (x0, x1) = window_bounds(x, half, width);
            let (n, s, q) = integral.window(x0, y0, x1, y1);
            let (mean, std) = mean_std(n, s, q);
            let threshold = mean * (1.0 + p.k * (std / p.dynamic_range - 1.0));
            out[y * width + x] = u8::from(f64::from(gray[y * width + x]) <= threshold);
        }
    }
    out
}

/// Wolf-Jolion: ink where `gray < (1-a) m + a M + a (s / S_max) (m - M)`, with `M` the page
/// minimum and `S_max` the largest local standard deviation on the page.
pub fn wolf(gray: &[u8], width: usize, height: usize, p: &WolfParams) -> Vec<u8> {
    let integral = Integral::new(gray, width, height);
    let half = p.window / 2;
    let stats_at = |x: usize, y: usize| {
        let (x0, x1) = window_bounds(x, half, width);
        let (y0, y1) = window_bounds(y, half, height);
        let (n, s, q) = integral.window(x0, y0, x1, y1);
        mean_std(n, s, q)
    };
    let mut max_std = 0.0f64;
    for y in 0..height {
        for x in 0..width {
            max_std = max_std.max(stats_at(x, y).1);
        }
    }
    let page_min = f64::from(gray.iter().copied().min().unwrap_or(0));
    let mut out = vec![0u8; gray.len()];
    for y in 0..height {
        for x in 0..width {
            let (mean, std) = stats_at(x, y);
            let contrast = if max_std > 0.0 { std / max_std } else { 0.0 };
            let threshold =
                (1.0 - p.a) * mean + p.a * page_min + p.a * contrast * (mean - page_min);
            // Strict, unlike Sauvola: with `<=` a perfectly flat page has threshold == gray and
            // would be all ink.
            out[y * width + x] = u8::from(f64::from(gray[y * width + x]) < threshold);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sauvola_defaults() -> SauvolaParams {
        SauvolaParams {
            window: 15,
            k: 0.2,
            dynamic_range: 128.0,
        }
    }

    /// Dark vertical bar on a light page with a smooth illumination ramp.
    fn bar_page(w: usize, h: usize) -> (Vec<u8>, Vec<u8>) {
        let mut gray = vec![0u8; w * h];
        let mut truth = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let paper = 140 + (x * 100 / w) as i32; // 140 .. 240 left to right
                let bar = (20..24).contains(&(x % 40)) && y > 4 && y < h - 4;
                gray[y * w + x] = if bar { (paper - 90) as u8 } else { paper as u8 };
                truth[y * w + x] = u8::from(bar);
            }
        }
        (gray, truth)
    }

    #[test]
    fn otsu_splits_two_flat_populations_and_breaks_ties_low() {
        let mut g = vec![30u8; 500];
        g.extend(vec![200u8; 500]);
        let t = otsu_threshold(&g);
        assert!((30..200).contains(&t), "t = {t}");
        assert_eq!(t, 30, "ties between equally good levels go to the lowest");
        let ink = otsu(&g);
        assert_eq!(ink.iter().filter(|&&v| v == 1).count(), 500);
    }

    #[test]
    fn otsu_on_a_constant_page_does_not_panic() {
        assert_eq!(otsu(&[77u8; 64]).len(), 64);
    }

    #[test]
    fn sauvola_follows_a_lighting_ramp_where_otsu_cannot() {
        let (w, h) = (160, 40);
        let (gray, truth) = bar_page(w, h);
        let errors = |pred: &[u8]| pred.iter().zip(&truth).filter(|(a, b)| a != b).count();
        let local = errors(&sauvola(&gray, w, h, &sauvola_defaults()));
        let global = errors(&otsu(&gray));
        assert!(local < global, "local {local} vs global {global}");
        assert!(local * 20 < truth.len(), "local errors {local}");
    }

    #[test]
    fn sauvola_flat_regions_are_paper_unless_black() {
        let p = sauvola_defaults();
        assert!(sauvola(&[255u8; 100], 10, 10, &p).iter().all(|&v| v == 0));
        assert!(sauvola(&[100u8; 100], 10, 10, &p).iter().all(|&v| v == 0));
        assert!(sauvola(&[0u8; 100], 10, 10, &p).iter().all(|&v| v == 1));
    }

    #[test]
    fn wolf_finds_the_bars_and_ignores_flat_paper() {
        let (w, h) = (160, 40);
        let (gray, truth) = bar_page(w, h);
        let p = WolfParams { window: 15, a: 0.5 };
        let pred = wolf(&gray, w, h, &p);
        let errors = pred.iter().zip(&truth).filter(|(a, b)| a != b).count();
        assert!(errors * 20 < truth.len(), "errors {errors}");
        assert!(wolf(&[200u8; 100], 10, 10, &p).iter().all(|&v| v == 0));
    }

    #[test]
    fn windows_larger_than_the_page_are_clipped() {
        let p = SauvolaParams {
            window: 999,
            ..sauvola_defaults()
        };
        assert_eq!(sauvola(&[10, 200, 10, 200], 2, 2, &p).len(), 4);
    }
}
