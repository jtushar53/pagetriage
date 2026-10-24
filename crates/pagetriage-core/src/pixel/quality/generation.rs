//! Photocopy-generation estimate (module M3). Experimental.
//!
//! Each pass through a copier changes the stroke geometry of text: strokes get thinner or
//! thicker relative to the letter size, their width becomes more uneven, letters break into
//! fragments, specks accumulate and the ink/paper contrast falls. This module turns five such
//! measurements into a score and rounds it to a generation count. No published method was found
//! for this; the mapping is hand-set (see `params/quality.toml`) and has not been validated on
//! real photocopies, so the confidence is capped at `gen_max_confidence` (0.6).

use super::config::QualityParams;
use super::stroke;
use crate::schema::GenerationEstimate;

/// Measurements the estimate is built from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GenerationInput {
    /// Histogram of stroke-ridge values of text components (chamfer units).
    pub ridge_hist: [u64; 256],
    /// Median text height in pixels (x-height proxy).
    pub xheight: f64,
    /// Fraction of text components that are small fragments.
    pub fragment_rate: f64,
    pub speckle_density: f64,
    /// Ink/paper gap in `[0, 1]`.
    pub gap: f64,
    /// Number of text components.
    pub components: usize,
}

/// Mean stroke width in pixels and its coefficient of variation, from a ridge histogram.
pub fn width_stats(hist: &[u64; 256]) -> Option<(f64, f64)> {
    let n: u64 = hist.iter().sum();
    if n == 0 {
        return None;
    }
    let widths: Vec<(f64, f64)> = hist
        .iter()
        .enumerate()
        .filter(|(_, c)| **c > 0)
        .map(|(k, &c)| (stroke::width_px(k as u16).max(1.0), c as f64))
        .collect();
    let terms: Vec<f64> = widths.iter().map(|(w, c)| w * c).collect();
    let mean = crate::math::pairwise_sum(&terms) / n as f64;
    let sq: Vec<f64> = widths
        .iter()
        .map(|(w, c)| (w - mean) * (w - mean) * c)
        .collect();
    let var = crate::math::pairwise_sum(&sq) / n as f64;
    Some((mean, var.sqrt() / mean))
}

/// 0 at `good`, 1 at `bad` (either order), clamped.
pub fn severity(value: f64, good: f64, bad: f64) -> f64 {
    ((value - good) / (bad - good)).clamp(0.0, 1.0)
}

/// Estimate, or `None` when there is too little text to measure strokes.
pub fn estimate(
    inp: &GenerationInput,
    p: &QualityParams,
) -> Option<(GenerationEstimate, [f64; 5])> {
    if inp.components < p.gen_min_components {
        return None;
    }
    let (mean_w, cv) = width_stats(&inp.ridge_hist)?;
    let rel_width = mean_w / inp.xheight.max(1.0);
    let sev = [
        severity(cv, p.gen_cv_good, p.gen_cv_bad),
        severity(rel_width, p.gen_width_good, p.gen_width_bad),
        severity(inp.fragment_rate, p.gen_fragment_good, p.gen_fragment_bad),
        severity(inp.speckle_density, p.gen_speckle_good, p.gen_speckle_bad),
        severity(inp.gap, p.gen_gap_good, p.gen_gap_bad),
    ];
    let w = [
        p.gen_w_cv,
        p.gen_w_width,
        p.gen_w_fragment,
        p.gen_w_speckle,
        p.gen_w_gap,
    ];
    let weighted: Vec<f64> = sev.iter().zip(&w).map(|(s, w)| s * w).collect();
    let score = crate::math::pairwise_sum(&weighted) / crate::math::pairwise_sum(&w);
    let value = (score * f64::from(p.gen_max_generation)).round() as u32;
    let evidence = (inp.components as f64 / p.gen_full_confidence_components as f64).min(1.0);
    Some((
        GenerationEstimate {
            value,
            confidence: (p.gen_max_confidence * evidence).min(p.gen_max_confidence),
        },
        sev,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pixel::quality::config::test_params;

    fn input(cv_spread: bool, frag: f64, speck: f64, gap: f64, comps: usize) -> GenerationInput {
        let mut hist = [0u64; 256];
        if cv_spread {
            // Widths 1, 3 and 7 pixels mixed: uneven.
            hist[3] = 400;
            hist[6] = 400;
            hist[12] = 200;
        } else {
            hist[6] = 1000; // all 3 px
        }
        GenerationInput {
            ridge_hist: hist,
            xheight: 12.0,
            fragment_rate: frag,
            speckle_density: speck,
            gap,
            components: comps,
        }
    }

    #[test]
    fn width_stats_of_uniform_strokes_has_zero_cv() {
        let mut h = [0u64; 256];
        h[6] = 50;
        let (mean, cv) = width_stats(&h).unwrap();
        assert!((mean - 3.0).abs() < 1e-12);
        assert_eq!(cv, 0.0);
        assert!(width_stats(&[0u64; 256]).is_none());
    }

    #[test]
    fn a_degraded_page_scores_higher_than_a_clean_one() {
        let p = test_params();
        let clean = estimate(&input(false, 0.05, 0.0001, 0.8, 500), &p)
            .unwrap()
            .0;
        let worn = estimate(&input(true, 0.5, 0.01, 0.2, 500), &p).unwrap().0;
        assert!(worn.value > clean.value, "{worn:?} vs {clean:?}");
        assert!(clean.value <= 1);
    }

    #[test]
    fn confidence_is_capped_and_grows_with_evidence() {
        let p = test_params();
        let few = estimate(&input(true, 0.3, 0.001, 0.5, p.gen_min_components), &p)
            .unwrap()
            .0;
        let many = estimate(&input(true, 0.3, 0.001, 0.5, 100_000), &p)
            .unwrap()
            .0;
        assert!(few.confidence < many.confidence);
        assert!(many.confidence <= 0.6 + 1e-12);
        assert!((many.confidence - p.gen_max_confidence).abs() < 1e-12);
    }

    #[test]
    fn too_little_text_gives_no_estimate() {
        let p = test_params();
        assert!(estimate(&input(true, 0.3, 0.001, 0.5, 3), &p).is_none());
    }
}
