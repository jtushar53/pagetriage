//! Embossed (blind) seals (module M11). See `docs/detectors/emboss.md`.
//!
//! A circle of faint shading: the page is reduced, faint gradient edges vote for circle centres, and
//! the best circles are checked for a dipole in their radial gradient (outwards on the lit half,
//! inwards on the shaded half), which an inked ring does not have.

use serde::Deserialize;
use serde_json::Map;

use super::Detection;
use crate::error::{Error, Result};
use crate::fusion::scorer;
use crate::math;
use crate::params::{FusionParams, Params};
use crate::schema::{Evidence, Finding, Kind};

pub const EMBOSS: &str = "emboss";
const DETECTOR_ID: &str = "emboss.relief";
const CITATION: &str = "module M11: an embossed seal is a circular relief seen as shading with opposite gradient signs on its two halves; hand set, no real seal seen";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbossParams {
    pub reduce: usize,
    pub smooth: usize,
    pub min_radius_mm: f64,
    pub max_radius_mm: f64,
    pub min_gradient: f64,
    pub max_gradient: f64,
    pub min_vote_share: f64,
    pub highpass: usize,
    pub annulus_inner: f64,
    pub annulus_outer: f64,
    pub antisymmetry_min: f64,
    pub angles: usize,
    pub edge_fraction: f64,
    pub centre_search: usize,
    pub max_tries: usize,
    pub max_shading: f64,
    pub min_contrast: f64,
    pub max_seals: usize,
    pub separation: f64,
    pub term_clip: f64,
    pub prior: f64,
    pub vote_threshold: f64,
    pub vote_coef: f64,
    pub antisymmetry_threshold: f64,
    pub antisymmetry_coef: f64,
}

impl EmbossParams {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParams(format!("emboss: {m}")));
        let all = [
            self.min_radius_mm,
            self.max_radius_mm,
            self.min_gradient,
            self.max_gradient,
            self.min_vote_share,
            self.annulus_inner,
            self.annulus_outer,
            self.antisymmetry_min,
            self.edge_fraction,
            self.max_shading,
            self.min_contrast,
            self.separation,
            self.term_clip,
            self.prior,
            self.vote_threshold,
            self.vote_coef,
            self.antisymmetry_threshold,
            self.antisymmetry_coef,
        ];
        if all.iter().any(|v| !v.is_finite() || v.abs() > 1.0e6) {
            return bad("every value must be finite and within 1e6");
        }
        if self.highpass == 0 || self.highpass > 10 {
            return bad("highpass must be in 1..=10");
        }
        if !(self.annulus_inner > 0.0 && self.annulus_outer > self.annulus_inner) {
            return bad("0 < annulus_inner < annulus_outer");
        }
        if self.angles < 8 || !(self.edge_fraction > 0.0 && self.edge_fraction <= 1.0) {
            return bad("angles >= 8 and edge_fraction in (0, 1]");
        }
        if self.centre_search > 6 || self.max_tries == 0 {
            return bad("centre_search must be at most 6 and max_tries at least 1");
        }
        if self.smooth > 4 {
            return bad("smooth must be at most 4");
        }
        if self.reduce == 0 || self.reduce > 16 {
            return bad("reduce must be in 1..=16");
        }
        if self.min_radius_mm <= 0.0 || self.max_radius_mm < self.min_radius_mm {
            return bad("0 < min_radius_mm <= max_radius_mm");
        }
        if self.min_gradient < 0.0 || self.max_gradient <= self.min_gradient {
            return bad("0 <= min_gradient < max_gradient");
        }
        if !(self.min_vote_share > 0.0 && self.min_vote_share <= 1.0) {
            return bad("min_vote_share must be in (0, 1]");
        }
        if self.max_seals == 0 || self.term_clip <= 0.0 || self.separation < 0.0 {
            return bad("max_seals >= 1, term_clip > 0 and separation >= 0");
        }
        Ok(())
    }
}

/// The page averaged in `k x k` blocks, each value in gray levels times `k * k`.
fn reduced(gray: &[u8], width: usize, height: usize, k: usize) -> (Vec<i32>, usize, usize) {
    let (w, h) = (width / k, height / k);
    let mut out = vec![0i32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut s = 0i32;
            for dy in 0..k {
                for dx in 0..k {
                    s += i32::from(gray[(y * k + dy) * width + x * k + dx]);
                }
            }
            out[y * w + x] = s;
        }
    }
    (out, w, h)
}

/// The reduced page smoothed by a box of side `2 * half + 1` (whole numbers; the mean of the box, rounded
/// down). The sensor noise averages out; the relief, a few pixels wide at this size, stays.
fn smoothed(s: &[i32], w: usize, h: usize, half: usize) -> Vec<i32> {
    if half == 0 {
        return s.to_vec();
    }
    let mut out = vec![0i32; w * h];
    for y in 0..h {
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(half), (x + half + 1).min(w));
            let (y0, y1) = (y.saturating_sub(half), (y + half + 1).min(h));
            let mut sum = 0i64;
            for yy in y0..y1 {
                for xx in x0..x1 {
                    sum += i64::from(s[yy * w + xx]);
                }
            }
            out[y * w + x] = (sum / ((x1 - x0) * (y1 - y0)) as i64) as i32;
        }
    }
    out
}

/// Sobel gradient in gray levels per reduced pixel, zero on the border.
fn gradient(s: &[i32], w: usize, h: usize, k: usize) -> (Vec<f64>, Vec<f64>) {
    let (mut gx, mut gy) = (vec![0.0; w * h], vec![0.0; w * h]);
    let scale = 8.0 * (k * k) as f64;
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let p = |dx: usize, dy: usize| s[(y + dy - 1) * w + x + dx - 1];
            let sx = (p(2, 0) + 2 * p(2, 1) + p(2, 2)) - (p(0, 0) + 2 * p(0, 1) + p(0, 2));
            let sy = (p(0, 2) + 2 * p(1, 2) + p(2, 2)) - (p(0, 0) + 2 * p(1, 0) + p(2, 0));
            gx[y * w + x] = f64::from(sx) / scale;
            gy[y * w + x] = f64::from(sy) / scale;
        }
    }
    (gx, gy)
}

/// The reduced page with the local mean removed, in gray levels.
fn high_pass(s: &[i32], w: usize, h: usize, half: usize, k: usize) -> Vec<f64> {
    // Integral image of the block sums.
    let mut integral = vec![0i64; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0i64;
        for x in 0..w {
            row += i64::from(s[y * w + x]);
            integral[(y + 1) * (w + 1) + x + 1] = integral[y * (w + 1) + x + 1] + row;
        }
    }
    let unit = (k * k) as f64;
    let mut out = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(half), (x + half + 1).min(w));
            let (y0, y1) = (y.saturating_sub(half), (y + half + 1).min(h));
            let sum = integral[y1 * (w + 1) + x1]
                - integral[y0 * (w + 1) + x1]
                - integral[y1 * (w + 1) + x0]
                + integral[y0 * (w + 1) + x0];
            let n = ((x1 - x0) * (y1 - y0)) as f64;
            out[y * w + x] = (f64::from(s[y * w + x]) - sum as f64 / n) / unit;
        }
    }
    out
}

/// How antisymmetric the shading is about (cx, cy) inside the annulus: minus the correlation of the
/// high-passed page with itself turned through 180 degrees (a relief lit from one side: +1; a printed
/// or stained ring, which looks the same turned round: negative), and the rms shading there.
fn antisymmetry(
    hp: &[f64],
    w: usize,
    h: usize,
    cx: usize,
    cy: usize,
    radius: f64,
    p: &EmbossParams,
) -> Option<(f64, f64)> {
    let (lo, hi) = (p.annulus_inner * radius, p.annulus_outer * radius);
    let reach = hi.ceil() as usize;
    if cx < reach || cy < reach || cx + reach >= w || cy + reach >= h {
        return None;
    }
    let (mut cross, mut energy) = (Vec::new(), Vec::new());
    for dy in 0..=reach {
        for dx in 0..=2 * reach {
            // One of each pair of opposite pixels: the half plane dy > 0, and dy = 0 with dx > reach.
            let (vx, vy) = (dx as i64 - reach as i64, dy as i64);
            if vy == 0 && vx <= 0 {
                continue;
            }
            let d = ((vx * vx + vy * vy) as f64).sqrt();
            if d < lo || d > hi {
                continue;
            }
            let a = hp[(cy as i64 + vy) as usize * w + (cx as i64 + vx) as usize];
            let b = hp[(cy as i64 - vy) as usize * w + (cx as i64 - vx) as usize];
            // Ink (text, a printed mark) is far stronger than relief shading: leave it out.
            if a.abs() > p.max_shading || b.abs() > p.max_shading {
                continue;
            }
            cross.push(a * b);
            energy.push(0.5 * (a * a + b * b));
        }
    }
    if energy.is_empty() {
        return None;
    }
    let (c, e) = (math::pairwise_sum(&cross), math::pairwise_sum(&energy));
    if e <= 0.0 {
        return None;
    }
    Some((-c / e, (e / energy.len() as f64).sqrt()))
}

/// Outermost radius (reduced pixels) in `range` whose mean gradient magnitude around (cx, cy) is at
/// least `edge_fraction` of the largest over the range.
fn edge_radius(
    gx: &[f64],
    gy: &[f64],
    w: usize,
    h: usize,
    centre: (f64, f64),
    range: (usize, usize),
    p: &EmbossParams,
) -> Option<f64> {
    let tau = 2.0 * std::f64::consts::PI;
    let mut profile: Vec<(usize, f64)> = Vec::new();
    for d in range.0..=range.1 {
        let mut terms = Vec::with_capacity(p.angles);
        for a in 0..p.angles {
            let t = tau * a as f64 / p.angles as f64;
            let (sx, sy) = (
                (centre.0 + d as f64 * math::cos(t)).round(),
                (centre.1 + d as f64 * math::sin(t)).round(),
            );
            if sx < 1.0 || sy < 1.0 || sx as usize >= w - 1 || sy as usize >= h - 1 {
                break;
            }
            let i = sy as usize * w + sx as usize;
            terms.push((gx[i] * gx[i] + gy[i] * gy[i]).sqrt());
        }
        if terms.len() == p.angles {
            profile.push((d, math::pairwise_sum(&terms) / p.angles as f64));
        }
    }
    let peak = profile.iter().map(|t| t.1).fold(0.0, f64::max);
    if peak <= 0.0 {
        return None;
    }
    profile
        .iter()
        .rev()
        .find(|t| t.1 >= p.edge_fraction * peak)
        .map(|t| t.0 as f64)
}

/// A circle: centre and radius in page pixels, and what was measured on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Seal {
    pub cx: f64,
    pub cy: f64,
    pub radius: f64,
    pub vote_share: f64,
    pub antisymmetry: f64,
    pub contrast: f64,
}

pub fn seals(gray: &[u8], width: usize, height: usize, dpi: u16, p: &EmbossParams) -> Vec<Seal> {
    let k = p.reduce;
    let (s, w, h) = reduced(gray, width, height, k);
    if w < 8 || h < 8 {
        return Vec::new();
    }
    let smooth = smoothed(&s, w, h, p.smooth);
    let (gx, gy) = gradient(&smooth, w, h, k);
    // Radii in reduced pixels.
    let px_per_mm = f64::from(dpi) / 25.4 / k as f64;
    let r_lo = ((p.min_radius_mm * px_per_mm).round() as usize).max(3);
    let r_hi = ((p.max_radius_mm * px_per_mm).round() as usize).max(r_lo);
    let n_r = r_hi - r_lo + 1;
    let mut acc = vec![0u16; n_r * w * h];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let (a, b) = (gx[y * w + x], gy[y * w + x]);
            let mag = (a * a + b * b).sqrt();
            if mag < p.min_gradient || mag > p.max_gradient {
                continue;
            }
            let (ux, uy) = (a / mag, b / mag);
            for (ri, r) in (r_lo..=r_hi).enumerate() {
                for sign in [-1.0, 1.0] {
                    let cx = x as f64 + sign * r as f64 * ux;
                    let cy = y as f64 + sign * r as f64 * uy;
                    let (ix, iy) = (cx.round(), cy.round());
                    if ix >= 0.0 && iy >= 0.0 && (ix as usize) < w && (iy as usize) < h {
                        let at = ri * w * h + iy as usize * w + ix as usize;
                        acc[at] = acc[at].saturating_add(1);
                    }
                }
            }
        }
    }
    // Votes of a radius and its neighbours, over the circumference; the best radius per centre.
    let tau = 2.0 * std::f64::consts::PI;
    let mut candidates: Vec<(f64, usize, usize, usize)> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let mut best = (0.0, 0usize);
            for ri in 0..n_r {
                let mut votes = 0u32;
                for rj in ri.saturating_sub(1)..=(ri + 1).min(n_r - 1) {
                    votes += u32::from(acc[rj * w * h + y * w + x]);
                }
                let share = f64::from(votes) / (tau * (r_lo + ri) as f64);
                if share > best.0 {
                    best = (share, ri);
                }
            }
            if best.0 >= p.min_vote_share {
                candidates.push((best.0, x, y, r_lo + best.1));
            }
        }
    }
    // Strongest first; ties by position, so the order is fixed.
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.2.cmp(&b.2)).then(a.1.cmp(&b.1)));
    // High-pass of the reduced page: the mean of a (2 * highpass + 1)-square box removed, so only the
    // shading remains. Computed with integral sums, in whole numbers.
    let hp = high_pass(&s, w, h, p.highpass, k);
    // Circles found, in reduced pixels.
    let mut out: Vec<Seal> = Vec::new();
    let mut tried: Vec<(f64, f64, f64)> = Vec::new();
    for (share, x, y, r) in candidates {
        if tried.len() >= p.max_tries {
            break;
        }
        // One try per neighbourhood: the votes of a circle pile up on many neighbouring pixels.
        if tried.iter().any(|&(tx, ty, tr)| {
            let (dx, dy) = (tx - x as f64, ty - y as f64);
            (dx * dx + dy * dy).sqrt() < 0.5 * tr.max(r as f64)
        }) {
            continue;
        }
        tried.push((x as f64, y as f64, r as f64));
        // The votes of a relief lit from one side lean towards its lit and shaded edges, and a seal
        // has several concentric edges, so the centre is searched for around the vote peak and the
        // radius over the whole range, as the pair that makes the shading most antisymmetric.
        let mut best: Option<(f64, usize, usize, f64, f64)> = None;
        let reach = p.centre_search as i64;
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let (ux, uy) = (
                    (x as i64 + dx).max(0) as usize,
                    (y as i64 + dy).max(0) as usize,
                );
                for rr in (r_lo..=r_hi).step_by(2) {
                    let Some((antisymmetry, contrast)) =
                        antisymmetry(&hp, w, h, ux, uy, rr as f64, p)
                    else {
                        continue;
                    };
                    if contrast >= p.min_contrast && best.is_none_or(|b| antisymmetry > b.0) {
                        best = Some((antisymmetry, ux, uy, rr as f64, contrast));
                    }
                }
            }
        }
        let Some((antisymmetry, x, y, rf, contrast)) = best else {
            continue;
        };
        if antisymmetry < p.antisymmetry_min {
            continue;
        }
        let (cx, cy) = (x as f64, y as f64);
        // The antisymmetry peaks over a wide annulus; the size of the seal is how far out its edges
        // reach: the outermost radius whose mean gradient magnitude is at least edge_fraction of the
        // strongest.
        let rf = edge_radius(&gx, &gy, w, h, (cx, cy), (r_lo, r_hi), p).unwrap_or(rf);
        let seal = Seal {
            cx,
            cy,
            radius: rf,
            vote_share: share,
            antisymmetry,
            contrast,
        };
        // A seal has several concentric edges; keep the outermost.
        let near = out.iter().position(|o| {
            let (dx, dy) = (o.cx - cx, o.cy - cy);
            (dx * dx + dy * dy).sqrt() < p.separation * o.radius.max(rf)
        });
        match near {
            Some(k) if out[k].radius < rf => out[k] = seal,
            Some(_) => {}
            None if out.len() < p.max_seals => out.push(seal),
            None => {}
        }
    }
    // To page pixels.
    let kf = k as f64;
    for o in &mut out {
        o.cx = o.cx * kf + kf / 2.0;
        o.cy = o.cy * kf + kf / 2.0;
        o.radius *= kf;
    }
    out
}

fn term(value: f64, threshold: f64, coef: f64, clip: f64) -> f64 {
    (coef * (value - threshold)).clamp(-clip, clip)
}

/// Run the embossed-seal detector on a gray page.
pub fn detect(
    gray: &[u8],
    width: usize,
    height: usize,
    dpi: u16,
    params: &Params,
) -> Result<Detection> {
    detect_with(gray, width, height, dpi, &params.emboss, &params.fusion)
}

pub fn detect_with(
    gray: &[u8],
    width: usize,
    height: usize,
    dpi: u16,
    p: &EmbossParams,
    fusion: &FusionParams,
) -> Result<Detection> {
    if width == 0 || height == 0 || gray.len() != width * height {
        return Err(Error::InvalidInput(format!(
            "gray buffer of {} bytes does not match {width}x{height}",
            gray.len()
        )));
    }
    let found = seals(gray, width, height, dpi, p);
    let mut det = Detection::default();
    det.features.insert("candidates".into(), found.len() as f64);
    let clip = |v: f64| v.clamp(-fusion.llr_clip, fusion.llr_clip);
    for s in &found {
        let terms = [
            term(s.vote_share, p.vote_threshold, p.vote_coef, p.term_clip),
            term(
                s.antisymmetry,
                p.antisymmetry_threshold,
                p.antisymmetry_coef,
                p.term_clip,
            ),
        ];
        let mk = |feature: &str, value: f64, threshold: f64, llr: f64, note: &str| {
            Evidence::new(
                DETECTOR_ID,
                feature,
                value,
                Some(threshold),
                clip(llr),
                note.into(),
                CITATION,
            )
        };
        let evidence = vec![
            mk(
                "vote_share",
                s.vote_share,
                p.vote_threshold,
                terms[0],
                "faint edges voting for this circle over its circumference",
            )?,
            mk(
                "antisymmetry",
                s.antisymmetry,
                p.antisymmetry_threshold,
                terms[1],
                "minus the correlation of the shading with itself turned through 180 degrees (1 for a relief lit from one side, negative for an inked or stained ring)",
            )?,
        ];
        let confidence = math::round_output(scorer::confidence_capped(
            p.prior,
            &evidence,
            fusion.group_cap,
        ));
        if confidence < fusion.emit_min_confidence {
            continue;
        }
        let r = s.radius;
        let clampi = |v: f64, hi: usize| v.round().clamp(0.0, hi as f64 - 1.0) as u32;
        let mut attributes = Map::new();
        attributes.insert("radius_px".into(), math::round_output(r).into());
        attributes.insert("contrast".into(), math::round_output(s.contrast).into());
        det.findings.push(Finding {
            kind: Kind::EmbossedSeal,
            confidence,
            bbox: Some([
                clampi(s.cx - r, width),
                clampi(s.cy - r, height),
                clampi(s.cx + r, width),
                clampi(s.cy + r, height),
            ]),
            polygon: None,
            attributes,
            evidence,
        });
    }
    det.features
        .insert("seals".into(), det.findings.len() as f64);
    Ok(det)
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 320;
    const H: usize = 260;

    fn paper() -> Vec<u8> {
        vec![240u8; W * H]
    }

    /// The shading of a ring lit from the left, as in tests/golden/gen_inputs.py `emboss_page`.
    fn embossed(amp: i32) -> Vec<u8> {
        let mut g = paper();
        let (cx, cy, radius) = (196i64, 130i64, 72i64);
        for y in 0..H as i64 {
            for x in 0..W as i64 {
                let (dx, dy) = (x - cx, y - cy);
                let d = ((dx * dx + dy * dy) as f64).sqrt() as i64;
                if d == 0 || d > radius {
                    continue;
                }
                let slope = match d {
                    d if (radius - 12..radius - 6).contains(&d) => 1,
                    d if (radius - 6..=radius).contains(&d) => -1,
                    d if (radius - 40..radius - 37).contains(&d) => 1,
                    d if (radius - 37..radius - 34).contains(&d) => -1,
                    _ => 0,
                };
                if slope != 0 {
                    let i = y as usize * W + x as usize;
                    g[i] = (i64::from(g[i]) + i64::from(amp) * slope * dx / d) as u8;
                }
            }
        }
        g
    }

    #[test]
    fn finds_a_relief_lit_from_the_side() {
        let params = Params::defaults().unwrap();
        let det = detect(&embossed(8), W, H, 150, &params).unwrap();
        assert_eq!(det.findings.len(), 1, "{:?}", det.features);
        let f = &det.findings[0];
        assert_eq!(f.kind, Kind::EmbossedSeal);
        let b = f.bbox.unwrap();
        // Centre (196, 130), radius 72: the box is close to [124, 58, 268, 202].
        for (got, want) in b.iter().zip([124u32, 58, 268, 202]) {
            assert!(got.abs_diff(want) <= 16, "{b:?}");
        }
        assert!(f.attributes.contains_key("radius_px"));
    }

    #[test]
    fn the_same_relief_lit_from_the_other_side_is_found_too() {
        let params = Params::defaults().unwrap();
        let g: Vec<u8> = embossed(8)
            .iter()
            .map(|&v| 480 - u16::from(v))
            .map(|v| v as u8)
            .collect();
        let det = detect(&g, W, H, 150, &params).unwrap();
        assert_eq!(det.findings.len(), 1);
    }

    #[test]
    fn nothing_on_paper_a_gradient_or_a_dark_ring() {
        let params = Params::defaults().unwrap();
        assert!(
            detect(&paper(), W, H, 150, &params)
                .unwrap()
                .findings
                .is_empty()
        );
        // Lighting that fades across the page.
        let mut g = paper();
        for y in 0..H {
            for x in 0..W {
                g[y * W + x] = 240 - (x / 16) as u8;
            }
        }
        assert!(detect(&g, W, H, 150, &params).unwrap().findings.is_empty());
        // An inked ring: dark, with strong edges.
        let mut g = paper();
        for y in 0..H {
            for x in 0..W {
                let (dx, dy) = (x as i64 - 196, y as i64 - 130);
                let d = ((dx * dx + dy * dy) as f64).sqrt() as i64;
                if (68..=72).contains(&d) {
                    g[y * W + x] = 60;
                }
            }
        }
        assert!(detect(&g, W, H, 150, &params).unwrap().findings.is_empty());
        // A faint dark ring: same sign all round.
        let mut g = paper();
        for y in 0..H {
            for x in 0..W {
                let (dx, dy) = (x as i64 - 196, y as i64 - 130);
                let d = ((dx * dx + dy * dy) as f64).sqrt() as i64;
                if (66..=74).contains(&d) {
                    g[y * W + x] = 228;
                }
            }
        }
        assert!(detect(&g, W, H, 150, &params).unwrap().findings.is_empty());
    }

    #[test]
    fn wrong_buffers_and_tiny_pages() {
        let params = Params::defaults().unwrap();
        assert!(detect(&paper(), W, H + 1, 150, &params).is_err());
        assert!(detect(&[], 0, 0, 150, &params).is_err());
        let tiny = vec![240u8; 20 * 20];
        assert!(
            detect(&tiny, 20, 20, 150, &params)
                .unwrap()
                .findings
                .is_empty()
        );
    }

    #[test]
    fn it_is_reproducible() {
        let params = Params::defaults().unwrap();
        let a = detect(&embossed(8), W, H, 150, &params).unwrap();
        let b = detect(&embossed(8), W, H, 150, &params).unwrap();
        assert_eq!(a.findings, b.findings);
    }

    #[test]
    fn validation() {
        let mut p = Params::defaults().unwrap().emboss;
        p.validate().unwrap();
        p.reduce = 0;
        assert!(p.validate().is_err());
        let mut p = Params::defaults().unwrap().emboss;
        p.max_radius_mm = 1.0;
        assert!(p.validate().is_err());
        let mut p = Params::defaults().unwrap().emboss;
        p.prior = f64::NAN;
        assert!(p.validate().is_err());
        let mut p = Params::defaults().unwrap().emboss;
        p.annulus_outer = p.annulus_inner;
        assert!(p.validate().is_err());
    }
}
