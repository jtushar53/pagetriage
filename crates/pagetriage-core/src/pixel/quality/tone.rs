//! Gray-level statistics: RMS contrast, the paper level and the ink level, all from exact integer
//! histograms (so the only float work is a final division and square root).

/// Gray histogram of the whole page and of two pixel classes.
pub struct Tone {
    /// RMS contrast: population standard deviation of the page over 255 (Peli's definition,
    /// E. Peli, "Contrast in complex images", JOSA A 7(10):2032-2040, 1990; recalled, unverified).
    pub contrast_rms: f64,
    /// Median gray of the pixels the binariser called paper (0 when there are none).
    pub paper_level: f64,
    /// Low percentile of the gray of text ink pixels (equals `paper_level` when there is no ink).
    pub ink_level: f64,
    /// `(paper_level - ink_level) / 255`, never negative: the ink/paper peak gap.
    pub gap: f64,
    /// Histogram of text-ink pixels, kept for the bleed-through measure.
    pub ink_hist: [u64; 256],
    /// Histogram of paper pixels.
    pub paper_hist: [u64; 256],
}

/// Gray level at which the cumulative count first reaches `fraction` of `hist`'s total, or `None`
/// for an empty histogram. `fraction` in (0, 1].
pub fn percentile(hist: &[u64; 256], fraction: f64) -> Option<u8> {
    let total: u64 = hist.iter().sum();
    if total == 0 {
        return None;
    }
    // Compare in integers: acc >= ceil(fraction * total).
    let target = (fraction * total as f64).ceil().max(1.0) as u64;
    let mut acc = 0u64;
    for (level, &n) in hist.iter().enumerate() {
        acc += n;
        if acc >= target {
            return Some(level as u8);
        }
    }
    Some(255)
}

/// `ink` is the binarised page (1 = ink) and `text_ink[i]` says whether pixel `i` belongs to a
/// text component (the caller decides what counts as text).
pub fn measure(
    gray: &[u8],
    ink: &[u8],
    text_ink: &dyn Fn(usize) -> bool,
    ink_percentile: f64,
) -> Tone {
    let mut all = [0u64; 256];
    let mut ink_hist = [0u64; 256];
    let mut paper_hist = [0u64; 256];
    for (i, &g) in gray.iter().enumerate() {
        all[usize::from(g)] += 1;
        if ink[i] == 0 {
            paper_hist[usize::from(g)] += 1;
        } else if text_ink(i) {
            ink_hist[usize::from(g)] += 1;
        }
    }
    let n = gray.len() as u128;
    let sum: u128 = all
        .iter()
        .enumerate()
        .map(|(v, &c)| v as u128 * u128::from(c))
        .sum();
    let sq: u128 = all
        .iter()
        .enumerate()
        .map(|(v, &c)| (v * v) as u128 * u128::from(c))
        .sum();
    // n * sq - sum^2 >= 0 by Cauchy-Schwarz.
    let variance = (n * sq - sum * sum) as f64 / (n as f64 * n as f64);
    let contrast_rms = variance.sqrt() / 255.0;

    let paper_level = percentile(&paper_hist, 0.5).map_or(0.0, f64::from);
    let ink_level = percentile(&ink_hist, ink_percentile).map_or(paper_level, f64::from);
    let gap = ((paper_level - ink_level) / 255.0).max(0.0);
    Tone {
        contrast_rms,
        paper_level,
        ink_level,
        gap,
        ink_hist,
        paper_hist,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_of_known_histograms() {
        let mut h = [0u64; 256];
        h[10] = 5;
        h[200] = 5;
        assert_eq!(percentile(&h, 0.5), Some(10));
        assert_eq!(percentile(&h, 0.51), Some(200));
        assert_eq!(percentile(&h, 1.0), Some(200));
        assert_eq!(percentile(&[0u64; 256], 0.5), None);
    }

    #[test]
    fn two_level_page_has_exact_contrast_and_gap() {
        // Half the pixels 0, half 255: std = 127.5, contrast 0.5.
        let gray: Vec<u8> = (0..100).map(|i| if i < 50 { 0 } else { 255 }).collect();
        let ink: Vec<u8> = gray.iter().map(|&g| u8::from(g == 0)).collect();
        let t = measure(&gray, &ink, &|_| true, 0.1);
        assert!((t.contrast_rms - 0.5).abs() < 1e-12);
        assert_eq!(t.paper_level, 255.0);
        assert_eq!(t.ink_level, 0.0);
        assert!((t.gap - 1.0).abs() < 1e-12);
    }

    #[test]
    fn flat_page_has_zero_contrast_and_zero_gap() {
        let gray = vec![200u8; 64];
        let t = measure(&gray, &[0u8; 64], &|_| true, 0.1);
        assert_eq!(t.contrast_rms, 0.0);
        assert_eq!(t.gap, 0.0);
        assert_eq!(t.paper_level, 200.0);
    }
}
