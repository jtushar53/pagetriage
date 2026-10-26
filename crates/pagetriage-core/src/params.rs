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
const PHASH_TOML: &str = include_str!("../params/phash.toml");
const QUALITY_TOML: &str = include_str!("../params/quality.toml");
const LAYOUT_TOML: &str = include_str!("../params/layout.toml");
const SCRIPT_TOML: &str = include_str!("../params/script.toml");
const HANDWRITING_TOML: &str = include_str!("../params/handwriting.toml");
const RULINGS_TOML: &str = include_str!("../params/rulings.toml");
const STAMP_TOML: &str = include_str!("../params/stamp.toml");
const SIGNATURE_TOML: &str = include_str!("../params/signature.toml");
const THUMB_TOML: &str = include_str!("../params/thumb.toml");
const REDACTION_TOML: &str = include_str!("../params/redaction.toml");
const QR_TOML: &str = include_str!("../params/qr.toml");
const PLAN_TOML: &str = include_str!("../params/plan.toml");
const ORIENTATION_TOML: &str = include_str!("../params/orientation.toml");
const PHOTO_TOML: &str = include_str!("../params/photo.toml");
const COVERAGE_TOML: &str = include_str!("../params/coverage.toml");
const EMBOSS_TOML: &str = include_str!("../params/emboss.toml");
const PAGE_TYPE_TOML: &str = include_str!("../params/page_type.toml");
const CONSISTENCY_TOML: &str = include_str!("../params/consistency.toml");
const CALIBRATION_TOML: &str = include_str!("../params/calibration.toml");
const DOCUMENT_TOML: &str = include_str!("../params/document.toml");

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

/// Perceptual hashing and near-duplicate pages (see `params/phash.toml`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhashParams {
    pub max_hamming: u32,
    pub dhash_max_hamming: u32,
    pub flat_rms: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FusionParams {
    pub emit_min_confidence: f64,
    pub llr_clip: f64,
    pub group_cap: f64,
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
    pub phash: PhashParams,
    pub quality: crate::pixel::quality::QualityParams,
    pub layout: crate::pixel::layout::LayoutParams,
    pub script: crate::pixel::script::ScriptParams,
    pub handwriting: crate::pixel::handwriting::HandwritingParams,
    pub rulings: crate::pixel::rulings::RulingsParams,
    pub stamp: crate::pixel::stamp::StampParams,
    pub signature: crate::pixel::signature::SignatureParams,
    pub thumb: crate::pixel::thumb::ThumbParams,
    pub redaction: crate::pixel::redaction::RedactionParams,
    pub qr: crate::pixel::qr::QrParams,
    pub plan: crate::fusion::plan::PlanParams,
    pub orientation: crate::pixel::orientation::OrientationParams,
    pub photo: crate::pixel::photo::PhotoParams,
    pub coverage: crate::pixel::coverage::CoverageParams,
    pub emboss: crate::pixel::emboss::EmbossParams,
    pub page_type: crate::fusion::page_type::PageTypeParams,
    pub consistency: crate::fusion::consistency::ConsistencyParams,
    pub calibration: crate::fusion::calibration::CalibrationParams,
    pub document: crate::fusion::document::DocumentParams,
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
             # --- howe ---\n[howe]\n{HOWE_TOML}\n# --- phash ---\n[phash]\n{PHASH_TOML}\n\
             # --- quality ---\n[quality]\n{QUALITY_TOML}\n# --- layout ---\n[layout]\n{LAYOUT_TOML}\n# --- script ---\n[script]\n{SCRIPT_TOML}\n# --- handwriting ---\n[handwriting]\n{HANDWRITING_TOML}\n# --- rulings ---\n[rulings]\n{RULINGS_TOML}\n# --- stamp ---\n[stamp]\n{STAMP_TOML}\n# --- signature ---\n[signature]\n{SIGNATURE_TOML}\n# --- thumb ---\n[thumb]\n{THUMB_TOML}\n# --- redaction ---\n[redaction]\n{REDACTION_TOML}\n# --- qr ---\n[qr]\n{QR_TOML}\n# --- plan ---\n[plan]\n{PLAN_TOML}\n# --- orientation ---\n[orientation]\n{ORIENTATION_TOML}\n# --- photo ---\n[photo]\n{PHOTO_TOML}\n# --- coverage ---\n[coverage]\n{COVERAGE_TOML}\n# --- emboss ---\n[emboss]\n{EMBOSS_TOML}\n# --- page_type ---\n[page_type]\n{PAGE_TYPE_TOML}\n# --- consistency ---\n[consistency]\n{CONSISTENCY_TOML}\n# --- calibration ---\n[calibration]\n{CALIBRATION_TOML}\n# --- document ---\n[document]\n{DOCUMENT_TOML}"
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
        self.quality.validate()?;
        self.layout.validate()?;
        self.script.validate()?;
        self.handwriting.validate()?;
        self.rulings.validate()?;
        self.stamp.validate()?;
        self.signature.validate()?;
        self.thumb.validate()?;
        self.redaction.validate()?;
        self.qr.validate()?;
        self.plan.validate()?;
        self.orientation.validate()?;
        self.photo.validate()?;
        self.coverage.validate()?;
        self.emboss.validate()?;
        self.page_type.validate()?;
        self.consistency.validate()?;
        self.calibration.validate()?;
        self.document.validate()?;
        self.validate_binarizers()?;
        self.validate_phash()
    }

    fn validate_phash(&self) -> Result<()> {
        let p = &self.phash;
        if p.max_hamming > 64 || p.dhash_max_hamming > 64 {
            return Err(Error::InvalidParams(
                "phash.max_hamming and phash.dhash_max_hamming must be in 0..=64".into(),
            ));
        }
        if !p.flat_rms.is_finite() || p.flat_rms < 0.0 {
            return Err(Error::InvalidParams(
                "phash.flat_rms must be finite and >= 0".into(),
            ));
        }
        Ok(())
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
    table.insert("phash".into(), parse("phash", PHASH_TOML)?);
    table.insert("quality".into(), parse("quality", QUALITY_TOML)?);
    table.insert("layout".into(), parse("layout", LAYOUT_TOML)?);
    table.insert("script".into(), parse("script", SCRIPT_TOML)?);
    table.insert(
        "handwriting".into(),
        parse("handwriting", HANDWRITING_TOML)?,
    );
    table.insert("rulings".into(), parse("rulings", RULINGS_TOML)?);
    table.insert("stamp".into(), parse("stamp", STAMP_TOML)?);
    table.insert("signature".into(), parse("signature", SIGNATURE_TOML)?);
    table.insert("thumb".into(), parse("thumb", THUMB_TOML)?);
    table.insert("redaction".into(), parse("redaction", REDACTION_TOML)?);
    table.insert("qr".into(), parse("qr", QR_TOML)?);
    table.insert("plan".into(), parse("plan", PLAN_TOML)?);
    table.insert(
        "orientation".into(),
        parse("orientation", ORIENTATION_TOML)?,
    );
    table.insert("photo".into(), parse("photo", PHOTO_TOML)?);
    table.insert("coverage".into(), parse("coverage", COVERAGE_TOML)?);
    table.insert("emboss".into(), parse("emboss", EMBOSS_TOML)?);
    table.insert("page_type".into(), parse("page_type", PAGE_TYPE_TOML)?);
    table.insert(
        "consistency".into(),
        parse("consistency", CONSISTENCY_TOML)?,
    );
    table.insert(
        "calibration".into(),
        parse("calibration", CALIBRATION_TOML)?,
    );
    table.insert("document".into(), parse("document", DOCUMENT_TOML)?);
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
