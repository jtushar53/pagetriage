//! Binariser choice and the A-D grade (module M3).
//!
//! Each measure is turned into a severity in `[0, 1]` (0 at its "good" value, 1 at its "bad"
//! value, linear in between). The page's badness mixes the worst severity with the weighted mean
//! of all of them, so one terrible measure cannot hide behind several good ones:
//!
//! `badness = w_max * max(severity) + (1 - w_max) * weighted_mean(severity)`
//!
//! Grades: A below `grade_a_max`, B below `grade_b_max`, C below `grade_c_max`, D otherwise. The
//! scale is documented in `docs/detectors/quality.md`; every number is in `params/quality.toml`.

use super::config::QualityParams;
use super::generation::severity;
use crate::math;
use crate::schema::Binarizer;

/// The measures the grade is computed from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradeInput {
    pub skew_deg: f64,
    /// Blur radius (Gaussian sigma) in millimetres at the page's dpi.
    pub blur_sigma_mm: f64,
    /// Ink/paper gap in `[0, 1]`.
    pub gap: f64,
    pub speckle_density: f64,
    pub bleed: f64,
    pub folds: u32,
    /// Fraction of the page that is text ink; below the floor the page has nothing to grade.
    pub ink_fraction: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Graded {
    pub grade: &'static str,
    pub badness: f64,
    pub severities: [f64; 6],
    pub binarizer: Binarizer,
}

pub fn assess(inp: &GradeInput, p: &QualityParams) -> Graded {
    let sev = [
        severity(inp.skew_deg.abs(), p.grade_skew_good, p.grade_skew_bad),
        severity(inp.blur_sigma_mm, p.grade_blur_good, p.grade_blur_bad),
        severity(inp.gap, p.grade_gap_good, p.grade_gap_bad),
        severity(
            inp.speckle_density,
            p.grade_speckle_good,
            p.grade_speckle_bad,
        ),
        severity(inp.bleed, p.grade_bleed_good, p.grade_bleed_bad),
        severity(f64::from(inp.folds), p.grade_folds_good, p.grade_folds_bad),
    ];
    let w = [
        p.grade_w_skew,
        p.grade_w_blur,
        p.grade_w_gap,
        p.grade_w_speckle,
        p.grade_w_bleed,
        p.grade_w_folds,
    ];
    let weighted: Vec<f64> = sev.iter().zip(&w).map(|(s, w)| s * w).collect();
    let mean = math::pairwise_sum(&weighted) / math::pairwise_sum(&w);
    let worst = sev.iter().copied().fold(0.0f64, f64::max);
    let badness = p.grade_w_max * worst + (1.0 - p.grade_w_max) * mean;
    let grade = if badness < p.grade_a_max {
        "A"
    } else if badness < p.grade_b_max {
        "B"
    } else if badness < p.grade_c_max {
        "C"
    } else {
        "D"
    };
    // Howe when the page looks hard to threshold locally: bleed-through or low contrast, and only
    // when there is ink to binarise at all.
    let hard = inp.bleed >= p.howe_bleed_min || inp.gap < p.howe_gap_max;
    let binarizer = if hard && inp.ink_fraction >= p.howe_min_ink_fraction {
        Binarizer::Howe
    } else {
        Binarizer::Sauvola
    };
    Graded {
        grade,
        badness,
        severities: sev,
        binarizer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pixel::quality::config::test_params;

    fn clean() -> GradeInput {
        GradeInput {
            skew_deg: 0.1,
            blur_sigma_mm: 0.05,
            gap: 0.8,
            speckle_density: 0.0,
            bleed: 0.0,
            folds: 0,
            ink_fraction: 0.05,
        }
    }

    #[test]
    fn a_clean_page_is_an_a_with_sauvola() {
        let g = assess(&clean(), &test_params());
        assert_eq!(g.grade, "A");
        assert_eq!(g.binarizer, Binarizer::Sauvola);
        assert!(g.badness < 0.05);
    }

    #[test]
    fn grades_get_worse_as_measures_get_worse() {
        let p = test_params();
        let mut last = -1.0;
        for step in 0..=4 {
            let t = f64::from(step) / 4.0;
            let inp = GradeInput {
                skew_deg: 6.0 * t,
                blur_sigma_mm: 0.4 * t,
                gap: 0.8 - 0.6 * t,
                speckle_density: 0.01 * t,
                bleed: t,
                folds: step,
                ink_fraction: 0.05,
            };
            let b = assess(&inp, &p).badness;
            assert!(b >= last, "badness {b} after {last}");
            last = b;
        }
        let worst = assess(
            &GradeInput {
                skew_deg: 12.0,
                blur_sigma_mm: 1.0,
                gap: 0.05,
                speckle_density: 0.1,
                bleed: 1.0,
                folds: 4,
                ink_fraction: 0.05,
            },
            &p,
        );
        assert_eq!(worst.grade, "D");
    }

    #[test]
    fn one_terrible_measure_is_enough_to_drop_a_grade() {
        let p = test_params();
        let mut inp = clean();
        inp.blur_sigma_mm = 5.0;
        assert_ne!(assess(&inp, &p).grade, "A");
    }

    #[test]
    fn howe_is_recommended_for_bleed_or_low_contrast_but_not_for_blank_pages() {
        let p = test_params();
        let mut inp = clean();
        inp.bleed = 0.9;
        assert_eq!(assess(&inp, &p).binarizer, Binarizer::Howe);
        let mut inp = clean();
        inp.gap = 0.1;
        assert_eq!(assess(&inp, &p).binarizer, Binarizer::Howe);
        inp.ink_fraction = 0.0;
        assert_eq!(assess(&inp, &p).binarizer, Binarizer::Sauvola);
    }
}
