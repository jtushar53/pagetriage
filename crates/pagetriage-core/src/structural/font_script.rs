//! Module M2 `structural.font_script`: per-page script shares and the legacy-font verdict, from
//! what the interpreter saw each font show. Pure: takes plain observations, so it is testable
//! without a PDF.

use std::collections::BTreeMap;

use super::legacy::{self, FontVerdict};
use super::params::FontScriptParams;
use super::types::{FontInfo, ScriptInfo};
use super::unicode::{Histogram, Script};
use crate::error::Result;
use crate::math;
use crate::schema::{Evidence, TextLayer, TextLayerState};

const DETECTOR_ID: &str = "structural.font_script";
const CITATION: &str =
    "module M2: legacy-encoded Indian fonts; undecodable text (Identity-H without ToUnicode)";

/// What one font showed on a page.
#[derive(Debug, Clone)]
pub struct FontObservation {
    /// `/BaseFont` and other names the font goes by (first is the display name).
    pub names: Vec<String>,
    pub subtype: String,
    pub embedded: bool,
    pub to_unicode: bool,
    pub encoding: Option<String>,
    pub glyphs: u32,
    pub undecodable: u32,
    pub hist: Histogram,
    pub sample: String,
}

/// Summary the text-layer reliability rule and the consistency rule need.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptSummary {
    pub info: ScriptInfo,
    pub undecodable_share: f64,
    /// Highest legacy log-odds among the legacy fonts (0 if none).
    pub legacy_logit: f64,
}

fn strip_subset_prefix(name: &str) -> &str {
    let b = name.as_bytes();
    if b.len() > 7 && b[6] == b'+' && b[..6].iter().all(u8::is_ascii_uppercase) {
        &name[7..]
    } else {
        name
    }
}

fn share(n: u32, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        f64::from(n) / f64::from(total)
    }
}

/// Combine the fonts of one page.
pub fn summarise(fonts: &[FontObservation], p: &FontScriptParams) -> Result<ScriptSummary> {
    let mut page_hist = Histogram::default();
    let mut infos: Vec<FontInfo> = Vec::new();
    let mut evidence: Vec<Evidence> = Vec::new();
    let mut total_glyphs = 0u32;
    let mut undecodable = 0u32;
    let mut legacy_glyphs = 0u32;
    let mut legacy_by_script: BTreeMap<usize, u32> = BTreeMap::new();
    let mut legacy_logit = f64::NEG_INFINITY;

    for (i, f) in fonts.iter().enumerate() {
        if f.glyphs == 0 {
            continue;
        }
        total_glyphs = total_glyphs.saturating_add(f.glyphs);
        undecodable = undecodable.saturating_add(f.undecodable);
        let label = format!("F{i}");
        let names: Vec<&[u8]> = f.names.iter().map(String::as_bytes).collect();
        let verdict: FontVerdict = legacy::judge_font(&names, &label, &f.sample, p)?;
        let mut hist = f.hist.clone();
        if verdict.legacy {
            // The Latin code points of a legacy font are Indic letters in disguise.
            let script = verdict.script.unwrap_or(Script::Devanagari);
            let moved = hist.letters[Script::Latin.index()];
            hist.letters[Script::Latin.index()] = 0;
            hist.letters[script.index()] += moved;
            legacy_glyphs = legacy_glyphs.saturating_add(f.glyphs);
            *legacy_by_script.entry(script.index()).or_insert(0) += f.glyphs;
            legacy_logit = legacy_logit.max(verdict.logit);
        }
        page_hist.merge(&hist);
        if (verdict.legacy || verdict.name_hit.is_some()) && evidence.len() < 16 {
            evidence.extend(verdict.evidence.iter().cloned());
        }
        infos.push(FontInfo {
            name: strip_subset_prefix(f.names.first().map_or("", String::as_str)).to_owned(),
            subtype: f.subtype.clone(),
            embedded: f.embedded,
            to_unicode: f.to_unicode,
            encoding: f.encoding.clone(),
            glyphs: f.glyphs,
            undecodable_glyphs: f.undecodable,
            legacy_encoded: verdict.legacy,
            legacy_name_match: verdict.name_hit.as_ref().map(|h| h.entry.clone()),
            legacy_confidence: verdict.confidence,
        });
    }
    infos.sort_by(|a, b| b.glyphs.cmp(&a.glyphs).then_with(|| a.name.cmp(&b.name)));
    infos.truncate(32);

    let letters = page_hist.total_letters();
    let mut shares = BTreeMap::new();
    if letters > 0 {
        for s in Script::ALL {
            let c = page_hist.count(s);
            if c > 0 {
                shares.insert(s.id().to_owned(), c as f64 / letters as f64);
            }
        }
    }
    let legacy_encoded = legacy_glyphs > 0;
    let legacy_script = legacy_by_script
        .iter()
        .max_by_key(|(idx, g)| (**g, std::cmp::Reverse(**idx)))
        .map(|(idx, _)| Script::ALL[*idx]);
    let dominant = page_hist.dominant();
    let (dominant_id, name) = match dominant {
        Some(s) if legacy_encoded && Some(s) == legacy_script => (
            s.id().to_owned(),
            format!("{} (legacy-encoded)", s.display_name()),
        ),
        Some(s) => (s.id().to_owned(), s.display_name().to_owned()),
        None if total_glyphs > 0 && undecodable > 0 => ("unknown".to_owned(), "Unknown".to_owned()),
        None => ("none".to_owned(), "None".to_owned()),
    };
    let info = ScriptInfo {
        dominant: dominant_id,
        name,
        shares,
        letters: letters.min(u64::from(u32::MAX)) as u32,
        glyphs_undecodable: undecodable,
        legacy_encoded,
        legacy_script: legacy_script
            .filter(|_| legacy_encoded)
            .map(|s| s.id().to_owned()),
        legacy_glyph_share: share(legacy_glyphs, total_glyphs),
        fonts: infos,
        evidence,
    };
    Ok(ScriptSummary {
        undecodable_share: share(undecodable, total_glyphs),
        legacy_logit: if legacy_logit.is_finite() {
            legacy_logit
        } else {
            0.0
        },
        info,
    })
}

/// Fill `text_layer.reliable` from the script summary, and add the evidence behind a `false`.
/// Only pages whose layer carries text get a verdict; others stay `None`.
pub fn apply_reliability(
    layer: &mut TextLayer,
    s: &ScriptSummary,
    p: &FontScriptParams,
) -> Result<()> {
    if !matches!(
        layer.state,
        TextLayerState::NativeText | TextLayerState::Mixed | TextLayerState::InvisibleOcrUnderImage
    ) {
        return Ok(());
    }
    let legacy_bad = s.info.legacy_encoded && s.info.legacy_glyph_share >= p.unreliable_glyph_share;
    let undec_bad = s.undecodable_share >= p.undecodable_unreliable_share;
    layer.reliable = Some(!(legacy_bad || undec_bad));
    if legacy_bad {
        layer.evidence.push(Evidence::new(
            DETECTOR_ID,
            "legacy_glyph_share",
            s.info.legacy_glyph_share,
            Some(p.unreliable_glyph_share),
            -s.legacy_logit,
            format!(
                "{:.0}% of glyphs are set in legacy-encoded {} fonts; extracted text is not Unicode",
                s.info.legacy_glyph_share * 100.0,
                s.info.legacy_script.as_deref().unwrap_or("Indic")
            ),
            CITATION,
        )?);
    }
    if undec_bad {
        let clamped = s.undecodable_share.clamp(0.01, 0.99);
        layer.evidence.push(Evidence::new(
            DETECTOR_ID,
            "undecodable_glyph_share",
            s.undecodable_share,
            Some(p.undecodable_unreliable_share),
            -math::ln(clamped / (1.0 - clamped)),
            format!(
                "{:.0}% of glyphs cannot be mapped to Unicode (no ToUnicode map)",
                s.undecodable_share * 100.0
            ),
            CITATION,
        )?);
    }
    Ok(())
}

/// Machine id of a state, as in the JSON output.
pub fn state_id(state: TextLayerState) -> &'static str {
    match state {
        TextLayerState::NativeText => "native_text",
        TextLayerState::RasterOnly => "raster_only",
        TextLayerState::InvisibleOcrUnderImage => "invisible_ocr_under_image",
        TextLayerState::Mixed => "mixed",
        TextLayerState::VectorOnly => "vector_only",
        TextLayerState::Empty => "empty",
    }
}

/// Consistency rule: a page with no text layer (`raster_only`, `vector_only`, `empty`) cannot also
/// be set in legacy-encoded fonts, because legacy fonts only exist where text is drawn.
pub fn conflicts(state: TextLayerState, script: &ScriptInfo) -> Vec<String> {
    let mut out = Vec::new();
    if script.legacy_encoded
        && matches!(
            state,
            TextLayerState::RasterOnly | TextLayerState::VectorOnly | TextLayerState::Empty
        )
    {
        out.push(format!(
            "{}+legacy_{}",
            state_id(state),
            script.legacy_script.as_deref().unwrap_or("indic")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structural::params::StructuralParams;

    fn params() -> FontScriptParams {
        StructuralParams::defaults().unwrap().font_script
    }

    fn font(name: &str, glyphs: u32, text: &str) -> FontObservation {
        let mut hist = Histogram::default();
        for c in text.chars() {
            hist.add(c as u32);
        }
        FontObservation {
            names: vec![name.to_owned()],
            subtype: "TrueType".into(),
            embedded: true,
            to_unicode: false,
            encoding: Some("WinAnsiEncoding".into()),
            glyphs,
            undecodable: 0,
            hist,
            sample: text.to_owned(),
        }
    }

    #[test]
    fn english_page_is_latin() {
        let t = "The vendor shall convey the property to the purchaser free from encumbrances";
        let s = summarise(&[font("ABCDEF+TimesNewRoman", 70, t)], &params()).unwrap();
        assert_eq!(s.info.dominant, "latin");
        assert_eq!(s.info.name, "Latin");
        assert!(!s.info.legacy_encoded);
        assert!((s.info.shares["latin"] - 1.0).abs() < 1e-12);
        assert_eq!(s.info.fonts[0].name, "TimesNewRoman");
    }

    #[test]
    fn unicode_devanagari_page() {
        let t = "भारत सरकार";
        let s = summarise(&[font("Mangal", 9, t)], &params()).unwrap();
        assert_eq!(s.info.dominant, "devanagari");
        assert_eq!(s.info.name, "Devanagari");
        assert!(!s.info.legacy_encoded);
    }

    #[test]
    fn legacy_font_is_relabelled_devanagari_and_marked_unreliable() {
        let t = "Hkkjr ljdkj ds ukeku ls ;g foys[k";
        let mut layer = TextLayer {
            state: TextLayerState::NativeText,
            confidence: 0.99,
            reliable: None,
            evidence: vec![],
        };
        let s = summarise(&[font("KrutiDev010", 30, t)], &params()).unwrap();
        assert!(s.info.legacy_encoded);
        assert_eq!(s.info.dominant, "devanagari");
        assert_eq!(s.info.name, "Devanagari (legacy-encoded)");
        assert_eq!(s.info.legacy_script.as_deref(), Some("devanagari"));
        assert!(!s.info.shares.contains_key("latin"));
        apply_reliability(&mut layer, &s, &params()).unwrap();
        assert_eq!(layer.reliable, Some(false));
        assert!(
            layer
                .evidence
                .iter()
                .any(|e| e.feature == "legacy_glyph_share" && e.llr < 0.0)
        );
    }

    #[test]
    fn small_legacy_heading_on_an_english_page_flags_but_stays_reliable() {
        let eng = "The vendor shall convey the property to the purchaser free from encumbrances "
            .repeat(10);
        let eng_glyphs = eng.chars().filter(|c| !c.is_whitespace()).count() as u32;
        let s = summarise(
            &[
                font("Times", eng_glyphs, &eng),
                font("KrutiDev010", 5, "Hkkjr"),
            ],
            &params(),
        )
        .unwrap();
        assert!(s.info.legacy_encoded);
        assert_eq!(s.info.dominant, "latin");
        assert_eq!(s.info.name, "Latin");
        assert!(s.info.legacy_glyph_share < 0.02);
        let mut layer = TextLayer {
            state: TextLayerState::NativeText,
            confidence: 0.99,
            reliable: None,
            evidence: vec![],
        };
        apply_reliability(&mut layer, &s, &params()).unwrap();
        assert_eq!(layer.reliable, Some(true));
    }

    #[test]
    fn undecodable_text_is_unreliable_and_dominant_is_unknown() {
        let mut f = font("ABCDEF+Arial", 100, "");
        f.subtype = "Type0".into();
        f.undecodable = 100;
        let s = summarise(&[f], &params()).unwrap();
        assert_eq!(s.info.dominant, "unknown");
        let mut layer = TextLayer {
            state: TextLayerState::NativeText,
            confidence: 0.9,
            reliable: None,
            evidence: vec![],
        };
        apply_reliability(&mut layer, &s, &params()).unwrap();
        assert_eq!(layer.reliable, Some(false));
        assert!(
            layer
                .evidence
                .iter()
                .any(|e| e.feature == "undecodable_glyph_share")
        );
    }

    #[test]
    fn pages_without_text_get_no_reliability_verdict_and_conflicts_are_flagged() {
        let s = summarise(&[font("KrutiDev010", 2, "Hk")], &params()).unwrap();
        let mut layer = TextLayer {
            state: TextLayerState::RasterOnly,
            confidence: 0.9,
            reliable: None,
            evidence: vec![],
        };
        apply_reliability(&mut layer, &s, &params()).unwrap();
        assert_eq!(layer.reliable, None);
        assert_eq!(
            conflicts(TextLayerState::RasterOnly, &s.info),
            vec!["raster_only+legacy_devanagari".to_string()]
        );
        assert!(conflicts(TextLayerState::NativeText, &s.info).is_empty());
    }

    #[test]
    fn no_fonts_is_none() {
        let s = summarise(&[], &params()).unwrap();
        assert_eq!(s.info.dominant, "none");
        assert!(s.info.shares.is_empty());
    }
}
