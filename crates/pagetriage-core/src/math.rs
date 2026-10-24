//! Deterministic numeric helpers.
//!
//! `+ - * /` and `sqrt` on `f64` are exactly specified by IEEE-754 and behave identically on
//! native, Python and WASM builds. The transcendental functions are not: `f64::exp` and friends
//! call the platform libm. `clippy.toml` bans them, so use these wrappers, which are backed by the
//! pure-Rust `libm` crate on every target.

use crate::error::{Error, Result};

/// Output precision: every float in a report is rounded to this many decimals before it is
/// serialised, so a last-ulp difference cannot change the bytes.
pub const OUTPUT_DECIMALS: i32 = 6;
const OUTPUT_SCALE: f64 = 1_000_000.0;

pub fn exp(x: f64) -> f64 {
    libm::exp(x)
}

pub fn ln(x: f64) -> f64 {
    libm::log(x)
}

pub fn pow(base: f64, exponent: f64) -> f64 {
    libm::pow(base, exponent)
}

/// Logistic function, numerically safe for large |x|.
pub fn sigmoid(x: f64) -> f64 {
    if x >= 0.0 {
        1.0 / (1.0 + exp(-x))
    } else {
        let e = exp(x);
        e / (1.0 + e)
    }
}

/// Round to [`OUTPUT_DECIMALS`] and normalise `-0.0` to `0.0`.
pub fn round_output(x: f64) -> f64 {
    let r = (x * OUTPUT_SCALE).round() / OUTPUT_SCALE;
    if r == 0.0 { 0.0 } else { r }
}

/// Reject NaN and infinities at the point a detector produces them.
pub fn finite(x: f64, what: &str) -> Result<f64> {
    if x.is_finite() {
        Ok(x)
    } else {
        Err(Error::NonFinite(format!("{what} = {x}")))
    }
}

/// Pairwise summation in a fixed order (determinism rule: any `f64` sum over pixels uses
/// Kahan or pairwise summation, never an order that depends on the platform). Blocks of up to
/// `BLOCK` values are summed left to right, then halves are combined recursively.
pub fn pairwise_sum(values: &[f64]) -> f64 {
    const BLOCK: usize = 8;
    if values.len() <= BLOCK {
        let mut s = 0.0;
        for v in values {
            s += *v;
        }
        return s;
    }
    let (left, right) = values.split_at(values.len() / 2);
    pairwise_sum(left) + pairwise_sum(right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sigmoid_is_symmetric_and_bounded() {
        assert_eq!(sigmoid(0.0), 0.5);
        for x in [0.5, 3.0, 12.0, 40.0, 800.0] {
            let s = sigmoid(x) + sigmoid(-x);
            assert!((s - 1.0).abs() < 1e-15, "x={x} s={s}");
            assert!(sigmoid(x) <= 1.0 && sigmoid(-x) >= 0.0);
        }
        assert!(sigmoid(f64::MAX).is_finite());
        assert!(sigmoid(-f64::MAX).is_finite());
    }

    #[test]
    fn round_output_rounds_and_clears_negative_zero() {
        assert_eq!(round_output(0.123_456_49), 0.123_456);
        assert_eq!(round_output(0.123_456_51), 0.123_457);
        let z = round_output(-1e-9);
        assert_eq!(z, 0.0);
        assert!(z.is_sign_positive());
    }

    #[test]
    fn finite_rejects_nan_and_inf() {
        assert!(finite(1.5, "x").is_ok());
        assert!(finite(f64::NAN, "x").is_err());
        assert!(finite(f64::INFINITY, "x").is_err());
    }

    #[test]
    fn pairwise_sum_is_exact_on_small_integers_and_order_fixed() {
        let v: Vec<f64> = (1..=1000).map(f64::from).collect();
        assert_eq!(pairwise_sum(&v), 500_500.0);
        // Same input, same bits, every time.
        let w: Vec<f64> = (0..4097).map(|i| 0.1 * f64::from(i)).collect();
        assert_eq!(pairwise_sum(&w).to_bits(), pairwise_sum(&w).to_bits());
    }

    #[test]
    fn libm_wrappers_match_known_values() {
        assert_eq!(exp(0.0), 1.0);
        assert_eq!(ln(1.0), 0.0);
        assert!((exp(1.0) - std::f64::consts::E).abs() < 1e-15);
        assert!((pow(2.0, 10.0) - 1024.0).abs() < 1e-12);
    }
}
