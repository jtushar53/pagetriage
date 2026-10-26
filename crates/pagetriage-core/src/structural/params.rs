//! Parameters of the structural layer (M1, M2, M12), loaded from `params/text_layer.toml`,
//! `font_script.toml`, `provenance.toml` and `structural_limits.toml`.
//!
//! They are kept apart from [`crate::params::Params`] on purpose: the structural layer takes bytes
//! of a PDF, not pixels, and has its own entry point and override string. Unknown keys are an
//! error, as for the pixel parameters.

use serde::Deserialize;

use crate::error::{Error, Result};

const TEXT_LAYER_TOML: &str = include_str!("../../params/text_layer.toml");
const FONT_SCRIPT_TOML: &str = include_str!("../../params/font_script.toml");
const PROVENANCE_TOML: &str = include_str!("../../params/provenance.toml");
const LIMITS_TOML: &str = include_str!("../../params/structural_limits.toml");

/// Evidence weights of the text-layer rules (see `params/text_layer.toml`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextLayerWeights {
    pub base_logit: f64,
    pub hidden_fraction: f64,
    pub image_cover: f64,
    pub glyph_overlap: f64,
    pub glyph_count: f64,
    pub producer_ocr: f64,
    pub vector_density: f64,
    pub no_image: f64,
    pub paint_ops: f64,
    pub mixed: f64,
}

/// Module M1 thresholds.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextLayerParams {
    pub min_glyphs: u32,
    pub hidden_glyph_fraction: f64,
    pub image_cover_fraction: f64,
    pub occluding_image_cover_fraction: f64,
    pub glyph_overlap_fraction: f64,
    pub mixed_image_cover_fraction: f64,
    pub raster_min_cover_fraction: f64,
    pub vector_min_paint_ops: u32,
    pub small_path_max_page_fraction: f64,
    pub small_path_min_segments: u32,
    pub outlined_text_min_paths: u32,
    pub coverage_grid: u32,
    pub approx_advance_em: f64,
    pub max_text_records: usize,
    pub redaction_dark_gray: f64,
    pub redaction_light_gray: f64,
    pub redaction_cover_fraction: f64,
    pub redaction_min_glyphs: u32,
    pub redaction_max_page_fraction: f64,
    pub weights: TextLayerWeights,
}

/// A piecewise-linear llr transform: `llr_at_lo` at or below `lo`, `llr_at_hi` at or above `hi`.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Piecewise {
    pub lo: f64,
    pub hi: f64,
    pub llr_at_lo: f64,
    pub llr_at_hi: f64,
}

impl Piecewise {
    pub fn apply(&self, x: f64) -> f64 {
        if x <= self.lo {
            self.llr_at_lo
        } else if x >= self.hi {
            self.llr_at_hi
        } else {
            let t = (x - self.lo) / (self.hi - self.lo);
            self.llr_at_lo + t * (self.llr_at_hi - self.llr_at_lo)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnglishBigrams {
    pub top_bigrams: Vec<String>,
}

/// How a curated font name is compared with a used font's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NameMatch {
    Prefix,
    Contains,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strength {
    Strong,
    Weak,
}

/// One entry of the curated legacy-font list.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyFont {
    pub name: String,
    pub script: String,
    #[serde(rename = "match")]
    pub matching: NameMatch,
    pub strength: Strength,
}

/// Module M2 thresholds and the curated legacy-font list.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontScriptParams {
    pub prior_logit: f64,
    pub emit_min_confidence: f64,
    pub strong_name_llr: f64,
    pub weak_name_llr: f64,
    pub min_sample_letters: usize,
    pub max_sample_chars: usize,
    pub tj_space_adjustment: f64,
    pub unreliable_glyph_share: f64,
    pub default_legacy_script: String,
    pub undecodable_unreliable_share: f64,
    pub max_cmap_entries: usize,
    pub bigram: Piecewise,
    pub mid_caps: Piecewise,
    pub marker: Piecewise,
    pub english: EnglishBigrams,
    pub legacy_font: Vec<LegacyFont>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProducerPattern {
    pub pattern: String,
    pub family: String,
    pub ocr: bool,
}

/// Module M12 (structure part) settings.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceParams {
    pub max_revisions: usize,
    pub max_eof_markers: usize,
    pub max_revision_parse_bytes: usize,
    pub max_images_listed: usize,
    pub max_jpeg_header_bytes: usize,
    pub max_meta_chars: usize,
    pub max_xmp_bytes: usize,
    pub ijg_fit_mean_abs_error: f64,
    pub producer: Vec<ProducerPattern>,
}

/// Resource limits (see `params/structural_limits.toml`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsParams {
    pub max_input_bytes: usize,
    pub max_decompressed_bytes: usize,
    pub max_objects: usize,
    pub max_pages: usize,
    pub max_page_tree_depth: usize,
    pub max_form_depth: usize,
    pub max_forms_per_page: usize,
    pub max_content_bytes_per_page: usize,
    /// `u64`: on wasm32 `usize` is 32 bits and this budget is meant to exceed 4 GiB.
    pub max_content_bytes_total: u64,
    pub max_ops_per_page: usize,
    pub max_operands: usize,
    pub max_q_depth: usize,
    pub max_fonts_per_page: usize,
    pub max_images_per_page: usize,
}

/// Everything the structural layer reads from `params/`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuralParams {
    pub text_layer: TextLayerParams,
    pub font_script: FontScriptParams,
    pub provenance: ProvenanceParams,
    pub limits: LimitsParams,
}

impl StructuralParams {
    /// The embedded defaults.
    pub fn defaults() -> Result<Self> {
        Self::from_table(defaults_table()?)
    }

    /// Embedded defaults with `overrides_toml` merged on top. Overrides use the section layout of
    /// [`StructuralParams::defaults_toml`]: `[text_layer]`, `[font_script]`, `[provenance]` and
    /// `[limits]`.
    pub fn with_overrides(overrides_toml: &str) -> Result<Self> {
        let mut table = defaults_table()?;
        let overrides: toml::Table = toml::from_str(overrides_toml)
            .map_err(|e| Error::InvalidParams(format!("override TOML: {e}")))?;
        merge(&mut table, overrides);
        Self::from_table(table)
    }

    /// The embedded defaults as one TOML document, comments included.
    pub fn defaults_toml() -> String {
        let section = |name: &str, src: &str| format!("[{name}]\n{}\n", nest(name, src));
        format!(
            "# --- text_layer (M1) ---\n{}\n# --- font_script (M2) ---\n{}\n\
             # --- provenance (M12) ---\n{}\n# --- limits ---\n{}",
            section("text_layer", TEXT_LAYER_TOML),
            section("font_script", FONT_SCRIPT_TOML),
            section("provenance", PROVENANCE_TOML),
            section("limits", LIMITS_TOML),
        )
    }

    fn from_table(table: toml::Table) -> Result<Self> {
        let p: StructuralParams = toml::Value::Table(table)
            .try_into()
            .map_err(|e| Error::InvalidParams(e.to_string()))?;
        p.validate()?;
        Ok(p)
    }

    fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParams(m.into()));
        let t = &self.text_layer;
        let fractions = [
            ("text_layer.hidden_glyph_fraction", t.hidden_glyph_fraction),
            ("text_layer.image_cover_fraction", t.image_cover_fraction),
            (
                "text_layer.occluding_image_cover_fraction",
                t.occluding_image_cover_fraction,
            ),
            (
                "text_layer.glyph_overlap_fraction",
                t.glyph_overlap_fraction,
            ),
            (
                "text_layer.mixed_image_cover_fraction",
                t.mixed_image_cover_fraction,
            ),
            (
                "text_layer.raster_min_cover_fraction",
                t.raster_min_cover_fraction,
            ),
            (
                "text_layer.small_path_max_page_fraction",
                t.small_path_max_page_fraction,
            ),
        ];
        for (name, v) in fractions {
            if !(v.is_finite() && v > 0.0 && v <= 1.0) {
                return bad(&format!("{name} must be in (0, 1], got {v}"));
            }
        }
        if !(2..=512).contains(&t.coverage_grid) {
            return bad("text_layer.coverage_grid must be in 2..=512");
        }
        if !(t.approx_advance_em.is_finite() && (0.0..=4.0).contains(&t.approx_advance_em)) {
            return bad("text_layer.approx_advance_em must be in [0, 4]");
        }
        for (name, v) in [
            ("redaction_dark_gray", t.redaction_dark_gray),
            ("redaction_light_gray", t.redaction_light_gray),
            ("redaction_cover_fraction", t.redaction_cover_fraction),
            ("redaction_max_page_fraction", t.redaction_max_page_fraction),
        ] {
            if !(v.is_finite() && (0.0..=1.0).contains(&v)) {
                return bad(&format!("text_layer.{name} must be in [0, 1], got {v}"));
            }
        }
        if t.redaction_dark_gray >= t.redaction_light_gray || t.redaction_min_glyphs == 0 {
            return bad(
                "text_layer.redaction_dark_gray must be below redaction_light_gray and redaction_min_glyphs at least 1",
            );
        }
        let w = &t.weights;
        for (name, v) in [
            ("base_logit", w.base_logit),
            ("hidden_fraction", w.hidden_fraction),
            ("image_cover", w.image_cover),
            ("glyph_overlap", w.glyph_overlap),
            ("glyph_count", w.glyph_count),
            ("producer_ocr", w.producer_ocr),
            ("vector_density", w.vector_density),
            ("no_image", w.no_image),
            ("paint_ops", w.paint_ops),
            ("mixed", w.mixed),
        ] {
            if !v.is_finite() {
                return bad(&format!("text_layer.weights.{name} must be finite"));
            }
        }

        let f = &self.font_script;
        for (name, v) in [
            ("prior_logit", f.prior_logit),
            ("strong_name_llr", f.strong_name_llr),
            ("weak_name_llr", f.weak_name_llr),
            ("tj_space_adjustment", f.tj_space_adjustment),
        ] {
            if !v.is_finite() {
                return bad(&format!("font_script.{name} must be finite"));
            }
        }
        for (name, v) in [
            ("emit_min_confidence", f.emit_min_confidence),
            ("unreliable_glyph_share", f.unreliable_glyph_share),
            (
                "undecodable_unreliable_share",
                f.undecodable_unreliable_share,
            ),
        ] {
            if !(0.0..=1.0).contains(&v) {
                return bad(&format!("font_script.{name} must be in [0, 1]"));
            }
        }
        for (name, p) in [
            ("bigram", &f.bigram),
            ("mid_caps", &f.mid_caps),
            ("marker", &f.marker),
        ] {
            let ok = [p.lo, p.hi, p.llr_at_lo, p.llr_at_hi]
                .iter()
                .all(|v| v.is_finite())
                && p.hi > p.lo;
            if !ok {
                return bad(&format!(
                    "font_script.{name} needs finite values and hi > lo"
                ));
            }
        }
        if f.english
            .top_bigrams
            .iter()
            .any(|b| b.len() != 2 || !b.bytes().all(|c| c.is_ascii_lowercase()))
        {
            return bad(
                "font_script.english.top_bigrams must be two lower-case ASCII letters each",
            );
        }
        if f.max_sample_chars == 0 || f.max_cmap_entries == 0 {
            return bad("font_script.max_sample_chars and max_cmap_entries must be > 0");
        }
        if super::unicode::Script::from_id(&f.default_legacy_script).is_none() {
            return bad("font_script.default_legacy_script is not a script id");
        }
        for e in &f.legacy_font {
            let norm = super::legacy::normalise_name(e.name.as_bytes());
            if norm != e.name || norm.len() < 3 {
                return bad(&format!(
                    "font_script.legacy_font {:?} must be normalised (lower-case letters and digits, 3+ chars)",
                    e.name
                ));
            }
            if super::unicode::Script::from_id(&e.script).is_none() {
                return bad(&format!(
                    "font_script.legacy_font {:?}: unknown script",
                    e.name
                ));
            }
        }

        let p = &self.provenance;
        if p.max_revisions == 0 || p.max_eof_markers == 0 || p.max_jpeg_header_bytes < 64 {
            return bad("provenance limits must be positive (max_jpeg_header_bytes >= 64)");
        }
        if !p.ijg_fit_mean_abs_error.is_finite() || p.ijg_fit_mean_abs_error < 0.0 {
            return bad("provenance.ijg_fit_mean_abs_error must be finite and >= 0");
        }
        if p.producer
            .iter()
            .any(|x| x.pattern.is_empty() || x.pattern != x.pattern.to_lowercase())
        {
            return bad("provenance.producer patterns must be non-empty and lower-case");
        }

        let l = &self.limits;
        if l.max_input_bytes == 0
            || l.max_decompressed_bytes == 0
            || l.max_pages == 0
            || l.max_page_tree_depth == 0
            || l.max_operands == 0
            || l.max_ops_per_page == 0
        {
            return bad("limits must be positive");
        }
        Ok(())
    }
}

/// Re-root the sub-tables of a params file under `section`, so that the file's `[weights]` becomes
/// `[text_layer.weights]` and `[[legacy_font]]` becomes `[[font_script.legacy_font]]`.
fn nest(section: &str, src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 64);
    for line in src.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("[[") {
            out.push_str(&format!("[[{section}.{rest}"));
        } else if t.starts_with('[') && t.ends_with(']') && t.len() > 2 {
            out.push_str(&format!("[{section}.{}", &t[1..]));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

fn defaults_table() -> Result<toml::Table> {
    let parse = |name: &str, src: &str| -> Result<toml::Value> {
        toml::from_str::<toml::Table>(src)
            .map(toml::Value::Table)
            .map_err(|e| Error::InvalidParams(format!("embedded params/{name}.toml: {e}")))
    };
    let mut table = toml::Table::new();
    table.insert("text_layer".into(), parse("text_layer", TEXT_LAYER_TOML)?);
    table.insert(
        "font_script".into(),
        parse("font_script", FONT_SCRIPT_TOML)?,
    );
    table.insert("provenance".into(), parse("provenance", PROVENANCE_TOML)?);
    table.insert("limits".into(), parse("structural_limits", LIMITS_TOML)?);
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
        let p = StructuralParams::defaults().unwrap();
        assert_eq!(p.text_layer.hidden_glyph_fraction, 0.90);
        assert_eq!(p.text_layer.image_cover_fraction, 0.85);
        assert!(p.font_script.legacy_font.len() > 20);
        assert_eq!(p.font_script.english.top_bigrams.len(), 50);
    }

    #[test]
    fn defaults_toml_round_trips() {
        let table: toml::Table = toml::from_str(&StructuralParams::defaults_toml()).unwrap();
        assert_eq!(
            StructuralParams::from_table(table).unwrap(),
            StructuralParams::defaults().unwrap()
        );
    }

    #[test]
    fn override_replaces_one_key() {
        let p = StructuralParams::with_overrides("[text_layer]\nmin_glyphs = 10\n").unwrap();
        assert_eq!(p.text_layer.min_glyphs, 10);
        assert_eq!(p.text_layer.coverage_grid, 64);
    }

    #[test]
    fn unknown_and_out_of_range_values_are_rejected() {
        for bad in [
            "[text_layer]\nmin_glyph = 3\n",
            "[nonexistent]\nx = 1\n",
            "[text_layer]\nimage_cover_fraction = 1.5\n",
            "[text_layer]\ncoverage_grid = 1\n",
            "[font_script]\nemit_min_confidence = 2.0\n",
            "[font_script.bigram]\nlo = 0.9\nhi = 0.1\n",
            "[limits]\nmax_pages = 0\n",
            "not toml [",
        ] {
            assert!(StructuralParams::with_overrides(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn piecewise_is_flat_outside_and_linear_inside() {
        let p = Piecewise {
            lo: 0.0,
            hi: 1.0,
            llr_at_lo: 2.0,
            llr_at_hi: -2.0,
        };
        assert_eq!(p.apply(-5.0), 2.0);
        assert_eq!(p.apply(5.0), -2.0);
        assert_eq!(p.apply(0.5), 0.0);
        assert_eq!(p.apply(0.25), 1.0);
    }
}
