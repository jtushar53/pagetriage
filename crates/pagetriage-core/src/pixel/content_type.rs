//! Skeleton `blank_page` detector (module M10, "blank" label).
//!
//! This exists in Phase 0 to drive the whole path (pixels -> evidence -> fusion -> finding) with
//! real float arithmetic, so the cross-runtime determinism test has something to bite on. It is
//! not the final detector: there is no despeckle step yet, and "paper" is taken as the most common
//! gray level, so a uniformly dark page also reads as blank. See `docs/detectors/blank_page.md`.

use serde_json::Map;

use super::Detection;
use crate::error::Result;
use crate::fusion::{evidence::clip_llr, scorer};
use crate::math;
use crate::params::{BlankPageParams, FusionParams};
use crate::schema::{Evidence, Finding, Kind};

pub const BLANK_PAGE: &str = "blank_page";
const DETECTOR_ID: &str = "content_type.blank_page";
const CITATION: &str =
    "module M10: blank = ink coverage < 0.2%; Phase 0 skeleton, no published basis";

pub fn detect_blank_page(
    gray: &[u8],
    p: &BlankPageParams,
    fusion: &FusionParams,
) -> Result<Detection> {
    let mut histogram = [0u64; 256];
    for &v in gray {
        histogram[usize::from(v)] += 1;
    }

    // Paper level: most common gray level; ties go to the darker level so the choice is fixed.
    let mut paper = 0usize;
    for (level, &count) in histogram.iter().enumerate() {
        if count > histogram[paper] {
            paper = level;
        }
    }
    let ink_below = (paper as u8).saturating_sub(p.ink_delta);
    let ink_pixels: u64 = histogram[..usize::from(ink_below)].iter().sum();
    let total = gray.len() as u64;

    let coverage = ink_pixels as f64 / total as f64;
    let llr = clip_llr(
        p.llr_per_ln_unit * math::ln(p.blank_ink_fraction / coverage.max(p.coverage_floor)),
        fusion.llr_clip,
    );

    let evidence = Evidence::new(
        DETECTOR_ID,
        "ink_coverage",
        coverage,
        Some(p.blank_ink_fraction),
        llr,
        format!(
            "{:.4}% of pixels darker than gray {} (paper level {} minus {})",
            coverage * 100.0,
            ink_below,
            paper,
            p.ink_delta
        ),
        CITATION,
    )?;

    let confidence = math::round_output(scorer::confidence(
        p.prior_logit,
        std::slice::from_ref(&evidence),
    ));

    let mut detection = Detection::default();
    detection.features.insert("ink_coverage".into(), coverage);
    detection
        .features
        .insert("ink_pixels".into(), ink_pixels as f64);
    detection
        .features
        .insert("paper_level".into(), paper as f64);

    if confidence >= fusion.emit_min_confidence {
        let mut attributes = Map::new();
        attributes.insert("paper_level".into(), paper.into());
        detection.findings.push(Finding {
            kind: Kind::BlankPage,
            confidence,
            bbox: None,
            polygon: None,
            attributes,
            evidence: vec![evidence],
        });
    }
    Ok(detection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::Params;

    fn run(gray: &[u8]) -> Detection {
        let p = Params::defaults().unwrap();
        detect_blank_page(gray, &p.blank_page, &p.fusion).unwrap()
    }

    #[test]
    fn white_page_is_blank_with_high_confidence() {
        let d = run(&vec![250u8; 100 * 100]);
        assert_eq!(d.findings.len(), 1);
        assert!(d.findings[0].confidence > 0.99);
        assert_eq!(d.features["ink_pixels"], 0.0);
        assert_eq!(d.features["paper_level"], 250.0);
    }

    #[test]
    fn page_with_text_like_ink_is_not_blank_but_features_are_still_reported() {
        let mut gray = vec![250u8; 100 * 100];
        for row in (5..95).step_by(10) {
            for x in 10..90 {
                gray[row * 100 + x] = 20;
            }
        }
        let d = run(&gray);
        assert!(d.findings.is_empty());
        assert!((d.features["ink_coverage"] - 0.072).abs() < 1e-12);
    }

    #[test]
    fn ink_exactly_at_the_threshold_sits_at_even_odds() {
        // 20 of 10_000 pixels = 0.2% = blank_ink_fraction, so llr is 0 and confidence is 0.5.
        let mut gray = vec![250u8; 10_000];
        for px in gray.iter_mut().take(20) {
            *px = 0;
        }
        let d = run(&gray);
        assert_eq!(d.findings.len(), 1, "0.5 >= emit_min_confidence is emitted");
        assert_eq!(d.findings[0].confidence, 0.5);
        assert_eq!(d.findings[0].evidence[0].llr, 0.0);
    }

    #[test]
    fn faint_marks_inside_ink_delta_do_not_count_as_ink() {
        let mut gray = vec![250u8; 10_000];
        for px in gray.iter_mut().take(2_000) {
            *px = 200; // 50 below paper, delta is 60
        }
        assert_eq!(run(&gray).features["ink_pixels"], 0.0);
    }
}
