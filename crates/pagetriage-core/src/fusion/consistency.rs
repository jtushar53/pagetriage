//! Cross-detector consistency rules (module M13) and the composites of module M11 that are built
//! from other detectors' findings: an e-stamp QR and a stamp-paper header. See
//! `docs/detectors/fusion.md`.
//!
//! A rule moves a finding's confidence in log-odds space and appends an evidence entry saying why, so
//! the explanation stays exact. Nothing here looks at pixels.

use serde::Deserialize;
use serde_json::Map;

use crate::error::{Error, Result};
use crate::math;
use crate::params::FusionParams;
use crate::pixel::layout::Layout;
use crate::schema::{Evidence, Finding, Kind};

const DETECTOR_ID: &str = "fusion.consistency";
const CITATION: &str = "module M13: cross-detector consistency rule, hand set, not calibrated";
const CITATION_M11: &str = "module M11: Indian stamp paper carries an e-stamp QR and a printed heading at the top; hand set, no sample seen";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsistencyParams {
    pub overlap_share: f64,
    pub overlap_penalty: f64,
    pub blank_penalty: f64,
    pub top_band: f64,
    pub qr_min_width_frac: f64,
    pub term_clip: f64,
    pub estamp_prior: f64,
    pub estamp_depth_coef: f64,
    pub header_prior: f64,
    pub header_ratio: f64,
    pub header_coef: f64,
    pub header_qr_bonus: f64,
    pub header_rule_bonus: f64,
}

impl ConsistencyParams {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParams(format!("consistency: {m}")));
        let all = [
            self.overlap_share,
            self.overlap_penalty,
            self.blank_penalty,
            self.top_band,
            self.qr_min_width_frac,
            self.term_clip,
            self.estamp_prior,
            self.estamp_depth_coef,
            self.header_prior,
            self.header_ratio,
            self.header_coef,
            self.header_qr_bonus,
            self.header_rule_bonus,
        ];
        if all.iter().any(|v| !v.is_finite() || v.abs() > 1.0e3) {
            return bad("every value must be finite and within 1e3");
        }
        if !(self.overlap_share > 0.0 && self.overlap_share <= 1.0) {
            return bad("overlap_share must be in (0, 1]");
        }
        if !(self.top_band > 0.0 && self.top_band <= 1.0) {
            return bad("top_band must be in (0, 1]");
        }
        if self.term_clip <= 0.0 || self.overlap_penalty < 0.0 || self.blank_penalty < 0.0 {
            return bad("term_clip must be > 0 and the penalties >= 0");
        }
        Ok(())
    }
}

/// What the rules need to know about the page beyond its findings.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PageContext {
    pub width: u32,
    pub height: u32,
    /// Tallest line glyph height in the top band over the median glyph height of the page; 0 when
    /// the page has fewer than three lines or no line in the band.
    pub header_ratio: f64,
    /// Bounding box (page frame) of the lines in the top band.
    pub header_bbox: Option<[u32; 4]>,
}

pub fn context(layout: &Layout, p: &ConsistencyParams) -> PageContext {
    let mut ctx = PageContext {
        width: layout.page_width as u32,
        height: layout.page_height as u32,
        ..PageContext::default()
    };
    if layout.lines.len() < 3 {
        return ctx;
    }
    let mut heights: Vec<u32> = layout.lines.iter().map(|l| l.glyph_height).collect();
    heights.sort_unstable();
    let median = f64::from(heights[heights.len() / 2]).max(1.0);
    let band = p.top_band * layout.page_height as f64;
    let mut tallest = 0u32;
    let mut bbox: Option<[u32; 4]> = None;
    for l in &layout.lines {
        let b = layout.rect_to_page(&l.bbox);
        if f64::from(b[1] + b[3]) / 2.0 >= band {
            continue;
        }
        tallest = tallest.max(l.glyph_height);
        bbox = Some(match bbox {
            None => b,
            Some(u) => [
                u[0].min(b[0]),
                u[1].min(b[1]),
                u[2].max(b[2]),
                u[3].max(b[3]),
            ],
        });
    }
    ctx.header_ratio = f64::from(tallest) / median;
    ctx.header_bbox = bbox;
    ctx
}

fn logit(c: f64) -> f64 {
    let c = c.clamp(1.0e-6, 1.0 - 1.0e-6);
    math::ln(c / (1.0 - c))
}

fn inside(b: [u32; 4]) -> bool {
    b[0] <= b[2] && b[1] <= b[3]
}

/// Share of the smaller box covered by the intersection.
fn overlap(a: [u32; 4], b: [u32; 4]) -> f64 {
    let ix = i64::from(a[2].min(b[2])) - i64::from(a[0].max(b[0])) + 1;
    let iy = i64::from(a[3].min(b[3])) - i64::from(a[1].max(b[1])) + 1;
    if ix <= 0 || iy <= 0 {
        return 0.0;
    }
    let area = |r: [u32; 4]| (i64::from(r[2] - r[0]) + 1) * (i64::from(r[3] - r[1]) + 1);
    (ix * iy) as f64 / area(a).min(area(b)) as f64
}

/// Move a finding's confidence by `delta` log-odds and record why.
fn shift(f: &mut Finding, delta: f64, feature: &str, value: f64, note: String) {
    let llr = delta;
    let Ok(e) = Evidence::new(DETECTOR_ID, feature, value, None, llr, note, CITATION) else {
        return;
    };
    f.confidence = math::round_output(math::sigmoid(logit(f.confidence) + llr));
    f.evidence.push(e);
}

fn is_mark(k: Kind) -> bool {
    matches!(
        k,
        Kind::Stamp | Kind::Seal | Kind::ThumbImpression | Kind::Signature
    )
}

fn is_content(k: Kind) -> bool {
    matches!(
        k,
        Kind::Handwriting
            | Kind::Signature
            | Kind::Stamp
            | Kind::Seal
            | Kind::EmbossedSeal
            | Kind::ThumbImpression
            | Kind::QrCode
            | Kind::Table
            | Kind::RulingLines
            | Kind::CheckboxGrid
            | Kind::RedactionBox
            | Kind::Photo
    )
}

/// Apply the rules to one page's findings, in place. Findings that fall below the emit threshold
/// are dropped; composites that reach it are appended.
pub fn apply(
    findings: &mut Vec<Finding>,
    ctx: &PageContext,
    p: &ConsistencyParams,
    fusion: &FusionParams,
) {
    // 1. Overlap: of two overlapping marks of different kinds, the less confident one loses.
    let n = findings.len();
    let mut losses: Vec<(usize, f64, Kind)> = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            let (a, b) = (&findings[i], &findings[j]);
            if !(is_mark(a.kind) && is_mark(b.kind)) || a.kind == b.kind {
                continue;
            }
            let (Some(ba), Some(bb)) = (a.bbox, b.bbox) else {
                continue;
            };
            let share = overlap(ba, bb);
            if share >= p.overlap_share {
                let (loser, other) = if a.confidence <= b.confidence {
                    (i, j)
                } else {
                    (j, i)
                };
                losses.push((loser, share, findings[other].kind));
            }
        }
    }
    for (i, share, other) in losses {
        let name = serde_json::to_value(other)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        shift(
            &mut findings[i],
            -p.overlap_penalty,
            "overlap_share",
            math::round_output(share),
            format!(
                "{:.0}% of it lies under a more confident {name} finding",
                share * 100.0
            ),
        );
    }

    // 2. A blank page with content on it is not blank.
    let has_content = findings
        .iter()
        .any(|f| is_content(f.kind) && f.confidence >= fusion.emit_min_confidence);
    if has_content {
        for f in findings.iter_mut().filter(|f| f.kind == Kind::BlankPage) {
            shift(
                f,
                -p.blank_penalty,
                "content_findings",
                1.0,
                "the page also has stamp, table, QR, handwriting or similar findings".into(),
            );
        }
    }
    findings.retain(|f| f.confidence >= fusion.emit_min_confidence);

    // 3. e-stamp QR and 4. stamp-paper header, from the top of the page.
    if ctx.width == 0 || ctx.height == 0 {
        return;
    }
    let band = p.top_band * f64::from(ctx.height);
    let mut composites: Vec<Finding> = Vec::new();
    let mut qr_top = false;
    let mut rule_top = false;
    for f in findings.iter() {
        let Some(b) = f.bbox.filter(|b| inside(*b)) else {
            continue;
        };
        let centre = f64::from(b[1] + b[3]) / 2.0;
        if centre >= band {
            continue;
        }
        match f.kind {
            Kind::QrCode
                if f64::from(b[2] - b[0] + 1) >= p.qr_min_width_frac * f64::from(ctx.width) =>
            {
                qr_top = true;
                let depth = (1.0 - centre / band).clamp(0.0, 1.0);
                let terms = [
                    p.estamp_prior,
                    (p.estamp_depth_coef * depth).clamp(-p.term_clip, p.term_clip),
                ];
                let Ok(e) = Evidence::new(
                    "fusion.estamp_qr",
                    "depth_in_top_band",
                    math::round_output(depth),
                    None,
                    terms[1],
                    "how near the top of the page the QR code sits (1 = at the edge)".into(),
                    CITATION_M11,
                ) else {
                    continue;
                };
                let confidence =
                    math::round_output(math::sigmoid(logit(f.confidence) + terms[0] + terms[1]));
                let mut evidence = f.evidence.clone();
                evidence.push(e);
                let mut attributes = Map::new();
                attributes.insert("decoded".into(), false.into());
                composites.push(Finding {
                    kind: Kind::EstampQr,
                    confidence,
                    bbox: f.bbox,
                    polygon: None,
                    attributes,
                    evidence,
                });
            }
            Kind::Table | Kind::RulingLines => rule_top = true,
            _ => {}
        }
    }
    if let Some(hb) = ctx.header_bbox
        && ctx.header_ratio > 0.0
    {
        let terms = [
            p.header_prior,
            (p.header_coef * (ctx.header_ratio - p.header_ratio)).clamp(-p.term_clip, p.term_clip),
            if qr_top { p.header_qr_bonus } else { 0.0 },
            if rule_top { p.header_rule_bonus } else { 0.0 },
        ];
        let confidence = math::round_output(math::sigmoid(math::pairwise_sum(&terms)));
        if confidence >= fusion.emit_min_confidence {
            let mk = |feature: &str, value: f64, llr: f64, note: &str| {
                Evidence::new(
                    "fusion.stamp_paper_header",
                    feature,
                    value,
                    None,
                    llr,
                    note.into(),
                    CITATION_M11,
                )
            };
            let evidence: Result<Vec<Evidence>> = [
                mk(
                    "header_ratio",
                    ctx.header_ratio,
                    terms[1],
                    "tallest line in the top band over the median glyph height",
                ),
                mk(
                    "qr_in_top_band",
                    f64::from(u8::from(qr_top)),
                    terms[2],
                    "a QR code sits in the top band",
                ),
                mk(
                    "rules_in_top_band",
                    f64::from(u8::from(rule_top)),
                    terms[3],
                    "a table or ruling finding sits in the top band",
                ),
            ]
            .into_iter()
            .collect();
            if let Ok(evidence) = evidence {
                let mut attributes = Map::new();
                attributes.insert("validated".into(), false.into());
                let bbox = composites.iter().filter_map(|c| c.bbox).fold(hb, |u, b| {
                    [
                        u[0].min(b[0]),
                        u[1].min(b[1]),
                        u[2].max(b[2]),
                        u[3].max(b[3]),
                    ]
                });
                composites.push(Finding {
                    kind: Kind::StampPaperHeader,
                    confidence,
                    bbox: Some(bbox),
                    polygon: None,
                    attributes,
                    evidence,
                });
            }
        }
    }
    composites.retain(|c| c.confidence >= fusion.emit_min_confidence);
    findings.extend(composites);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::Params;

    fn finding(kind: Kind, confidence: f64, bbox: [u32; 4]) -> Finding {
        Finding {
            kind,
            confidence,
            bbox: Some(bbox),
            polygon: None,
            attributes: Map::new(),
            evidence: vec![],
        }
    }

    fn setup() -> (ConsistencyParams, FusionParams) {
        let p = Params::defaults().unwrap();
        (p.consistency, p.fusion)
    }

    const CTX: PageContext = PageContext {
        width: 1000,
        height: 1400,
        header_ratio: 0.0,
        header_bbox: None,
    };

    #[test]
    fn the_weaker_of_two_overlapping_marks_loses() {
        let (p, fusion) = setup();
        let mut fs = vec![
            finding(Kind::Stamp, 0.97, [100, 100, 300, 300]),
            finding(Kind::ThumbImpression, 0.9, [120, 120, 280, 280]),
        ];
        apply(&mut fs, &CTX, &p, &fusion);
        assert_eq!(fs.len(), 2);
        assert_eq!(fs[0].confidence, 0.97, "the stronger one is untouched");
        assert!(fs[1].confidence < 0.9 && fs[1].confidence >= 0.5);
        assert_eq!(fs[1].evidence.len(), 1);
        assert!(fs[1].evidence[0].llr < 0.0);
    }

    #[test]
    fn a_weak_mark_under_a_strong_one_is_dropped() {
        let (p, fusion) = setup();
        let mut fs = vec![
            finding(Kind::Seal, 0.9, [100, 100, 300, 300]),
            finding(Kind::Signature, 0.55, [150, 150, 250, 250]),
        ];
        apply(&mut fs, &CTX, &p, &fusion);
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].kind, Kind::Seal);
    }

    #[test]
    fn marks_of_one_kind_or_far_apart_are_left_alone() {
        let (p, fusion) = setup();
        let mut fs = vec![
            finding(Kind::Stamp, 0.9, [100, 100, 300, 300]),
            finding(Kind::Stamp, 0.6, [120, 120, 280, 280]),
            finding(Kind::Signature, 0.6, [600, 600, 700, 700]),
        ];
        let before = fs.clone();
        apply(&mut fs, &CTX, &p, &fusion);
        assert_eq!(fs, before);
    }

    #[test]
    fn a_blank_page_with_a_table_is_not_blank() {
        let (p, fusion) = setup();
        let mut fs = vec![
            finding(Kind::BlankPage, 0.9997, [0, 0, 999, 1399]),
            finding(Kind::Table, 0.9, [100, 800, 900, 1200]),
        ];
        apply(&mut fs, &CTX, &p, &fusion);
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].kind, Kind::Table);
        // On its own it stays.
        let mut fs = vec![finding(Kind::BlankPage, 0.9997, [0, 0, 999, 1399])];
        apply(&mut fs, &CTX, &p, &fusion);
        assert_eq!(fs.len(), 1);
    }

    #[test]
    fn a_qr_at_the_top_gives_an_estamp_qr_and_one_below_does_not() {
        let (p, fusion) = setup();
        let mut fs = vec![finding(Kind::QrCode, 0.95, [800, 40, 940, 180])];
        apply(&mut fs, &CTX, &p, &fusion);
        assert_eq!(fs.len(), 2);
        assert_eq!(fs[1].kind, Kind::EstampQr);
        assert!(fs[1].confidence >= 0.5 && fs[1].confidence < fs[0].confidence);
        assert_eq!(fs[1].attributes["decoded"], false);
        let mut fs = vec![finding(Kind::QrCode, 0.95, [800, 1000, 940, 1140])];
        apply(&mut fs, &CTX, &p, &fusion);
        assert_eq!(fs.len(), 1);
        // A small QR (narrower than 5% of the page) is not an e-stamp.
        let mut fs = vec![finding(Kind::QrCode, 0.95, [800, 40, 830, 70])];
        apply(&mut fs, &CTX, &p, &fusion);
        assert_eq!(fs.len(), 1);
    }

    #[test]
    fn a_large_heading_with_a_qr_gives_a_stamp_paper_header() {
        let (p, fusion) = setup();
        let ctx = PageContext {
            header_ratio: 2.4,
            header_bbox: Some([100, 60, 700, 140]),
            ..CTX
        };
        let mut fs = vec![finding(Kind::QrCode, 0.95, [800, 40, 940, 180])];
        apply(&mut fs, &ctx, &p, &fusion);
        let header = fs
            .iter()
            .find(|f| f.kind == Kind::StampPaperHeader)
            .expect("header");
        assert_eq!(header.bbox, Some([100, 40, 940, 180]));
        assert_eq!(header.attributes["validated"], false);
        // Without the QR or rules the same heading is not enough.
        let mut fs: Vec<Finding> = vec![];
        apply(&mut fs, &ctx, &p, &fusion);
        assert!(fs.is_empty());
        // An ordinary heading size is not enough either.
        let ctx = PageContext {
            header_ratio: 1.2,
            ..ctx
        };
        let mut fs = vec![finding(Kind::QrCode, 0.95, [800, 40, 940, 180])];
        apply(&mut fs, &ctx, &p, &fusion);
        assert!(fs.iter().all(|f| f.kind != Kind::StampPaperHeader));
    }

    #[test]
    fn validation() {
        let (mut p, _) = setup();
        p.validate().unwrap();
        p.overlap_share = 0.0;
        assert!(p.validate().is_err());
        let (mut p, _) = setup();
        p.estamp_prior = f64::INFINITY;
        assert!(p.validate().is_err());
    }
}
