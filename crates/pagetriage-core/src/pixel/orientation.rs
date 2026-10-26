//! Page orientation: which way the text runs and which way is up. See `docs/detectors/orientation.md`.
//!
//! It answers "by how many degrees clockwise must this image be turned to read it" with 0, 90, 180 or
//! 270, from the ink profile alone (no layout, so it works when the page is on its side and the
//! layout finds nothing). A page that is not text, or whose text is too mixed, is reported with a low
//! confidence and no finding.

use serde::Deserialize;
use serde_json::Map;

use super::Detection;
use super::binarize::local;
use crate::error::{Error, Result};
use crate::math;
use crate::params::{FusionParams, Params, SauvolaParams};
use crate::schema::{Evidence, Finding, Kind};

pub const ORIENTATION: &str = "orientation";
const DETECTOR_ID: &str = "orientation.profile";
const CITATION: &str = "text lines swing an ink profile across lines but not along them; Latin lines carry more ink above their core than below (ascenders against descenders) and headline scripts a sharp row at the top; hand set";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrientationParams {
    pub max_tilt_deg: f64,
    pub tilt_step_deg: f64,
    pub strip_in: f64,
    pub min_strip_ink: f64,
    pub smooth_in: f64,
    pub band_share: f64,
    pub min_band_in: f64,
    pub max_band_in: f64,
    pub merge_gap_in: f64,
    pub core_share: f64,
    pub headline_sharpness: f64,
    pub headline_edge: f64,
    pub axis_coef: f64,
    pub axis_offset: f64,
    pub up_coef: f64,
    pub up_ref: f64,
    pub term_clip: f64,
}

impl OrientationParams {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParams(format!("orientation: {m}")));
        let all = [
            self.max_tilt_deg,
            self.tilt_step_deg,
            self.strip_in,
            self.min_strip_ink,
            self.smooth_in,
            self.band_share,
            self.min_band_in,
            self.max_band_in,
            self.merge_gap_in,
            self.core_share,
            self.headline_sharpness,
            self.headline_edge,
            self.axis_coef,
            self.axis_offset,
            self.up_coef,
            self.up_ref,
            self.term_clip,
        ];
        if all.iter().any(|v| !v.is_finite() || v.abs() > 1.0e3) {
            return bad("every value must be finite and within 1e3");
        }
        if self.strip_in <= 0.0
            || self.smooth_in < 0.0
            || self.merge_gap_in < 0.0
            || self.term_clip <= 0.0
        {
            return bad("strip_in > 0, smooth_in >= 0, merge_gap_in >= 0, term_clip > 0");
        }
        for (name, v) in [
            ("min_strip_ink", self.min_strip_ink),
            ("band_share", self.band_share),
            ("core_share", self.core_share),
            ("headline_edge", self.headline_edge),
        ] {
            if !(v > 0.0 && v <= 1.0) {
                return bad(&format!("{name} must be in (0, 1]"));
            }
        }
        if self.min_band_in <= 0.0
            || self.max_band_in <= self.min_band_in
            || self.headline_sharpness < 1.0
        {
            return bad("0 < min_band_in < max_band_in and headline_sharpness >= 1");
        }
        Ok(())
    }
}

/// What the profile reading found for one reading direction.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Reading {
    /// Swing of the profile across lines, averaged over strips, weighted by ink.
    pub strength: f64,
    /// Positive: upright for this direction; negative: upside down.
    pub up: f64,
    /// The page is read by its headline row (set when the bands are sharp and the sharp row sits at
    /// an edge of the band).
    pub headline: bool,
    /// Share of the band ink whose bands point the same way as the page's up score (0.5 is a coin flip).
    pub agreement: f64,
    /// The up score over its standard error across bands (ink-weighted): how many standard errors from
    /// a coin flip. Bands from neighbouring strips overlap, so this overstates the certainty.
    pub up_z: f64,
    pub bands: usize,
    /// Ink-weighted mean sharpness (strongest row over the band's mean row) and mean position of
    /// that row in the band (0 top, 1 bottom): for tuning the headline cue.
    pub sharpness: f64,
    pub peak_at: f64,
}

/// The row shift of every column that best levels text lines tilted up to `max_tilt_deg`: the angle
/// (searched in `tilt_step_deg` steps, then refined by half a step) that makes the row profile of
/// the whole page swing most, which is when the lines lie along rows.
pub fn level(mask: &[u8], w: usize, h: usize, p: &OrientationParams) -> Vec<i32> {
    let ink: Vec<(u32, u32)> = (0..h)
        .flat_map(|y| {
            (0..w)
                .filter(move |&x| mask[y * w + x] != 0)
                .map(move |x| (x as u32, y as u32))
        })
        .collect();
    let offsets = |deg: f64| -> Vec<i32> {
        let r = deg * std::f64::consts::PI / 180.0;
        let slope = math::sin(r) / math::cos(r);
        (0..w).map(|x| (x as f64 * slope).round() as i32).collect()
    };
    let score = |off: &[i32]| -> u64 {
        let lo = off.iter().copied().min().unwrap_or(0);
        let hi = off.iter().copied().max().unwrap_or(0);
        let base = -lo.min(0);
        let mut prof = vec![0u32; h + (base + hi.max(0)) as usize + 1];
        for &(x, y) in &ink {
            prof[(y as i32 + off[x as usize] + base) as usize] += 1;
        }
        prof.iter().map(|&c| u64::from(c) * u64::from(c)).sum()
    };
    let steps = (p.max_tilt_deg / p.tilt_step_deg).floor() as i32;
    let mut best = (score(&offsets(0.0)), 0.0f64);
    for k in -steps..=steps {
        let deg = f64::from(k) * p.tilt_step_deg;
        let sc = score(&offsets(deg));
        if sc > best.0 {
            best = (sc, deg);
        }
    }
    for deg in [
        best.1 - p.tilt_step_deg / 2.0,
        best.1 + p.tilt_step_deg / 2.0,
    ] {
        if deg.abs() <= p.max_tilt_deg {
            let sc = score(&offsets(deg));
            if sc > best.0 {
                best = (sc, deg);
            }
        }
    }
    offsets(best.1)
}

/// The mask with every column moved by `off` rows (the page levelled), on the same size canvas.
fn shifted(mask: &[u8], w: usize, h: usize, off: &[i32]) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let ny = y as i32 + off[x];
            if mask[y * w + x] != 0 && ny >= 0 && (ny as usize) < h {
                out[ny as usize * w + x] = 1;
            }
        }
    }
    out
}

/// Read a mask whose text is supposed to run along rows. The mask is levelled first.
pub fn read_rows(mask: &[u8], w: usize, h: usize, dpi: u16, p: &OrientationParams) -> Reading {
    let off = level(mask, w, h, p);
    let levelled = shifted(mask, w, h, &off);
    read_levelled(&levelled, w, h, dpi, p)
}

fn read_levelled(mask: &[u8], w: usize, h: usize, dpi: u16, p: &OrientationParams) -> Reading {
    let px = |inches: f64| ((inches * f64::from(dpi)).round() as usize).max(1);
    let strip = px(p.strip_in).max(16);
    let smooth = px(p.smooth_in);
    let (min_band, max_band, gap) = (px(p.min_band_in), px(p.max_band_in), px(p.merge_gap_in));
    let mut strengths: Vec<(f64, f64)> = Vec::new(); // (value, weight)
    let (mut asym_sum, mut asym_w) = (Vec::new(), Vec::new());
    let (mut sharp_sum, mut at_sum) = (Vec::new(), Vec::new());
    let mut all_w = Vec::new();
    // Per band: weight, which side of the band its strongest row is on (+1 top half), and the sign of its asymmetry.
    let mut band_votes: Vec<(f64, f64, f64)> = Vec::new();
    let mut bands = 0usize;
    let mut x0 = 0;
    while x0 < w {
        let (start, x1) = (x0, (x0 + strip).min(w));
        if x1 - start < strip / 2 && start > 0 {
            break;
        }
        // Row profile of this strip, smoothed.
        let raw: Vec<f64> = (0..h)
            .map(|y| {
                mask[y * w + x0..y * w + x1]
                    .iter()
                    .map(|&v| f64::from(v))
                    .sum::<f64>()
            })
            .collect();
        let ink = math::pairwise_sum(&raw);
        x0 = x1;
        if ink < p.min_strip_ink * ((x1 - start) * h) as f64 || ink < 8.0 {
            continue;
        }
        let prof = box_smooth(&raw, smooth);
        let mean = math::pairwise_sum(&prof) / h as f64;
        let var = math::pairwise_sum(
            &prof
                .iter()
                .map(|v| (v - mean) * (v - mean))
                .collect::<Vec<_>>(),
        ) / h as f64;
        if mean > 0.0 {
            strengths.push((var / (mean * mean), ink));
        }
        // Bands of rows.
        let peak = prof.iter().copied().fold(0.0, f64::max);
        if peak <= 0.0 {
            continue;
        }
        let thr = p.band_share * peak;
        let mut runs: Vec<(usize, usize)> = Vec::new();
        let mut y = 0;
        while y < h {
            if prof[y] >= thr {
                let s = y;
                while y < h && prof[y] >= thr {
                    y += 1;
                }
                runs.push((s, y));
            } else {
                y += 1;
            }
        }
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for r in runs {
            match merged.last_mut() {
                Some(last) if r.0 - last.1 <= gap => last.1 = r.1,
                _ => merged.push(r),
            }
        }
        for (r0, r1) in merged {
            let height = r1 - r0;
            if height < min_band || height > max_band {
                continue;
            }
            let band = &prof[r0..r1];
            let total = math::pairwise_sum(band);
            let bmax = band.iter().copied().fold(0.0, f64::max);
            if total <= 0.0 || bmax <= 0.0 {
                continue;
            }
            let core_top = band
                .iter()
                .position(|&v| v >= p.core_share * bmax)
                .unwrap_or(0);
            let core_bot = band
                .iter()
                .rposition(|&v| v >= p.core_share * bmax)
                .unwrap_or(height - 1);
            let above = math::pairwise_sum(&band[..core_top]);
            let below = math::pairwise_sum(&band[core_bot + 1..]);
            bands += 1;
            // Relative to the ink outside the core: ascenders against descenders, not against the whole line.
            let asym_band = if above + below > 0.0 {
                (above - below) / (above + below)
            } else {
                0.0
            };
            if above + below > 0.0 {
                asym_sum.push(total * asym_band);
                asym_w.push(total);
            }
            // Where the strongest row sits in the band, and how sharp it is.
            let peak_at = band.iter().position(|&v| v == bmax).unwrap_or(0) as f64 / height as f64;
            let sharp = bmax / (total / height as f64);
            sharp_sum.push(total * sharp);
            at_sum.push(total * peak_at);
            band_votes.push((total, 0.5 - peak_at, asym_band));
            all_w.push(total);
        }
    }
    let wsum = |v: &[f64]| math::pairwise_sum(v);
    let weight: f64 = wsum(&strengths.iter().map(|s| s.1).collect::<Vec<_>>());
    let strength = if weight > 0.0 {
        wsum(&strengths.iter().map(|s| s.0 * s.1).collect::<Vec<_>>()) / weight
    } else {
        0.0
    };
    let total_w = wsum(&all_w);
    if total_w <= 0.0 {
        return Reading {
            strength,
            ..Reading::default()
        };
    }
    let sharpness = wsum(&sharp_sum) / total_w;
    let peak_at = wsum(&at_sum) / total_w;
    let asym_weight = wsum(&asym_w);
    let asym = if asym_weight > 0.0 {
        wsum(&asym_sum) / asym_weight
    } else {
        0.0
    };
    // A headline script (one sharp row of ink along the top of every line) is read by where that row
    // sits; everything else by ascenders against descenders.
    let headline = sharpness >= p.headline_sharpness
        && (peak_at <= p.headline_edge || peak_at >= 1.0 - p.headline_edge);
    let up = if headline { 1.0 - 2.0 * peak_at } else { asym };
    // The up score against its standard error over the bands (weights are the bands' ink).
    let votes: Vec<(f64, f64)> = band_votes
        .iter()
        .map(|&(w, top, a)| (w, if headline { 2.0 * top } else { a }))
        .collect();
    let vw = wsum(&votes.iter().map(|v| v.0).collect::<Vec<_>>());
    let up_z = if vw > 0.0 && votes.len() > 1 {
        let mean = wsum(&votes.iter().map(|v| v.0 * v.1).collect::<Vec<_>>()) / vw;
        let var = wsum(
            &votes
                .iter()
                .map(|v| v.0 * (v.1 - mean) * (v.1 - mean))
                .collect::<Vec<_>>(),
        ) / vw;
        let n_eff = vw * vw / wsum(&votes.iter().map(|v| v.0 * v.0).collect::<Vec<_>>());
        if var > 1e-12 {
            mean / (var / n_eff).sqrt()
        } else {
            0.0
        }
    } else {
        0.0
    };
    // How consistent the bands are: the share of ink in bands that vote the same way as the page.
    let voters: Vec<(f64, f64)> = band_votes
        .iter()
        .map(|&(w, top, a)| (w, if headline { top } else { a }))
        .filter(|v| v.1 != 0.0)
        .collect();
    let voted = wsum(&voters.iter().map(|v| v.0).collect::<Vec<_>>());
    let agreement = if voted > 0.0 {
        let same = wsum(
            &voters
                .iter()
                .filter(|v| (v.1 > 0.0) == (up > 0.0))
                .map(|v| v.0)
                .collect::<Vec<_>>(),
        );
        same / voted
    } else {
        0.5
    };
    Reading {
        strength,
        up,
        headline,
        agreement,
        up_z,
        bands,
        sharpness,
        peak_at,
    }
}

fn box_smooth(v: &[f64], width: usize) -> Vec<f64> {
    if width <= 1 {
        return v.to_vec();
    }
    let half = width / 2;
    let n = v.len();
    let mut prefix = vec![0.0f64; n + 1];
    for i in 0..n {
        prefix[i + 1] = prefix[i] + v[i];
    }
    (0..n)
        .map(|i| {
            let (a, b) = (i.saturating_sub(half), (i + half + 1).min(n));
            (prefix[b] - prefix[a]) / (b - a) as f64
        })
        .collect()
}

/// The mask turned a quarter clockwise: the new row `x` is the old column `x` read bottom to top.
pub fn rotate_cw(mask: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            out[x * h + (h - 1 - y)] = mask[y * w + x];
        }
    }
    out
}

/// The estimate: degrees clockwise to turn the image to make its text upright, with the readings.
#[derive(Debug, Clone, PartialEq)]
pub struct Estimate {
    pub rotation_deg: u16,
    pub axis_ratio: f64,
    pub up: f64,
    pub headline: bool,
    pub agreement: f64,
    pub up_z: f64,
    pub bands: usize,
    pub sharpness: f64,
    pub peak_at: f64,
}

pub fn estimate(mask: &[u8], w: usize, h: usize, dpi: u16, p: &OrientationParams) -> Estimate {
    let rows = read_rows(mask, w, h, dpi, p);
    // Text running down the page is read along rows after a quarter turn.
    let turned = rotate_cw(mask, w, h);
    let cols = read_rows(&turned, h, w, dpi, p);
    let ratio = (rows.strength.max(1e-9) / cols.strength.max(1e-9)).clamp(1e-3, 1e3);
    let horizontal = ratio >= 1.0;
    // `cols` is the reading of the page turned 90 degrees clockwise: upright there means 90 is right.
    let (reading, upright_rotation, flipped_rotation) = if horizontal {
        (rows, 0, 180)
    } else {
        (cols, 90, 270)
    };
    let rotation_deg = if reading.up >= 0.0 {
        upright_rotation
    } else {
        flipped_rotation
    };
    Estimate {
        rotation_deg,
        axis_ratio: ratio,
        up: reading.up,
        headline: reading.headline,
        agreement: reading.agreement,
        up_z: reading.up_z,
        bands: reading.bands,
        sharpness: reading.sharpness,
        peak_at: reading.peak_at,
    }
}

pub fn detect(
    gray: &[u8],
    width: usize,
    height: usize,
    dpi: u16,
    params: &Params,
) -> Result<Detection> {
    detect_with(
        gray,
        width,
        height,
        dpi,
        &params.orientation,
        &params.sauvola,
        &params.fusion,
    )
}

pub fn detect_with(
    gray: &[u8],
    width: usize,
    height: usize,
    dpi: u16,
    p: &OrientationParams,
    sauvola: &SauvolaParams,
    fusion: &FusionParams,
) -> Result<Detection> {
    if width == 0 || height == 0 || gray.len() != width * height {
        return Err(Error::InvalidInput(format!(
            "gray buffer of {} bytes does not match {width}x{height}",
            gray.len()
        )));
    }
    let mask = local::sauvola(gray, width, height, sauvola);
    let e = estimate(&mask, width, height, dpi, p);
    let mut det = Detection::default();
    // The rotation is right when the lines are clearly along one axis AND the up/down cue is clearly one
    // way, so the two confidences multiply (each is a logistic of its own evidence).
    let axis_llr = (p.axis_coef * math::ln(e.axis_ratio).abs() - p.axis_offset)
        .clamp(-p.term_clip, p.term_clip);
    let up_llr = (p.up_coef * (e.up_z.abs() - p.up_ref)).clamp(-p.term_clip, p.term_clip);
    let evidence = vec![
        Evidence::new(
            DETECTOR_ID,
            "line_axis_ratio",
            math::round_output(e.axis_ratio),
            None,
            axis_llr,
            "ink-profile swing along rows over along columns (or the reverse for text on its side)"
                .into(),
            CITATION,
        )?,
        Evidence::new(
            DETECTOR_ID,
            "up_z",
            math::round_output(e.up_z),
            Some(p.up_ref),
            up_llr,
            "standard errors by which the text lines lean one way up: more ink above their core than below (ascenders over descenders), or the headline row at the top"
                .into(),
            CITATION,
        )?,
    ];
    let confidence = math::round_output(math::sigmoid(axis_llr) * math::sigmoid(up_llr));
    det.features
        .insert("rotation_deg".into(), f64::from(e.rotation_deg));
    det.features.insert("confidence".into(), confidence);
    det.features.insert("axis_ratio".into(), e.axis_ratio);
    det.features.insert("up_score".into(), e.up);
    det.features
        .insert("headline".into(), f64::from(u8::from(e.headline)));
    det.features.insert("agreement".into(), e.agreement);
    det.features.insert("up_z".into(), e.up_z);
    det.features.insert("bands".into(), e.bands as f64);
    det.features.insert("mean_sharpness".into(), e.sharpness);
    det.features.insert("mean_peak_at".into(), e.peak_at);
    if e.rotation_deg != 0 && confidence >= fusion.emit_min_confidence {
        let mut attributes = Map::new();
        attributes.insert(
            "rotate_clockwise_deg".into(),
            u64::from(e.rotation_deg).into(),
        );
        det.findings.push(Finding {
            kind: Kind::Misoriented,
            confidence,
            bbox: None,
            polygon: None,
            attributes,
            evidence,
        });
    }
    Ok(det)
}
