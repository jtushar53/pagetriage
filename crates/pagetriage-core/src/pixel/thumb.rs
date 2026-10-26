//! Thumb and finger impressions (module M8): compact dark ovals of ridges. See
//! `docs/detectors/thumb.md`.
//!
//! Blocks of 0.1 in are tested for fingerprint ridges: a strong, coherent gray gradient (the
//! structure tensor, after Bazen and Gerez, IEEE TPAMI 24(7), 2002) and a ridge period of about half
//! a millimetre, counted from the zero crossings of the gray along the gradient direction. Touching
//! ridged blocks form regions; a region is an impression when it is the size of a thumb, roughly oval,
//! dark (half ink), and its ridges run in many directions (a ruled area, a hatch or a barcode has one).
//! Everything up to the per-block statistics is integer arithmetic.

use serde::Deserialize;
use serde_json::Map;

use super::Detection;
use super::components;
use crate::error::{Error, Result};
use crate::fusion::scorer;
use crate::math;
use crate::params::{FusionParams, Params};
use crate::schema::{Evidence, Finding, Kind};

pub const THUMB: &str = "thumb";
const DETECTOR_ID: &str = "thumb.ridges";
const CITATION: &str = "module M8: a thumb impression is a dark oval of ridges with coherent, varied orientation (Bazen and Gerez 2002 coherence)";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThumbParams {
    pub block_in: f64,
    pub min_energy: f64,
    pub min_coherence: f64,
    pub min_period_in: f64,
    pub max_period_in: f64,
    pub min_side_in: f64,
    pub max_side_in: f64,
    pub max_aspect: f64,
    pub min_fill: f64,
    pub max_fill: f64,
    pub dark_delta: u8,
    pub min_dark_fraction: f64,
    pub min_orientation_spread: f64,
    pub term_clip: f64,
    pub prior: f64,
    pub coherence_threshold: f64,
    pub coherence_coef: f64,
    pub spread_threshold: f64,
    pub spread_coef: f64,
    pub dark_threshold: f64,
    pub dark_coef: f64,
    pub fill_threshold: f64,
    pub fill_coef: f64,
    pub max_impressions: usize,
}

impl ThumbParams {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParams(format!("thumb: {m}")));
        let all = [
            ("block_in", self.block_in),
            ("min_energy", self.min_energy),
            ("min_coherence", self.min_coherence),
            ("min_period_in", self.min_period_in),
            ("max_period_in", self.max_period_in),
            ("min_side_in", self.min_side_in),
            ("max_side_in", self.max_side_in),
            ("max_aspect", self.max_aspect),
            ("min_fill", self.min_fill),
            ("max_fill", self.max_fill),
            ("min_dark_fraction", self.min_dark_fraction),
            ("min_orientation_spread", self.min_orientation_spread),
            ("term_clip", self.term_clip),
            ("prior", self.prior),
            ("coherence_threshold", self.coherence_threshold),
            ("coherence_coef", self.coherence_coef),
            ("spread_threshold", self.spread_threshold),
            ("spread_coef", self.spread_coef),
            ("dark_threshold", self.dark_threshold),
            ("dark_coef", self.dark_coef),
            ("fill_threshold", self.fill_threshold),
            ("fill_coef", self.fill_coef),
        ];
        for (name, v) in all {
            if !v.is_finite() || v.abs() > 1.0e6 {
                return bad(&format!("{name} must be finite and within 1e6, got {v}"));
            }
        }
        if self.block_in <= 0.0 || self.block_in > 1.0 {
            return bad("block_in must be in (0, 1]");
        }
        if self.min_period_in <= 0.0 || self.max_period_in < self.min_period_in {
            return bad("0 < min_period_in <= max_period_in");
        }
        if self.min_side_in <= 0.0 || self.max_side_in < self.min_side_in || self.max_aspect < 1.0 {
            return bad("0 < min_side_in <= max_side_in and max_aspect >= 1");
        }
        if !(0.0..=1.0).contains(&self.min_fill)
            || !(0.0..=1.0).contains(&self.max_fill)
            || self.min_fill > self.max_fill
            || !(0.0..=1.0).contains(&self.min_coherence)
            || !(0.0..=1.0).contains(&self.min_dark_fraction)
            || !(0.0..=1.0).contains(&self.min_orientation_spread)
        {
            return bad("fractions must be in [0, 1] and min_fill <= max_fill");
        }
        if self.term_clip <= 0.0 || self.max_impressions == 0 {
            return bad("term_clip must be > 0 and max_impressions >= 1");
        }
        Ok(())
    }
}

/// One block's ridge statistics.
#[derive(Debug, Clone, Copy, Default)]
struct Block {
    ridged: bool,
    coherence: f64,
    /// `(cos 2t, sin 2t)` of the gradient direction `t`, for the circular variance.
    doubled: (f64, f64),
}

/// A candidate impression and what was measured on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Impression {
    pub bbox: [u32; 4],
    pub width_in: f64,
    pub height_in: f64,
    pub coherence: f64,
    pub orientation_spread: f64,
    pub dark_fraction: f64,
    pub fill: f64,
    pub blocks: usize,
}

/// Paper level: the most common gray in the page's upper quartile of brightness.
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

/// Find the impressions of a page. `gray` is `width * height`.
pub fn impressions(
    gray: &[u8],
    width: usize,
    height: usize,
    dpi: u16,
    p: &ThumbParams,
) -> Result<Vec<Impression>> {
    if width == 0 || height == 0 || gray.len() != width * height || dpi == 0 {
        return Err(Error::InvalidInput(format!(
            "gray buffer of {} bytes does not match {width}x{height} at {dpi} dpi",
            gray.len()
        )));
    }
    let dpi_f = f64::from(dpi);
    let side = ((p.block_in * dpi_f).round() as usize).max(4);
    let (bw, bh) = (width / side, height / side);
    if bw < 2 || bh < 2 {
        return Ok(Vec::new());
    }
    // Sobel gradients of the interior.
    let at = |x: usize, y: usize| i32::from(gray[y * width + x]);
    let mut blocks = vec![Block::default(); bw * bh];
    let (min_p, max_p) = (p.min_period_in * dpi_f, p.max_period_in * dpi_f);
    for by in 0..bh {
        for bx in 0..bw {
            let (x0, y0) = (bx * side, by * side);
            let (mut gxx, mut gyy, mut gxy) = (0i64, 0i64, 0i64);
            for y in y0.max(1)..(y0 + side).min(height - 1) {
                for x in x0.max(1)..(x0 + side).min(width - 1) {
                    let gx = (at(x + 1, y - 1) + 2 * at(x + 1, y) + at(x + 1, y + 1))
                        - (at(x - 1, y - 1) + 2 * at(x - 1, y) + at(x - 1, y + 1));
                    let gy = (at(x - 1, y + 1) + 2 * at(x, y + 1) + at(x + 1, y + 1))
                        - (at(x - 1, y - 1) + 2 * at(x, y - 1) + at(x + 1, y - 1));
                    gxx += i64::from(gx * gx);
                    gyy += i64::from(gy * gy);
                    gxy += i64::from(gx * gy);
                }
            }
            let energy = (gxx + gyy) as f64 / (side * side) as f64;
            if energy < p.min_energy {
                continue;
            }
            let (a, b) = ((gxx - gyy) as f64, (2 * gxy) as f64);
            let mag = (a * a + b * b).sqrt();
            let coherence = mag / (gxx + gyy) as f64;
            if coherence < p.min_coherence {
                continue;
            }
            // Gradient direction t = 0.5 atan2(2 Gxy, Gxx - Gyy); cos t and sin t via the doubled
            // angle: cos 2t = a / mag, sin 2t = b / mag.
            let (c2, s2) = (a / mag, b / mag);
            let (ct, st) = (
                ((1.0 + c2) / 2.0).sqrt(),
                if s2 >= 0.0 {
                    ((1.0 - c2) / 2.0).sqrt()
                } else {
                    -((1.0 - c2) / 2.0).sqrt()
                },
            );
            // Sample the gray along the gradient direction through the block centre.
            let (cx, cy) = ((x0 + side / 2) as f64, (y0 + side / 2) as f64);
            let len = 2 * side;
            let mut line: Vec<f64> = Vec::with_capacity(len);
            for k in 0..len {
                let t = k as f64 - len as f64 / 2.0;
                let (sx, sy) = (cx + t * ct, cy + t * st);
                let (ix, iy) = (sx.round() as i64, sy.round() as i64);
                if ix < 0 || iy < 0 || ix >= width as i64 || iy >= height as i64 {
                    line.clear();
                    break;
                }
                line.push(f64::from(gray[iy as usize * width + ix as usize]));
            }
            if line.is_empty() {
                continue;
            }
            let mean = math::pairwise_sum(&line) / line.len() as f64;
            let mut crossings = 0usize;
            let mut prev = line[0] - mean;
            for &v in &line[1..] {
                let d = v - mean;
                if (prev < 0.0) != (d < 0.0) && (prev != 0.0 || d != 0.0) {
                    crossings += 1;
                }
                prev = d;
            }
            if crossings < 2 {
                continue;
            }
            let period = 2.0 * len as f64 / crossings as f64;
            if period < min_p || period > max_p {
                continue;
            }
            blocks[by * bw + bx] = Block {
                ridged: true,
                coherence,
                doubled: (c2, s2),
            };
        }
    }

    // Regions of touching ridged blocks.
    let mask: Vec<u8> = blocks.iter().map(|b| u8::from(b.ridged)).collect();
    let lab = components::label_8(&mask, bw, bh);
    let paper = paper_level(gray);
    let dark_limit = paper.saturating_sub(p.dark_delta);
    let mut out = Vec::new();
    for (k, c) in lab.components.iter().enumerate() {
        let (w_in, h_in) = (
            f64::from(c.width()) * side as f64 / dpi_f,
            f64::from(c.height()) * side as f64 / dpi_f,
        );
        let (long, short) = (w_in.max(h_in), w_in.min(h_in));
        if short < p.min_side_in || long > p.max_side_in || long / short.max(1e-9) > p.max_aspect {
            continue;
        }
        let members: Vec<&Block> = lab
            .labels
            .iter()
            .zip(&blocks)
            .filter(|&(&l, _)| l == k as u32 + 1)
            .map(|(_, b)| b)
            .collect();
        let fill = members.len() as f64 / (c.width() * c.height()) as f64;
        if fill < p.min_fill || fill > p.max_fill {
            continue;
        }
        let coherence =
            math::pairwise_sum(&members.iter().map(|b| b.coherence).collect::<Vec<_>>())
                / members.len() as f64;
        let (sc, ss) = (
            math::pairwise_sum(&members.iter().map(|b| b.doubled.0).collect::<Vec<_>>())
                / members.len() as f64,
            math::pairwise_sum(&members.iter().map(|b| b.doubled.1).collect::<Vec<_>>())
                / members.len() as f64,
        );
        let spread = 1.0 - (sc * sc + ss * ss).sqrt();
        if spread < p.min_orientation_spread {
            continue;
        }
        // Dark share of the pixels of the box.
        let (px0, py0) = (c.x0 as usize * side, c.y0 as usize * side);
        let (px1, py1) = (
            ((c.x1 as usize + 1) * side).min(width),
            ((c.y1 as usize + 1) * side).min(height),
        );
        let mut dark = 0u64;
        for y in py0..py1 {
            for x in px0..px1 {
                dark += u64::from(gray[y * width + x] <= dark_limit);
            }
        }
        let dark_fraction = dark as f64 / ((px1 - px0) * (py1 - py0)) as f64;
        if dark_fraction < p.min_dark_fraction {
            continue;
        }
        out.push(Impression {
            bbox: [px0 as u32, py0 as u32, px1 as u32 - 1, py1 as u32 - 1],
            width_in: w_in,
            height_in: h_in,
            coherence,
            orientation_spread: spread,
            dark_fraction,
            fill,
            blocks: members.len(),
        });
    }
    Ok(out)
}

fn term(value: f64, threshold: f64, coef: f64, clip: f64) -> f64 {
    (coef * (value - threshold)).clamp(-clip, clip)
}

/// Run M8 on a page.
pub fn detect(
    gray: &[u8],
    width: usize,
    height: usize,
    dpi: u16,
    params: &Params,
) -> Result<Detection> {
    detect_with(gray, width, height, dpi, &params.thumb, &params.fusion)
}

pub fn detect_with(
    gray: &[u8],
    width: usize,
    height: usize,
    dpi: u16,
    p: &ThumbParams,
    fusion: &FusionParams,
) -> Result<Detection> {
    let imps = impressions(gray, width, height, dpi, p)?;
    let mut det = Detection::default();
    det.features.insert("candidates".into(), imps.len() as f64);
    let mut kept: Vec<(usize, Finding)> = Vec::new();
    for i in &imps {
        let terms = [
            (
                "coherence",
                i.coherence,
                p.coherence_threshold,
                p.coherence_coef,
            ),
            (
                "orientation_spread",
                i.orientation_spread,
                p.spread_threshold,
                p.spread_coef,
            ),
            (
                "dark_fraction",
                i.dark_fraction,
                p.dark_threshold,
                p.dark_coef,
            ),
            ("fill", i.fill, p.fill_threshold, p.fill_coef),
        ];
        let evidence = terms
            .iter()
            .map(|&(name, value, thr, coef)| {
                Evidence::new(
                    DETECTOR_ID,
                    name,
                    value,
                    Some(thr),
                    term(value, thr, coef, p.term_clip).clamp(-fusion.llr_clip, fusion.llr_clip),
                    format!(
                        "{:.2} x {:.2} in, {} ridged blocks",
                        i.width_in, i.height_in, i.blocks
                    ),
                    CITATION,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let confidence = math::round_output(scorer::confidence(p.prior, &evidence));
        if confidence < fusion.emit_min_confidence {
            continue;
        }
        let mut attributes = Map::new();
        attributes.insert("width_in".into(), math::round_output(i.width_in).into());
        attributes.insert("height_in".into(), math::round_output(i.height_in).into());
        attributes.insert("ridged_blocks".into(), (i.blocks as u64).into());
        attributes.insert("coherence".into(), math::round_output(i.coherence).into());
        attributes.insert(
            "orientation_spread".into(),
            math::round_output(i.orientation_spread).into(),
        );
        attributes.insert(
            "dark_fraction".into(),
            math::round_output(i.dark_fraction).into(),
        );
        kept.push((
            i.blocks,
            Finding {
                kind: Kind::ThumbImpression,
                confidence,
                bbox: Some(i.bbox),
                polygon: None,
                attributes,
                evidence,
            },
        ));
    }
    kept.sort_by_key(|(n, _)| std::cmp::Reverse(*n));
    kept.truncate(p.max_impressions);
    kept.sort_by_key(|(_, f)| f.bbox.map(|b| (b[1], b[0])));
    det.findings = kept.into_iter().map(|(_, f)| f).collect();
    det.features
        .insert("impressions".into(), det.findings.len() as f64);
    Ok(det)
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 360;
    const H: usize = 300;

    fn paper() -> Vec<u8> {
        vec![240u8; W * H]
    }

    /// Concentric elliptical ridges with a 4-px period (2 px ink, 2 px paper), about `(cx, cy)`.
    fn print(g: &mut [u8], cx: i64, cy: i64, a: i64, b: i64) {
        for y in cy - b..=cy + b {
            for x in cx - a..=cx + a {
                let (dx, dy) = (x - cx, y - cy);
                // Normalised so the edge of the ellipse is at 1.0 (in thousandths).
                let e2 = 1_000_000 * dx * dx / (a * a) + 1_000_000 * dy * dy / (b * b);
                if e2 <= 1_000_000 {
                    // Distance in pixels from the centre along an ellipse-shaped metric.
                    let r = (((dx * dx * 9 + dy * dy * 4) as f64).sqrt() / 3.0) as i64;
                    if r % 4 < 2 {
                        g[y as usize * W + x as usize] = 50;
                    }
                }
            }
        }
    }

    fn lines(
        g: &mut [u8],
        x0: usize,
        y0: usize,
        x1: usize,
        y1: usize,
        period: usize,
        thick: usize,
    ) {
        let mut y = y0;
        while y + thick <= y1 {
            for dy in 0..thick {
                for x in x0..x1 {
                    g[(y + dy) * W + x] = 50;
                }
            }
            y += period;
        }
    }

    fn run(g: &[u8]) -> Detection {
        let p = Params::defaults().unwrap();
        detect(g, W, H, 150, &p).unwrap()
    }

    #[test]
    fn a_dark_oval_of_ridges_is_an_impression() {
        let mut g = paper();
        print(&mut g, 180, 150, 60, 75);
        let d = run(&g);
        assert_eq!(d.findings.len(), 1, "{:?}", d.features);
        let f = &d.findings[0];
        assert_eq!(f.kind, Kind::ThumbImpression);
        let b = f.bbox.unwrap();
        assert!(
            b[0] >= 100 && b[2] <= 260 && b[1] >= 60 && b[3] <= 240,
            "{b:?}"
        );
        assert_eq!(f.evidence.len(), 4);
    }

    #[test]
    fn hatching_is_one_direction_and_is_not_an_impression() {
        let mut g = paper();
        lines(&mut g, 100, 60, 260, 240, 4, 2);
        let d = run(&g);
        assert!(d.findings.is_empty(), "{:?} {:?}", d.features, d.findings);
    }

    #[test]
    fn a_solid_blob_and_a_blank_page_are_not() {
        let mut g = paper();
        for y in 70..230 {
            for x in 120..240 {
                let (dx, dy) = (x as i64 - 180, y as i64 - 150);
                if dx * dx * 4 / 3600 + dy * dy * 4 / 5625 <= 4 {
                    g[y * W + x] = 50;
                }
            }
        }
        assert!(run(&g).findings.is_empty());
        assert!(run(&paper()).findings.is_empty());
    }

    #[test]
    fn a_ridged_region_too_small_or_too_big_is_not() {
        let mut g = paper();
        print(&mut g, 90, 90, 20, 25);
        assert!(run(&g).findings.is_empty(), "too small");
        let mut g = paper();
        print(&mut g, 180, 150, 170, 140);
        assert!(run(&g).findings.is_empty(), "too big");
    }

    #[test]
    fn bad_input_and_params() {
        let p = Params::defaults().unwrap();
        assert!(detect(&[0; 5], 2, 2, 150, &p).is_err());
        assert!(detect(&paper(), W, H, 0, &p).is_err());
        assert!(
            detect(&[200u8; 9], 3, 3, 150, &p)
                .unwrap()
                .findings
                .is_empty()
        );
        let mut tp = p.thumb.clone();
        assert!(tp.validate().is_ok());
        tp.max_period_in = tp.min_period_in / 2.0;
        assert!(tp.validate().is_err());
        let mut tp = p.thumb.clone();
        tp.min_fill = 1.5;
        assert!(tp.validate().is_err());
    }

    #[test]
    fn detection_is_repeatable() {
        let mut g = paper();
        print(&mut g, 180, 150, 60, 75);
        assert_eq!(run(&g).findings, run(&g).findings);
    }
}
