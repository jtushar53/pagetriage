use std::collections::BTreeMap;

use crate::math;
use crate::schema::Evidence;

/// Calibrated-by-construction confidence: `sigmoid(prior_logit + sum(llr))`. The sum uses the
/// fixed-order pairwise reduction so it cannot depend on the platform.
pub fn confidence(prior_logit: f64, evidence: &[Evidence]) -> f64 {
    let llrs: Vec<f64> = evidence.iter().map(|e| e.llr).collect();
    math::sigmoid(prior_logit + math::pairwise_sum(&llrs))
}

/// Like [`confidence`], but the evidence entries of one detector (same `detector` string) are first
/// summed and the sum clamped to `[-group_cap, group_cap]`: correlated features of one detector must
/// not add up to more than the cap. Groups are summed in name order, so the result is fixed.
pub fn confidence_capped(prior_logit: f64, evidence: &[Evidence], group_cap: f64) -> f64 {
    let mut groups: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for e in evidence {
        groups.entry(e.detector.as_str()).or_default().push(e.llr);
    }
    let sums: Vec<f64> = groups
        .values()
        .map(|v| math::pairwise_sum(v).clamp(-group_cap, group_cap))
        .collect();
    math::sigmoid(prior_logit + math::pairwise_sum(&sums))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(llr: f64) -> Evidence {
        Evidence::new("t", "f", 0.0, None, llr, String::new(), "test").unwrap()
    }

    #[test]
    fn no_evidence_gives_the_prior() {
        assert_eq!(confidence(0.0, &[]), 0.5);
    }

    fn ev_by(detector: &str, llr: f64) -> Evidence {
        Evidence::new(detector, "f", 0.0, None, llr, String::new(), "test").unwrap()
    }

    #[test]
    fn a_detectors_evidence_is_capped_but_different_detectors_add() {
        let many = [ev_by("a", 8.0), ev_by("a", 8.0), ev_by("a", 8.0)];
        assert_eq!(confidence_capped(0.0, &many, 12.0), confidence(12.0, &[]));
        let two = [ev_by("a", 8.0), ev_by("a", 8.0), ev_by("b", 3.0)];
        assert_eq!(confidence_capped(0.0, &two, 12.0), confidence(15.0, &[]));
        // Below the cap it is the plain sum, in either sign.
        let small = [ev_by("a", 1.0), ev_by("a", -3.0), ev_by("b", 0.5)];
        assert_eq!(confidence_capped(0.0, &small, 12.0), confidence(-1.5, &[]));
        let neg = [ev_by("a", -9.0), ev_by("a", -9.0)];
        assert_eq!(confidence_capped(0.0, &neg, 12.0), confidence(-12.0, &[]));
    }

    #[test]
    fn llrs_add_in_log_odds_space() {
        let p = confidence(0.0, &[ev(1.0), ev(2.0)]);
        let q = confidence(3.0, &[]);
        assert_eq!(p, q);
        assert!(confidence(0.0, &[ev(-4.0)]) < 0.05);
    }
}
