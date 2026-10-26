//! Rubber stamps and seals (module M6): coloured ink regions with a circular ring or a rectangular
//! border. See `docs/detectors/stamp.md`.
//!
//! The page's coloured ink is found by chroma alone (tinted paper and black text are not coloured),
//! widened so that a ring, its text and its date join, and cut into connected regions. A region of
//! stamp size is scored on how concentrated its hue is, how complete its frame is (the share of 36
//! angle sectors holding ring ink, or the weakest side of the best-tilted rectangle) and how
//! saturated it is. A circle with a second concentric ring is a seal. Without a colour page only
//! circular rings are accepted, at a lower prior, because a bordered paragraph is indistinguishable
//! from a rectangular stamp in grays.

use serde::Deserialize;
use serde_json::Map;

use super::Detection;
use super::components;
use crate::error::{Error, Result};
use crate::fusion::scorer;
use crate::math;
use crate::params::{FusionParams, Params};
use crate::schema::{Evidence, Finding, Kind};

pub const STAMP: &str = "stamp";
const DETECTOR_ID: &str = "stamp.colour_frame";
const CITATION: &str = "module M6: a rubber stamp is a single-hue ink region with a ring or a border (colour clustering and shape)";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StampParams {
    pub chroma_min: u8,
    pub merge_in: f64,
    pub min_in: f64,
    pub max_in: f64,
    pub min_pixels: usize,
    pub hue_bins: usize,
    pub hue_window_bins: usize,
    pub ring_band_start: f64,
    pub ring_sectors: usize,
    pub ring_sector_pixels: usize,
    pub circle_aspect_tol: f64,
    pub inner_band_start: f64,
    pub inner_band_end: f64,
    pub border_band: f64,
    pub tilt_step_deg: f64,
    pub tilt_max_deg: f64,
    pub term_clip: f64,
    pub prior: f64,
    pub hue_conc_threshold: f64,
    pub hue_conc_coef: f64,
    pub frame_threshold: f64,
    pub frame_coef: f64,
    pub chroma_threshold: f64,
    pub chroma_coef: f64,
    pub min_interior_share: f64,
    pub max_fill: f64,
    pub max_stamps: usize,
}

impl StampParams {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParams(format!("stamp: {m}")));
        let all = [
            ("merge_in", self.merge_in),
            ("min_in", self.min_in),
            ("max_in", self.max_in),
            ("ring_band_start", self.ring_band_start),
            ("circle_aspect_tol", self.circle_aspect_tol),
            ("inner_band_start", self.inner_band_start),
            ("inner_band_end", self.inner_band_end),
            ("border_band", self.border_band),
            ("tilt_step_deg", self.tilt_step_deg),
            ("tilt_max_deg", self.tilt_max_deg),
            ("term_clip", self.term_clip),
            ("prior", self.prior),
            ("hue_conc_threshold", self.hue_conc_threshold),
            ("hue_conc_coef", self.hue_conc_coef),
            ("frame_threshold", self.frame_threshold),
            ("frame_coef", self.frame_coef),
            ("chroma_threshold", self.chroma_threshold),
            ("chroma_coef", self.chroma_coef),
            ("min_interior_share", self.min_interior_share),
            ("max_fill", self.max_fill),
        ];
        for (name, v) in all {
            if !v.is_finite() || v.abs() > 1.0e6 {
                return bad(&format!("{name} must be finite and within 1e6, got {v}"));
            }
        }
        if self.merge_in < 0.0
            || self.merge_in > 2.0
            || self.min_in <= 0.0
            || self.max_in < self.min_in
        {
            return bad("merge_in must be in [0, 2] and 0 < min_in <= max_in");
        }
        if self.hue_bins == 0 || self.hue_bins > 360 || self.hue_window_bins >= self.hue_bins {
            return bad("hue_bins must be in 1..=360 and hue_window_bins < hue_bins");
        }
        if self.ring_sectors < 4 || self.ring_sectors > 360 {
            return bad("ring_sectors must be in 4..=360");
        }
        if !(0.0..1.0).contains(&self.ring_band_start)
            || !(0.0..1.0).contains(&self.inner_band_start)
            || self.inner_band_end <= self.inner_band_start
            || self.inner_band_end > self.ring_band_start
        {
            return bad("bands must satisfy 0 <= inner_start < inner_end <= ring_band_start < 1");
        }
        if !(0.0..=0.5).contains(&self.border_band)
            || self.tilt_step_deg <= 0.0
            || self.tilt_max_deg < 0.0
            || self.tilt_max_deg > 45.0
        {
            return bad(
                "border_band must be in [0, 0.5], tilt_step_deg > 0 and tilt_max_deg in [0, 45]",
            );
        }
        if self.term_clip <= 0.0 || self.max_stamps == 0 {
            return bad("term_clip must be > 0 and max_stamps >= 1");
        }
        Ok(())
    }
}

fn hue_deg(r: u8, g: u8, b: u8) -> (u32, u32) {
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    if d == 0 {
        return (0, 0);
    }
    let h = if max == r {
        60 * (g - b) / d
    } else if max == g {
        120 + 60 * (b - r) / d
    } else {
        240 + 60 * (r - g) / d
    };
    (h.rem_euclid(360) as u32, d as u32)
}

fn hue_name(deg: f64) -> &'static str {
    match deg {
        d if !(15.0..345.0).contains(&d) => "red",
        d if d < 45.0 => "orange",
        d if d < 70.0 => "yellow",
        d if d < 165.0 => "green",
        d if d < 200.0 => "cyan",
        d if d < 255.0 => "blue",
        d if d < 290.0 => "violet",
        _ => "magenta",
    }
}

/// Widen a 0/1 mask by `r` pixels in each direction (a square window), in linear time.
fn dilate(mask: &[u8], width: usize, height: usize, r: usize) -> Vec<u8> {
    let widen =
        |src: &[u8], len: usize, stride: usize, lines: usize, line_stride: usize| -> Vec<u8> {
            let mut out = vec![0u8; src.len()];
            for line in 0..lines {
                let base = line * line_stride;
                let mut prefix = vec![0u32; len + 1];
                for i in 0..len {
                    prefix[i + 1] = prefix[i] + u32::from(src[base + i * stride] != 0);
                }
                for i in 0..len {
                    let (lo, hi) = (i.saturating_sub(r), (i + r + 1).min(len));
                    out[base + i * stride] = u8::from(prefix[hi] - prefix[lo] > 0);
                }
            }
            out
        };
    let horizontal = widen(mask, width, 1, height, width);
    widen(&horizontal, height, width, width, 1)
}

/// What the frame tests found for one region.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub ring: f64,
    pub inner_ring: f64,
    pub rectangle: f64,
    pub tilt_deg: f64,
    /// Share of the region's ink that is not on the ring (radius below `ring_band_start`).
    pub ring_interior: f64,
    /// Share of the region's ink that is not within the border band of the best-tilted rectangle.
    pub rect_interior: f64,
}

/// Score a region's pixels (page coordinates) as a ring and as a rectangle.
pub fn frame(pixels: &[(u32, u32)], bbox: [u32; 4], p: &StampParams) -> Frame {
    let (cx, cy) = (
        (f64::from(bbox[0]) + f64::from(bbox[2])) / 2.0,
        (f64::from(bbox[1]) + f64::from(bbox[3])) / 2.0,
    );
    // Radii and angles about the box centre.
    let polar: Vec<(f64, usize)> = pixels
        .iter()
        .map(|&(x, y)| {
            let (dx, dy) = (f64::from(x) - cx, f64::from(y) - cy);
            let angle = math::atan2(dy, dx) + std::f64::consts::PI;
            let sector = ((angle / (2.0 * std::f64::consts::PI)) * p.ring_sectors as f64) as usize;
            ((dx * dx + dy * dy).sqrt(), sector.min(p.ring_sectors - 1))
        })
        .collect();
    let mut radii: Vec<u32> = polar.iter().map(|&(r, _)| r as u32).collect();
    radii.sort_unstable();
    let radius = f64::from(radii[((radii.len() * 98) / 100).min(radii.len() - 1)].max(1));
    let band_coverage = |from: f64, to: f64| -> f64 {
        let mut counts = vec![0usize; p.ring_sectors];
        for &(r, s) in &polar {
            if r >= from * radius && r <= to * radius {
                counts[s] += 1;
            }
        }
        counts
            .iter()
            .filter(|&&c| c >= p.ring_sector_pixels)
            .count() as f64
            / p.ring_sectors as f64
    };
    let ring = band_coverage(p.ring_band_start, 1.0);
    let ring_interior = polar
        .iter()
        .filter(|&&(r, _)| r < p.ring_band_start * radius)
        .count() as f64
        / polar.len() as f64;
    let inner_ring = band_coverage(p.inner_band_start, p.inner_band_end);

    // Rectangle: the weakest side at the best tilt.
    let steps = (p.tilt_max_deg / p.tilt_step_deg).floor() as i32;
    let (mut best, mut best_tilt, mut rect_interior) = (0.0f64, 0.0f64, 0.0f64);
    for k in -steps..=steps {
        let tilt = f64::from(k) * p.tilt_step_deg;
        let rad = tilt * std::f64::consts::PI / 180.0;
        let (s, c) = (math::sin(rad), math::cos(rad));
        let uv: Vec<(f64, f64)> = pixels
            .iter()
            .map(|&(x, y)| {
                let (dx, dy) = (f64::from(x) - cx, f64::from(y) - cy);
                (dx * c + dy * s, -dx * s + dy * c)
            })
            .collect();
        let (mut u0, mut u1, mut v0, mut v1) = (
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        );
        for &(u, v) in &uv {
            u0 = u0.min(u);
            u1 = u1.max(u);
            v0 = v0.min(v);
            v1 = v1.max(v);
        }
        let (uw, vw) = ((u1 - u0).ceil() as usize + 1, (v1 - v0).ceil() as usize + 1);
        let band = (p.border_band * uw.min(vw) as f64).max(1.0);
        // For each side, which positions along it have ink within the band.
        let (mut top, mut bottom) = (vec![false; uw], vec![false; uw]);
        let (mut left, mut right) = (vec![false; vw], vec![false; vw]);
        for &(u, v) in &uv {
            let (du, dv) = (
                ((u - u0) as usize).min(uw - 1),
                ((v - v0) as usize).min(vw - 1),
            );
            if v - v0 <= band {
                top[du] = true;
            }
            if v1 - v <= band {
                bottom[du] = true;
            }
            if u - u0 <= band {
                left[dv] = true;
            }
            if u1 - u <= band {
                right[dv] = true;
            }
        }
        let share = |side: &[bool]| side.iter().filter(|&&b| b).count() as f64 / side.len() as f64;
        let weakest = share(&top)
            .min(share(&bottom))
            .min(share(&left))
            .min(share(&right));
        if weakest > best {
            best = weakest;
            best_tilt = tilt;
            let inside = uv
                .iter()
                .filter(|&&(u, v)| u - u0 > band && u1 - u > band && v - v0 > band && v1 - v > band)
                .count();
            rect_interior = inside as f64 / uv.len() as f64;
        }
    }
    Frame {
        ring,
        inner_ring,
        rectangle: best,
        tilt_deg: best_tilt,
        ring_interior,
        rect_interior,
    }
}

fn term(value: f64, threshold: f64, coef: f64, clip: f64) -> f64 {
    (coef * (value - threshold)).clamp(-clip, clip)
}

/// Run M6 on a page. `rgb` is the colour page (`3 * width * height`) when there is one.
pub fn detect(
    gray: &[u8],
    rgb: Option<&[u8]>,
    width: usize,
    height: usize,
    dpi: u16,
    params: &Params,
) -> Result<Detection> {
    detect_with(gray, rgb, width, height, dpi, &params.stamp, &params.fusion)
}

/// One candidate region and everything measured on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub bbox: [u32; 4],
    pub pixels: usize,
    pub hue_concentration: f64,
    pub mean_chroma: f64,
    pub peak_hue_deg: f64,
    pub frame: Frame,
    pub round_box: bool,
}

/// Find the page's candidate regions of stamp size, with their measurements.
pub fn regions(
    px: &[u8],
    width: usize,
    height: usize,
    dpi: u16,
    p: &StampParams,
) -> Result<Vec<Region>> {
    if width == 0 || height == 0 || dpi == 0 {
        return Err(Error::InvalidInput(format!(
            "empty page: {width}x{height} at {dpi} dpi"
        )));
    }
    if px.len() != 3 * width * height {
        return Err(Error::InvalidInput(format!(
            "rgb buffer of {} bytes does not match {width}x{height}",
            px.len()
        )));
    }
    let mask: Vec<u8> = px
        .as_chunks::<3>()
        .0
        .iter()
        .map(|c| {
            let d = c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2]);
            u8::from(d >= p.chroma_min)
        })
        .collect();
    let dpi_f = f64::from(dpi);
    let merge = (p.merge_in * dpi_f).round() as usize;
    let wide = dilate(&mask, width, height, merge);
    let lab = components::label_8(&wide, width, height);
    let (min_px, max_px) = (p.min_in * dpi_f, p.max_in * dpi_f);
    // Components from the biggest down. One of stamp size starts a candidate; anything that sits
    // inside a candidate's box (the text inside a ring, a seal's inner ring) joins it.
    let mut order: Vec<usize> = (0..lab.components.len()).collect();
    order.sort_by_key(|&k| {
        let c = &lab.components[k];
        std::cmp::Reverse(u64::from(c.width()) * u64::from(c.height()))
    });
    let mut hosts: Vec<(usize, Vec<usize>)> = Vec::new();
    for k in order {
        let c = &lab.components[k];
        let (cx, cy) = ((c.x0 + c.x1) / 2, (c.y0 + c.y1) / 2);
        let inside = hosts.iter_mut().find(|(h, _)| {
            let hc = &lab.components[*h];
            cx >= hc.x0
                && cx <= hc.x1
                && cy >= hc.y0
                && cy <= hc.y1
                && c.width() <= hc.width()
                && c.height() <= hc.height()
        });
        if let Some((_, members)) = inside {
            members.push(k);
            continue;
        }
        let (w, h) = (
            f64::from(c.width()).max(1.0) - 2.0 * merge as f64,
            f64::from(c.height()).max(1.0) - 2.0 * merge as f64,
        );
        if w.max(h) >= min_px && w.max(h) <= max_px && w.min(h) >= 0.4 * min_px {
            hosts.push((k, vec![k]));
        }
    }
    hosts.sort_by_key(|(k, _)| *k);
    let mut out = Vec::new();
    for (_, members) in &hosts {
        let mut pixels: Vec<(u32, u32)> = Vec::new();
        for &k in members {
            let c = &lab.components[k];
            let label = k as u32 + 1;
            for y in c.y0 as usize..=c.y1 as usize {
                for x in c.x0 as usize..=c.x1 as usize {
                    let i = y * width + x;
                    if mask[i] != 0 && lab.labels[i] == label {
                        pixels.push((x as u32, y as u32));
                    }
                }
            }
        }
        if pixels.len() < p.min_pixels {
            continue;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        for &(x, y) in &pixels {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        let bbox = [x0, y0, x1, y1];
        let box_area = f64::from(x1 - x0 + 1) * f64::from(y1 - y0 + 1);
        if pixels.len() as f64 > p.max_fill * box_area {
            // A solid block, not strokes: a logo, a highlighter bar, a photo.
            continue;
        }

        // Hue concentration and mean chroma.
        let mut hist = vec![0usize; p.hue_bins];
        let mut chroma_sum = 0u64;
        for &(x, y) in &pixels {
            let i = (y as usize * width + x as usize) * 3;
            let (h, d) = hue_deg(px[i], px[i + 1], px[i + 2]);
            hist[(h as usize * p.hue_bins / 360).min(p.hue_bins - 1)] += 1;
            chroma_sum += u64::from(d);
        }
        // Best window of 2 * hue_window_bins + 1 bins, circular; ties go to the lowest centre. The
        // reported hue is the window's count-weighted centre, not the position that won the tie.
        let n = p.hue_bins;
        let window = 2 * p.hue_window_bins + 1;
        let bin_at = |centre: usize, o: usize| (centre + n + o - p.hue_window_bins) % n;
        let (mut best, mut best_at) = (0usize, 0usize);
        for centre in 0..n {
            let sum: usize = (0..window).map(|o| hist[bin_at(centre, o)]).sum();
            if sum > best {
                best = sum;
                best_at = centre;
            }
        }
        let offset: f64 = (0..window)
            .map(|o| hist[bin_at(best_at, o)] as f64 * (o as f64 - p.hue_window_bins as f64))
            .sum::<f64>()
            / best.max(1) as f64;
        let conc = best as f64 / pixels.len() as f64;
        let mean_chroma = chroma_sum as f64 / pixels.len() as f64;
        let peak_deg = ((best_at as f64 + offset + 0.5) * 360.0 / n as f64).rem_euclid(360.0);
        let (w_box, h_box) = (f64::from(x1 - x0 + 1), f64::from(y1 - y0 + 1));
        out.push(Region {
            bbox,
            pixels: pixels.len(),
            hue_concentration: conc,
            mean_chroma,
            peak_hue_deg: peak_deg,
            frame: frame(&pixels, bbox, p),
            round_box: (w_box - h_box).abs() <= p.circle_aspect_tol * w_box.max(h_box),
        });
    }
    Ok(out)
}

pub fn detect_with(
    gray: &[u8],
    rgb: Option<&[u8]>,
    width: usize,
    height: usize,
    dpi: u16,
    p: &StampParams,
    fusion: &FusionParams,
) -> Result<Detection> {
    if width == 0 || height == 0 || gray.len() != width * height || dpi == 0 {
        return Err(Error::InvalidInput(format!(
            "gray buffer of {} bytes does not match {width}x{height} at {dpi} dpi",
            gray.len()
        )));
    }
    let mut det = Detection::default();
    // Without colour there is nothing to cluster, and the detector says so rather than guess: a
    // ring over text in grays cannot be told from a seal or from a bordered paragraph.
    let Some(rgb) = rgb else {
        det.features.insert("coloured".into(), 0.0);
        return Ok(det);
    };
    let regs = regions(rgb, width, height, dpi, p)?;
    let dpi_f = f64::from(dpi);
    let mut kept: Vec<(usize, Finding)> = Vec::new();
    for r in &regs {
        let (f, conc, mean_chroma, peak_deg) =
            (&r.frame, r.hue_concentration, r.mean_chroma, r.peak_hue_deg);
        let bbox = r.bbox;
        let (w_box, h_box) = (
            f64::from(bbox[2] - bbox[0] + 1),
            f64::from(bbox[3] - bbox[1] + 1),
        );
        let round_box = r.round_box;
        let circle = round_box && f.ring >= p.frame_threshold;
        let rectangle = !circle && f.rectangle >= p.frame_threshold;
        let (shape, frame_cov) = if circle {
            ("circle", f.ring)
        } else if rectangle {
            ("rectangle", f.rectangle)
        } else {
            (
                "unframed",
                if round_box {
                    f.ring.max(f.rectangle)
                } else {
                    f.rectangle
                },
            )
        };
        let seal = circle && f.inner_ring >= p.frame_threshold;
        // A bare frame (a coloured table border, a ruled box) has nothing inside it.
        let interior = if circle {
            f.ring_interior
        } else {
            f.rect_interior
        };
        let bare = shape != "unframed" && interior < p.min_interior_share;

        let clip = p.term_clip;
        let mut evidence = Vec::new();
        let prior = p.prior;
        {
            evidence.push(Evidence::new(
                DETECTOR_ID,
                "hue_concentration",
                conc,
                Some(p.hue_conc_threshold),
                term(conc, p.hue_conc_threshold, p.hue_conc_coef, clip),
                format!(
                    "{:.0}% of the ink within {} degrees of {} hue",
                    conc * 100.0,
                    (2 * p.hue_window_bins + 1) * 360 / p.hue_bins / 2,
                    hue_name(peak_deg)
                ),
                CITATION,
            )?);
            evidence.push(Evidence::new(
                DETECTOR_ID,
                "mean_chroma",
                mean_chroma,
                Some(p.chroma_threshold),
                term(mean_chroma, p.chroma_threshold, p.chroma_coef, clip),
                "mean max-min of the ink pixels".into(),
                CITATION,
            )?);
        }
        evidence.push(Evidence::new(
            DETECTOR_ID,
            "frame_coverage",
            frame_cov,
            Some(p.frame_threshold),
            term(frame_cov, p.frame_threshold, p.frame_coef, clip),
            format!(
                "{shape}: ring sectors {:.2}, rectangle sides {:.2} at {:.0} degrees",
                f.ring, f.rectangle, f.tilt_deg
            ),
            CITATION,
        )?);
        for e in &mut evidence {
            e.llr = e.llr.clamp(-fusion.llr_clip, fusion.llr_clip);
        }
        let confidence = math::round_output(scorer::confidence_capped(
            prior,
            &evidence,
            fusion.group_cap,
        ));
        if shape == "unframed" || bare || confidence < fusion.emit_min_confidence {
            continue;
        }
        let mut attributes = Map::new();
        attributes.insert("shape".into(), shape.into());
        attributes.insert("colour".into(), hue_name(peak_deg).into());
        attributes.insert(
            "diameter_in".into(),
            math::round_output(w_box.max(h_box) / dpi_f).into(),
        );
        attributes.insert("ring_sectors".into(), math::round_output(f.ring).into());
        attributes.insert("inner_ring".into(), math::round_output(f.inner_ring).into());
        attributes.insert("interior_share".into(), math::round_output(interior).into());
        attributes.insert("pixels".into(), (r.pixels as u64).into());
        kept.push((
            r.pixels,
            Finding {
                kind: if seal { Kind::Seal } else { Kind::Stamp },
                confidence,
                bbox: Some(bbox),
                polygon: None,
                attributes,
                evidence,
            },
        ));
    }
    det.features.insert("coloured".into(), 1.0);
    det.features.insert("candidates".into(), regs.len() as f64);
    kept.sort_by_key(|(n, _)| std::cmp::Reverse(*n));
    kept.truncate(p.max_stamps);
    kept.sort_by_key(|(_, f)| f.bbox.map(|b| (b[1], b[0])));
    det.findings = kept.into_iter().map(|(_, f)| f).collect();
    det.features
        .insert("stamps".into(), det.findings.len() as f64);
    Ok(det)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::rgb_to_gray;

    const W: usize = 420;
    const H: usize = 360;
    const BLUE: [u8; 3] = [25, 55, 170];

    fn page() -> Vec<u8> {
        vec![250u8; W * H * 3]
    }

    fn put(img: &mut [u8], x: i64, y: i64, c: [u8; 3]) {
        if x >= 0 && y >= 0 && (x as usize) < W && (y as usize) < H {
            let i = (y as usize * W + x as usize) * 3;
            img[i..i + 3].copy_from_slice(&c);
        }
    }

    fn ring(img: &mut [u8], cx: i64, cy: i64, r: f64, thick: f64, c: [u8; 3]) {
        for y in cy - r as i64 - 3..=cy + r as i64 + 3 {
            for x in cx - r as i64 - 3..=cx + r as i64 + 3 {
                let d = (((x - cx) * (x - cx) + (y - cy) * (y - cy)) as f64).sqrt();
                if (d - r).abs() <= thick / 2.0 {
                    put(img, x, y, c);
                }
            }
        }
    }

    fn bar(img: &mut [u8], x0: i64, y0: i64, x1: i64, y1: i64, c: [u8; 3]) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                put(img, x, y, c);
            }
        }
    }

    /// Words of a stamp: short bars on two rows.
    fn words(img: &mut [u8], cx: i64, cy: i64, c: [u8; 3]) {
        for (row, n) in [(cy - 12, 5), (cy + 14, 6)] {
            for k in 0..n {
                bar(img, cx - 30 + k * 11, row, cx - 30 + k * 11 + 6, row + 9, c);
            }
        }
    }

    /// Edges of a rectangle tilted by `deg` about `(cx, cy)`.
    fn tilted_frame(img: &mut [u8], cx: f64, cy: f64, w: f64, h: f64, deg: f64, c: [u8; 3]) {
        let (s, co) = (
            math::sin(deg * std::f64::consts::PI / 180.0),
            math::cos(deg * std::f64::consts::PI / 180.0),
        );
        let corner = |u: f64, v: f64| (cx + u * co - v * s, cy + u * s + v * co);
        let pts = [
            corner(-w / 2.0, -h / 2.0),
            corner(w / 2.0, -h / 2.0),
            corner(w / 2.0, h / 2.0),
            corner(-w / 2.0, h / 2.0),
        ];
        for k in 0..4 {
            let (a, b) = (pts[k], pts[(k + 1) % 4]);
            let n = ((b.0 - a.0).abs().max((b.1 - a.1).abs())) as i64 + 1;
            for t in 0..=n {
                let (x, y) = (
                    a.0 + (b.0 - a.0) * t as f64 / n as f64,
                    a.1 + (b.1 - a.1) * t as f64 / n as f64,
                );
                bar(img, x as i64, y as i64, x as i64 + 2, y as i64 + 2, c);
            }
        }
    }

    fn run(img: &[u8]) -> Detection {
        let gray = rgb_to_gray(img).unwrap();
        let p = Params::defaults().unwrap();
        detect(&gray, Some(img), W, H, 100, &p).unwrap()
    }

    fn one(d: &Detection) -> &Finding {
        assert_eq!(d.findings.len(), 1, "{:?} {:?}", d.features, d.findings);
        &d.findings[0]
    }

    #[test]
    fn a_blue_ring_with_words_inside_is_a_circular_stamp() {
        let mut img = page();
        ring(&mut img, 200, 180, 80.0, 4.0, BLUE);
        words(&mut img, 200, 180, BLUE);
        let d = run(&img);
        let f = one(&d);
        assert_eq!(f.kind, Kind::Stamp);
        assert_eq!(f.attributes["shape"], "circle");
        assert_eq!(f.attributes["colour"], "blue");
        let b = f.bbox.unwrap();
        assert!(
            b[0] >= 118 && b[2] <= 282 && b[1] >= 98 && b[3] <= 262,
            "{b:?}"
        );
        assert!(f.confidence > 0.8, "{}", f.confidence);
    }

    #[test]
    fn two_concentric_rings_are_a_seal() {
        let mut img = page();
        ring(&mut img, 200, 180, 85.0, 4.0, [110, 40, 150]);
        ring(&mut img, 200, 180, 58.0, 3.0, [110, 40, 150]);
        words(&mut img, 200, 180, [110, 40, 150]);
        let d = run(&img);
        let f = one(&d);
        assert_eq!(f.kind, Kind::Seal);
        assert_eq!(f.attributes["colour"], "violet");
    }

    #[test]
    fn a_tilted_rectangle_with_words_is_a_rectangular_stamp() {
        let mut img = page();
        tilted_frame(&mut img, 200.0, 180.0, 180.0, 90.0, 12.0, [185, 30, 40]);
        words(&mut img, 200, 180, [185, 30, 40]);
        let d = run(&img);
        let f = one(&d);
        assert_eq!(f.attributes["shape"], "rectangle");
        assert_eq!(f.attributes["colour"], "red");
    }

    #[test]
    fn bare_frames_solid_blocks_and_black_ink_are_not_stamps() {
        // A red frame with nothing in it.
        let mut img = page();
        tilted_frame(&mut img, 200.0, 180.0, 180.0, 90.0, 0.0, [185, 30, 40]);
        assert!(run(&img).findings.is_empty());
        // A solid blue block.
        let mut img = page();
        bar(&mut img, 120, 120, 300, 240, BLUE);
        assert!(run(&img).findings.is_empty());
        // The same ring and words in black.
        let mut img = page();
        ring(&mut img, 200, 180, 80.0, 4.0, [20, 20, 20]);
        words(&mut img, 200, 180, [20, 20, 20]);
        assert!(run(&img).findings.is_empty());
    }

    #[test]
    fn a_stamp_in_two_hues_is_not_one() {
        let mut img = page();
        ring(&mut img, 200, 180, 80.0, 4.0, [185, 30, 40]);
        words(&mut img, 200, 180, [30, 150, 60]);
        let d = run(&img);
        // The ring is red and the text green: the hue is split, and the confidence suffers.
        assert!(
            d.findings.iter().all(|f| f.confidence < 0.99),
            "{:?}",
            d.findings
        );
    }

    #[test]
    fn without_a_colour_page_nothing_is_reported() {
        let mut img = page();
        ring(&mut img, 200, 180, 80.0, 4.0, BLUE);
        words(&mut img, 200, 180, BLUE);
        let gray = rgb_to_gray(&img).unwrap();
        let p = Params::defaults().unwrap();
        let d = detect(&gray, None, W, H, 100, &p).unwrap();
        assert!(d.findings.is_empty());
        assert_eq!(d.features["coloured"], 0.0);
        assert!(detect(&gray, Some(&img[..10]), W, H, 100, &p).is_err());
        assert!(detect(&gray[..10], Some(&img), W, H, 100, &p).is_err());
    }

    #[test]
    fn hues_and_names() {
        assert_eq!(hue_deg(255, 0, 0), (0, 255));
        assert_eq!(hue_deg(0, 255, 0).0, 120);
        assert_eq!(hue_deg(0, 0, 255).0, 240);
        assert_eq!(hue_deg(30, 30, 30), (0, 0));
        assert_eq!(hue_name(5.0), "red");
        assert_eq!(hue_name(355.0), "red");
        assert_eq!(hue_name(230.0), "blue");
        assert_eq!(hue_name(270.0), "violet");
    }

    #[test]
    fn detection_is_repeatable_and_params_validate() {
        let mut img = page();
        ring(&mut img, 200, 180, 80.0, 4.0, BLUE);
        words(&mut img, 200, 180, BLUE);
        assert_eq!(run(&img).findings, run(&img).findings);
        let mut p = Params::defaults().unwrap().stamp;
        assert!(p.validate().is_ok());
        p.hue_window_bins = p.hue_bins;
        assert!(p.validate().is_err());
        let mut p = Params::defaults().unwrap().stamp;
        p.inner_band_end = 0.95;
        assert!(p.validate().is_err());
    }
}
