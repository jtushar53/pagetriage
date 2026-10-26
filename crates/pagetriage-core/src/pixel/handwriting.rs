//! Handwritten vs machine-printed text (module M5), per word of the layout pass's lines, merged
//! into regions. No model: four hand-designed features, fixed thresholds and a handful of frozen
//! logistic coefficients. See `docs/detectors/handwriting.md`.
//!
//! A word is a run of glyphs on one line with small gaps; a handwritten fill-in on a printed form
//! is a word (or a few) on the same line as the printed label, so it is found without separating
//! the two by pixel. Features per word: baseline wander, share of edge strength along the axes,
//! stroke-width variation and, for words of three or more glyphs, height variation. Scores borrow a
//! fraction of their neighbours' on the line.

use serde::Deserialize;
use serde_json::Map;

use super::Detection;
use super::layout::{GlyphBox, Layout, Rect};
use super::quality::stroke;
use crate::error::{Error, Result};
use crate::math;
use crate::params::{FusionParams, Params};
use crate::schema::{Evidence, Finding, Kind};

pub const HANDWRITING: &str = "handwriting";
const DETECTOR_ID: &str = "handwriting.pixel";
const CITATION: &str = "module M5: stroke, baseline and edge-orientation regularity of print against handwriting (Garlapati and Chalamala 2017; Belaid et al., arXiv 1303.4614)";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandwritingParams {
    pub word_gap_xh: f64,
    pub tall_fraction: f64,
    pub baseline_dev_threshold: f64,
    pub baseline_dev_coef: f64,
    pub axis_share_threshold: f64,
    pub axis_share_coef: f64,
    pub stroke_cv_threshold: f64,
    pub stroke_cv_coef: f64,
    pub height_cv_threshold: f64,
    pub height_cv_coef: f64,
    pub min_glyphs_for_height_cv: usize,
    pub term_clip: f64,
    pub prior_logit: f64,
    pub neighbour_weight: f64,
    pub region_gap_xh: f64,
    pub emit_min_probability: f64,
    pub min_region_glyphs: usize,
    pub max_regions: usize,
}

impl HandwritingParams {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParams(format!("handwriting: {m}")));
        let all = [
            ("word_gap_xh", self.word_gap_xh),
            ("tall_fraction", self.tall_fraction),
            ("baseline_dev_threshold", self.baseline_dev_threshold),
            ("baseline_dev_coef", self.baseline_dev_coef),
            ("axis_share_threshold", self.axis_share_threshold),
            ("axis_share_coef", self.axis_share_coef),
            ("stroke_cv_threshold", self.stroke_cv_threshold),
            ("stroke_cv_coef", self.stroke_cv_coef),
            ("height_cv_threshold", self.height_cv_threshold),
            ("height_cv_coef", self.height_cv_coef),
            ("term_clip", self.term_clip),
            ("prior_logit", self.prior_logit),
            ("neighbour_weight", self.neighbour_weight),
            ("region_gap_xh", self.region_gap_xh),
            ("emit_min_probability", self.emit_min_probability),
        ];
        for (name, v) in all {
            if !v.is_finite() {
                return bad(&format!("{name} must be finite, got {v}"));
            }
        }
        if self.word_gap_xh < 0.0 || self.region_gap_xh < 0.0 || self.neighbour_weight < 0.0 {
            return bad("word_gap_xh, region_gap_xh and neighbour_weight must be >= 0");
        }
        if !(0.0..=1.0).contains(&self.tall_fraction)
            || !(0.0..=1.0).contains(&self.emit_min_probability)
        {
            return bad("tall_fraction and emit_min_probability must be in [0, 1]");
        }
        if self.term_clip <= 0.0 || self.term_clip > 1.0e3 {
            return bad("term_clip must be in (0, 1000]");
        }
        if self.min_glyphs_for_height_cv < 2 || self.min_region_glyphs == 0 {
            return bad("min_glyphs_for_height_cv must be >= 2 and min_region_glyphs >= 1");
        }
        Ok(())
    }
}

/// One word and its measurements.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub bbox: Rect,
    pub line: usize,
    pub glyphs: u32,
    pub ink: u64,
    pub baseline_dev: f64,
    pub axis_share: f64,
    pub stroke_cv: f64,
    /// `None` for a word with fewer than `min_glyphs_for_height_cv` tall glyphs.
    pub height_cv: Option<f64>,
    /// Probability of handwriting after the neighbour step.
    pub probability: f64,
}

fn median_i64(v: &mut [i64]) -> i64 {
    v.sort_unstable();
    v[v.len() / 2]
}

/// Share of edge strength within about 22 degrees of an axis, over the box. 3x3 Sobel of the 0/1
/// ink mask, integers throughout; a gradient is axis-aligned when the smaller component is under
/// 0.4 of the larger (tan 22.5 degrees is 0.414).
pub(crate) fn axis_share(layout: &Layout, r: &Rect) -> f64 {
    let (w, h) = (layout.width, layout.height);
    let at = |x: i64, y: i64| -> i64 {
        if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
            0
        } else {
            i64::from(layout.ink[y as usize * w + x as usize])
        }
    };
    let (mut axis, mut total) = (0i64, 0i64);
    for y in i64::from(r.y0)..=i64::from(r.y1) {
        for x in i64::from(r.x0)..=i64::from(r.x1) {
            let gx = (at(x + 1, y - 1) + 2 * at(x + 1, y) + at(x + 1, y + 1))
                - (at(x - 1, y - 1) + 2 * at(x - 1, y) + at(x - 1, y + 1));
            let gy = (at(x - 1, y + 1) + 2 * at(x, y + 1) + at(x + 1, y + 1))
                - (at(x - 1, y - 1) + 2 * at(x, y - 1) + at(x + 1, y - 1));
            let (ax, ay) = (gx.abs(), gy.abs());
            if ax + ay == 0 {
                continue;
            }
            total += ax + ay;
            if ax.min(ay) * 5 < ax.max(ay) * 2 {
                axis += ax + ay;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        axis as f64 / total as f64
    }
}

/// Coefficient of variation of the distance-transform value on stroke centre lines in the box.
fn stroke_cv(layout: &Layout, dist: &[u16], r: &Rect) -> f64 {
    let (w, h) = (layout.width, layout.height);
    let mut values: Vec<f64> = Vec::new();
    for y in r.y0 as usize..=r.y1 as usize {
        for x in r.x0 as usize..=r.x1 as usize {
            if dist[y * w + x] > 0 && stroke::is_ridge(dist, w, h, x, y) {
                values.push(f64::from(dist[y * w + x]));
            }
        }
    }
    if values.len() < 4 {
        return 0.0;
    }
    let n = values.len() as f64;
    let mean = math::pairwise_sum(&values) / n;
    let sq: Vec<f64> = values.iter().map(|v| (v - mean) * (v - mean)).collect();
    let var = math::pairwise_sum(&sq) / n;
    if mean <= 0.0 { 0.0 } else { var.sqrt() / mean }
}

/// Measure every word of every layout line.
pub fn words(layout: &Layout, p: &HandwritingParams) -> Vec<Word> {
    let dist = stroke::chamfer_distance(&layout.ink, layout.width, layout.height);
    let gap = p.word_gap_xh * layout.xheight;
    let mut out = Vec::new();
    for (li, line) in layout.lines.iter().enumerate() {
        let mut glyphs: Vec<_> = layout
            .glyphs
            .iter()
            .filter(|g| g.line == Some(li))
            .collect();
        if glyphs.is_empty() {
            continue;
        }
        glyphs.sort_by_key(|g| (g.bbox.x0, g.bbox.y0));
        let tall_min = p.tall_fraction * f64::from(line.glyph_height);
        let tall = |g: &GlyphBox| f64::from(g.bbox.height()) >= tall_min;
        let mut bottoms: Vec<i64> = glyphs
            .iter()
            .filter(|g| tall(g))
            .map(|g| i64::from(g.bbox.y1))
            .collect();
        let baseline = if bottoms.is_empty() {
            i64::from(line.bbox.y1)
        } else {
            median_i64(&mut bottoms)
        };

        // Split into words.
        let mut groups: Vec<Vec<&GlyphBox>> = Vec::new();
        let mut right = i64::MIN;
        for g in glyphs {
            let starts_new = groups.is_empty() || (i64::from(g.bbox.x0) - right - 1) as f64 > gap;
            if starts_new {
                groups.push(Vec::new());
            }
            groups.last_mut().expect("pushed above").push(g);
            right = right.max(i64::from(g.bbox.x1));
        }

        let first = out.len();
        for grp in groups {
            let mut bbox = grp[0].bbox;
            for g in &grp {
                bbox = Rect {
                    x0: bbox.x0.min(g.bbox.x0),
                    y0: bbox.y0.min(g.bbox.y0),
                    x1: bbox.x1.max(g.bbox.x1),
                    y1: bbox.y1.max(g.bbox.y1),
                };
            }
            let talls: Vec<_> = grp.iter().filter(|g| tall(g)).collect();
            let baseline_dev = if talls.is_empty() {
                0.0
            } else {
                let devs: Vec<f64> = talls
                    .iter()
                    .map(|g| (i64::from(g.bbox.y1) - baseline).abs() as f64)
                    .collect();
                math::pairwise_sum(&devs) / devs.len() as f64 / layout.xheight
            };
            let height_cv = if talls.len() >= p.min_glyphs_for_height_cv {
                let hs: Vec<f64> = talls.iter().map(|g| f64::from(g.bbox.height())).collect();
                let mean = math::pairwise_sum(&hs) / hs.len() as f64;
                let sq: Vec<f64> = hs.iter().map(|v| (v - mean) * (v - mean)).collect();
                Some((math::pairwise_sum(&sq) / hs.len() as f64).sqrt() / mean)
            } else {
                None
            };
            out.push(Word {
                bbox,
                line: li,
                glyphs: grp.len() as u32,
                ink: grp.iter().map(|g| u64::from(g.area)).sum(),
                baseline_dev,
                axis_share: axis_share(layout, &bbox),
                stroke_cv: stroke_cv(layout, &dist, &bbox),
                height_cv,
                probability: 0.0,
            });
        }
        // Scores, then each word's own plus a fraction of its line neighbours' mean.
        let own: Vec<f64> = out[first..].iter().map(|w| word_logit(w, p)).collect();
        for (k, w) in out[first..].iter_mut().enumerate() {
            let mut neighbours = Vec::new();
            if k > 0 {
                neighbours.push(own[k - 1]);
            }
            if k + 1 < own.len() {
                neighbours.push(own[k + 1]);
            }
            let context = if neighbours.is_empty() {
                0.0
            } else {
                math::pairwise_sum(&neighbours) / neighbours.len() as f64
            };
            w.probability = math::sigmoid(own[k] + p.neighbour_weight * context);
        }
    }
    out
}

fn clip_term(coef: f64, value: f64, threshold: f64, clip: f64) -> f64 {
    (coef * (value - threshold)).clamp(-clip, clip)
}

/// The terms of a word, in the order baseline, axis, stroke, height. A term is zero when the word
/// cannot supply the feature.
fn terms(w: &Word, p: &HandwritingParams) -> [f64; 4] {
    let c = p.term_clip;
    [
        clip_term(
            p.baseline_dev_coef,
            w.baseline_dev,
            p.baseline_dev_threshold,
            c,
        ),
        clip_term(-p.axis_share_coef, w.axis_share, p.axis_share_threshold, c),
        clip_term(p.stroke_cv_coef, w.stroke_cv, p.stroke_cv_threshold, c),
        w.height_cv.map_or(0.0, |v| {
            clip_term(p.height_cv_coef, v, p.height_cv_threshold, c)
        }),
    ]
}

fn word_logit(w: &Word, p: &HandwritingParams) -> f64 {
    p.prior_logit + math::pairwise_sum(&terms(w, p))
}

/// Run M5 on a segmented page.
pub fn detect(layout: &Layout, params: &Params) -> Result<Detection> {
    detect_with(layout, &params.handwriting, &params.fusion)
}

pub fn detect_with(
    layout: &Layout,
    p: &HandwritingParams,
    fusion: &FusionParams,
) -> Result<Detection> {
    let ws = words(layout, p);
    let mut det = Detection::default();
    let hand: Vec<&Word> = ws.iter().filter(|w| w.probability >= 0.5).collect();
    let ink_total: u64 = ws.iter().map(|w| w.ink).sum();
    let ink_hand: u64 = hand.iter().map(|w| w.ink).sum();
    det.features.insert("words".into(), ws.len() as f64);
    det.features.insert("hand_words".into(), hand.len() as f64);
    det.features.insert(
        "hand_ink_share".into(),
        if ink_total == 0 {
            0.0
        } else {
            ink_hand as f64 / ink_total as f64
        },
    );

    // Regions: runs of handwritten words on one line with small gaps.
    let gap = p.region_gap_xh * layout.xheight;
    let mut regions: Vec<Vec<&Word>> = Vec::new();
    for w in ws.iter().filter(|w| w.probability >= 0.5) {
        let joins = regions.last().is_some_and(|r| {
            let last = r.last().expect("regions are never empty");
            last.line == w.line
                && (i64::from(w.bbox.x0) - i64::from(last.bbox.x1) - 1) as f64 <= gap
        });
        if joins {
            regions.last_mut().expect("checked above").push(w);
        } else {
            regions.push(vec![w]);
        }
    }
    let mut kept: Vec<(u64, Finding)> = Vec::new();
    for r in regions {
        let glyphs: u32 = r.iter().map(|w| w.glyphs).sum();
        let probs: Vec<f64> = r.iter().map(|w| w.probability).collect();
        let mean = math::pairwise_sum(&probs) / probs.len() as f64;
        let confidence = math::round_output(mean);
        if (glyphs as usize) < p.min_region_glyphs || confidence < p.emit_min_probability {
            continue;
        }
        let mut bbox = r[0].bbox;
        for w in &r {
            bbox = Rect {
                x0: bbox.x0.min(w.bbox.x0),
                y0: bbox.y0.min(w.bbox.y0),
                x1: bbox.x1.max(w.bbox.x1),
                y1: bbox.y1.max(w.bbox.y1),
            };
        }
        let n = r.len() as f64;
        let mean_of = |f: &dyn Fn(&Word) -> f64| {
            math::pairwise_sum(&r.iter().map(|w| f(w)).collect::<Vec<_>>()) / n
        };
        let t: Vec<[f64; 4]> = r.iter().map(|w| terms(w, p)).collect();
        let mean_term =
            |i: usize| math::pairwise_sum(&t.iter().map(|x| x[i]).collect::<Vec<_>>()) / n;
        let ev = |feature: &str, value: f64, threshold: f64, llr: f64| {
            Evidence::new(
                DETECTOR_ID,
                feature,
                value,
                Some(threshold),
                llr.clamp(-fusion.llr_clip, fusion.llr_clip),
                format!("mean over the {} words of the region", r.len()),
                CITATION,
            )
        };
        let evidence = vec![
            ev(
                "baseline_dev",
                mean_of(&|w| w.baseline_dev),
                p.baseline_dev_threshold,
                mean_term(0),
            )?,
            ev(
                "axis_share",
                mean_of(&|w| w.axis_share),
                p.axis_share_threshold,
                mean_term(1),
            )?,
            ev(
                "stroke_cv",
                mean_of(&|w| w.stroke_cv),
                p.stroke_cv_threshold,
                mean_term(2),
            )?,
        ];
        let mut attributes = Map::new();
        attributes.insert("words".into(), (r.len() as u64).into());
        attributes.insert("glyphs".into(), u64::from(glyphs).into());
        let ink: u64 = r.iter().map(|w| w.ink).sum();
        kept.push((
            ink,
            Finding {
                kind: Kind::Handwriting,
                confidence,
                bbox: Some(layout.rect_to_page(&bbox)),
                polygon: None,
                attributes,
                evidence,
            },
        ));
    }
    // Largest by ink first; ties keep reading order (the sort is stable).
    kept.sort_by_key(|(ink, _)| std::cmp::Reverse(*ink));
    kept.truncate(p.max_regions);
    kept.sort_by_key(|(_, f)| f.bbox.map(|b| (b[1], b[0])));
    det.findings = kept.into_iter().map(|(_, f)| f).collect();
    Ok(det)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pixel::layout;

    struct Lcg(u32);
    impl Lcg {
        fn next(&mut self, bound: u32) -> u32 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (self.0 >> 16) % bound
        }
    }

    fn plot(g: &mut [u8], w: usize, x: i64, y: i64, pen: i64) {
        for dy in 0..pen {
            for dx in 0..pen {
                let (px, py) = (x + dx, y + dy);
                if px >= 0
                    && py >= 0
                    && (px as usize) < w
                    && (py as usize) * w + (px as usize) < g.len()
                {
                    g[py as usize * w + px as usize] = 20;
                }
            }
        }
    }

    fn line(g: &mut [u8], w: usize, (x0, y0): (i64, i64), (x1, y1): (i64, i64), pen: i64) {
        let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
        for s in 0..=steps {
            plot(
                g,
                w,
                x0 + (x1 - x0) * s / steps,
                y0 + (y1 - y0) * s / steps,
                pen,
            );
        }
    }

    /// Printed-looking text: separate stems on one baseline, `words` words per line.
    fn print_lines(g: &mut [u8], w: usize, y0: usize, lines: usize, words: usize) {
        for l in 0..lines {
            let y = (y0 + l * 24) as i64;
            let mut x = 20i64;
            for _ in 0..words {
                for _ in 0..4 {
                    line(g, w, (x, y), (x, y + 9), 2);
                    x += 6;
                }
                x += 6;
            }
        }
    }

    /// Looping cursive: round letters of changing size and slant on a wandering baseline, the pen
    /// pressing harder on some letters, each letter joined to the last.
    fn scribble(g: &mut [u8], w: usize, rng: &mut Lcg, x0: i64, base: i64, letters: usize) -> i64 {
        let (mut x, mut wander, mut prev) = (x0 as f64, 0i64, None::<(i64, i64)>);
        for _ in 0..letters {
            wander = (wander + i64::from(rng.next(3)) - 1).clamp(-2, 2);
            let cw = 4.0 + f64::from(rng.next(4));
            let h = 5.0 + f64::from(rng.next(6));
            let slant = 0.2 + f64::from(rng.next(3)) * 0.1;
            let pen = 1 + i64::from(rng.next(2));
            let turns = 1.0 + f64::from(rng.next(3)) * 0.25;
            let steps = 18;
            for k in 0..=steps {
                let t = 2.0 * std::f64::consts::PI * turns * f64::from(k) / f64::from(steps);
                let px = x + cw * (0.5 + 0.5 * math::sin(t));
                let py = (base + wander) as f64 - h * (0.5 - 0.5 * math::cos(t));
                let p = (
                    (px + slant * ((base + wander) as f64 - py)).round() as i64,
                    py.round() as i64,
                );
                if let Some(q) = prev {
                    line(g, w, q, p, pen);
                }
                prev = Some(p);
            }
            x += cw * 0.9;
        }
        x as i64
    }

    /// The test scribbles are weaker than the cursive the thresholds were set on, so these tests run
    /// at an emission threshold of 0.5 and check the mechanism (words, regions, boxes); where the
    /// threshold should sit is a matter for `bench/handwriting_synth_eval.py`.
    fn run(g: &[u8], w: usize, h: usize) -> (Detection, Vec<Word>) {
        let p = Params::with_overrides("[handwriting]\nemit_min_probability = 0.5").unwrap();
        let l = layout::segment(g, w, h, 0.0, &p).unwrap();
        (detect(&l, &p).unwrap(), words(&l, &p.handwriting))
    }

    #[test]
    fn printed_lines_are_not_handwriting() {
        let (w, h) = (260usize, 200usize);
        let mut g = vec![240u8; w * h];
        print_lines(&mut g, w, 20, 6, 5);
        let (d, ws) = run(&g, w, h);
        assert!(d.findings.is_empty());
        assert_eq!(d.features["hand_words"], 0.0);
        assert!(ws.iter().all(|x| x.probability < 0.1), "{ws:?}");
    }

    #[test]
    fn a_scribbled_line_among_print_is_one_region_with_page_coordinates() {
        let (w, h) = (300usize, 200usize);
        let mut g = vec![240u8; w * h];
        print_lines(&mut g, w, 10, 3, 6);
        let mut rng = Lcg(5);
        let mut x = 20;
        for _ in 0..3 {
            x = scribble(&mut g, w, &mut rng, x, 130, 9) + 8;
        }
        print_lines(&mut g, w, 150, 2, 6);
        let (d, ws) = run(&g, w, h);
        let dbg: Vec<_> = ws
            .iter()
            .filter(|x| x.bbox.y0 > 100 && x.bbox.y0 < 145)
            .map(|x| {
                (
                    x.bbox,
                    x.glyphs,
                    x.baseline_dev,
                    x.axis_share,
                    x.stroke_cv,
                    x.height_cv,
                    x.probability,
                )
            })
            .collect();
        assert_eq!(d.findings.len(), 1, "{:?}\n{dbg:#?}", d.features);
        let f = &d.findings[0];
        assert_eq!(f.kind, Kind::Handwriting);
        let b = f.bbox.unwrap();
        assert!(
            b[1] >= 110 && b[3] <= 145,
            "bbox {b:?} should hug the scribble row"
        );
        assert!(f.confidence >= 0.5);
        assert_eq!(f.evidence.len(), 3);
    }

    #[test]
    fn handwriting_across_a_printed_rule_is_measured_as_words_not_fused_to_the_rule() {
        let (w, h) = (300usize, 120usize);
        let mut g = vec![240u8; w * h];
        for x in 10..290 {
            plot(&mut g, w, x, 62, 1);
        }
        let mut rng = Lcg(9);
        let mut x = 20;
        for _ in 0..3 {
            x = scribble(&mut g, w, &mut rng, x, 66, 9) + 8;
        }
        print_lines(&mut g, w, 10, 2, 6);
        let p = Params::defaults().unwrap();
        let l = layout::segment(&g, w, h, 0.0, &p).unwrap();
        assert!(!l.rules.is_empty(), "the 280-px rule is set aside");
        let near: Vec<_> = words(&l, &p.handwriting)
            .into_iter()
            .filter(|x| x.bbox.y0 > 40)
            .collect();
        assert!(near.len() >= 2, "{near:?}");
        // Fused to the rule, a word would be 280 px wide.
        assert!(near.iter().all(|x| x.bbox.width() < 150), "{near:?}");
    }

    #[test]
    fn empty_pages_and_bad_params_are_handled() {
        let p = Params::defaults().unwrap();
        let l = layout::segment(&[240u8; 32 * 32], 32, 32, 0.0, &p).unwrap();
        let d = detect(&l, &p).unwrap();
        assert!(d.findings.is_empty());
        assert_eq!(d.features["words"], 0.0);
        let mut hp = p.handwriting.clone();
        assert!(hp.validate().is_ok());
        hp.emit_min_probability = 1.5;
        assert!(hp.validate().is_err());
        let mut hp = p.handwriting.clone();
        hp.stroke_cv_coef = f64::NAN;
        assert!(hp.validate().is_err());
    }

    #[test]
    fn detection_is_repeatable() {
        let (w, h) = (300usize, 120usize);
        let mut g = vec![240u8; w * h];
        let mut rng = Lcg(3);
        scribble(&mut g, w, &mut rng, 20, 60, 8);
        let (a, _) = run(&g, w, h);
        let (b, _) = run(&g, w, h);
        assert_eq!(a.features, b.features);
        assert_eq!(a.findings, b.findings);
    }
}
