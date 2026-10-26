//! Script family from pixels (module M4): headline, cursive or discrete, from the text lines of the
//! layout pass. No model: four hand-designed features per line, fixed thresholds, a linear score per
//! family and a softmax. See `docs/detectors/script.md`.
//!
//! Tiers. *Headline*: the densest row in the top part of a line is a continuous bar (the shirorekha
//! of Devanagari, Bengali and Gurmukhi), after U. Pal and B. B. Chaudhuri, "Script line separation
//! from Indian multi-script documents", ICDAR 1999. *Component*: how many connected components a
//! line has per unit of width, and how many of them are small, separate the scripts whose letters
//! are joined from those whose letters are not (profile and component features of U. Pal, S. Sinha
//! and B. B. Chaudhuri, ICDAR 2003). The texture tier of the plan (Gabor filter banks) is not built:
//! it needs real pages of the scripts it would separate, and there are none here.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Map;

use super::Detection;
use super::layout::{Layout, Rect};
use crate::error::{Error, Result};
use crate::math;
use crate::params::{FusionParams, Params};
use crate::schema::{Evidence, Finding, Kind};

pub const SCRIPT: &str = "script";
const DETECTOR_ID: &str = "script.pixel";

const CITE_HEADLINE: &str = "module M4 headline tier: Pal and Chaudhuri, ICDAR 1999 (shirorekha as the densest horizontal stroke)";
const CITE_COMPONENT: &str = "module M4 component tier: Pal, Sinha and Chaudhuri, ICDAR 2003 (component and profile features)";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptParams {
    pub top_fraction: f64,
    pub small_height_fraction: f64,
    pub min_lines: usize,
    pub term_clip: f64,
    pub top_peak_threshold: f64,
    pub top_peak_coef: f64,
    pub low_peak_threshold: f64,
    pub low_peak_coef: f64,
    pub comp_density_threshold: f64,
    pub comp_density_coef: f64,
    pub small_share_threshold: f64,
    pub small_share_coef: f64,
    pub secondary_line_share: f64,
    pub emit_min_probability: f64,
}

impl ScriptParams {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParams(format!("script: {m}")));
        let all = [
            ("top_fraction", self.top_fraction),
            ("small_height_fraction", self.small_height_fraction),
            ("term_clip", self.term_clip),
            ("top_peak_threshold", self.top_peak_threshold),
            ("top_peak_coef", self.top_peak_coef),
            ("low_peak_threshold", self.low_peak_threshold),
            ("low_peak_coef", self.low_peak_coef),
            ("comp_density_threshold", self.comp_density_threshold),
            ("comp_density_coef", self.comp_density_coef),
            ("small_share_threshold", self.small_share_threshold),
            ("small_share_coef", self.small_share_coef),
            ("secondary_line_share", self.secondary_line_share),
            ("emit_min_probability", self.emit_min_probability),
        ];
        for (name, v) in all {
            if !v.is_finite() {
                return bad(&format!("{name} must be finite, got {v}"));
            }
        }
        if !(0.0..=1.0).contains(&self.top_fraction)
            || !(0.0..=1.0).contains(&self.small_height_fraction)
        {
            return bad("top_fraction and small_height_fraction must be in [0, 1]");
        }
        if self.term_clip <= 0.0 || self.term_clip > 1.0e3 {
            return bad("term_clip must be in (0, 1000]");
        }
        if !(0.0..=1.0).contains(&self.emit_min_probability)
            || !(0.0..=1.0).contains(&self.secondary_line_share)
        {
            return bad("emit_min_probability and secondary_line_share must be in [0, 1]");
        }
        if self.min_lines == 0 {
            return bad("min_lines must be at least 1");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Headline,
    Cursive,
    Discrete,
}

impl Family {
    pub const ALL: [Family; 3] = [Family::Headline, Family::Cursive, Family::Discrete];

    pub fn name(self) -> &'static str {
        match self {
            Family::Headline => "headline",
            Family::Cursive => "cursive",
            Family::Discrete => "discrete",
        }
    }

    /// The scripts the family stands for. M4 cannot choose among them.
    pub fn scripts(self) -> &'static [&'static str] {
        match self {
            Family::Headline => &["devanagari", "bengali", "gurmukhi"],
            Family::Cursive => &["arabic"],
            Family::Discrete => &[
                "latin",
                "gujarati",
                "odia",
                "tamil",
                "telugu",
                "kannada",
                "malayalam",
            ],
        }
    }
}

/// The measurements of one text line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineFeatures {
    pub width: u32,
    /// Ink in the densest top row / line width.
    pub top_peak: f64,
    /// Ink in the densest low row / line width.
    pub low_peak: f64,
    /// Glyph components per x-height of width.
    pub comp_density: f64,
    /// Share of the line's glyphs that are small.
    pub small_share: f64,
}

/// The measurements of every layout line, in the layout's order.
pub fn line_features(layout: &Layout, p: &ScriptParams) -> Vec<LineFeatures> {
    let mut by_line: Vec<Vec<(u32, u32)>> = vec![Vec::new(); layout.lines.len()];
    for g in &layout.glyphs {
        if let Some(l) = g.line {
            by_line[l].push((g.bbox.height(), g.area));
        }
    }
    layout
        .lines
        .iter()
        .zip(by_line)
        .map(|(line, glyphs)| {
            let r: Rect = line.bbox;
            let (w, h) = (r.width() as usize, r.height() as usize);
            let top_rows = ((p.top_fraction * h as f64).ceil() as usize).clamp(1, h);
            let (mut top_max, mut low_max) = (0usize, 0usize);
            for row in 0..h {
                let start = (r.y0 as usize + row) * layout.width + r.x0 as usize;
                let count = layout.ink[start..start + w]
                    .iter()
                    .filter(|&&v| v != 0)
                    .count();
                if row < top_rows {
                    top_max = top_max.max(count);
                } else {
                    low_max = low_max.max(count);
                }
            }
            let small_limit = p.small_height_fraction * f64::from(line.glyph_height);
            let small = glyphs
                .iter()
                .filter(|&&(gh, _)| f64::from(gh) < small_limit)
                .count();
            LineFeatures {
                width: r.width(),
                top_peak: top_max as f64 / w as f64,
                low_peak: low_max as f64 / w as f64,
                comp_density: glyphs.len() as f64 / (w as f64 / layout.xheight),
                small_share: small as f64 / glyphs.len().max(1) as f64,
            }
        })
        .collect()
}

/// One score term: `coef * (x - threshold)`, clipped. `sign` flips it for a feature that argues
/// for the family when it is *low*.
struct Term {
    feature: &'static str,
    value: f64,
    threshold: f64,
    contribution: f64,
    cite: &'static str,
}

fn term(
    feature: &'static str,
    value: f64,
    threshold: f64,
    coef: f64,
    sign: f64,
    clip: f64,
    cite: &'static str,
) -> Term {
    Term {
        feature,
        value,
        threshold,
        contribution: (sign * coef * (value - threshold)).clamp(-clip, clip),
        cite,
    }
}

/// The three family scores for one set of feature values, with their terms.
fn scores(f: &LineFeatures, p: &ScriptParams) -> [Vec<Term>; 3] {
    let c = p.term_clip;
    let top = |s: f64| {
        term(
            "top_peak",
            f.top_peak,
            p.top_peak_threshold,
            p.top_peak_coef,
            s,
            c,
            CITE_HEADLINE,
        )
    };
    let low = |s: f64| {
        term(
            "low_peak",
            f.low_peak,
            p.low_peak_threshold,
            p.low_peak_coef,
            s,
            c,
            CITE_COMPONENT,
        )
    };
    let dens = |s: f64| {
        term(
            "comp_density",
            f.comp_density,
            p.comp_density_threshold,
            p.comp_density_coef,
            s,
            c,
            CITE_COMPONENT,
        )
    };
    let small = |s: f64| {
        term(
            "small_share",
            f.small_share,
            p.small_share_threshold,
            p.small_share_coef,
            s,
            c,
            CITE_COMPONENT,
        )
    };
    [
        // Headline: a bar at the top, joined letters, not many dots.
        vec![top(1.0), dens(-1.0), small(-0.5)],
        // Cursive: no bar at the top, a baseline, joined letters, many dots.
        vec![top(-1.0), low(1.0), dens(-1.0), small(1.0)],
        // Discrete: no bar at the top, separate letters, not many dots.
        vec![top(-1.0), dens(1.0), small(-0.5)],
    ]
}

/// Softmax over the three family scores.
fn probabilities(totals: [f64; 3]) -> [f64; 3] {
    let max = totals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let e = totals.map(|t| math::exp(t - max));
    let sum = math::pairwise_sum(&e);
    e.map(|v| v / sum)
}

fn total(terms: &[Term]) -> f64 {
    math::pairwise_sum(&terms.iter().map(|t| t.contribution).collect::<Vec<_>>())
}

fn winner(probs: &[f64; 3]) -> usize {
    let mut best = 0;
    for i in 1..3 {
        if probs[i] > probs[best] {
            best = i;
        }
    }
    best
}

/// The family of one line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineScript {
    pub family: Family,
    pub probability: f64,
}

pub fn classify_lines(features: &[LineFeatures], p: &ScriptParams) -> Vec<LineScript> {
    features
        .iter()
        .map(|f| {
            let s = scores(f, p);
            let probs = probabilities([total(&s[0]), total(&s[1]), total(&s[2])]);
            let w = winner(&probs);
            LineScript {
                family: Family::ALL[w],
                probability: probs[w],
            }
        })
        .collect()
}

/// Width-weighted median of `value` over lines.
fn weighted_median(items: &mut [(f64, u32)]) -> f64 {
    items.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: u64 = items.iter().map(|&(_, w)| u64::from(w)).sum();
    let mut acc = 0u64;
    for &(v, w) in items.iter() {
        acc += u64::from(w);
        if acc * 2 >= total {
            return v;
        }
    }
    0.0
}

/// Run M4 on a segmented page.
pub fn detect(layout: &Layout, params: &Params) -> Result<Detection> {
    detect_with(layout, &params.script, &params.fusion)
}

pub fn detect_with(layout: &Layout, p: &ScriptParams, fusion: &FusionParams) -> Result<Detection> {
    let lf = line_features(layout, p);
    let mut det = Detection::default();
    det.features.insert("lines".into(), lf.len() as f64);
    if lf.is_empty() {
        return Ok(det);
    }

    let median = |pick: fn(&LineFeatures) -> f64| {
        let mut items: Vec<(f64, u32)> = lf.iter().map(|f| (pick(f), f.width)).collect();
        weighted_median(&mut items)
    };
    let page = LineFeatures {
        width: 0,
        top_peak: median(|f| f.top_peak),
        low_peak: median(|f| f.low_peak),
        comp_density: median(|f| f.comp_density),
        small_share: median(|f| f.small_share),
    };
    det.features.insert("top_peak".into(), page.top_peak);
    det.features.insert("low_peak".into(), page.low_peak);
    det.features
        .insert("comp_density".into(), page.comp_density);
    det.features.insert("small_share".into(), page.small_share);

    let s = scores(&page, p);
    let probs = probabilities([total(&s[0]), total(&s[1]), total(&s[2])]);
    for (fam, pr) in Family::ALL.iter().zip(probs) {
        det.features.insert(format!("p_{}", fam.name()), pr);
    }
    let votes = classify_lines(&lf, p);
    for fam in Family::ALL {
        let n = votes.iter().filter(|v| v.family == fam).count();
        det.features
            .insert(format!("lines_{}", fam.name()), n as f64);
    }

    let w = winner(&probs);
    let confidence = math::round_output(probs[w]);
    if lf.len() < p.min_lines || confidence < p.emit_min_probability {
        return Ok(det);
    }
    let family = Family::ALL[w];
    let evidence = s[w]
        .iter()
        .map(|t| {
            Evidence::new(
                DETECTOR_ID,
                t.feature,
                t.value,
                Some(t.threshold),
                t.contribution.clamp(-fusion.llr_clip, fusion.llr_clip),
                format!(
                    "page value of {} for the {} family: {:.3} against {:.3}",
                    t.feature,
                    family.name(),
                    t.value,
                    t.threshold
                ),
                t.cite,
            )
        })
        .collect::<Result<Vec<_>>>()?;

    let mut attributes = Map::new();
    attributes.insert("family".into(), family.name().into());
    attributes.insert(
        "scripts".into(),
        family
            .scripts()
            .iter()
            .map(|s| (*s).into())
            .collect::<Vec<serde_json::Value>>()
            .into(),
    );
    let mut probabilities_map = Map::new();
    for (fam, pr) in Family::ALL.iter().zip(probs) {
        probabilities_map.insert(fam.name().into(), math::round_output(pr).into());
    }
    attributes.insert("probabilities".into(), probabilities_map.into());
    attributes.insert("lines".into(), (lf.len() as u64).into());
    attributes.insert(
        "lines_agreeing".into(),
        (votes.iter().filter(|v| v.family == family).count() as u64).into(),
    );
    attributes.insert("secondary".into(), false.into());
    det.findings.push(Finding {
        kind: Kind::Script,
        confidence,
        bbox: None,
        polygon: None,
        attributes,
        evidence,
    });

    // Other families that enough lines vote for: a mixed-script page.
    for other in Family::ALL {
        let mine: Vec<f64> = votes
            .iter()
            .filter(|v| v.family == other)
            .map(|v| v.probability)
            .collect();
        let share = mine.len() as f64 / lf.len() as f64;
        if other == family || mine.len() < p.min_lines || share < p.secondary_line_share {
            continue;
        }
        let mean = math::pairwise_sum(&mine) / mine.len() as f64;
        let ev = Evidence::new(
            DETECTOR_ID,
            "line_share",
            share,
            Some(p.secondary_line_share),
            0.0,
            format!(
                "{} of {} lines vote for the {} family",
                mine.len(),
                lf.len(),
                other.name()
            ),
            CITE_COMPONENT,
        )?;
        let mut attributes = Map::new();
        attributes.insert("family".into(), other.name().into());
        attributes.insert(
            "scripts".into(),
            other
                .scripts()
                .iter()
                .map(|s| (*s).into())
                .collect::<Vec<serde_json::Value>>()
                .into(),
        );
        attributes.insert("lines".into(), (lf.len() as u64).into());
        attributes.insert("lines_agreeing".into(), (mine.len() as u64).into());
        attributes.insert("secondary".into(), true.into());
        det.findings.push(Finding {
            kind: Kind::Script,
            confidence: math::round_output(mean),
            bbox: None,
            polygon: None,
            attributes,
            evidence: vec![ev],
        });
    }
    Ok(det)
}

/// Feature names reported under `debug.script`, for documentation tests.
pub fn feature_names() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        ("top_peak", "densest top row / line width, page median"),
        ("low_peak", "densest low row / line width, page median"),
        (
            "comp_density",
            "glyph components per x-height of width, page median",
        ),
        ("small_share", "share of small glyphs, page median"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pixel::layout;

    /// Paper with ink rectangles `(x0, y0, x1, y1)` inclusive.
    fn page(w: usize, h: usize, ink: &[(usize, usize, usize, usize)]) -> Vec<u8> {
        let mut g = vec![240u8; w * h];
        for &(x0, y0, x1, y1) in ink {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    g[y * w + x] = 20;
                }
            }
        }
        g
    }

    #[derive(Clone, Copy)]
    enum Style {
        /// Separate stems.
        Discrete,
        /// Stems under a 2-px bar across each word.
        Headline,
        /// Stems standing on a 2-px baseline across each word, dots above.
        Cursive,
    }

    /// `lines` lines, 24 px apart, of 5 words of 5 letters (6 px cells, 8 px tall).
    fn lines_of(style: Style, lines: usize) -> Vec<(usize, usize, usize, usize)> {
        let mut out = Vec::new();
        for l in 0..lines {
            let y = 20 + l * 24;
            let mut x = 20;
            for _ in 0..5 {
                let word_x = x;
                for _ in 0..5 {
                    match style {
                        Style::Discrete | Style::Headline => out.push((x, y, x + 2, y + 9)),
                        Style::Cursive => {
                            out.push((x + 2, y + 2, x + 3, y + 7));
                            out.push((x + 2, y - 4, x + 3, y - 3));
                        }
                    }
                    x += 6;
                }
                match style {
                    Style::Headline => out.push((word_x, y, x - 4, y + 1)),
                    Style::Cursive => out.push((word_x, y + 8, x - 2, y + 9)),
                    Style::Discrete => {}
                }
                x += 6;
            }
        }
        out
    }

    fn families(style: Style, lines: usize) -> (Detection, Vec<String>) {
        let g = page(240, 20 + lines * 24 + 20, &lines_of(style, lines));
        let p = Params::defaults().unwrap();
        let l = layout::segment(&g, 240, 20 + lines * 24 + 20, 0.0, &p).unwrap();
        let d = detect(&l, &p).unwrap();
        let fams = d
            .findings
            .iter()
            .map(|f| f.attributes["family"].as_str().unwrap().to_owned())
            .collect();
        (d, fams)
    }

    #[test]
    fn a_bar_over_each_word_is_the_headline_family() {
        let (d, fams) = families(Style::Headline, 6);
        assert_eq!(fams, ["headline"]);
        assert!(d.features["top_peak"] > 0.7);
        assert_eq!(d.features["lines_headline"], 6.0);
    }

    #[test]
    fn separate_letters_are_the_discrete_family() {
        let (d, fams) = families(Style::Discrete, 6);
        assert_eq!(fams, ["discrete"]);
        assert!(d.features["comp_density"] > 1.0);
    }

    #[test]
    fn joined_letters_on_a_baseline_are_the_cursive_family() {
        let (d, fams) = families(Style::Cursive, 6);
        assert_eq!(fams, ["cursive"], "{:?}", d.features);
        assert!(d.features["low_peak"] > 0.7 && d.features["top_peak"] < 0.7);
    }

    #[test]
    fn fewer_than_min_lines_abstains_but_reports_features() {
        let (d, fams) = families(Style::Headline, 2);
        assert!(fams.is_empty());
        assert_eq!(d.features["lines"], 2.0);
        assert!(d.features.contains_key("p_headline"));
    }

    #[test]
    fn a_blank_page_gives_no_features_and_no_finding() {
        let p = Params::defaults().unwrap();
        let l = layout::segment(&vec![240u8; 64 * 64], 64, 64, 0.0, &p).unwrap();
        let d = detect(&l, &p).unwrap();
        assert!(d.findings.is_empty());
        assert_eq!(d.features["lines"], 0.0);
    }

    #[test]
    fn probabilities_sum_to_one_and_the_winner_is_first_on_ties() {
        let p = probabilities([1.0, 3.0, 2.0]);
        assert!((p.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        assert_eq!(winner(&p), 1);
        assert_eq!(winner(&probabilities([0.0, 0.0, 0.0])), 0);
        // A huge score does not overflow.
        assert!(probabilities([1.0e6, 0.0, 0.0])[0] > 0.999);
    }

    #[test]
    fn a_half_and_half_page_reports_a_secondary_family() {
        let mut ink = lines_of(Style::Headline, 4);
        ink.extend(
            lines_of(Style::Discrete, 4)
                .into_iter()
                .map(|(a, b, c, d)| (a, b + 4 * 24, c, d + 4 * 24)),
        );
        let h = 20 + 8 * 24 + 20;
        let g = page(240, h, &ink);
        let p = Params::defaults().unwrap();
        let l = layout::segment(&g, 240, h, 0.0, &p).unwrap();
        let d = detect(&l, &p).unwrap();
        let fams: Vec<_> = d
            .findings
            .iter()
            .map(|f| {
                (
                    f.attributes["family"].as_str().unwrap(),
                    f.attributes["secondary"].as_bool().unwrap(),
                )
            })
            .collect();
        assert_eq!(fams.len(), 2, "{fams:?}");
        assert_ne!(fams[0].0, fams[1].0);
        assert!(!fams[0].1 && fams[1].1);
    }

    #[test]
    fn detection_is_repeatable_and_params_validate() {
        let (a, _) = families(Style::Headline, 6);
        let (b, _) = families(Style::Headline, 6);
        assert_eq!(a.features, b.features);
        let mut p = Params::defaults().unwrap().script;
        assert!(p.validate().is_ok());
        p.term_clip = 0.0;
        assert!(p.validate().is_err());
        let mut p = Params::defaults().unwrap().script;
        p.top_peak_coef = f64::INFINITY;
        assert!(p.validate().is_err());
    }
}
