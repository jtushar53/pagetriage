//! Page-level entry point shared by the CLI, the Python binding and the WASM binding.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::params::Params;
use crate::pixel::content_type::{self, BLANK_PAGE};
use crate::schema::{self, PageReport};

/// Detectors that can be named in `Options::detectors`. Grows as phases land.
pub const DETECTORS: &[&str] = &[BLANK_PAGE];

/// One rasterised page. `gray` is `width * height` bytes, row-major. `rgb`, when present, is
/// `3 * width * height` interleaved bytes of the same page (colour detectors use it from Phase 3).
#[derive(Debug, Clone, Copy)]
pub struct PageInput<'a> {
    pub gray: &'a [u8],
    pub rgb: Option<&'a [u8]>,
    pub width: u32,
    pub height: u32,
    pub dpi: u16,
}

/// Parsed from `opts_json`. Unknown keys are rejected so a typo cannot silently do nothing.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Options {
    /// Include raw detector feature values in the report under `debug`.
    pub debug: bool,
    /// Run only these detectors; `None` runs all of [`DETECTORS`].
    pub detectors: Option<Vec<String>>,
    /// TOML merged over the embedded `params/*.toml` defaults.
    pub params: Option<String>,
    /// Value for the report's `index` field (the page's position in its document).
    pub page_index: u32,
}

impl Options {
    pub fn from_json(opts_json: &str) -> Result<Self> {
        if opts_json.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str(opts_json).map_err(|e| Error::InvalidOptions(e.to_string()))
    }
}

/// Analyse one page and return the canonical JSON of its `PageReport` (see
/// [`schema::to_canonical_json`]). This is the string the determinism test compares byte for byte.
///
/// `structural_json` is the structural-layer result for this page. It is reserved for Phase 1: it
/// must be a JSON object if given, and is otherwise ignored for now.
pub fn analyse_page(
    input: &PageInput<'_>,
    structural_json: Option<&str>,
    opts_json: &str,
) -> Result<String> {
    schema::to_canonical_json(&analyse_page_report(input, structural_json, opts_json)?)
}

/// Same as [`analyse_page`] but returns the typed report.
pub fn analyse_page_report(
    input: &PageInput<'_>,
    structural_json: Option<&str>,
    opts_json: &str,
) -> Result<PageReport> {
    let options = Options::from_json(opts_json)?;
    validate_input(input)?;
    if let Some(s) = structural_json {
        match serde_json::from_str::<serde_json::Value>(s) {
            Ok(serde_json::Value::Object(_)) => {}
            Ok(_) => {
                return Err(Error::InvalidInput(
                    "structural_json must be a JSON object".into(),
                ));
            }
            Err(e) => return Err(Error::InvalidInput(format!("structural_json: {e}"))),
        }
    }

    let params = match &options.params {
        Some(toml) => Params::with_overrides(toml)?,
        None => Params::defaults()?,
    };

    let selected: Vec<&str> = match &options.detectors {
        None => DETECTORS.to_vec(),
        Some(names) => {
            for n in names {
                if !DETECTORS.contains(&n.as_str()) {
                    return Err(Error::InvalidOptions(format!(
                        "unknown detector {n:?}; available: {}",
                        DETECTORS.join(", ")
                    )));
                }
            }
            // Run in registry order regardless of the order the caller listed them.
            DETECTORS
                .iter()
                .copied()
                .filter(|d| names.iter().any(|n| n == d))
                .collect()
        }
    };

    let mut findings = Vec::new();
    let mut debug: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
    for name in selected {
        let detection = match name {
            BLANK_PAGE => {
                content_type::detect_blank_page(input.gray, &params.blank_page, &params.fusion)?
            }
            other => unreachable!("detector {other:?} is in DETECTORS but not dispatched"),
        };
        findings.extend(detection.findings);
        debug.insert(name.to_owned(), detection.features);
    }

    let dpi = f64::from(input.dpi);
    Ok(PageReport {
        index: options.page_index,
        size_pt: [
            f64::from(input.width) * 72.0 / dpi,
            f64::from(input.height) * 72.0 / dpi,
        ],
        analysis_dpi: input.dpi,
        timings_ms: None,
        text_layer: None,
        quality: None,
        findings,
        page_type: None,
        phash: None,
        near_duplicate_of: None,
        debug: options.debug.then_some(debug),
    })
}

fn validate_input(input: &PageInput<'_>) -> Result<()> {
    if input.width == 0 || input.height == 0 {
        return Err(Error::InvalidInput(format!(
            "empty page: {}x{}",
            input.width, input.height
        )));
    }
    if input.dpi == 0 {
        return Err(Error::InvalidInput("dpi must be at least 1".into()));
    }
    let pixels = (input.width as usize)
        .checked_mul(input.height as usize)
        .ok_or_else(|| Error::InvalidInput("page dimensions overflow".into()))?;
    if input.gray.len() != pixels {
        return Err(Error::InvalidInput(format!(
            "gray buffer has {} bytes, expected {} for {}x{}",
            input.gray.len(),
            pixels,
            input.width,
            input.height
        )));
    }
    if let Some(rgb) = input.rgb {
        let expected = pixels
            .checked_mul(3)
            .ok_or_else(|| Error::InvalidInput("page dimensions overflow".into()))?;
        if rgb.len() != expected {
            return Err(Error::InvalidInput(format!(
                "rgb buffer has {} bytes, expected {} for {}x{}",
                rgb.len(),
                expected,
                input.width,
                input.height
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn white(w: u32, h: u32) -> Vec<u8> {
        vec![250u8; (w * h) as usize]
    }

    fn input(gray: &[u8], w: u32, h: u32) -> PageInput<'_> {
        PageInput {
            gray,
            rgb: None,
            width: w,
            height: h,
            dpi: 150,
        }
    }

    #[test]
    fn blank_page_report_has_a_finding_and_no_debug_by_default() {
        let g = white(40, 60);
        let r = analyse_page_report(&input(&g, 40, 60), None, "{}").unwrap();
        assert_eq!(r.findings.len(), 1);
        assert!(r.debug.is_none());
        assert_eq!(r.analysis_dpi, 150);
        assert_eq!(r.size_pt, [40.0 * 72.0 / 150.0, 60.0 * 72.0 / 150.0]);
    }

    #[test]
    fn debug_option_exposes_raw_features() {
        let g = white(10, 10);
        let r = analyse_page_report(&input(&g, 10, 10), None, r#"{"debug":true}"#).unwrap();
        assert_eq!(r.debug.unwrap()["blank_page"]["paper_level"], 250.0);
    }

    #[test]
    fn repeated_calls_give_identical_bytes() {
        let g = white(32, 32);
        let a = analyse_page(&input(&g, 32, 32), None, r#"{"debug":true}"#).unwrap();
        let b = analyse_page(&input(&g, 32, 32), None, r#"{"debug":true}"#).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn empty_options_string_means_defaults() {
        let g = white(8, 8);
        assert!(analyse_page(&input(&g, 8, 8), None, "").is_ok());
    }

    #[test]
    fn bad_options_are_rejected() {
        let g = white(8, 8);
        let i = input(&g, 8, 8);
        for bad in [
            "not json",
            r#"{"debug":"yes"}"#,
            r#"{"degub":true}"#,
            r#"{"detectors":["nope"]}"#,
            r#"{"params":"[blank_page]\nink_delat = 3"}"#,
        ] {
            assert!(analyse_page(&i, None, bad).is_err(), "should reject {bad}");
        }
    }

    #[test]
    fn detector_filter_can_select_none() {
        let g = white(8, 8);
        let r = analyse_page_report(&input(&g, 8, 8), None, r#"{"detectors":[]}"#).unwrap();
        assert!(r.findings.is_empty());
    }

    #[test]
    fn params_override_changes_the_outcome() {
        // 30% of pixels are faint (50 below paper): not ink at ink_delta 60, ink at ink_delta 40.
        let mut g = white(10, 10);
        for px in g.iter_mut().take(30) {
            *px = 200;
        }
        let i = input(&g, 10, 10);
        let default = analyse_page_report(&i, None, "{}").unwrap();
        assert_eq!(default.findings.len(), 1);
        let tight =
            analyse_page_report(&i, None, r#"{"params":"[blank_page]\nink_delta = 40"}"#).unwrap();
        assert!(tight.findings.is_empty());
    }

    #[test]
    fn page_index_option_sets_index() {
        let g = white(4, 4);
        let r = analyse_page_report(&input(&g, 4, 4), None, r#"{"page_index":7}"#).unwrap();
        assert_eq!(r.index, 7);
    }

    #[test]
    fn buffer_and_dimension_validation() {
        let g = white(4, 4);
        assert!(
            analyse_page(&input(&g, 5, 4), None, "{}").is_err(),
            "length mismatch"
        );
        assert!(
            analyse_page(&input(&[], 0, 4), None, "{}").is_err(),
            "zero width"
        );
        let mut i = input(&g, 4, 4);
        i.dpi = 0;
        assert!(analyse_page(&i, None, "{}").is_err(), "zero dpi");
        let rgb = vec![0u8; 4 * 4 * 3 - 1];
        let mut i = input(&g, 4, 4);
        i.rgb = Some(&rgb);
        assert!(analyse_page(&i, None, "{}").is_err(), "short rgb");
        let rgb = vec![0u8; 4 * 4 * 3];
        i.rgb = Some(&rgb);
        assert!(analyse_page(&i, None, "{}").is_ok());
    }

    #[test]
    fn structural_json_must_be_an_object_when_given() {
        let g = white(4, 4);
        let i = input(&g, 4, 4);
        assert!(analyse_page(&i, Some("{}"), "{}").is_ok());
        assert!(analyse_page(&i, Some("[]"), "{}").is_err());
        assert!(analyse_page(&i, Some("{oops"), "{}").is_err());
    }
}
