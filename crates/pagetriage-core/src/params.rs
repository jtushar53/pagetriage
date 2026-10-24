//! Detector parameters. Defaults are the `params/*.toml` files embedded at build time; callers can
//! override any key at runtime with a TOML string. Unknown keys are an error, so a typo in an
//! override cannot silently do nothing.

use serde::Deserialize;

use crate::error::{Error, Result};

const FUSION_TOML: &str = include_str!("../params/fusion.toml");
const BLANK_PAGE_TOML: &str = include_str!("../params/blank_page.toml");

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FusionParams {
    pub emit_min_confidence: f64,
    pub llr_clip: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlankPageParams {
    pub ink_delta: u8,
    pub blank_ink_fraction: f64,
    pub coverage_floor: f64,
    pub llr_per_ln_unit: f64,
    pub prior_logit: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Params {
    pub fusion: FusionParams,
    pub blank_page: BlankPageParams,
}

impl Params {
    /// The embedded defaults.
    pub fn defaults() -> Result<Self> {
        Self::from_table(defaults_table()?)
    }

    /// Embedded defaults with `overrides_toml` merged on top. Overrides use the same layout as the
    /// combined table: `[blank_page]` / `[fusion]` sections with the keys from `params/*.toml`.
    pub fn with_overrides(overrides_toml: &str) -> Result<Self> {
        let mut table = defaults_table()?;
        let overrides: toml::Table = toml::from_str(overrides_toml)
            .map_err(|e| Error::InvalidParams(format!("override TOML: {e}")))?;
        merge(&mut table, overrides);
        Self::from_table(table)
    }

    /// The embedded defaults as one TOML document (what `pagetriage params` prints).
    pub fn defaults_toml() -> String {
        format!("# --- fusion ---\n[fusion]\n{FUSION_TOML}\n# --- blank_page ---\n[blank_page]\n{BLANK_PAGE_TOML}")
    }

    fn from_table(table: toml::Table) -> Result<Self> {
        let params: Params = toml::Value::Table(table)
            .try_into()
            .map_err(|e| Error::InvalidParams(e.to_string()))?;
        params.validate()?;
        Ok(params)
    }

    fn validate(&self) -> Result<()> {
        let finite = [
            ("fusion.emit_min_confidence", self.fusion.emit_min_confidence),
            ("fusion.llr_clip", self.fusion.llr_clip),
            ("blank_page.blank_ink_fraction", self.blank_page.blank_ink_fraction),
            ("blank_page.coverage_floor", self.blank_page.coverage_floor),
            ("blank_page.llr_per_ln_unit", self.blank_page.llr_per_ln_unit),
            ("blank_page.prior_logit", self.blank_page.prior_logit),
        ];
        for (name, v) in finite {
            if !v.is_finite() {
                return Err(Error::InvalidParams(format!("{name} must be finite, got {v}")));
            }
        }
        if !(0.0..=1.0).contains(&self.fusion.emit_min_confidence) {
            return Err(Error::InvalidParams("fusion.emit_min_confidence must be in [0, 1]".into()));
        }
        if self.fusion.llr_clip < 0.0 {
            return Err(Error::InvalidParams("fusion.llr_clip must be >= 0".into()));
        }
        if !(self.blank_page.blank_ink_fraction > 0.0 && self.blank_page.blank_ink_fraction <= 1.0) {
            return Err(Error::InvalidParams("blank_page.blank_ink_fraction must be in (0, 1]".into()));
        }
        if self.blank_page.coverage_floor <= 0.0 {
            return Err(Error::InvalidParams("blank_page.coverage_floor must be > 0".into()));
        }
        Ok(())
    }
}

fn defaults_table() -> Result<toml::Table> {
    let parse = |name: &str, src: &str| -> Result<toml::Value> {
        toml::from_str::<toml::Table>(src)
            .map(toml::Value::Table)
            .map_err(|e| Error::InvalidParams(format!("embedded params/{name}.toml: {e}")))
    };
    let mut table = toml::Table::new();
    table.insert("fusion".into(), parse("fusion", FUSION_TOML)?);
    table.insert("blank_page".into(), parse("blank_page", BLANK_PAGE_TOML)?);
    Ok(table)
}

/// Recursive table merge: values from `over` replace values in `base`.
fn merge(base: &mut toml::Table, over: toml::Table) {
    for (key, value) in over {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge(b, o),
            (_, v) => {
                base.insert(key, v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_defaults_parse_and_validate() {
        let p = Params::defaults().unwrap();
        assert_eq!(p.blank_page.ink_delta, 60);
        assert_eq!(p.blank_page.blank_ink_fraction, 0.002);
    }

    #[test]
    fn defaults_toml_round_trips_to_the_same_params() {
        let combined = Params::defaults_toml();
        let table: toml::Table = toml::from_str(&combined).unwrap();
        assert_eq!(Params::from_table(table).unwrap(), Params::defaults().unwrap());
    }

    #[test]
    fn override_replaces_one_key_and_keeps_the_rest() {
        let p = Params::with_overrides("[blank_page]\nink_delta = 30\n").unwrap();
        assert_eq!(p.blank_page.ink_delta, 30);
        assert_eq!(p.blank_page.blank_ink_fraction, 0.002);
    }

    #[test]
    fn unknown_keys_and_sections_are_rejected() {
        assert!(Params::with_overrides("[blank_page]\nink_delat = 30\n").is_err());
        assert!(Params::with_overrides("[nonexistent]\nx = 1\n").is_err());
    }

    #[test]
    fn out_of_range_values_are_rejected() {
        assert!(Params::with_overrides("[fusion]\nemit_min_confidence = 1.5\n").is_err());
        assert!(Params::with_overrides("[blank_page]\nink_delta = 300\n").is_err());
        assert!(Params::with_overrides("[blank_page]\ncoverage_floor = 0.0\n").is_err());
        assert!(Params::with_overrides("not toml at all [").is_err());
    }
}
