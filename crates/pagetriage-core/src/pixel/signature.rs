//! Wet signatures (module M7): isolated, tangled, curvy pen marks bigger than text. See
//! `docs/detectors/signature.md`.
//!
//! Candidates are the layout pass's large components that are not text: its graphics, and glyphs far
//! taller than their line. Pieces within a quarter inch are one signature. Each is scored on the
//! stroke path length against the box diagonal (a tangle is long for its size), the number of closed
//! loops (the Euler number of the ink), the share of edge strength along the axes, and how much
//! taller than text it stands. Integer measurements throughout; the score is a clipped linear sum.

use serde::Deserialize;
use serde_json::Map;

use super::Detection;
use super::components;
use super::handwriting;
use super::layout::{Layout, Rect};
use super::quality::stroke;
use crate::error::{Error, Result};
use crate::fusion::scorer;
use crate::math;
use crate::params::{FusionParams, Params};
use crate::schema::{Evidence, Finding, Kind};

pub const SIGNATURE: &str = "signature";
const DETECTOR_ID: &str = "signature.tangle";
const CITATION: &str = "module M7: a wet signature is an isolated tangle of curved strokes taller than text (path length, loops, edge orientation)";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignatureParams {
    pub min_height_xh: f64,
    pub min_width_xh: f64,
    pub merge_in: f64,
    pub min_width_in: f64,
    pub max_width_in: f64,
    pub min_height_in: f64,
    pub max_height_in: f64,
    pub max_fill: f64,
    pub min_fill: f64,
    pub term_clip: f64,
    pub prior: f64,
    pub tortuosity_threshold: f64,
    pub tortuosity_coef: f64,
    pub loops_threshold: f64,
    pub loops_coef: f64,
    pub loops_cap: f64,
    pub axis_threshold: f64,
    pub axis_coef: f64,
    pub height_xh_threshold: f64,
    pub height_xh_coef: f64,
    pub hole_threshold: f64,
    pub hole_coef: f64,
    pub veto_hole: f64,
    pub max_signatures: usize,
}

impl SignatureParams {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParams(format!("signature: {m}")));
        let all = [
            ("min_height_xh", self.min_height_xh),
            ("min_width_xh", self.min_width_xh),
            ("merge_in", self.merge_in),
            ("min_width_in", self.min_width_in),
            ("max_width_in", self.max_width_in),
            ("min_height_in", self.min_height_in),
            ("max_height_in", self.max_height_in),
            ("max_fill", self.max_fill),
            ("min_fill", self.min_fill),
            ("term_clip", self.term_clip),
            ("prior", self.prior),
            ("tortuosity_threshold", self.tortuosity_threshold),
            ("tortuosity_coef", self.tortuosity_coef),
            ("loops_threshold", self.loops_threshold),
            ("loops_coef", self.loops_coef),
            ("loops_cap", self.loops_cap),
            ("axis_threshold", self.axis_threshold),
            ("axis_coef", self.axis_coef),
            ("height_xh_threshold", self.height_xh_threshold),
            ("height_xh_coef", self.height_xh_coef),
            ("hole_threshold", self.hole_threshold),
            ("hole_coef", self.hole_coef),
            ("veto_hole", self.veto_hole),
        ];
        for (name, v) in all {
            if !v.is_finite() || v.abs() > 1.0e6 {
                return bad(&format!("{name} must be finite and within 1e6, got {v}"));
            }
        }
        if self.min_width_in > self.max_width_in || self.min_height_in > self.max_height_in {
            return bad("size windows must have min <= max");
        }
        if !(0.0..=1.0).contains(&self.max_fill)
            || !(0.0..=1.0).contains(&self.min_fill)
            || self.min_fill > self.max_fill
        {
            return bad("min_fill and max_fill must satisfy 0 <= min <= max <= 1");
        }
        if self.term_clip <= 0.0 || self.max_signatures == 0 || self.merge_in < 0.0 {
            return bad("term_clip must be > 0, max_signatures >= 1 and merge_in >= 0");
        }
        Ok(())
    }
}

/// What was measured on one signature candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub bbox: Rect,
    pub fill: f64,
    /// The biggest enclosed background region over the box area: a ring has one big hole, a
    /// flourish has several small ones.
    pub max_hole: f64,
    pub tortuosity: f64,
    pub loops: usize,
    pub axis_share: f64,
    pub height_xh: f64,
    pub width_in: f64,
    pub height_in: f64,
}

fn union(a: &Rect, b: &Rect) -> Rect {
    Rect {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    }
}

/// Gray (1971) bit-quad Euler number of an 8-connected binary image: `(Q1 - Q3 - 2 QD) / 4`.
fn euler_8(mask: &[u8], w: usize, h: usize) -> i64 {
    let at = |x: i64, y: i64| -> u8 {
        if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
            0
        } else {
            u8::from(mask[y as usize * w + x as usize] != 0)
        }
    };
    let (mut q1, mut q3, mut qd) = (0i64, 0i64, 0i64);
    for y in -1..h as i64 {
        for x in -1..w as i64 {
            let (a, b, c, d) = (at(x, y), at(x + 1, y), at(x, y + 1), at(x + 1, y + 1));
            match a + b + c + d {
                1 => q1 += 1,
                3 => q3 += 1,
                2 if a == d => qd += 1,
                _ => {}
            }
        }
    }
    (q1 - q3 - 2 * qd) / 4
}

/// Measure the ink inside `r` (the levelled frame).
fn measure(layout: &Layout, r: &Rect, dpi: f64) -> Option<Candidate> {
    let w = layout.width;
    let (bw, bh) = (r.width() as usize, r.height() as usize);
    let mut sub = vec![0u8; bw * bh];
    let mut ink = 0u64;
    for y in 0..bh {
        for x in 0..bw {
            if layout.ink[(r.y0 as usize + y) * w + r.x0 as usize + x] != 0 {
                sub[y * bw + x] = 1;
                ink += 1;
            }
        }
    }
    if ink == 0 {
        return None;
    }
    let lab = components::label_8(&sub, bw, bh);
    let holes = lab.components.len() as i64 - euler_8(&sub, bw, bh);
    // The biggest hole: background regions that do not reach the border of a padded copy.
    let (pw, ph) = (bw + 2, bh + 2);
    let mut background = vec![1u8; pw * ph];
    for y in 0..bh {
        for x in 0..bw {
            background[(y + 1) * pw + x + 1] = u8::from(sub[y * bw + x] == 0);
        }
    }
    let bg = components::label_8(&background, pw, ph);
    let outside = bg.labels[0];
    let max_hole = bg
        .components
        .iter()
        .enumerate()
        .filter(|(k, _)| *k as u32 + 1 != outside)
        .map(|(_, c)| c.area)
        .max()
        .unwrap_or(0) as f64
        / (bw * bh) as f64;
    // Stroke width from the centre-line distance values.
    let dist = stroke::chamfer_distance(&sub, bw, bh);
    let mut ridge: Vec<u16> = Vec::new();
    for y in 0..bh {
        for x in 0..bw {
            if dist[y * bw + x] > 0 && stroke::is_ridge(&dist, bw, bh, x, y) {
                ridge.push(dist[y * bw + x]);
            }
        }
    }
    ridge.sort_unstable();
    let width_px = stroke::width_px(ridge.get(ridge.len() / 2).copied().unwrap_or(2)).max(1.0);
    let diagonal = ((bw * bw + bh * bh) as f64).sqrt();
    Some(Candidate {
        bbox: *r,
        fill: ink as f64 / (bw * bh) as f64,
        max_hole,
        tortuosity: ink as f64 / width_px / diagonal,
        loops: holes.max(0) as usize,
        axis_share: handwriting::axis_share(layout, r),
        height_xh: bh as f64 / layout.xheight,
        width_in: bw as f64 / dpi,
        height_in: bh as f64 / dpi,
    })
}

/// Candidate regions of the page: its graphics and its glyphs far taller than their line, merged.
pub fn candidates(layout: &Layout, dpi: u16, p: &SignatureParams) -> Vec<Candidate> {
    let xh = layout.xheight;
    let dpi_f = f64::from(dpi);
    let mut boxes: Vec<Rect> = layout.graphics.clone();
    for g in &layout.glyphs {
        let unit = g
            .line
            .map_or(xh, |l| f64::from(layout.lines[l].glyph_height).max(xh));
        if f64::from(g.bbox.height()) >= p.min_height_xh * unit
            && f64::from(g.bbox.width()) >= p.min_width_xh * xh
        {
            boxes.push(g.bbox);
        }
    }
    // Merge boxes that are within `merge_in` of each other.
    let gap = (p.merge_in * dpi_f).round() as i64;
    let mut parent: Vec<usize> = (0..boxes.len()).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for i in 0..boxes.len() {
        for j in i + 1..boxes.len() {
            let (a, b) = (&boxes[i], &boxes[j]);
            let dx = i64::from(a.x0.max(b.x0)) - i64::from(a.x1.min(b.x1)) - 1;
            let dy = i64::from(a.y0.max(b.y0)) - i64::from(a.y1.min(b.y1)) - 1;
            if dx <= gap && dy <= gap {
                let (ra, rb) = (find(&mut parent, i), find(&mut parent, j));
                if ra != rb {
                    parent[ra.max(rb)] = ra.min(rb);
                }
            }
        }
    }
    let mut merged: Vec<Option<Rect>> = vec![None; boxes.len()];
    for (i, b) in boxes.iter().enumerate() {
        let root = find(&mut parent, i);
        merged[root] = Some(merged[root].map_or(*b, |m| union(&m, b)));
    }
    let mut out = Vec::new();
    for r in merged.into_iter().flatten() {
        let (w_in, h_in) = (f64::from(r.width()) / dpi_f, f64::from(r.height()) / dpi_f);
        if w_in < p.min_width_in
            || w_in > p.max_width_in
            || h_in < p.min_height_in
            || h_in > p.max_height_in
        {
            continue;
        }
        if let Some(c) =
            measure(layout, &r, dpi_f).filter(|c| c.fill >= p.min_fill && c.fill <= p.max_fill)
        {
            out.push(c);
        }
    }
    out
}

fn term(value: f64, threshold: f64, coef: f64, clip: f64) -> f64 {
    (coef * (value - threshold)).clamp(-clip, clip)
}

/// Run M7 on a segmented page.
pub fn detect(layout: &Layout, dpi: u16, params: &Params) -> Result<Detection> {
    detect_with(layout, dpi, &params.signature, &params.fusion)
}

pub fn detect_with(
    layout: &Layout,
    dpi: u16,
    p: &SignatureParams,
    fusion: &FusionParams,
) -> Result<Detection> {
    if dpi == 0 {
        return Err(Error::InvalidInput("dpi must be at least 1".into()));
    }
    let cands = candidates(layout, dpi, p);
    let mut det = Detection::default();
    det.features.insert("candidates".into(), cands.len() as f64);
    let mut kept: Vec<(f64, Finding)> = Vec::new();
    for c in &cands {
        let loops_per_in = (c.loops as f64).min(p.loops_cap * c.width_in) / c.width_in.max(0.1);
        let terms = [
            (
                "tortuosity",
                c.tortuosity,
                p.tortuosity_threshold,
                term(
                    c.tortuosity,
                    p.tortuosity_threshold,
                    p.tortuosity_coef,
                    p.term_clip,
                ),
            ),
            (
                "loops_per_inch",
                loops_per_in,
                p.loops_threshold,
                term(loops_per_in, p.loops_threshold, p.loops_coef, p.term_clip),
            ),
            (
                "axis_share",
                c.axis_share,
                p.axis_threshold,
                term(c.axis_share, p.axis_threshold, -p.axis_coef, p.term_clip),
            ),
            (
                "height_xh",
                c.height_xh,
                p.height_xh_threshold,
                term(
                    c.height_xh,
                    p.height_xh_threshold,
                    p.height_xh_coef,
                    p.term_clip,
                ),
            ),
            (
                "max_hole",
                c.max_hole,
                p.hole_threshold,
                term(c.max_hole, p.hole_threshold, -p.hole_coef, p.term_clip),
            ),
        ];
        let evidence = terms
            .iter()
            .map(|&(name, value, threshold, llr)| {
                Evidence::new(
                    DETECTOR_ID,
                    name,
                    value,
                    Some(threshold),
                    llr.clamp(-fusion.llr_clip, fusion.llr_clip),
                    format!(
                        "{:.2} x {:.2} in, {} loops, fill {:.2}",
                        c.width_in, c.height_in, c.loops, c.fill
                    ),
                    CITATION,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let confidence = math::round_output(scorer::confidence(p.prior, &evidence));
        // A box with a hole this big is a ring or a frame (a stamp, a circled word), not a flourish.
        if c.max_hole >= p.veto_hole || confidence < fusion.emit_min_confidence {
            continue;
        }
        let mut attributes = Map::new();
        attributes.insert("width_in".into(), math::round_output(c.width_in).into());
        attributes.insert("height_in".into(), math::round_output(c.height_in).into());
        attributes.insert("loops".into(), (c.loops as u64).into());
        attributes.insert("tortuosity".into(), math::round_output(c.tortuosity).into());
        attributes.insert("fill".into(), math::round_output(c.fill).into());
        attributes.insert("max_hole".into(), math::round_output(c.max_hole).into());
        kept.push((
            confidence,
            Finding {
                kind: Kind::Signature,
                confidence,
                bbox: Some(layout.rect_to_page(&c.bbox)),
                polygon: None,
                attributes,
                evidence,
            },
        ));
    }
    kept.sort_by(|a, b| b.0.total_cmp(&a.0));
    kept.truncate(p.max_signatures);
    kept.sort_by_key(|(_, f)| f.bbox.map(|b| (b[1], b[0])));
    det.findings = kept.into_iter().map(|(_, f)| f).collect();
    det.features
        .insert("signatures".into(), det.findings.len() as f64);
    Ok(det)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pixel::layout;

    const W: usize = 420;
    const H: usize = 300;

    fn paper() -> Vec<u8> {
        vec![240u8; W * H]
    }

    fn put(g: &mut [u8], x: i64, y: i64, pen: i64) {
        for dy in 0..pen {
            for dx in 0..pen {
                let (px, py) = (x + dx, y + dy);
                if px >= 0 && py >= 0 && (px as usize) < W && (py as usize) < H {
                    g[py as usize * W + px as usize] = 25;
                }
            }
        }
    }

    fn line(g: &mut [u8], a: (i64, i64), b: (i64, i64), pen: i64) {
        let n = (b.0 - a.0).abs().max((b.1 - a.1).abs()).max(1);
        for s in 0..=n {
            put(g, a.0 + (b.0 - a.0) * s / n, a.1 + (b.1 - a.1) * s / n, pen);
        }
    }

    /// Lines of stem-like text so the page has an x-height of 8 px.
    fn text(g: &mut [u8], y0: usize, lines: usize) {
        for l in 0..lines {
            for k in 0..40 {
                for dy in 0..8 {
                    g[(y0 + l * 17 + dy) * W + 20 + k * 6] = 20;
                    g[(y0 + l * 17 + dy) * W + 21 + k * 6] = 20;
                }
            }
        }
    }

    /// A flourish: loops walking right with an underswing, from a tiny LCG. ~290 x 70 px.
    fn flourish(g: &mut [u8], x0: i64, y0: i64) {
        let mut s = 7u32;
        let mut next = |m: u32| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            i64::from((s >> 16) % m)
        };
        let mut prev = (x0, y0 + 40);
        for i in 0..9 {
            let cx = x0 + i * 30;
            for k in 0..6 {
                let ang = f64::from(k) * std::f64::consts::PI / 3.0;
                let p = (
                    cx + (22.0 * math::cos(ang)) as i64 + next(8),
                    y0 + 35 + (30.0 * math::sin(ang)) as i64 + next(8) - 4,
                );
                line(g, prev, p, 2);
                prev = p;
            }
        }
        line(g, prev, (x0 + 40, y0 + 68), 2);
        line(g, (x0 + 40, y0 + 68), (x0 + 270, y0 + 55), 2);
    }

    fn run(g: &[u8]) -> Detection {
        let p = Params::defaults().unwrap();
        let l = layout::segment(g, W, H, 0.0, &p).unwrap();
        detect(&l, 100, &p).unwrap()
    }

    #[test]
    fn a_tangled_flourish_below_text_is_a_signature() {
        let mut g = paper();
        text(&mut g, 20, 6);
        flourish(&mut g, 60, 170);
        let d = run(&g);
        assert_eq!(d.findings.len(), 1, "{:?}", d.features);
        let f = &d.findings[0];
        assert_eq!(f.kind, Kind::Signature);
        let b = f.bbox.unwrap();
        assert!(b[1] >= 160 && b[3] <= 270 && b[0] >= 30, "{b:?}");
        assert_eq!(f.evidence.len(), 5);
    }

    #[test]
    fn a_ring_a_solid_block_and_plain_text_are_not_signatures() {
        let mut g = paper();
        text(&mut g, 20, 6);
        // A double ring with a word's worth of marks inside.
        for r in [55.0f64, 38.0] {
            for k in 0..720 {
                let a = f64::from(k) * std::f64::consts::PI / 360.0;
                put(
                    &mut g,
                    210 + (r * math::cos(a)) as i64,
                    220 + (r * math::sin(a)) as i64,
                    3,
                );
            }
        }
        for k in 0..6 {
            line(&mut g, (190 + k * 8, 205), (190 + k * 8, 230), 3);
        }
        // A solid block.
        for y in 170..230 {
            for x in 320..400 {
                g[y * W + x] = 40;
            }
        }
        let d = run(&g);
        assert!(d.findings.is_empty(), "{:?}", d.findings);
    }

    #[test]
    fn euler_number_counts_loops() {
        // A ring: one component, one hole.
        let mut m = vec![0u8; 100];
        for i in 2..8 {
            m[2 * 10 + i] = 1;
            m[7 * 10 + i] = 1;
            m[i * 10 + 2] = 1;
            m[i * 10 + 7] = 1;
        }
        assert_eq!(euler_8(&m, 10, 10), 0);
        let lab = components::label_8(&m, 10, 10);
        assert_eq!(lab.components.len() as i64 - euler_8(&m, 10, 10), 1);
        // A dot has Euler number 1 and no hole.
        let mut d = vec![0u8; 25];
        d[12] = 1;
        assert_eq!(euler_8(&d, 5, 5), 1);
    }

    #[test]
    fn blank_pages_bad_dpi_and_params() {
        let p = Params::defaults().unwrap();
        let l = layout::segment(&paper(), W, H, 0.0, &p).unwrap();
        let d = detect(&l, 150, &p).unwrap();
        assert!(d.findings.is_empty());
        assert_eq!(d.features["candidates"], 0.0);
        assert!(detect(&l, 0, &p).is_err());
        let mut sp = p.signature.clone();
        assert!(sp.validate().is_ok());
        sp.max_fill = 1.5;
        assert!(sp.validate().is_err());
        let mut sp = p.signature.clone();
        sp.min_width_in = 9.0;
        assert!(sp.validate().is_err());
    }

    #[test]
    fn detection_is_repeatable() {
        let mut g = paper();
        text(&mut g, 20, 6);
        flourish(&mut g, 60, 170);
        assert_eq!(run(&g).findings, run(&g).findings);
    }
}
