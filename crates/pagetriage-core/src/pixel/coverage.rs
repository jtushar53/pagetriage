//! Extraction coverage: how much of the page's text the supplied text boxes cover. See
//! `docs/detectors/coverage.md`.
//!
//! The boxes come from a PDF's text layer (hidden OCR text included) or from an OCR run, in page
//! pixels. The page's text is the layout's glyphs. A line is covered when enough of its glyphs have
//! their centre inside a box; the finding says how much ink is left uncovered.

use serde::Deserialize;
use serde_json::Map;

use super::Detection;
use super::layout::Layout;
use crate::error::{Error, Result};
use crate::fusion::scorer;
use crate::math;
use crate::params::Params;
use crate::schema::{Evidence, Finding, Kind};

pub const COVERAGE: &str = "coverage";
const DETECTOR_ID: &str = "coverage.text_boxes";
const CITATION: &str = "extraction coverage: text on the page that no text box sits on is missing from the text layer or OCR; hand set";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageParams {
    pub covered_line_fraction: f64,
    pub box_pad_in: f64,
    pub min_lines: usize,
    pub max_boxes: usize,
    pub prior: f64,
    pub gap_threshold: f64,
    pub gap_coef: f64,
    pub term_clip: f64,
}

impl CoverageParams {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParams(format!("coverage: {m}")));
        let all = [
            self.covered_line_fraction,
            self.box_pad_in,
            self.prior,
            self.gap_threshold,
            self.gap_coef,
            self.term_clip,
        ];
        if all.iter().any(|v| !v.is_finite() || v.abs() > 1.0e3) {
            return bad("every value must be finite and within 1e3");
        }
        if !(self.covered_line_fraction > 0.0 && self.covered_line_fraction <= 1.0) {
            return bad("covered_line_fraction must be in (0, 1]");
        }
        if self.box_pad_in < 0.0
            || self.term_clip <= 0.0
            || self.min_lines == 0
            || self.max_boxes == 0
        {
            return bad("box_pad_in >= 0, term_clip > 0, min_lines >= 1 and max_boxes >= 1");
        }
        Ok(())
    }
}

/// What was measured on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct Coverage {
    pub lines: usize,
    pub lines_covered: usize,
    pub ink_covered_share: f64,
    pub boxes: usize,
    pub boxes_without_ink: usize,
    /// Union box (page pixels) of the lines that are not covered.
    pub gap_bbox: Option<[u32; 4]>,
}

pub fn measure(layout: &Layout, boxes: &[[u32; 4]], dpi: u16, p: &CoverageParams) -> Coverage {
    let (pw, ph) = (layout.page_width, layout.page_height);
    let pad = (p.box_pad_in * f64::from(dpi)).round() as i64;
    let boxes = &boxes[..boxes.len().min(p.max_boxes)];
    let mut mask = vec![false; pw * ph];
    for b in boxes {
        let (x0, y0) = (
            (i64::from(b[0]) - pad).max(0) as usize,
            (i64::from(b[1]) - pad).max(0) as usize,
        );
        let (x1, y1) = (
            ((i64::from(b[2]) + pad).max(0) as usize).min(pw.saturating_sub(1)),
            ((i64::from(b[3]) + pad).max(0) as usize).min(ph.saturating_sub(1)),
        );
        for y in y0..=y1.max(y0).min(ph.saturating_sub(1)) {
            for x in x0..=x1.max(x0).min(pw.saturating_sub(1)) {
                mask[y * pw + x] = true;
            }
        }
    }
    // Glyph centres in the page frame, and whether each lies under a box.
    let n_lines = layout.lines.len();
    let mut line_glyphs = vec![0usize; n_lines];
    let mut line_covered = vec![0usize; n_lines];
    let (mut ink_all, mut ink_covered) = (0u64, 0u64);
    let mut centres = vec![false; pw * ph];
    for g in &layout.glyphs {
        let (cx, cy) = layout.to_page((g.bbox.x0 + g.bbox.x1) / 2, (g.bbox.y0 + g.bbox.y1) / 2);
        if cx < 0 || cy < 0 || cx as usize >= pw || cy as usize >= ph {
            continue;
        }
        let at = cy as usize * pw + cx as usize;
        centres[at] = true;
        let under = mask[at];
        ink_all += u64::from(g.area);
        if under {
            ink_covered += u64::from(g.area);
        }
        if let Some(l) = g.line {
            line_glyphs[l] += 1;
            if under {
                line_covered[l] += 1;
            }
        }
    }
    let mut lines_covered = 0;
    let mut gap: Option<[u32; 4]> = None;
    for (i, line) in layout.lines.iter().enumerate() {
        let share = if line_glyphs[i] == 0 {
            0.0
        } else {
            line_covered[i] as f64 / line_glyphs[i] as f64
        };
        if share >= p.covered_line_fraction {
            lines_covered += 1;
        } else {
            let b = layout.rect_to_page(&line.bbox);
            gap = Some(match gap {
                None => b,
                Some(u) => [
                    u[0].min(b[0]),
                    u[1].min(b[1]),
                    u[2].max(b[2]),
                    u[3].max(b[3]),
                ],
            });
        }
    }
    // Boxes with no glyph centre inside the original (unpadded) box: text the page does not show.
    let mut empty = 0;
    for b in boxes {
        let (x1, y1) = (
            (b[2] as usize).min(pw.saturating_sub(1)),
            (b[3] as usize).min(ph.saturating_sub(1)),
        );
        let (x0, y0) = ((b[0] as usize).min(x1), (b[1] as usize).min(y1));
        let mut any = false;
        'scan: for y in y0..=y1 {
            for x in x0..=x1 {
                if centres[y * pw + x] {
                    any = true;
                    break 'scan;
                }
            }
        }
        if !any {
            empty += 1;
        }
    }
    Coverage {
        lines: n_lines,
        lines_covered,
        ink_covered_share: if ink_all == 0 {
            1.0
        } else {
            ink_covered as f64 / ink_all as f64
        },
        boxes: boxes.len(),
        boxes_without_ink: empty,
        gap_bbox: gap,
    }
}

/// Run on a page's layout and the text boxes in page pixels; `None` boxes means nothing is known
/// about the page's text layer, which is not the same as having none.
pub fn detect(
    layout: &Layout,
    boxes: Option<&[[u32; 4]]>,
    dpi: u16,
    params: &Params,
) -> Result<Detection> {
    let p = &params.coverage;
    let mut det = Detection::default();
    let Some(boxes) = boxes else {
        det.features.insert("have_boxes".into(), 0.0);
        return Ok(det);
    };
    let c = measure(layout, boxes, dpi, p);
    det.features.insert("have_boxes".into(), 1.0);
    det.features.insert("lines".into(), c.lines as f64);
    det.features
        .insert("lines_covered".into(), c.lines_covered as f64);
    det.features
        .insert("ink_covered_share".into(), c.ink_covered_share);
    det.features.insert("boxes".into(), c.boxes as f64);
    det.features
        .insert("boxes_without_ink".into(), c.boxes_without_ink as f64);
    if c.lines < p.min_lines {
        return Ok(det);
    }
    let uncovered = 1.0 - c.ink_covered_share;
    let llr = (p.gap_coef * (uncovered - p.gap_threshold)).clamp(-p.term_clip, p.term_clip);
    let evidence = vec![Evidence::new(
        DETECTOR_ID,
        "uncovered_ink_share",
        math::round_output(uncovered),
        Some(p.gap_threshold),
        llr.clamp(-params.fusion.llr_clip, params.fusion.llr_clip),
        format!(
            "{} of {} text lines have most of their glyphs under a text box; {:.0}% of the ink is under none",
            c.lines_covered,
            c.lines,
            uncovered * 100.0
        ),
        CITATION,
    )?];
    let confidence = math::round_output(scorer::confidence_capped(
        p.prior,
        &evidence,
        params.fusion.group_cap,
    ));
    if confidence >= params.fusion.emit_min_confidence {
        let mut attributes = Map::new();
        attributes.insert("lines".into(), (c.lines as u64).into());
        attributes.insert(
            "lines_uncovered".into(),
            ((c.lines - c.lines_covered) as u64).into(),
        );
        attributes.insert(
            "ink_covered_share".into(),
            math::round_output(c.ink_covered_share).into(),
        );
        attributes.insert("boxes".into(), (c.boxes as u64).into());
        det.findings.push(Finding {
            kind: Kind::ExtractionGap,
            confidence,
            bbox: c.gap_bbox,
            polygon: None,
            attributes,
            evidence,
        });
    }
    Ok(det)
}
