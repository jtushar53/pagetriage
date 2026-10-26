//! Page typing (module M10). See `docs/detectors/page_type.md`.
//!
//! Eight features in [0, 1], taken from the layout and from the findings other detectors made, are
//! weighed by hand-set coefficients per label and passed through a softmax. Only as good as the
//! detectors whose findings it reads: with just `layout` and no other detector it can tell text from
//! a picture or an empty page, and little else.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::math;
use crate::pixel::layout::Layout;
use crate::schema::{Finding, Kind, PageLabel, PageType};

/// Coefficients of one label. A missing key is 0.
#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Weights {
    pub bias: f64,
    pub text: f64,
    pub rules: f64,
    pub table: f64,
    pub annot: f64,
    pub graphic: f64,
    pub qr_top: f64,
    pub header: f64,
    pub empty: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageTypeParams {
    pub text_lines_full: f64,
    pub ink_blank: f64,
    pub graphic_full: f64,
    pub dark_delta: u8,
    pub table_full: f64,
    pub rules_full: f64,
    pub rule_span_full: f64,
    pub annot_full: f64,
    pub top_band: f64,
    pub temperature: f64,
    pub alternative_min: f64,
    pub text: Weights,
    pub text_with_annotations: Weights,
    pub form: Weights,
    pub table: Weights,
    pub photo: Weights,
    pub mixed: Weights,
    pub blank: Weights,
    pub cover_stamp_paper: Weights,
}

impl PageTypeParams {
    fn weights(&self) -> [(PageLabel, &Weights); 8] {
        [
            (PageLabel::Text, &self.text),
            (PageLabel::TextWithAnnotations, &self.text_with_annotations),
            (PageLabel::Form, &self.form),
            (PageLabel::Table, &self.table),
            (PageLabel::Photo, &self.photo),
            (PageLabel::Mixed, &self.mixed),
            (PageLabel::Blank, &self.blank),
            (PageLabel::CoverStampPaper, &self.cover_stamp_paper),
        ]
    }

    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParams(format!("page_type: {m}")));
        let scales = [
            ("text_lines_full", self.text_lines_full),
            ("ink_blank", self.ink_blank),
            ("graphic_full", self.graphic_full),
            ("table_full", self.table_full),
            ("rules_full", self.rules_full),
            ("rule_span_full", self.rule_span_full),
            ("annot_full", self.annot_full),
        ];
        for (name, v) in scales {
            if !v.is_finite() || v <= 0.0 {
                return bad(&format!("{name} must be finite and > 0, got {v}"));
            }
        }
        if !(self.top_band > 0.0 && self.top_band <= 1.0) {
            return bad("top_band must be in (0, 1]");
        }
        if !self.temperature.is_finite() || self.temperature <= 0.0 {
            return bad("temperature must be finite and > 0");
        }
        if !(0.0..=1.0).contains(&self.alternative_min) {
            return bad("alternative_min must be in [0, 1]");
        }
        for (label, w) in self.weights() {
            let all = [
                w.bias, w.text, w.rules, w.table, w.annot, w.graphic, w.qr_top, w.header, w.empty,
            ];
            if all.iter().any(|v| !v.is_finite() || v.abs() > 100.0) {
                return bad(&format!(
                    "{label:?} has a coefficient that is not finite or over 100"
                ));
            }
        }
        Ok(())
    }
}

/// What the page typing reads from one page.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PageFacts {
    pub lines: usize,
    pub ink_fraction: f64,
    pub graphic_share: f64,
    pub table_share: f64,
    pub rules: f64,
    pub annotations: usize,
    pub qr_top: bool,
    pub header: bool,
}

fn area(b: [u32; 4]) -> f64 {
    f64::from(b[2].saturating_sub(b[0]) + 1) * f64::from(b[3].saturating_sub(b[1]) + 1)
}

pub fn facts(layout: &Layout, findings: &[Finding], gray: &[u8], p: &PageTypeParams) -> PageFacts {
    let layout_px = (layout.width * layout.height) as f64;
    let page_px = (layout.page_width * layout.page_height) as f64;
    let page_h = layout.page_height as f64;
    let graphic_area: Vec<f64> = layout
        .graphics
        .iter()
        .map(|r| f64::from(r.width()) * f64::from(r.height()))
        .collect();
    // Dark area on the gray page: the paper is the most common gray (ties to the darker value).
    let mut histogram = [0u64; 256];
    for &v in gray {
        histogram[usize::from(v)] += 1;
    }
    let mut paper = 0usize;
    for (level, &count) in histogram.iter().enumerate() {
        if count > histogram[paper] {
            paper = level;
        }
    }
    let dark_below = paper.saturating_sub(usize::from(p.dark_delta));
    let dark: u64 = histogram[..dark_below].iter().sum();
    let non_text_dark = (dark as f64 - layout.glyph_ink_pixels as f64).max(0.0) / gray.len() as f64;
    let mut f = PageFacts {
        lines: layout.lines.len(),
        ink_fraction: layout.ink_pixels as f64 / layout_px,
        graphic_share: (math::pairwise_sum(&graphic_area) / layout_px).max(non_text_dark),
        ..PageFacts::default()
    };
    let page_w = layout.page_width as f64;
    let attr = |finding: &Finding, key: &str| {
        finding
            .attributes
            .get(key)
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    };
    for finding in findings {
        match finding.kind {
            Kind::Table => {
                if let Some(b) = finding.bbox {
                    f.table_share = f.table_share.max(area(b) / page_px);
                }
            }
            Kind::RulingLines => {
                let span = (attr(finding, "longest_px") / (p.rule_span_full * page_w)).min(1.0);
                f.rules += (attr(finding, "horizontal") + attr(finding, "vertical")) * span;
            }
            Kind::CheckboxGrid => f.rules += attr(finding, "boxes").max(2.0),
            Kind::Handwriting
            | Kind::Signature
            | Kind::Stamp
            | Kind::Seal
            | Kind::ThumbImpression
            | Kind::RedactionBox => f.annotations += 1,
            Kind::QrCode | Kind::EstampQr => {
                if let Some(b) = finding.bbox
                    && f64::from(b[1] + b[3]) / 2.0 < p.top_band * page_h
                {
                    f.qr_top = true;
                }
            }
            Kind::StampPaperHeader => f.header = true,
            _ => {}
        }
    }
    f
}

/// The facts as debug features.
pub fn features(f: &PageFacts) -> BTreeMap<String, f64> {
    BTreeMap::from([
        ("annotations".to_owned(), f.annotations as f64),
        ("graphic_share".to_owned(), f.graphic_share),
        ("header".to_owned(), f64::from(u8::from(f.header))),
        ("ink_fraction".to_owned(), f.ink_fraction),
        ("lines".to_owned(), f.lines as f64),
        ("qr_top".to_owned(), f64::from(u8::from(f.qr_top))),
        ("rules".to_owned(), f.rules),
        ("table_share".to_owned(), f.table_share),
    ])
}

/// Label, score and softmax probability for every label, in the order of `PageLabel`.
pub fn classify(f: &PageFacts, p: &PageTypeParams) -> PageType {
    let unit = |v: f64| v.clamp(0.0, 1.0);
    let x = [
        unit(f.lines as f64 / p.text_lines_full),
        unit(f.rules / p.rules_full),
        unit(f.table_share / p.table_full),
        unit(f.annotations as f64 / p.annot_full),
        unit(f.graphic_share / p.graphic_full),
        f64::from(u8::from(f.qr_top)),
        f64::from(u8::from(f.header)),
        unit(1.0 - f.ink_fraction / p.ink_blank),
    ];
    let scores: Vec<(PageLabel, f64)> = p
        .weights()
        .iter()
        .map(|(label, w)| {
            let terms = [
                w.bias,
                w.text * x[0],
                w.rules * x[1],
                w.table * x[2],
                w.annot * x[3],
                w.graphic * x[4],
                w.qr_top * x[5],
                w.header * x[6],
                w.empty * x[7],
            ];
            (*label, math::pairwise_sum(&terms) / p.temperature)
        })
        .collect();
    let top = scores.iter().map(|s| s.1).fold(f64::NEG_INFINITY, f64::max);
    let exps: Vec<f64> = scores.iter().map(|s| math::exp(s.1 - top)).collect();
    let total = math::pairwise_sum(&exps);
    let probs: Vec<(PageLabel, f64)> = scores
        .iter()
        .zip(&exps)
        .map(|((label, _), e)| (*label, e / total))
        .collect();
    // The winner is the first label with the highest probability: the order of `weights` breaks ties.
    let mut best = 0;
    for (i, (_, pr)) in probs.iter().enumerate() {
        if *pr > probs[best].1 {
            best = i;
        }
    }
    let alternatives: BTreeMap<String, f64> = probs
        .iter()
        .enumerate()
        .filter(|&(i, (_, pr))| i != best && *pr >= p.alternative_min)
        .map(|(_, (label, pr))| (label_name(*label), math::round_output(*pr)))
        .collect();
    PageType {
        label: probs[best].0,
        confidence: math::round_output(probs[best].1),
        alternatives,
    }
}

fn label_name(label: PageLabel) -> String {
    match serde_json::to_value(label) {
        Ok(serde_json::Value::String(s)) => s,
        _ => unreachable!("PageLabel serialises as a string"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::Params;

    fn params() -> PageTypeParams {
        Params::defaults().unwrap().page_type
    }

    fn label(f: PageFacts) -> PageLabel {
        classify(&f, &params()).label
    }

    #[test]
    fn typical_pages() {
        let ink = 0.05;
        let base = PageFacts {
            ink_fraction: ink,
            ..PageFacts::default()
        };
        assert_eq!(
            label(PageFacts {
                ink_fraction: 0.0,
                ..base
            }),
            PageLabel::Blank
        );
        assert_eq!(label(PageFacts { lines: 30, ..base }), PageLabel::Text);
        assert_eq!(
            label(PageFacts {
                lines: 30,
                annotations: 3,
                ..base
            }),
            PageLabel::TextWithAnnotations
        );
        assert_eq!(
            label(PageFacts {
                lines: 12,
                rules: 9.0,
                ..base
            }),
            PageLabel::Form
        );
        assert_eq!(
            label(PageFacts {
                lines: 6,
                rules: 4.0,
                table_share: 0.6,
                ..base
            }),
            PageLabel::Table
        );
        assert_eq!(
            label(PageFacts {
                graphic_share: 0.5,
                ..base
            }),
            PageLabel::Photo
        );
        assert_eq!(
            label(PageFacts {
                lines: 20,
                graphic_share: 0.25,
                ..base
            }),
            PageLabel::Mixed
        );
        assert_eq!(
            label(PageFacts {
                lines: 4,
                qr_top: true,
                header: true,
                ..base
            }),
            PageLabel::CoverStampPaper
        );
    }

    #[test]
    fn probabilities_sum_to_one_and_alternatives_exclude_the_winner() {
        let f = PageFacts {
            lines: 10,
            ink_fraction: 0.05,
            rules: 2.0,
            annotations: 1,
            ..PageFacts::default()
        };
        let t = classify(&f, &params());
        assert!(t.confidence > 0.0 && t.confidence <= 1.0);
        let total: f64 = t.alternatives.values().sum::<f64>() + t.confidence;
        assert!(total <= 1.0 + 1e-5, "{total}");
        let winner = serde_json::to_value(t.label).unwrap();
        assert!(!t.alternatives.contains_key(winner.as_str().unwrap()));
    }

    #[test]
    fn validation() {
        let mut p = params();
        p.validate().unwrap();
        p.temperature = 0.0;
        assert!(p.validate().is_err());
        let mut p = params();
        p.text.bias = f64::NAN;
        assert!(p.validate().is_err());
        let mut p = params();
        p.top_band = 0.0;
        assert!(p.validate().is_err());
    }
}
