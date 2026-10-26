//! Photographs and other continuous-tone pictures (module M10, region typing). See
//! `docs/detectors/photo.md`.

use serde::Deserialize;
use serde_json::Map;

use super::Detection;
use crate::error::{Error, Result};
use crate::fusion::scorer;
use crate::math;
use crate::params::Params;
use crate::schema::{Evidence, Finding, Kind};

pub const PHOTO: &str = "photo";
const DETECTOR_ID: &str = "photo.continuous_tone";
const CITATION: &str = "module M10: a photograph is an area where much of the page is mid-gray, which text, rules and line art are not; hand set";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhotoParams {
    pub tile_in: f64,
    pub paper_band: u8,
    pub ink_depth: u8,
    pub flat_gradient: f64,
    pub min_mid_share: f64,
    pub close_tiles: usize,
    pub min_area_sq_in: f64,
    pub min_fill: f64,
    pub max_regions: usize,
    pub term_clip: f64,
    pub prior: f64,
    pub area_threshold: f64,
    pub area_coef: f64,
    pub mid_threshold: f64,
    pub mid_coef: f64,
}

impl PhotoParams {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParams(format!("photo: {m}")));
        let all = [
            self.tile_in,
            self.flat_gradient,
            self.min_mid_share,
            self.min_area_sq_in,
            self.min_fill,
            self.term_clip,
            self.prior,
            self.area_threshold,
            self.area_coef,
            self.mid_threshold,
            self.mid_coef,
        ];
        if all.iter().any(|v| !v.is_finite() || v.abs() > 1.0e3) {
            return bad("every value must be finite and within 1e3");
        }
        if !(self.tile_in > 0.0 && self.tile_in <= 2.0) {
            return bad("tile_in must be in (0, 2]");
        }
        if self.paper_band >= self.ink_depth {
            return bad("paper_band must be below ink_depth");
        }
        if !(self.min_mid_share > 0.0 && self.min_mid_share <= 1.0)
            || !(self.min_fill > 0.0 && self.min_fill <= 1.0)
        {
            return bad("min_mid_share and min_fill must be in (0, 1]");
        }
        if self.close_tiles > 4
            || self.max_regions == 0
            || self.term_clip <= 0.0
            || self.min_area_sq_in < 0.0
        {
            return bad("close_tiles <= 4, max_regions >= 1, term_clip > 0, min_area_sq_in >= 0");
        }
        Ok(())
    }
}

/// A picture region: box in page pixels and what was measured.
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub bbox: [u32; 4],
    pub tiles: usize,
    pub fill: f64,
    pub mid_share: f64,
    pub area_sq_in: f64,
}

pub fn regions(gray: &[u8], width: usize, height: usize, dpi: u16, p: &PhotoParams) -> Vec<Region> {
    let tile = ((p.tile_in * f64::from(dpi)).round() as usize).max(4);
    let (tw, th) = (width / tile, height / tile);
    if tw == 0 || th == 0 {
        return Vec::new();
    }
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
    let paper_lo = paper.saturating_sub(usize::from(p.paper_band));
    let ink_hi = paper.saturating_sub(usize::from(p.ink_depth));
    // Flat mid-gray: a mid-gray pixel whose neighbourhood is smooth. The edges of blurred text are
    // mid-gray too, but steep; the inside of a picture is mid-gray and gentle.
    let smooth = box3(gray, width, height);
    let flat_limit = (p.flat_gradient * 9.0).round() as i32; // the smoothed values are sums of 9
    // Mid-gray share per tile.
    let mut mid = vec![0.0f64; tw * th];
    for ty in 0..th {
        for tx in 0..tw {
            let mut n = 0usize;
            for y in ty * tile..(ty + 1) * tile {
                for x in tx * tile..(tx + 1) * tile {
                    let v = usize::from(gray[y * width + x]);
                    if v < paper_lo
                        && v > ink_hi
                        && gentle(&smooth, width, height, x, y, flat_limit)
                    {
                        n += 1;
                    }
                }
            }
            mid[ty * tw + tx] = n as f64 / (tile * tile) as f64;
        }
    }
    let tone: Vec<bool> = mid.iter().map(|&m| m >= p.min_mid_share).collect();
    // Closing: dilate then erode by close_tiles (square), so skies and shadows inside a picture join.
    let closed = erode(&dilate(&tone, tw, th, p.close_tiles), tw, th, p.close_tiles);
    // 4-connected components.
    let mut label = vec![usize::MAX; tw * th];
    let mut out = Vec::new();
    for start in 0..tw * th {
        if !closed[start] || label[start] != usize::MAX {
            continue;
        }
        let id = out.len();
        let mut stack = vec![start];
        label[start] = id;
        let (mut x0, mut y0, mut x1, mut y1) = (tw, th, 0, 0);
        let (mut count, mut tone_count, mut mid_sum) = (0usize, 0usize, Vec::new());
        while let Some(i) = stack.pop() {
            let (x, y) = (i % tw, i / tw);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            count += 1;
            if tone[i] {
                tone_count += 1;
                mid_sum.push(mid[i]);
            }
            for (nx, ny) in [
                (x.wrapping_sub(1), y),
                (x + 1, y),
                (x, y.wrapping_sub(1)),
                (x, y + 1),
            ] {
                if nx < tw && ny < th {
                    let j = ny * tw + nx;
                    if closed[j] && label[j] == usize::MAX {
                        label[j] = id;
                        stack.push(j);
                    }
                }
            }
        }
        let box_tiles = (x1 - x0 + 1) * (y1 - y0 + 1);
        let fill = count as f64 / box_tiles as f64;
        let area = count as f64 * (tile * tile) as f64 / (f64::from(dpi) * f64::from(dpi));
        let mid_share = if tone_count == 0 {
            0.0
        } else {
            math::pairwise_sum(&mid_sum) / tone_count as f64
        };
        out.push(Region {
            bbox: [
                (x0 * tile) as u32,
                (y0 * tile) as u32,
                (((x1 + 1) * tile).min(width) - 1) as u32,
                (((y1 + 1) * tile).min(height) - 1) as u32,
            ],
            tiles: count,
            fill,
            mid_share,
            area_sq_in: area,
        });
    }
    out.retain(|r| r.area_sq_in >= p.min_area_sq_in && r.fill >= p.min_fill);
    out.sort_by(|a, b| {
        b.area_sq_in
            .total_cmp(&a.area_sq_in)
            .then(a.bbox.cmp(&b.bbox))
    });
    out
}

/// 3 x 3 box sums of the gray page (nine times the mean), the border replicated.
fn box3(gray: &[u8], w: usize, h: usize) -> Vec<i32> {
    let mut out = vec![0i32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut s = 0i32;
            for dy in [y.saturating_sub(1), y, (y + 1).min(h - 1)] {
                for dx in [x.saturating_sub(1), x, (x + 1).min(w - 1)] {
                    s += i32::from(gray[dy * w + dx]);
                }
            }
            out[y * w + x] = s;
        }
    }
    out
}

/// Is the smoothed page's gradient at (x, y) at most `limit` (in the same nine-fold units)?
fn gentle(s: &[i32], w: usize, h: usize, x: usize, y: usize, limit: i32) -> bool {
    let at = |xx: usize, yy: usize| s[yy * w + xx];
    let gx = at((x + 2).min(w - 1), y) - at(x.saturating_sub(2), y);
    let gy = at(x, (y + 2).min(h - 1)) - at(x, y.saturating_sub(2));
    gx.abs() + gy.abs() <= 4 * limit
}

fn dilate(m: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    let mut out = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            'a: for dy in y.saturating_sub(r)..=(y + r).min(h - 1) {
                for dx in x.saturating_sub(r)..=(x + r).min(w - 1) {
                    if m[dy * w + dx] {
                        out[y * w + x] = true;
                        break 'a;
                    }
                }
            }
        }
    }
    out
}

fn erode(m: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    let mut out = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            // Outside the page counts as set, so a picture at the page edge is not eaten.
            let mut all = true;
            'a: for dy in y.saturating_sub(r)..=(y + r).min(h - 1) {
                for dx in x.saturating_sub(r)..=(x + r).min(w - 1) {
                    if !m[dy * w + dx] {
                        all = false;
                        break 'a;
                    }
                }
            }
            out[y * w + x] = all;
        }
    }
    out
}

fn term(value: f64, threshold: f64, coef: f64, clip: f64) -> f64 {
    (coef * (value - threshold)).clamp(-clip, clip)
}

pub fn detect(
    gray: &[u8],
    width: usize,
    height: usize,
    dpi: u16,
    params: &Params,
) -> Result<Detection> {
    if width == 0 || height == 0 || gray.len() != width * height {
        return Err(Error::InvalidInput(format!(
            "gray buffer of {} bytes does not match {width}x{height}",
            gray.len()
        )));
    }
    let p = &params.photo;
    let rs = regions(gray, width, height, dpi, p);
    let mut det = Detection::default();
    det.features.insert("candidates".into(), rs.len() as f64);
    let clip = |v: f64| v.clamp(-params.fusion.llr_clip, params.fusion.llr_clip);
    for r in rs.iter().take(p.max_regions) {
        let evidence = vec![
            Evidence::new(
                DETECTOR_ID,
                "area_sq_in",
                math::round_output(r.area_sq_in),
                Some(p.area_threshold),
                clip(term(
                    r.area_sq_in,
                    p.area_threshold,
                    p.area_coef,
                    p.term_clip,
                )),
                "area of the continuous-tone region in square inches".into(),
                CITATION,
            )?,
            Evidence::new(
                DETECTOR_ID,
                "mid_share",
                math::round_output(r.mid_share),
                Some(p.mid_threshold),
                clip(term(r.mid_share, p.mid_threshold, p.mid_coef, p.term_clip)),
                "share of its pixels that are neither paper nor solid ink".into(),
                CITATION,
            )?,
        ];
        let confidence = math::round_output(scorer::confidence_capped(
            p.prior,
            &evidence,
            params.fusion.group_cap,
        ));
        if confidence < params.fusion.emit_min_confidence {
            continue;
        }
        let mut attributes = Map::new();
        attributes.insert("area_sq_in".into(), math::round_output(r.area_sq_in).into());
        attributes.insert("fill".into(), math::round_output(r.fill).into());
        det.findings.push(Finding {
            kind: Kind::Photo,
            confidence,
            bbox: Some(r.bbox),
            polygon: None,
            attributes,
            evidence,
        });
    }
    det.features
        .insert("photos".into(), det.findings.len() as f64);
    Ok(det)
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 300;
    const H: usize = 300;

    fn page_with_picture() -> Vec<u8> {
        let mut g = vec![245u8; W * H];
        for y in 80..220 {
            for x in 60..240 {
                // A gentle gradient in the middle of the gray range, with a little texture.
                g[y * W + x] = (90 + (x - 60) / 2 + (y - 80) / 3 + ((x * 7 + y * 13) % 5)) as u8;
            }
        }
        g
    }

    #[test]
    fn finds_a_gradient_picture() {
        let params = Params::defaults().unwrap();
        let det = detect(&page_with_picture(), W, H, 100, &params).unwrap();
        assert_eq!(det.findings.len(), 1, "{:?}", det.features);
        let f = &det.findings[0];
        assert_eq!(f.kind, Kind::Photo);
        let b = f.bbox.unwrap();
        for (got, want) in b.iter().zip([60u32, 80, 239, 219]) {
            assert!(got.abs_diff(want) <= 16, "{b:?}");
        }
    }

    #[test]
    fn paper_a_solid_disc_and_a_bilevel_picture_are_not_photos() {
        let params = Params::defaults().unwrap();
        assert!(
            detect(&vec![245u8; W * H], W, H, 100, &params)
                .unwrap()
                .findings
                .is_empty()
        );
        let mut g = vec![245u8; W * H];
        for y in 80..220 {
            for x in 60..240 {
                g[y * W + x] = 30;
            }
        }
        assert!(detect(&g, W, H, 100, &params).unwrap().findings.is_empty());
        // Checkerboard of black and white: no mid-gray at all.
        let mut g = vec![245u8; W * H];
        for y in 80..220 {
            for x in 60..240 {
                g[y * W + x] = if (x / 2 + y / 2) % 2 == 0 { 30 } else { 245 };
            }
        }
        assert!(detect(&g, W, H, 100, &params).unwrap().findings.is_empty());
    }

    #[test]
    fn a_small_picture_is_too_small_and_edges_are_not_flat() {
        let params = Params::defaults().unwrap();
        let mut g = vec![245u8; W * H];
        for y in 100..140 {
            for x in 100..140 {
                g[y * W + x] = 120;
            }
        }
        // 0.4 in square at 100 dpi: 0.16 sq in, under the minimum area.
        assert!(detect(&g, W, H, 100, &params).unwrap().findings.is_empty());
        // Stripes of mid-gray and paper, 4 px wide: steep edges everywhere.
        let mut g = vec![245u8; W * H];
        for y in 60..240 {
            for x in 60..240 {
                g[y * W + x] = if (x / 4) % 2 == 0 { 120 } else { 245 };
            }
        }
        assert!(detect(&g, W, H, 100, &params).unwrap().findings.is_empty());
    }

    #[test]
    fn wrong_buffers_and_validation() {
        let params = Params::defaults().unwrap();
        assert!(detect(&[0u8; 10], 4, 4, 100, &params).is_err());
        assert!(detect(&[], 0, 0, 100, &params).is_err());
        let mut p = params.photo.clone();
        p.validate().unwrap();
        p.paper_band = 200;
        assert!(p.validate().is_err());
        let mut p = params.photo.clone();
        p.min_fill = 0.0;
        assert!(p.validate().is_err());
    }
}
