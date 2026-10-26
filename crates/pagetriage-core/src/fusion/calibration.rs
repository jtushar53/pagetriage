//! Confidence calibration (module M13): an optional monotone piecewise-linear map per finding kind.
//! No map ships (see `params/calibration.toml`); the machinery is here for when labelled data is.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::math;
use crate::schema::{Finding, Kind};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationParams {
    /// Kind (snake_case, as in the report) to `[raw, calibrated]` knots.
    pub maps: BTreeMap<String, Vec<[f64; 2]>>,
}

fn kind_name(kind: Kind) -> String {
    match serde_json::to_value(kind) {
        Ok(serde_json::Value::String(s)) => s,
        _ => unreachable!("Kind serialises as a string"),
    }
}

impl CalibrationParams {
    pub fn validate(&self) -> Result<()> {
        for (kind, knots) in &self.maps {
            let bad = |m: &str| {
                Err(Error::InvalidParams(format!(
                    "calibration.maps.{kind}: {m}"
                )))
            };
            if serde_json::from_value::<Kind>(serde_json::Value::String(kind.clone())).is_err() {
                return bad("not a finding kind");
            }
            if knots.len() < 2 {
                return bad("needs at least two knots");
            }
            for k in knots {
                if !(0.0..=1.0).contains(&k[0]) || !(0.0..=1.0).contains(&k[1]) {
                    return bad("knots must be inside [0, 1] (NaN included)");
                }
            }
            for w in knots.windows(2) {
                if w[1][0] <= w[0][0] {
                    return bad("raw values must be strictly increasing");
                }
                if w[1][1] < w[0][1] {
                    return bad("calibrated values must not decrease");
                }
            }
        }
        Ok(())
    }
}

fn interpolate(knots: &[[f64; 2]], x: f64) -> f64 {
    let (first, last) = (knots[0], knots[knots.len() - 1]);
    if x <= first[0] {
        return first[1];
    }
    if x >= last[0] {
        return last[1];
    }
    for w in knots.windows(2) {
        if x <= w[1][0] {
            let t = (x - w[0][0]) / (w[1][0] - w[0][0]);
            return w[0][1] + t * (w[1][1] - w[0][1]);
        }
    }
    last[1]
}

/// Replace each finding's confidence by its kind's map, if there is one. Kinds without a map are
/// untouched (not even rounded again).
pub fn apply(findings: &mut [Finding], p: &CalibrationParams) {
    if p.maps.is_empty() {
        return;
    }
    for f in findings {
        if let Some(knots) = p.maps.get(&kind_name(f.kind)) {
            f.confidence = math::round_output(interpolate(knots, f.confidence));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    fn finding(kind: Kind, confidence: f64) -> Finding {
        Finding {
            kind,
            confidence,
            bbox: None,
            polygon: None,
            attributes: Map::new(),
            evidence: vec![],
        }
    }

    fn params(map: &[[f64; 2]]) -> CalibrationParams {
        CalibrationParams {
            maps: BTreeMap::from([("stamp".to_owned(), map.to_vec())]),
        }
    }

    #[test]
    fn interpolates_and_holds_flat_outside() {
        let p = params(&[[0.2, 0.0], [0.6, 0.5], [1.0, 0.9]]);
        p.validate().unwrap();
        let mut fs = vec![
            finding(Kind::Stamp, 0.1),
            finding(Kind::Stamp, 0.4),
            finding(Kind::Stamp, 1.0),
            finding(Kind::Seal, 0.4),
        ];
        apply(&mut fs, &p);
        assert_eq!(fs[0].confidence, 0.0);
        assert_eq!(fs[1].confidence, 0.25);
        assert_eq!(fs[2].confidence, 0.9);
        assert_eq!(fs[3].confidence, 0.4, "a kind without a map is untouched");
    }

    #[test]
    fn identity_without_maps() {
        let mut fs = vec![finding(Kind::Stamp, 0.123_456_789)];
        apply(
            &mut fs,
            &CalibrationParams {
                maps: BTreeMap::new(),
            },
        );
        assert_eq!(fs[0].confidence, 0.123_456_789);
    }

    #[test]
    fn bad_maps_are_rejected() {
        assert!(params(&[[0.0, 0.0]]).validate().is_err());
        assert!(params(&[[0.5, 0.0], [0.5, 1.0]]).validate().is_err());
        assert!(params(&[[0.0, 0.8], [1.0, 0.2]]).validate().is_err());
        assert!(params(&[[0.0, 0.0], [1.0, 1.5]]).validate().is_err());
        assert!(params(&[[0.0, f64::NAN], [1.0, 1.0]]).validate().is_err());
        let p = CalibrationParams {
            maps: BTreeMap::from([("nonsense".to_owned(), vec![[0.0, 0.0], [1.0, 1.0]])]),
        };
        assert!(p.validate().is_err());
    }
}
