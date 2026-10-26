//! Document-level pass (modules M11 and M13): marks that repeat on many pages. See
//! `docs/detectors/endorsement.md`.
//!
//! A registered deed carries the registrar's endorsement (a stamp, a seal or a signature block) on
//! every page. One page cannot know that, so this runs over a document's page reports, after the page
//! analyses, like `pixel::phash::mark_near_duplicates`.

use serde::Deserialize;
use serde_json::Map;

use crate::error::{Error, Result};
use crate::math;
use crate::params::FusionParams;
use crate::schema::{Evidence, Finding, Kind, PageReport};

const DETECTOR_ID: &str = "document.endorsement";
const CITATION: &str = "module M11: a registrar's endorsement repeats on the pages of a registered deed; hand set, no real deed seen";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentParams {
    pub position_tolerance: f64,
    pub size_ratio: f64,
    pub min_pages: usize,
    pub min_share: f64,
    pub prior: f64,
    pub share_coef: f64,
    pub margin_frac: f64,
    pub margin_bonus: f64,
    pub spread_coef: f64,
    pub term_clip: f64,
}

impl DocumentParams {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParams(format!("document: {m}")));
        let all = [
            self.position_tolerance,
            self.size_ratio,
            self.min_share,
            self.prior,
            self.share_coef,
            self.margin_frac,
            self.margin_bonus,
            self.spread_coef,
            self.term_clip,
        ];
        if all.iter().any(|v| !v.is_finite() || v.abs() > 1.0e3) {
            return bad("every value must be finite and within 1e3");
        }
        if self.position_tolerance <= 0.0 || self.size_ratio < 1.0 || self.term_clip <= 0.0 {
            return bad("position_tolerance > 0, size_ratio >= 1 and term_clip > 0");
        }
        if !(0.0..=1.0).contains(&self.min_share) || !(0.0..=0.5).contains(&self.margin_frac) {
            return bad("min_share must be in [0, 1] and margin_frac in [0, 0.5]");
        }
        if self.min_pages < 2 {
            return bad("min_pages must be at least 2");
        }
        Ok(())
    }
}

/// One mark on one page, in fractions of the page.
#[derive(Debug, Clone, Copy)]
struct Mark {
    page: usize,
    finding: usize,
    kind: Kind,
    cx: f64,
    cy: f64,
    w: f64,
    h: f64,
}

fn marks_of(page: usize, report: &PageReport) -> Vec<Mark> {
    let dpi = f64::from(report.analysis_dpi);
    let (pw, ph) = (
        report.size_pt[0] * dpi / 72.0,
        report.size_pt[1] * dpi / 72.0,
    );
    if !(pw >= 1.0 && ph >= 1.0) {
        return Vec::new();
    }
    report
        .findings
        .iter()
        .enumerate()
        .filter(|(_, f)| matches!(f.kind, Kind::Stamp | Kind::Seal | Kind::Signature))
        .filter_map(|(i, f)| {
            let b = f.bbox?;
            let (w, h) = (
                f64::from(b[2]) - f64::from(b[0]) + 1.0,
                f64::from(b[3]) - f64::from(b[1]) + 1.0,
            );
            (w > 0.0 && h > 0.0).then(|| Mark {
                page,
                finding: i,
                kind: f.kind,
                cx: (f64::from(b[0]) + f64::from(b[2])) / 2.0 / pw,
                cy: (f64::from(b[1]) + f64::from(b[3])) / 2.0 / ph,
                w: w / pw,
                h: h / ph,
            })
        })
        .collect()
}

fn same_place(a: &Mark, b: &Mark, p: &DocumentParams) -> bool {
    let ratio = |x: f64, y: f64| x.max(y) / x.min(y);
    a.kind == b.kind
        && (a.cx - b.cx).abs() <= p.position_tolerance
        && (a.cy - b.cy).abs() <= p.position_tolerance
        && ratio(a.w, b.w) <= p.size_ratio
        && ratio(a.h, b.h) <= p.size_ratio
}

/// Add a `registrar_endorsement` finding to every page that carries a mark repeated on at least
/// `min_pages` pages and `min_share` of the document. Existing `registrar_endorsement` findings are
/// removed first, so the pass can be run again.
pub fn mark_repeated_endorsements(
    reports: &mut [PageReport],
    p: &DocumentParams,
    fusion: &FusionParams,
) -> Result<()> {
    for r in reports.iter_mut() {
        r.findings.retain(|f| f.kind != Kind::RegistrarEndorsement);
    }
    let pages = reports.len();
    if pages < p.min_pages {
        return Ok(());
    }
    let mut marks: Vec<Mark> = Vec::new();
    for (i, r) in reports.iter().enumerate() {
        marks.extend(marks_of(i, r));
    }
    let mut used = vec![false; marks.len()];
    let mut additions: Vec<(usize, Finding)> = Vec::new();
    for seed in 0..marks.len() {
        if used[seed] {
            continue;
        }
        // The seed's group: on every other page, its first matching mark not already in a group.
        let mut members = vec![seed];
        for other_page in marks[seed].page + 1..pages {
            if let Some(j) = (0..marks.len()).find(|&j| {
                !used[j] && marks[j].page == other_page && same_place(&marks[seed], &marks[j], p)
            }) {
                members.push(j);
            }
        }
        let share = members.len() as f64 / pages as f64;
        if members.len() < p.min_pages || share < p.min_share {
            continue;
        }
        for &m in &members {
            used[m] = true;
        }
        let cxs: Vec<f64> = members.iter().map(|&m| marks[m].cx).collect();
        let cys: Vec<f64> = members.iter().map(|&m| marks[m].cy).collect();
        let spread = |v: &[f64]| {
            v.iter().copied().fold(f64::NEG_INFINITY, f64::max)
                - v.iter().copied().fold(f64::INFINITY, f64::min)
        };
        let spread = spread(&cxs).max(spread(&cys));
        let n = members.len() as f64;
        let (mx, my) = (math::pairwise_sum(&cxs) / n, math::pairwise_sum(&cys) / n);
        let edge = mx.min(1.0 - mx).min(my).min(1.0 - my);
        let in_margin = edge <= p.margin_frac;
        let clip = |v: f64| v.clamp(-p.term_clip, p.term_clip);
        let terms = [
            clip(p.share_coef * (share - p.min_share)),
            if in_margin { clip(p.margin_bonus) } else { 0.0 },
            clip(p.spread_coef * (p.position_tolerance - spread)),
        ];
        let notes = [
            format!("on {} of {} pages", members.len(), pages),
            format!("nearest page edge at {:.0}% of the page", edge * 100.0),
            format!(
                "centres within {:.1}% of the page across the pages",
                spread * 100.0
            ),
        ];
        let values = [share, edge, spread];
        let features = ["page_share", "edge_distance", "position_spread"];
        let mut evidence = Vec::new();
        for k in 0..3 {
            evidence.push(Evidence::new(
                DETECTOR_ID,
                features[k],
                math::round_output(values[k]),
                None,
                terms[k].clamp(-fusion.llr_clip, fusion.llr_clip),
                notes[k].clone(),
                CITATION,
            )?);
        }
        let confidence = math::round_output(crate::fusion::scorer::confidence(p.prior, &evidence));
        if confidence < fusion.emit_min_confidence {
            continue;
        }
        for &m in &members {
            let mark = &marks[m];
            let source = &reports[mark.page].findings[mark.finding];
            let mut attributes = Map::new();
            attributes.insert("pages".into(), (members.len() as u64).into());
            attributes.insert("document_pages".into(), (pages as u64).into());
            attributes.insert(
                "mark".into(),
                serde_json::to_value(mark.kind).map_err(|e| Error::InvalidInput(e.to_string()))?,
            );
            additions.push((
                mark.page,
                Finding {
                    kind: Kind::RegistrarEndorsement,
                    confidence,
                    bbox: source.bbox,
                    polygon: None,
                    attributes,
                    evidence: evidence.clone(),
                },
            ));
        }
    }
    for (page, finding) in additions {
        reports[page].findings.push(finding);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::Params;

    fn report(index: u32, marks: &[(Kind, [u32; 4])]) -> PageReport {
        PageReport {
            index,
            size_pt: [595.0, 842.0],
            analysis_dpi: 72,
            timings_ms: None,
            text_layer: None,
            quality: None,
            findings: marks
                .iter()
                .map(|(kind, b)| Finding {
                    kind: *kind,
                    confidence: 0.9,
                    bbox: Some(*b),
                    polygon: None,
                    attributes: Map::new(),
                    evidence: vec![],
                })
                .collect(),
            page_type: None,
            phash: None,
            dhash: None,
            near_duplicate_of: None,
            debug: None,
        }
    }

    fn run(reports: &mut [PageReport]) {
        let p = Params::defaults().unwrap();
        mark_repeated_endorsements(reports, &p.document, &p.fusion).unwrap();
    }

    fn endorsed(r: &PageReport) -> usize {
        r.findings
            .iter()
            .filter(|f| f.kind == Kind::RegistrarEndorsement)
            .count()
    }

    #[test]
    fn a_stamp_in_the_same_corner_on_every_page_is_an_endorsement() {
        let corner = [440, 740, 560, 820];
        let mut pages: Vec<PageReport> = (0..5)
            .map(|i| {
                report(
                    i,
                    &[(
                        Kind::Stamp,
                        [corner[0] + i, corner[1], corner[2] + i, corner[3]],
                    )],
                )
            })
            .collect();
        run(&mut pages);
        for r in &pages {
            assert_eq!(endorsed(r), 1);
            let f = r.findings.last().unwrap();
            assert_eq!(f.attributes["pages"], 5);
            assert_eq!(f.bbox, r.findings[0].bbox);
            assert!(f.confidence >= 0.5);
        }
    }

    #[test]
    fn marks_in_different_places_or_on_few_pages_are_not() {
        let mut pages = vec![
            report(0, &[(Kind::Stamp, [440, 740, 560, 820])]),
            report(1, &[(Kind::Stamp, [40, 40, 160, 120])]),
            report(2, &[(Kind::Stamp, [300, 300, 420, 380])]),
            report(3, &[]),
        ];
        run(&mut pages);
        assert!(pages.iter().all(|r| endorsed(r) == 0));
        // Two of five pages: under both min_pages and min_share.
        let m = (Kind::Seal, [440, 740, 560, 820]);
        let mut pages = vec![
            report(0, &[m]),
            report(1, &[m]),
            report(2, &[]),
            report(3, &[]),
            report(4, &[]),
        ];
        run(&mut pages);
        assert!(pages.iter().all(|r| endorsed(r) == 0));
    }

    #[test]
    fn a_different_kind_or_size_is_not_the_same_mark() {
        let mut pages = vec![
            report(0, &[(Kind::Stamp, [440, 740, 560, 820])]),
            report(1, &[(Kind::Seal, [440, 740, 560, 820])]),
            report(2, &[(Kind::Stamp, [400, 700, 600, 840])]),
        ];
        run(&mut pages);
        assert!(pages.iter().all(|r| endorsed(r) == 0));
    }

    #[test]
    fn running_again_gives_the_same_result() {
        let m = (Kind::Stamp, [440, 740, 560, 820]);
        let mut pages: Vec<PageReport> = (0..4).map(|i| report(i, &[m])).collect();
        run(&mut pages);
        let once = pages.clone();
        run(&mut pages);
        assert_eq!(pages, once);
    }

    #[test]
    fn short_documents_and_pages_without_size_are_left_alone() {
        let m = (Kind::Stamp, [440, 740, 560, 820]);
        let mut pages = vec![report(0, &[m]), report(1, &[m])];
        run(&mut pages);
        assert!(pages.iter().all(|r| endorsed(r) == 0));
        let mut r = report(0, &[m]);
        r.size_pt = [0.0, 0.0];
        assert!(marks_of(0, &r).is_empty());
    }

    #[test]
    fn validation() {
        let mut p = Params::defaults().unwrap().document;
        p.validate().unwrap();
        p.min_pages = 1;
        assert!(p.validate().is_err());
        let mut p = Params::defaults().unwrap().document;
        p.size_ratio = 0.5;
        assert!(p.validate().is_err());
    }
}
