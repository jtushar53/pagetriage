//! Detector parameters. Defaults are the `params/*.toml` files embedded at build time; callers can
//! override any key at runtime with a TOML string. Unknown keys are an error, so a typo in an
//! override cannot silently do nothing.

use serde::Deserialize;

use crate::error::{Error, Result};

const FUSION_TOML: &str = include_str!("../params/fusion.toml");
const BLANK_PAGE_TOML: &str = include_str!("../params/blank_page.toml");
const SAUVOLA_TOML: &str = include_str!("../params/sauvola.toml");
const WOLF_TOML: &str = include_str!("../params/wolf.toml");
const HOWE_TOML: &str = include_str!("../params/howe.toml");

/// Sauvola local threshold (see `params/sauvola.toml`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SauvolaParams {
    pub window: usize,
    pub k: f64,
    pub dynamic_range: f64,
}

/// Wolf-Jolion local threshold (see `params/wolf.toml`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WolfParams {
    pub window: usize,
    pub a: f64,
}

/// How much of Howe's parameter tuning to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoweMode {
    Static,
    TuneC,
    Dual,
}

/// Howe graph-cut binariser (see `params/howe.toml`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoweParams {
    pub mode: HoweMode,
    pub radius: f64,
    pub outlier_sigmas: f64,
    pub phi: f64,
    pub cost_scale: f64,
    pub t_lo: f64,
    pub sigma_e: f64,
    pub static_c: f64,
    pub static_t_hi: f64,
    pub tau1: f64,
    pub tau2: f64,
    pub c_start: f64,
    pub c_steps: usize,
    pub steps_per_octave: u32,
    pub curve_smoothing: Vec<f64>,
}

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
    pub sauvola: SauvolaParams,
    pub wolf: WolfParams,
    pub howe: HoweParams,
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
        format!(
            "# --- fusion ---\n[fusion]\n{FUSION_TOML}\n# --- blank_page ---\n[blank_page]\n{BLANK_PAGE_TOML}\n\
             # --- sauvola ---\n[sauvola]\n{SAUVOLA_TOML}\n# --- wolf ---\n[wolf]\n{WOLF_TOML}\n\
             # --- howe ---\n[howe]\n{HOWE_TOML}"
        )
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
            (
                "fusion.emit_min_confidence",
                self.fusion.emit_min_confidence,
            ),
            ("fusion.llr_clip", self.fusion.llr_clip),
            (
                "blank_page.blank_ink_fraction",
                self.blank_page.blank_ink_fraction,
            ),
            ("blank_page.coverage_floor", self.blank_page.coverage_floor),
            (
                "blank_page.llr_per_ln_unit",
                self.blank_page.llr_per_ln_unit,
            ),
            ("blank_page.prior_logit", self.blank_page.prior_logit),
        ];
        for (name, v) in finite {
            if !v.is_finite() {
                return Err(Error::InvalidParams(format!(
                    "{name} must be finite, got {v}"
                )));
            }
        }
        if !(0.0..=1.0).contains(&self.fusion.emit_min_confidence) {
            return Err(Error::InvalidParams(
                "fusion.emit_min_confidence must be in [0, 1]".into(),
            ));
        }
        if self.fusion.llr_clip < 0.0 {
            return Err(Error::InvalidParams("fusion.llr_clip must be >= 0".into()));
        }
        if !(self.blank_page.blank_ink_fraction > 0.0 && self.blank_page.blank_ink_fraction <= 1.0)
        {
            return Err(Error::InvalidParams(
                "blank_page.blank_ink_fraction must be in (0, 1]".into(),
            ));
        }
        if self.blank_page.coverage_floor <= 0.0 {
            return Err(Error::InvalidParams(
                "blank_page.coverage_floor must be > 0".into(),
            ));
        }
        self.validate_binarizers()
    }

    fn validate_binarizers(&self) -> Result<()> {
        let bad = |msg: &str| Err(Error::InvalidParams(msg.into()));
        let (s, w, h) = (&self.sauvola, &self.wolf, &self.howe);
        for (name, window) in [("sauvola.window", s.window), ("wolf.window", w.window)] {
            if window == 0 || window.is_multiple_of(2) || window > 10_001 {
                return bad(&format!(
                    "{name} must be odd and in 1..=10001, got {window}"
                ));
            }
        }
        let finite = [
            ("sauvola.k", s.k),
            ("sauvola.dynamic_range", s.dynamic_range),
            ("wolf.a", w.a),
            ("howe.radius", h.radius),
            ("howe.outlier_sigmas", h.outlier_sigmas),
            ("howe.phi", h.phi),
            ("howe.cost_scale", h.cost_scale),
            ("howe.t_lo", h.t_lo),
            ("howe.sigma_e", h.sigma_e),
            ("howe.static_c", h.static_c),
            ("howe.static_t_hi", h.static_t_hi),
            ("howe.tau1", h.tau1),
            ("howe.tau2", h.tau2),
            ("howe.c_start", h.c_start),
        ];
        for (name, v) in finite {
            if !v.is_finite() {
                return bad(&format!("{name} must be finite, got {v}"));
            }
        }
        if !(0.0..=1.0).contains(&s.k) || s.dynamic_range <= 0.0 {
            return bad("sauvola.k must be in [0, 1] and sauvola.dynamic_range > 0");
        }
        if !(0.0..=1.0).contains(&w.a) {
            return bad("wolf.a must be in [0, 1]");
        }
        if h.radius <= 0.0 || h.radius > 500.0 || h.sigma_e <= 0.0 || h.sigma_e > 50.0 {
            return bad("howe.radius must be in (0, 500] and howe.sigma_e in (0, 50]");
        }
        if h.outlier_sigmas < 0.0 || h.outlier_sigmas > 20.0 {
            return bad("howe.outlier_sigmas must be in [0, 20]");
        }
        if h.phi >= 0.0 || h.cost_scale <= 0.0 || h.cost_scale > 1000.0 {
            return bad("howe.phi must be negative and howe.cost_scale in (0, 1000]");
        }
        for (name, v) in [
            ("howe.t_lo", h.t_lo),
            ("howe.static_t_hi", h.static_t_hi),
            ("howe.tau1", h.tau1),
            ("howe.tau2", h.tau2),
        ] {
            if !(0.0..1.0).contains(&v) {
                return bad(&format!("{name} must be in [0, 1), got {v}"));
            }
        }
        if h.c_start <= 0.0 || h.static_c <= 0.0 || h.c_steps < 2 || h.c_steps > 200 {
            return bad("howe.c_start and howe.static_c must be > 0 and howe.c_steps in 2..=200");
        }
        if h.steps_per_octave == 0 || h.steps_per_octave > 64 {
            return bad("howe.steps_per_octave must be in 1..=64");
        }
        // The solver uses 32-bit capacities: the largest scaled cost must stay far from overflow.
        let top = h.c_start.max(h.static_c)
            * crate::math::pow(2.0, h.c_steps as f64 / f64::from(h.steps_per_octave))
            * h.cost_scale;
        if top > 1.0e8 || (-h.phi * h.cost_scale) > 1.0e8 {
            return bad("howe: scaled costs would overflow the 32-bit solver capacities");
        }
        if h.curve_smoothing.is_empty()
            || h.curve_smoothing.len() > 15
            || h.curve_smoothing.iter().any(|v| !v.is_finite() || *v < 0.0)
        {
            return bad("howe.curve_smoothing must be 1..=15 finite non-negative taps");
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
    table.insert("sauvola".into(), parse("sauvola", SAUVOLA_TOML)?);
    table.insert("wolf".into(), parse("wolf", WOLF_TOML)?);
    table.insert("howe".into(), parse("howe", HOWE_TOML)?);
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
        assert_eq!(
            Params::from_table(table).unwrap(),
            Params::defaults().unwrap()
        );
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
