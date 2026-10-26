//! Page-level entry point shared by the CLI, the Python binding and the WASM binding.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::fusion::{calibration, consistency, page_type, plan};
use crate::params::Params;
use crate::pixel::content_type::{self, BLANK_PAGE};
use crate::pixel::coverage::{self, COVERAGE};
use crate::pixel::emboss::{self, EMBOSS};
use crate::pixel::handwriting::{self, HANDWRITING};
use crate::pixel::layout::{self, LAYOUT};
use crate::pixel::orientation::{self, ORIENTATION};
use crate::pixel::phash::{self, PHASH};
use crate::pixel::photo::{self, PHOTO};
use crate::pixel::qr::{self, QR};
use crate::pixel::quality;
use crate::pixel::redaction::{self, REDACTION};
use crate::pixel::rulings::{self, RULINGS};
use crate::pixel::script::{self, SCRIPT};
use crate::pixel::signature::{self, SIGNATURE};
use crate::pixel::stamp::{self, STAMP};
use crate::pixel::thumb::{self, THUMB};
use crate::schema::{self, Evidence, Finding, Kind, PageReport};
use crate::structural::PageStructure;

/// The page plan is not a detector with findings of its own: naming it runs the layout and the
/// orientation reading and attaches `plan` to the report.
pub const PLAN: &str = "plan";

/// Detectors that can be named in `Options::detectors`. Grows as phases land. One name per line, in
/// alphabetical order, so that branches adding detectors do not edit the same line.
#[rustfmt::skip]
pub const DETECTORS: &[&str] = &[
    BLANK_PAGE,
    COVERAGE,
    EMBOSS,
    HANDWRITING,
    LAYOUT,
    ORIENTATION,
    PHASH,
    PHOTO,
    PLAN,
    QR,
    REDACTION,
    RULINGS,
    SCRIPT,
    SIGNATURE,
    STAMP,
    THUMB,
];

/// Detectors in [`DETECTORS`] that run only when `Options::detectors` names them. A new detector
/// starts here, so the default report (and every golden file) stays as it is until the detector has
/// been reviewed; promoting it means deleting its line. Same layout rule as [`DETECTORS`].
#[rustfmt::skip]
pub const OPT_IN: &[&str] = &[
    COVERAGE,
    EMBOSS,
    HANDWRITING,
    LAYOUT,
    ORIENTATION,
    PHOTO,
    PLAN,
    QR,
    REDACTION,
    RULINGS,
    SCRIPT,
    SIGNATURE,
    STAMP,
    THUMB,
];

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
    /// Run only these detectors; `None` runs all of [`DETECTORS`] except those in [`OPT_IN`].
    pub detectors: Option<Vec<String>>,
    /// TOML merged over the embedded `params/*.toml` defaults.
    pub params: Option<String>,
    /// Where the page's text layer or an OCR run put text, `[x0, y0, x1, y1]` in page pixels at the
    /// analysis DPI, for the `coverage` detector. An empty list means "no text"; leaving it out means
    /// "unknown" (a PDF's own text boxes from `structural_json` are used then).
    pub text_boxes: Option<Vec<[u32; 4]>>,
    /// Refuse pages with more pixels than this (an `InvalidInput` error instead of running out of
    /// memory; a poster sheet at 300 dpi is about 70 million). `None` sets no limit.
    pub max_pixels: Option<u64>,
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
/// `structural_json` is the structural-layer result for this page: one element of the `pages` array
/// that `structural::analyse_pdf_structure` returns. Its `text_layer` fills the report's
/// `text_layer`, and a legacy-encoded script becomes a `script` finding. It must be a JSON object;
/// one without a `text_layer` key (such as `{}`) is accepted and ignored.
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
    validate_input(input, options.max_pixels)?;
    let structure = match structural_json {
        Some(s) => parse_structural(s)?,
        None => None,
    };

    let params = match &options.params {
        Some(toml) => Params::with_overrides(toml)?,
        None => Params::defaults()?,
    };

    let mut selected: Vec<&str> = match &options.detectors {
        None => DETECTORS
            .iter()
            .copied()
            .filter(|d| !OPT_IN.contains(d))
            .collect(),
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

    // The plan reads the orientation finding and the layout, so naming it brings them in.
    if selected.contains(&PLAN) && !selected.contains(&ORIENTATION) {
        selected.push(ORIENTATION);
        selected.sort_by_key(|d| DETECTORS.iter().position(|x| x == d));
    }

    // Module M3: degradation triage. Always runs; it is not a finding. It runs first because the
    // layout pass levels the page by its skew.
    let assessment = quality::assess(
        input.gray,
        input.width as usize,
        input.height as usize,
        input.dpi,
        &params,
    )?;

    let layout_selected = selected.contains(&LAYOUT);
    let plan_selected = selected.contains(&PLAN);

    // Text boxes for the coverage detector: the caller's, else the PDF's own from the structure.
    let text_boxes: Option<Vec<[u32; 4]>> = options.text_boxes.clone().or_else(|| {
        let scale = f64::from(input.dpi) / 72.0;
        structure
            .as_ref()
            .and_then(|ps| ps.text_boxes_pt.as_ref())
            .map(|boxes| {
                boxes
                    .iter()
                    .map(|b| {
                        let px = |v: f64| (v * scale).max(0.0).round() as u32;
                        [px(b[0]), px(b[1]), px(b[2]), px(b[3])]
                    })
                    .collect()
            })
    });

    // The layout pass is shared by every detector that works line by line; run it once.
    let page_layout = if selected.iter().any(|d| {
        matches!(
            *d,
            COVERAGE | HANDWRITING | LAYOUT | PLAN | REDACTION | RULINGS | SCRIPT | SIGNATURE
        )
    }) {
        Some(layout::segment(
            input.gray,
            input.width as usize,
            input.height as usize,
            assessment.quality.skew_deg,
            &params,
        )?)
    } else {
        None
    };

    let mut findings = Vec::new();
    let mut debug: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
    let mut hashes = None;
    for name in selected {
        let detection = match name {
            BLANK_PAGE => {
                content_type::detect_blank_page(input.gray, &params.blank_page, &params.fusion)?
            }
            COVERAGE => coverage::detect(
                page_layout.as_ref().expect("segmented above"),
                text_boxes.as_deref(),
                input.dpi,
                &params,
            )?,
            EMBOSS => emboss::detect(
                input.gray,
                input.width as usize,
                input.height as usize,
                input.dpi,
                &params,
            )?,
            HANDWRITING => {
                handwriting::detect(page_layout.as_ref().expect("segmented above"), &params)?
            }
            LAYOUT => layout::summarize(page_layout.as_ref().expect("segmented above")),
            ORIENTATION => orientation::detect(
                input.gray,
                input.width as usize,
                input.height as usize,
                input.dpi,
                &params,
            )?,
            PLAN => crate::pixel::Detection::default(),
            PHOTO => photo::detect(
                input.gray,
                input.width as usize,
                input.height as usize,
                input.dpi,
                &params,
            )?,
            QR => qr::detect(
                input.gray,
                input.width as usize,
                input.height as usize,
                &params,
            )?,
            REDACTION => redaction::detect(
                page_layout.as_ref().expect("segmented above"),
                input.gray,
                &params,
            )?,
            RULINGS => rulings::detect(
                page_layout.as_ref().expect("segmented above"),
                input.gray,
                input.dpi,
                &params,
            )?,
            SIGNATURE => signature::detect(
                page_layout.as_ref().expect("segmented above"),
                input.dpi,
                &params,
            )?,
            THUMB => thumb::detect(
                input.gray,
                input.width as usize,
                input.height as usize,
                input.dpi,
                &params,
            )?,
            STAMP => stamp::detect(
                input.gray,
                input.rgb,
                input.width as usize,
                input.height as usize,
                input.dpi,
                &params,
            )?,
            SCRIPT => script::detect(page_layout.as_ref().expect("segmented above"), &params)?,
            PHASH => {
                let (detection, page_hashes) = phash::detect(
                    input.gray,
                    input.width as usize,
                    input.height as usize,
                    &params.phash,
                )?;
                hashes = Some(page_hashes);
                detection
            }
            other => unreachable!("detector {other:?} is in DETECTORS but not dispatched"),
        };
        findings.extend(detection.findings);
        debug.insert(name.to_owned(), detection.features);
    }

    // The structural layer's verdict for this page (M1) and its legacy-font finding (M2).
    if let Some(ps) = &structure {
        findings.extend(structure_findings(ps, input.dpi));
    }

    // Module M13: rules across detectors, then the composites of module M11 that are built from
    // other findings (e-stamp QR, stamp-paper header).
    let context = match &page_layout {
        Some(l) => consistency::context(l, &params.consistency),
        None => consistency::PageContext {
            width: input.width,
            height: input.height,
            ..consistency::PageContext::default()
        },
    };
    consistency::apply(&mut findings, &context, &params.consistency, &params.fusion);

    // Module M10: page typing needs the layout, so it runs when `layout` was asked for.
    let page_type = match (&page_layout, layout_selected) {
        (Some(l), true) => {
            let facts = page_type::facts(l, &findings, input.gray, &params.page_type);
            debug.insert("page_type".to_owned(), page_type::features(&facts));
            Some(page_type::classify(&facts, &params.page_type))
        }
        _ => None,
    };

    let page_plan = match (&page_layout, plan_selected) {
        (Some(l), true) => Some(plan::build(
            &plan::PlanInput {
                gray: input.gray,
                width: input.width as usize,
                height: input.height as usize,
                dpi: input.dpi,
                layout: l,
                findings: &findings,
                quality: &assessment.quality,
                badness: assessment
                    .features
                    .get("grade_badness")
                    .copied()
                    .unwrap_or(0.0),
            },
            &params.plan,
        )),
        _ => None,
    };

    calibration::apply(&mut findings, &params.calibration);

    debug.insert("quality".to_owned(), assessment.features);

    let dpi = f64::from(input.dpi);
    Ok(PageReport {
        index: options.page_index,
        size_pt: [
            f64::from(input.width) * 72.0 / dpi,
            f64::from(input.height) * 72.0 / dpi,
        ],
        analysis_dpi: input.dpi,
        timings_ms: None,
        text_layer: structure.as_ref().and_then(|ps| ps.text_layer.clone()),
        quality: Some(assessment.quality),
        findings,
        page_type,
        plan: page_plan,
        phash: hashes.map(|h| h.phash_hex()),
        dhash: hashes.map(|h| h.dhash_hex()),
        near_duplicate_of: None,
        debug: options.debug.then_some(debug),
    })
}

/// The document-level pass over a document's page reports: near-duplicate pages (module M12, when the
/// reports carry hashes) and registrar-endorsement repetition (module M11). `pages_json` is a JSON
/// array of page reports in page order, as `analyse_page` returns them (the `pages` of a `Report`
/// work too); the result is the canonical JSON of the same array with `near_duplicate_of` and the
/// `registrar_endorsement` findings filled. `opts_json` takes `params` (TOML merged over the
/// defaults). Running it again on its own output changes nothing.
pub fn finish_document(pages_json: &str, opts_json: &str) -> Result<String> {
    let options = Options::from_json(opts_json)?;
    let params = match &options.params {
        Some(toml) => Params::with_overrides(toml)?,
        None => Params::defaults()?,
    };
    let mut reports: Vec<PageReport> = serde_json::from_str(pages_json)
        .map_err(|e| Error::InvalidInput(format!("pages_json: {e}")))?;
    if reports.iter().any(|r| r.phash.is_some()) {
        phash::mark_near_duplicates(&mut reports, &params.phash)?;
    }
    crate::fusion::document::mark_repeated_endorsements(
        &mut reports,
        &params.document,
        &params.fusion,
    )?;
    // A page that repeats an earlier one need not be sent anywhere again.
    for r in &mut reports {
        let repeat = r.near_duplicate_of;
        if let Some(plan) = &mut r.plan {
            let is_dup_skip = plan
                .skip
                .as_ref()
                .is_some_and(|s| s.reason == "near_duplicate");
            match repeat {
                Some(of) if plan.skip.is_none() || is_dup_skip => {
                    plan.skip = Some(schema::PlanSkip {
                        reason: "near_duplicate".into(),
                        of: Some(of),
                    });
                }
                None if is_dup_skip => plan.skip = None,
                _ => {}
            }
        }
    }
    schema::to_canonical_json(&reports)
}

/// `structural_json` must be a JSON object. If it has a `text_layer` key it is read as one
/// [`PageStructure`] (an element of `analyse_pdf_structure`'s `pages`); an object without that key,
/// such as `{}`, is accepted and ignored.
fn parse_structural(s: &str) -> Result<Option<PageStructure>> {
    match serde_json::from_str::<serde_json::Value>(s) {
        Ok(serde_json::Value::Object(map)) => {
            if map.contains_key("text_layer") {
                serde_json::from_value(serde_json::Value::Object(map))
                    .map(Some)
                    .map_err(|e| Error::InvalidInput(format!("structural_json: {e}")))
            } else {
                Ok(None)
            }
        }
        Ok(_) => Err(Error::InvalidInput(
            "structural_json must be a JSON object".into(),
        )),
        Err(e) => Err(Error::InvalidInput(format!("structural_json: {e}"))),
    }
}

/// A legacy-encoded page becomes a `script` finding: its extracted text is not Unicode, so any
/// consumer that reads the text layer needs to know.
fn structure_findings(ps: &PageStructure, dpi: u16) -> Vec<Finding> {
    let mut out = covered_text_findings(ps, dpi);
    out.extend(legacy_script_finding(ps));
    out
}

/// A filled box over text that is still in the file (module M12, PDF side): a `redaction_box` whose
/// text can be copied out, or a white one (white-out).
fn covered_text_findings(ps: &PageStructure, dpi: u16) -> Vec<Finding> {
    let scale = f64::from(dpi) / 72.0;
    ps.covered_text
        .iter()
        .map(|c| {
            let mut attributes = serde_json::Map::new();
            attributes.insert("source".into(), "pdf_content_stream".into());
            attributes.insert("fill".into(), c.fill.clone().into());
            attributes.insert("text_recoverable".into(), true.into());
            attributes.insert("glyphs_covered".into(), c.glyphs.into());
            // Seen in the file, not estimated from pixels: the confidence rises with the glyphs
            // under the box (0.5 log-odds each, capped at 12 glyphs) and is not calibrated.
            let llr = 0.5 * f64::from(c.glyphs.min(12));
            let evidence: Vec<Evidence> = Evidence::new(
                "structural.covered_text",
                "glyphs_covered",
                f64::from(c.glyphs),
                None,
                llr,
                format!("{} glyphs of text-show operations lie under a {} box", c.glyphs, c.fill),
                "module M12: a filled rectangle painted over text that is still in the content stream",
            )
            .into_iter()
            .collect();
            let confidence = crate::math::round_output(crate::fusion::scorer::confidence(0.0, &evidence));
            Finding {
                kind: Kind::RedactionBox,
                confidence,
                bbox: c.bbox_pt.map(|b| {
                    let px = |v: f64| (v * scale).max(0.0).round() as u32;
                    [px(b[0]), px(b[1]), px(b[2]), px(b[3])]
                }),
                polygon: None,
                attributes,
                evidence,
            }
        })
        .collect()
}

fn legacy_script_finding(ps: &PageStructure) -> Vec<Finding> {
    let Some(script) = ps.script.as_ref().filter(|s| s.legacy_encoded) else {
        return Vec::new();
    };
    let confidence = script
        .fonts
        .iter()
        .filter(|f| f.legacy_encoded)
        .map(|f| f.legacy_confidence)
        .fold(0.0, f64::max);
    let mut attributes = serde_json::Map::new();
    attributes.insert("script".into(), script.name.clone().into());
    attributes.insert("dominant".into(), script.dominant.clone().into());
    attributes.insert("legacy_encoded".into(), true.into());
    if let Some(s) = &script.legacy_script {
        attributes.insert("legacy_script".into(), s.clone().into());
    }
    attributes.insert(
        "legacy_glyph_share".into(),
        script.legacy_glyph_share.into(),
    );
    vec![Finding {
        kind: Kind::Script,
        confidence,
        bbox: None,
        polygon: None,
        attributes,
        evidence: script.evidence.clone(),
    }]
}

fn validate_input(input: &PageInput<'_>, max_pixels: Option<u64>) -> Result<()> {
    if input.width == 0 || input.height == 0 {
        return Err(Error::InvalidInput(format!(
            "empty page: {}x{}",
            input.width, input.height
        )));
    }
    if input.dpi == 0 {
        return Err(Error::InvalidInput("dpi must be at least 1".into()));
    }
    if let Some(limit) = max_pixels {
        let n = u64::from(input.width) * u64::from(input.height);
        if n > limit {
            return Err(Error::InvalidInput(format!(
                "page of {n} pixels ({}x{}) exceeds max_pixels {limit}",
                input.width, input.height
            )));
        }
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
    fn opt_in_detectors_are_registered_and_listed_in_order() {
        for name in OPT_IN {
            assert!(
                DETECTORS.contains(name),
                "{name} is in OPT_IN but not in DETECTORS"
            );
        }
        for list in [DETECTORS, OPT_IN] {
            assert!(
                list.windows(2).all(|w| w[0] < w[1]),
                "keep {list:?} sorted, one name per line"
            );
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
    fn max_pixels_refuses_a_large_page_with_a_clear_error() {
        let gray = vec![255u8; 100 * 50];
        let input = PageInput {
            gray: &gray,
            rgb: None,
            width: 100,
            height: 50,
            dpi: 150,
        };
        assert!(analyse_page(&input, None, r#"{"max_pixels":5000}"#).is_ok());
        let e = analyse_page(&input, None, r#"{"max_pixels":4999}"#).unwrap_err();
        assert!(e.to_string().contains("exceeds max_pixels 4999"), "{e}");
        assert!(analyse_page(&input, None, r#"{"max_pixels":null}"#).is_ok());
    }

    #[test]
    fn covered_text_becomes_a_redaction_box_in_page_pixels() {
        let structural = r#"{"index":0,"size_pt":[612.0,792.0],"rotation":0,"text_layer":null,"script":null,"features":null,"images":[],"conflicts":[],"notes":[],"covered_text":[{"bbox_pt":[72.0,100.0,144.0,124.0],"fill":"black","glyphs":10},{"bbox_pt":null,"fill":"white","glyphs":4}]}"#;
        let gray = vec![255u8; 8 * 8];
        let input = PageInput {
            gray: &gray,
            rgb: None,
            width: 8,
            height: 8,
            dpi: 144,
        };
        let r = analyse_page_report(&input, Some(structural), r#"{"detectors":[]}"#).unwrap();
        let boxes: Vec<_> = r
            .findings
            .iter()
            .filter(|f| f.kind == Kind::RedactionBox)
            .collect();
        assert_eq!(boxes.len(), 2);
        // 144 dpi is two pixels per point.
        assert_eq!(boxes[0].bbox, Some([144, 200, 288, 248]));
        assert_eq!(boxes[0].attributes["fill"], "black");
        assert_eq!(boxes[0].attributes["text_recoverable"], true);
        assert!(
            boxes[0].confidence > boxes[1].confidence,
            "more glyphs, more confident"
        );
        assert_eq!(boxes[1].bbox, None, "a rotated page's box is not placed");
    }

    #[test]
    fn structural_page_fills_text_layer_and_legacy_finding() {
        use crate::schema::{TextLayer, TextLayerState};
        use crate::structural::types::{FontInfo, PageStructure, ScriptInfo};
        let ps = PageStructure {
            covered_text: Vec::new(),
            text_boxes_pt: None,
            index: 0,
            size_pt: [612.0, 792.0],
            rotation: 0,
            text_layer: Some(TextLayer {
                state: TextLayerState::NativeText,
                confidence: 0.99,
                reliable: Some(false),
                evidence: vec![],
            }),
            script: Some(ScriptInfo {
                dominant: "devanagari".into(),
                name: "Devanagari (legacy-encoded)".into(),
                shares: std::collections::BTreeMap::new(),
                letters: 10,
                glyphs_undecodable: 0,
                legacy_encoded: true,
                legacy_script: Some("devanagari".into()),
                legacy_glyph_share: 1.0,
                fonts: vec![FontInfo {
                    name: "KrutiDev010".into(),
                    subtype: "TrueType".into(),
                    embedded: true,
                    to_unicode: false,
                    encoding: None,
                    glyphs: 10,
                    undecodable_glyphs: 0,
                    legacy_encoded: true,
                    legacy_name_match: Some("krutidev".into()),
                    legacy_confidence: 0.97,
                }],
                evidence: vec![],
            }),
            features: None,
            images: vec![],
            conflicts: vec![],
            notes: vec![],
        };
        let json = serde_json::to_string(&ps).unwrap();
        let g = white(8, 8);
        let r = analyse_page_report(&input(&g, 8, 8), Some(&json), r#"{"detectors":[]}"#).unwrap();
        assert_eq!(r.text_layer.unwrap().state, TextLayerState::NativeText);
        assert_eq!(r.findings.len(), 1);
        assert_eq!(r.findings[0].kind, Kind::Script);
        assert_eq!(r.findings[0].confidence, 0.97);
        // A structural object that claims a text layer but has the wrong shape is refused.
        assert!(analyse_page(&input(&g, 8, 8), Some(r#"{"text_layer":5}"#), "{}").is_err());
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
