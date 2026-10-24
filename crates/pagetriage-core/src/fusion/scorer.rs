use crate::math;
use crate::schema::Evidence;

/// Calibrated-by-construction confidence: `sigmoid(prior_logit + sum(llr))`. The sum uses the
/// fixed-order pairwise reduction so it cannot depend on the platform.
pub fn confidence(prior_logit: f64, evidence: &[Evidence]) -> f64 {
    let llrs: Vec<f64> = evidence.iter().map(|e| e.llr).collect();
    math::sigmoid(prior_logit + math::pairwise_sum(&llrs))
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

    #[test]
    fn llrs_add_in_log_odds_space() {
        let p = confidence(0.0, &[ev(1.0), ev(2.0)]);
        let q = confidence(3.0, &[]);
        assert_eq!(p, q);
        assert!(confidence(0.0, &[ev(-4.0)]) < 0.05);
    }
}
