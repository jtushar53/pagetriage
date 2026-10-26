//! Redaction boxes (module M12, pixel part): solid dark rectangles in or against running text. See
//! `docs/detectors/redaction.md`.
//!
//! Candidates are the layout's glyph-class components that are rectangles (nearly all ink in the box,
//! nearly all of each edge inked) of a size that is a word to a paragraph. A box is scored on how many
//! sides have running text beside it: a name blacked out in a line has text left and right, a paragraph
//! blocked out has lines above and below. A filled checkbox is too small, a table's shaded bar has no
//! text on its short sides and a logo is not a rectangle.

use serde::Deserialize;
use serde_json::Map;

use super::Detection;
use super::components;
use super::layout::{Layout, Rect};
use crate::error::{Error, Result};
use crate::fusion::scorer;
use crate::math;
use crate::params::{FusionParams, Params};
use crate::schema::{Evidence, Finding, Kind};

pub const REDACTION: &str = "redaction";
const DETECTOR_ID: &str = "redaction.solid_box";
const CITATION: &str =
    "module M12: a redaction box is a solid straight-edged rectangle with running text beside it";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedactionParams {
    pub dark_delta: u8,
    pub min_fill: f64,
    pub min_edge: f64,
    pub min_height_xh: f64,
    pub min_width_xh: f64,
    pub max_height_xh: f64,
    pub max_aspect: f64,
    pub neighbour_xh: f64,
    pub term_clip: f64,
    pub prior: f64,
    pub neighbour_threshold: f64,
    pub neighbour_coef: f64,
    pub solid_threshold: f64,
    pub solid_coef: f64,
    pub max_boxes: usize,
}

impl RedactionParams {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParams(format!("redaction: {m}")));
        let all = [
            ("min_fill", self.min_fill),
            ("min_edge", self.min_edge),
            ("min_height_xh", self.min_height_xh),
            ("min_width_xh", self.min_width_xh),
            ("max_height_xh", self.max_height_xh),
            ("max_aspect", self.max_aspect),
            ("neighbour_xh", self.neighbour_xh),
            ("term_clip", self.term_clip),
            ("prior", self.prior),
            ("neighbour_threshold", self.neighbour_threshold),
            ("neighbour_coef", self.neighbour_coef),
            ("solid_threshold", self.solid_threshold),
            ("solid_coef", self.solid_coef),
        ];
        for (name, v) in all {
            if !v.is_finite() || v.abs() > 1.0e6 {
                return bad(&format!("{name} must be finite and within 1e6, got {v}"));
            }
        }
        if !(0.0..=1.0).contains(&self.min_fill) || !(0.0..=1.0).contains(&self.min_edge) {
            return bad("min_fill and min_edge must be in [0, 1]");
        }
        if self.min_height_xh <= 0.0
            || self.max_height_xh < self.min_height_xh
            || self.min_width_xh <= 0.0
            || self.max_aspect < 1.0
        {
            return bad("0 < min_height_xh <= max_height_xh, min_width_xh > 0 and max_aspect >= 1");
        }
        if self.term_clip <= 0.0 || self.max_boxes == 0 || self.neighbour_xh < 0.0 {
            return bad("term_clip must be > 0, max_boxes >= 1 and neighbour_xh >= 0");
        }
        Ok(())
    }
}

/// A rectangle candidate and what was measured on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Box_ {
    pub bbox: Rect,
    pub fill: f64,
    pub edge: f64,
    /// How many of left, right, above and below have running text beside the box.
    pub sides: usize,
    pub height_xh: f64,
    pub width_xh: f64,
}

/// The page's most common bright gray: the paper.
fn paper_level(gray: &[u8]) -> u8 {
    let mut hist = [0u64; 256];
    for &v in gray {
        hist[usize::from(v)] += 1;
    }
    let mut best = 255usize;
    for v in (128..256).rev() {
        if hist[v] > hist[best] {
            best = v;
        }
    }
    best as u8
}

/// Find solid dark rectangles. `levelled` is the gray page in the layout's frame
/// ([`Layout::level_gray`]). The layout's own ink mask cannot be used: Sauvola leaves the inside of a
/// big solid block as background (its local variance is zero, so its threshold is zero).
pub fn boxes(layout: &Layout, levelled: &[u8], p: &RedactionParams) -> Vec<Box_> {
    let w = layout.width;
    let xh = layout.xheight;
    let level = paper_level(levelled).saturating_sub(p.dark_delta);
    let dark: Vec<u8> = levelled.iter().map(|&v| u8::from(v <= level)).collect();
    let ink = |x: u32, y: u32| dark[y as usize * w + x as usize] != 0;
    let lab = components::label_8(&dark, w, layout.height);
    let reach = (p.neighbour_xh * xh).round() as i64;
    let mut out = Vec::new();
    for c in &lab.components {
        let r = Rect {
            x0: c.x0,
            y0: c.y0,
            x1: c.x1,
            y1: c.y1,
        };
        let (bw, bh) = (f64::from(r.width()), f64::from(r.height()));
        if bh < p.min_height_xh * xh || bh > p.max_height_xh * xh || bw < p.min_width_xh * xh {
            continue;
        }
        if bw.max(bh) / bw.min(bh) > p.max_aspect {
            continue;
        }
        let fill = f64::from(c.area) / (bw * bh);
        if fill < p.min_fill {
            continue;
        }
        // Each edge counts as inked where the outer two rows (columns) have ink: a rotated or blurred
        // rectangle has a staircase edge a pixel deep.
        let y_in = |y: u32| y.min(r.y1);
        let y_out = |y: u32| y.max(r.y0);
        let edge = [
            (r.x0..=r.x1)
                .filter(|&x| ink(x, r.y0) || ink(x, y_in(r.y0 + 1)))
                .count() as f64
                / bw,
            (r.x0..=r.x1)
                .filter(|&x| ink(x, r.y1) || ink(x, y_out(r.y1.saturating_sub(1))))
                .count() as f64
                / bw,
            (r.y0..=r.y1)
                .filter(|&y| ink(r.x0, y) || ink((r.x0 + 1).min(r.x1), y))
                .count() as f64
                / bh,
            (r.y0..=r.y1)
                .filter(|&y| ink(r.x1, y) || ink(r.x1.saturating_sub(1).max(r.x0), y))
                .count() as f64
                / bh,
        ]
        .into_iter()
        .fold(f64::INFINITY, f64::min);
        if edge < p.min_edge {
            continue;
        }
        // Running text beside the box.
        let (mut left, mut right, mut above, mut below) = (false, false, false, false);
        for o in &layout.glyphs {
            if o.line.is_none() {
                continue;
            }
            let b = o.bbox;
            let (oy0, oy1) = (i64::from(b.y0), i64::from(b.y1));
            let overlap_y = oy1.min(i64::from(r.y1)) - oy0.max(i64::from(r.y0)) + 1;
            if overlap_y * 2 >= i64::from(b.height()) {
                let gap_right = i64::from(b.x0) - i64::from(r.x1) - 1;
                let gap_left = i64::from(r.x0) - i64::from(b.x1) - 1;
                right |= (0..=reach).contains(&gap_right);
                left |= (0..=reach).contains(&gap_left);
            }
            let overlap_x =
                i64::from(b.x1).min(i64::from(r.x1)) - i64::from(b.x0).max(i64::from(r.x0)) + 1;
            if overlap_x > 0 {
                let gap_below = i64::from(b.y0) - i64::from(r.y1) - 1;
                let gap_above = i64::from(r.y0) - i64::from(b.y1) - 1;
                below |= (0..=reach).contains(&gap_below);
                above |= (0..=reach).contains(&gap_above);
            }
        }
        out.push(Box_ {
            bbox: r,
            fill,
            edge,
            sides: usize::from(left) + usize::from(right) + usize::from(above) + usize::from(below),
            height_xh: bh / xh,
            width_xh: bw / xh,
        });
    }
    out
}

fn term(value: f64, threshold: f64, coef: f64, clip: f64) -> f64 {
    (coef * (value - threshold)).clamp(-clip, clip)
}

/// Run the pixel part of M12 on a segmented page.
pub fn detect(layout: &Layout, gray: &[u8], params: &Params) -> Result<Detection> {
    detect_with(layout, gray, &params.redaction, &params.fusion)
}

pub fn detect_with(
    layout: &Layout,
    gray: &[u8],
    p: &RedactionParams,
    fusion: &FusionParams,
) -> Result<Detection> {
    let bs = boxes(layout, &layout.level_gray(gray), p);
    let mut det = Detection::default();
    det.features.insert("candidates".into(), bs.len() as f64);
    let mut kept: Vec<(f64, Finding)> = Vec::new();
    for b in &bs {
        let solid = b.fill.min(b.edge);
        let evidence = vec![
            Evidence::new(
                DETECTOR_ID,
                "sides_with_text",
                b.sides as f64,
                Some(p.neighbour_threshold),
                term(
                    b.sides as f64,
                    p.neighbour_threshold,
                    p.neighbour_coef,
                    p.term_clip,
                )
                .clamp(-fusion.llr_clip, fusion.llr_clip),
                format!("{:.1} x {:.1} x-heights", b.width_xh, b.height_xh),
                CITATION,
            )?,
            Evidence::new(
                DETECTOR_ID,
                "solidity",
                solid,
                Some(p.solid_threshold),
                term(solid, p.solid_threshold, p.solid_coef, p.term_clip)
                    .clamp(-fusion.llr_clip, fusion.llr_clip),
                format!("fill {:.3}, weakest edge {:.3}", b.fill, b.edge),
                CITATION,
            )?,
        ];
        let confidence = math::round_output(scorer::confidence(p.prior, &evidence));
        if confidence < fusion.emit_min_confidence {
            continue;
        }
        let mut attributes = Map::new();
        attributes.insert("sides_with_text".into(), (b.sides as u64).into());
        attributes.insert("height_xh".into(), math::round_output(b.height_xh).into());
        attributes.insert("width_xh".into(), math::round_output(b.width_xh).into());
        attributes.insert("fill".into(), math::round_output(b.fill).into());
        kept.push((
            f64::from(b.bbox.width()) * f64::from(b.bbox.height()),
            Finding {
                kind: Kind::RedactionBox,
                confidence,
                bbox: Some(layout.rect_to_page(&b.bbox)),
                polygon: None,
                attributes,
                evidence,
            },
        ));
    }
    kept.sort_by(|a, b| b.0.total_cmp(&a.0));
    kept.truncate(p.max_boxes);
    kept.sort_by_key(|(_, f)| f.bbox.map(|b| (b[1], b[0])));
    det.findings = kept.into_iter().map(|(_, f)| f).collect();
    det.features
        .insert("boxes".into(), det.findings.len() as f64);
    Ok(det)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pixel::layout;

    const W: usize = 420;
    const H: usize = 260;

    fn paper() -> Vec<u8> {
        vec![240u8; W * H]
    }

    fn fill(g: &mut [u8], x0: usize, y0: usize, x1: usize, y1: usize, v: u8) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                g[y * W + x] = v;
            }
        }
    }

    /// Lines of stem text, 17 px apart, with stems 10 px apart and an x-height of 8.
    fn text(g: &mut [u8], x0: usize, y0: usize, lines: usize, n: usize) {
        for l in 0..lines {
            for k in 0..n {
                fill(
                    g,
                    x0 + k * 10,
                    y0 + l * 17,
                    x0 + k * 10 + 2,
                    y0 + l * 17 + 7,
                    20,
                );
            }
        }
    }

    fn run(g: &[u8]) -> Detection {
        let p = Params::defaults().unwrap();
        let l = layout::segment(g, W, H, 0.0, &p).unwrap();
        detect(&l, g, &p).unwrap()
    }

    #[test]
    fn a_word_blacked_out_inside_a_line_is_a_redaction() {
        let mut g = paper();
        text(&mut g, 20, 20, 8, 38);
        fill(&mut g, 125, 36, 175, 51, 0); // over one line
        let d = run(&g);
        assert_eq!(d.findings.len(), 1, "{:?}", d.features);
        let f = &d.findings[0];
        assert_eq!(f.kind, Kind::RedactionBox);
        let b = f.bbox.unwrap();
        assert!(
            b[0] >= 124 && b[2] <= 176 && b[1] >= 35 && b[3] <= 52,
            "{b:?}"
        );
        assert!(f.confidence > 0.5);
    }

    #[test]
    fn a_big_block_over_several_lines_is_found_though_sauvola_would_not() {
        let mut g = paper();
        text(&mut g, 20, 20, 12, 38);
        fill(&mut g, 20, 69, 375, 136, 0); // 356 x 68 px of solid black
        let d = run(&g);
        assert_eq!(d.findings.len(), 1, "{:?} {:?}", d.features, d.findings);
        assert!(d.findings[0].bbox.unwrap()[2] >= 370);
    }

    #[test]
    fn shapes_that_are_not_redactions_are_not_found() {
        let mut g = paper();
        text(&mut g, 20, 20, 12, 38);
        // A filled checkbox about as big as a letter, a thin heavy rule, a reversed heading and a
        // round blob.
        fill(&mut g, 4, 40, 15, 51, 20);
        fill(&mut g, 20, 100, 400, 102, 20);
        fill(&mut g, 60, 150, 200, 180, 20);
        fill(&mut g, 80, 158, 90, 172, 240);
        fill(&mut g, 120, 158, 140, 172, 240);
        for y in 0..60usize {
            for x in 0..60usize {
                let (dx, dy) = (x as i64 - 30, y as i64 - 30);
                if dx * dx + dy * dy <= 900 {
                    g[(190 + y.min(59)) * W + 330 + x.min(59)] = 20;
                }
            }
        }
        let d = run(&g);
        assert!(d.findings.is_empty(), "{:?} {:?}", d.features, d.findings);
    }

    #[test]
    fn a_dark_box_with_no_text_near_is_not_enough() {
        let mut g = paper();
        fill(&mut g, 100, 100, 200, 130, 0);
        assert!(run(&g).findings.is_empty());
    }

    #[test]
    fn blank_pages_and_params() {
        assert!(run(&paper()).findings.is_empty());
        let p = Params::defaults().unwrap();
        let mut rp = p.redaction.clone();
        assert!(rp.validate().is_ok());
        rp.min_fill = 1.5;
        assert!(rp.validate().is_err());
        let mut rp = p.redaction.clone();
        rp.max_height_xh = 0.1;
        assert!(rp.validate().is_err());
    }

    #[test]
    fn detection_is_repeatable() {
        let mut g = paper();
        text(&mut g, 20, 20, 8, 38);
        fill(&mut g, 125, 36, 175, 51, 0);
        assert_eq!(run(&g).findings, run(&g).findings);
    }
}
