//! QR codes (module M11, e-stamp QR): located from their three finder patterns. See
//! `docs/detectors/qr.md`. The code is not decoded.
//!
//! A finder pattern is a 7 x 7 module square of dark, light, dark (3 x 3), light, dark rings, so a scan
//! line through its centre crosses runs in the ratio 1:1:3:1:1 whatever the module size, in any
//! direction. Rows are scanned for such runs, each hit is checked down its column, hits are merged into
//! finder centres, and three centres at the corners of a right isoceles triangle with equal modules are a
//! code. The runs keep their ratio along any scan direction, so rotated codes are found too (checked up to 45 degrees on synthetic symbols); a mirrored or colour-inverted code is not looked for.

use serde::Deserialize;
use serde_json::Map;

use super::Detection;
use super::binarize::local;
use crate::error::{Error, Result};
use crate::fusion::scorer;
use crate::math;
use crate::params::{FusionParams, Params, SauvolaParams};
use crate::schema::{Evidence, Finding, Kind};

pub const QR: &str = "qr";
const DETECTOR_ID: &str = "qr.finder_patterns";
const CITATION: &str = "module M11: a QR code has three 1:1:3:1:1 finder patterns at the corners of a right isoceles triangle (ISO/IEC 18004)";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QrParams {
    pub run_tolerance: f64,
    pub min_module_px: f64,
    pub max_module_px: f64,
    pub module_ratio: f64,
    pub leg_tolerance: f64,
    pub hypotenuse_tolerance: f64,
    pub min_leg_modules: f64,
    pub term_clip: f64,
    pub prior: f64,
    pub leg_threshold: f64,
    pub leg_coef: f64,
    pub hyp_threshold: f64,
    pub hyp_coef: f64,
    pub module_threshold: f64,
    pub module_coef: f64,
    pub max_codes: usize,
}

impl QrParams {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParams(format!("qr: {m}")));
        let all = [
            ("run_tolerance", self.run_tolerance),
            ("min_module_px", self.min_module_px),
            ("max_module_px", self.max_module_px),
            ("module_ratio", self.module_ratio),
            ("leg_tolerance", self.leg_tolerance),
            ("hypotenuse_tolerance", self.hypotenuse_tolerance),
            ("min_leg_modules", self.min_leg_modules),
            ("term_clip", self.term_clip),
            ("prior", self.prior),
            ("leg_threshold", self.leg_threshold),
            ("leg_coef", self.leg_coef),
            ("hyp_threshold", self.hyp_threshold),
            ("hyp_coef", self.hyp_coef),
            ("module_threshold", self.module_threshold),
            ("module_coef", self.module_coef),
        ];
        for (name, v) in all {
            if !v.is_finite() || v.abs() > 1.0e6 {
                return bad(&format!("{name} must be finite and within 1e6, got {v}"));
            }
        }
        if !(0.0..1.0).contains(&self.run_tolerance) {
            return bad("run_tolerance must be in [0, 1)");
        }
        if self.min_module_px <= 0.0
            || self.max_module_px < self.min_module_px
            || self.module_ratio < 1.0
        {
            return bad("0 < min_module_px <= max_module_px and module_ratio >= 1");
        }
        if self.leg_tolerance < 0.0
            || self.hypotenuse_tolerance < 0.0
            || self.min_leg_modules <= 0.0
        {
            return bad("tolerances must be >= 0 and min_leg_modules > 0");
        }
        if self.term_clip <= 0.0 || self.max_codes == 0 {
            return bad("term_clip must be > 0 and max_codes >= 1");
        }
        Ok(())
    }
}

/// A finder pattern: centre in pixels and module size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Finder {
    pub x: f64,
    pub y: f64,
    pub module: f64,
}

/// Do five runs (the first dark) match 1:1:3:1:1? Returns the module size.
fn ratio_match(runs: &[usize; 5], tol: f64) -> Option<f64> {
    let total: usize = runs.iter().sum();
    let module = total as f64 / 7.0;
    for (r, ideal) in runs.iter().zip([1.0, 1.0, 3.0, 1.0, 1.0]) {
        if (*r as f64 - ideal * module).abs() > tol * module * ideal {
            return None;
        }
    }
    Some(module)
}

/// Runs of `line` (a 0/1 sequence) as `(start, length, value)`.
fn runs_of(line: impl Iterator<Item = u8>) -> Vec<(usize, usize, u8)> {
    let mut out: Vec<(usize, usize, u8)> = Vec::new();
    for (i, v) in line.enumerate() {
        match out.last_mut() {
            Some((_, len, val)) if *val == v => *len += 1,
            _ => out.push((i, 1, v)),
        }
    }
    out
}

fn dist(a: &Finder, b: &Finder) -> f64 {
    let (dx, dy) = (a.x - b.x, a.y - b.y);
    (dx * dx + dy * dy).sqrt()
}

/// Finder patterns of a binary mask (1 = dark), `width * height`.
pub fn finders(mask: &[u8], width: usize, height: usize, p: &QrParams) -> Vec<Finder> {
    let mut hits: Vec<Finder> = Vec::new();
    for y in 0..height {
        let rs = runs_of((0..width).map(|x| mask[y * width + x]));
        for w in rs.windows(5) {
            if w[0].2 != 1 {
                continue;
            }
            let runs = [w[0].1, w[1].1, w[2].1, w[3].1, w[4].1];
            let Some(m) = ratio_match(&runs, p.run_tolerance) else {
                continue;
            };
            if m < p.min_module_px || m > p.max_module_px {
                continue;
            }
            let cx = w[0].0 as f64 + (runs[0] + runs[1]) as f64 + runs[2] as f64 / 2.0;
            // The same ratio down the column through the centre.
            let col = runs_of((0..height).map(|yy| mask[yy * width + cx as usize]));
            let Some(k) = col
                .iter()
                .position(|&(s, l, v)| v == 1 && s <= y && y < s + l)
            else {
                continue;
            };
            if k < 2 || k + 2 >= col.len() {
                continue;
            }
            let vr = [
                col[k - 2].1,
                col[k - 1].1,
                col[k].1,
                col[k + 1].1,
                col[k + 2].1,
            ];
            if col[k - 2].2 != 1 {
                continue;
            }
            let Some(vm) = ratio_match(&vr, p.run_tolerance) else {
                continue;
            };
            if (vm / m).max(m / vm) > p.module_ratio {
                continue;
            }
            let cy = col[k - 2].0 as f64 + (vr[0] + vr[1]) as f64 + vr[2] as f64 / 2.0;
            hits.push(Finder {
                x: cx,
                y: cy,
                module: (m + vm) / 2.0,
            });
        }
    }
    // Merge hits of one pattern: within three modules of a cluster's first member.
    let mut clusters: Vec<(Finder, Vec<Finder>)> = Vec::new();
    for h in hits {
        match clusters
            .iter_mut()
            .find(|(c, _)| dist(c, &h) <= 3.0 * c.module.max(h.module))
        {
            Some((_, members)) => members.push(h),
            None => clusters.push((h, vec![h])),
        }
    }
    clusters
        .into_iter()
        .filter(|(_, m)| m.len() >= 3)
        .map(|(_, m)| {
            let n = m.len() as f64;
            Finder {
                x: math::pairwise_sum(&m.iter().map(|f| f.x).collect::<Vec<_>>()) / n,
                y: math::pairwise_sum(&m.iter().map(|f| f.y).collect::<Vec<_>>()) / n,
                module: math::pairwise_sum(&m.iter().map(|f| f.module).collect::<Vec<_>>()) / n,
            }
        })
        .collect()
}

/// One code: its three finder patterns and the square they span.
#[derive(Debug, Clone, PartialEq)]
pub struct Code {
    pub bbox: [u32; 4],
    pub module: f64,
    pub modules: usize,
    pub leg_error: f64,
    pub hyp_error: f64,
    pub module_spread: f64,
}

pub fn codes(fs: &[Finder], width: usize, height: usize, p: &QrParams) -> Vec<Code> {
    let mut out = Vec::new();
    for i in 0..fs.len() {
        for j in i + 1..fs.len() {
            for k in j + 1..fs.len() {
                let trio = [&fs[i], &fs[j], &fs[k]];
                let ms: Vec<f64> = trio.iter().map(|f| f.module).collect();
                let (mmin, mmax) = (
                    ms.iter().copied().fold(f64::INFINITY, f64::min),
                    ms.iter().copied().fold(0.0, f64::max),
                );
                if mmax / mmin > p.module_ratio {
                    continue;
                }
                // d[i] is opposite finder i; the corner of the right angle is opposite the longest.
                let d = [
                    dist(trio[1], trio[2]),
                    dist(trio[0], trio[2]),
                    dist(trio[0], trio[1]),
                ];
                let c = (0..3)
                    .max_by(|&a, &b| d[a].total_cmp(&d[b]))
                    .expect("three distances");
                let (a, b) = ((c + 1) % 3, (c + 2) % 3);
                let (leg1, leg2, hyp) = (d[b], d[a], d[c]);
                let leg = (leg1 + leg2) / 2.0;
                let module = math::pairwise_sum(&ms) / 3.0;
                if leg / module < p.min_leg_modules {
                    continue;
                }
                let leg_error = (leg1 - leg2).abs() / leg;
                let hyp_error = (hyp - leg * std::f64::consts::SQRT_2).abs() / hyp;
                if leg_error > p.leg_tolerance || hyp_error > p.hypotenuse_tolerance {
                    continue;
                }
                // The fourth corner and the box of the square, with the finder patterns' own half
                // width (3.5 modules) beyond the centres.
                let (pc, pa, pb) = (trio[c], trio[a], trio[b]);
                let (dx, dy) = (pa.x + pb.x - pc.x, pa.y + pb.y - pc.y);
                let pts = [(pc.x, pc.y), (pa.x, pa.y), (pb.x, pb.y), (dx, dy)];
                let pad = 3.5 * module;
                let x0 = pts.iter().map(|p| p.0).fold(f64::INFINITY, f64::min) - pad;
                let y0 = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min) - pad;
                let x1 = pts.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max) + pad;
                let y1 = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max) + pad;
                out.push(Code {
                    bbox: [
                        x0.max(0.0) as u32,
                        y0.max(0.0) as u32,
                        (x1.min(width as f64 - 1.0)) as u32,
                        (y1.min(height as f64 - 1.0)) as u32,
                    ],
                    module,
                    modules: (leg / module).round() as usize + 7,
                    leg_error,
                    hyp_error,
                    module_spread: mmax / mmin - 1.0,
                });
            }
        }
    }
    out
}

fn term(value: f64, threshold: f64, coef: f64, clip: f64) -> f64 {
    (coef * (threshold - value)).clamp(-clip, clip)
}

/// Run the QR finder on a page.
pub fn detect(gray: &[u8], width: usize, height: usize, params: &Params) -> Result<Detection> {
    detect_with(
        gray,
        width,
        height,
        &params.qr,
        &params.sauvola,
        &params.fusion,
    )
}

pub fn detect_with(
    gray: &[u8],
    width: usize,
    height: usize,
    p: &QrParams,
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
    let fs = finders(&mask, width, height, p);
    let cs = codes(&fs, width, height, p);
    let mut det = Detection::default();
    det.features
        .insert("finder_patterns".into(), fs.len() as f64);
    let mut kept: Vec<(f64, Finding)> = Vec::new();
    for c in &cs {
        let evidence = vec![
            Evidence::new(
                DETECTOR_ID,
                "leg_error",
                c.leg_error,
                Some(p.leg_threshold),
                term(c.leg_error, p.leg_threshold, p.leg_coef, p.term_clip)
                    .clamp(-fusion.llr_clip, fusion.llr_clip),
                "the two legs of the right angle differ by this fraction".into(),
                CITATION,
            )?,
            Evidence::new(
                DETECTOR_ID,
                "hypotenuse_error",
                c.hyp_error,
                Some(p.hyp_threshold),
                term(c.hyp_error, p.hyp_threshold, p.hyp_coef, p.term_clip)
                    .clamp(-fusion.llr_clip, fusion.llr_clip),
                "the hypotenuse differs from sqrt(2) legs by this fraction".into(),
                CITATION,
            )?,
            Evidence::new(
                DETECTOR_ID,
                "module_spread",
                c.module_spread,
                Some(p.module_threshold),
                term(
                    c.module_spread,
                    p.module_threshold,
                    p.module_coef,
                    p.term_clip,
                )
                .clamp(-fusion.llr_clip, fusion.llr_clip),
                "the three modules differ by this fraction".into(),
                CITATION,
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
        let mut attributes = Map::new();
        attributes.insert("modules".into(), (c.modules as u64).into());
        attributes.insert(
            "version_estimate".into(),
            (((c.modules as i64 - 17) / 4).max(1) as u64).into(),
        );
        attributes.insert("module_px".into(), math::round_output(c.module).into());
        attributes.insert("decoded".into(), false.into());
        kept.push((
            confidence,
            Finding {
                kind: Kind::QrCode,
                confidence,
                bbox: Some(c.bbox),
                polygon: None,
                attributes,
                evidence,
            },
        ));
    }
    // One code can be matched by more than one trio (a fourth, spurious finder); keep the most
    // confident of boxes that overlap by half or more.
    kept.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut chosen: Vec<Finding> = Vec::new();
    for (_, f) in kept {
        let b = f.bbox.expect("set above");
        let overlaps = chosen.iter().any(|g| {
            let o = g.bbox.expect("set above");
            let ix = i64::from(b[2].min(o[2])) - i64::from(b[0].max(o[0])) + 1;
            let iy = i64::from(b[3].min(o[3])) - i64::from(b[1].max(o[1])) + 1;
            let area = |r: [u32; 4]| i64::from(r[2] - r[0] + 1) * i64::from(r[3] - r[1] + 1);
            ix > 0 && iy > 0 && ix * iy * 2 >= area(b).min(area(o))
        });
        if !overlaps {
            chosen.push(f);
        }
    }
    chosen.truncate(p.max_codes);
    chosen.sort_by_key(|f| f.bbox.map(|b| (b[1], b[0])));
    det.features.insert("codes".into(), chosen.len() as f64);
    det.findings = chosen;
    Ok(det)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> QrParams {
        Params::defaults().unwrap().qr
    }

    /// A white page with a 25-module symbol (finders, no data) at `scale` px per module.
    fn symbol_page(scale: usize) -> (Vec<u8>, usize, usize) {
        let (n, q) = (25usize, 4usize);
        let side = (n + 2 * q) * scale;
        let mut gray = vec![255u8; side * side];
        let mut dark = |mx: usize, my: usize| {
            for y in 0..scale {
                for x in 0..scale {
                    gray[((my + q) * scale + y) * side + (mx + q) * scale + x] = 0;
                }
            }
        };
        for (fx, fy) in [(0, 0), (n - 7, 0), (0, n - 7)] {
            for dy in 0..7 {
                for dx in 0..7 {
                    let ring = dx == 0 || dx == 6 || dy == 0 || dy == 6;
                    let core = (2..=4).contains(&dx) && (2..=4).contains(&dy);
                    if ring || core {
                        dark(fx + dx, fy + dy);
                    }
                }
            }
        }
        (gray, side, side)
    }

    #[test]
    fn ratio_accepts_ideal_and_rejects_wrong_runs() {
        assert!((ratio_match(&[4, 4, 12, 4, 4], 0.5).unwrap() - 4.0).abs() < 1e-9);
        assert!(ratio_match(&[4, 4, 4, 4, 4], 0.5).is_none());
        assert!(ratio_match(&[2, 10, 12, 2, 2], 0.5).is_none());
    }

    #[test]
    fn finds_a_bare_symbol_with_its_box() {
        let p = Params::defaults().unwrap();
        for scale in [3, 5] {
            let (gray, w, h) = symbol_page(scale);
            let det = detect(&gray, w, h, &p).unwrap();
            assert_eq!(det.findings.len(), 1, "scale {scale}");
            let f = &det.findings[0];
            assert_eq!(f.kind, Kind::QrCode);
            assert!(f.confidence >= 0.5);
            let b = f.bbox.unwrap();
            let want = (4 * scale, 4 * scale, 29 * scale, 29 * scale);
            for (got, want) in [
                (b[0], want.0),
                (b[1], want.1),
                (b[2], want.2),
                (b[3], want.3),
            ] {
                assert!(
                    (got as i64 - want as i64).abs() <= 2 * scale as i64,
                    "{b:?}"
                );
            }
            assert_eq!(det.features["finder_patterns"], 3.0);
        }
    }

    #[test]
    fn plain_and_wrong_input() {
        let p = Params::defaults().unwrap();
        let blank = vec![255u8; 200 * 200];
        assert!(detect(&blank, 200, 200, &p).unwrap().findings.is_empty());
        assert!(detect(&blank, 200, 100, &p).is_err());
        assert!(detect(&[], 0, 0, &p).is_err());
    }

    #[test]
    fn two_finders_are_not_a_code() {
        let p = params();
        let fs = [
            Finder {
                x: 10.0,
                y: 10.0,
                module: 3.0,
            },
            Finder {
                x: 80.0,
                y: 10.0,
                module: 3.0,
            },
        ];
        assert!(codes(&fs, 200, 200, &p).is_empty());
    }

    #[test]
    fn a_stretched_triangle_is_not_a_code() {
        let p = params();
        // Legs of 60 and 120 px: not isoceles.
        let fs = [
            Finder {
                x: 10.0,
                y: 10.0,
                module: 3.0,
            },
            Finder {
                x: 70.0,
                y: 10.0,
                module: 3.0,
            },
            Finder {
                x: 10.0,
                y: 130.0,
                module: 3.0,
            },
        ];
        assert!(codes(&fs, 200, 200, &p).is_empty());
        let ok = [
            Finder {
                x: 10.0,
                y: 10.0,
                module: 3.0,
            },
            Finder {
                x: 70.0,
                y: 10.0,
                module: 3.0,
            },
            Finder {
                x: 10.0,
                y: 70.0,
                module: 3.0,
            },
        ];
        assert_eq!(codes(&ok, 200, 200, &p).len(), 1);
    }

    #[test]
    fn modules_that_differ_too_much_are_not_a_code() {
        let p = params();
        let fs = [
            Finder {
                x: 10.0,
                y: 10.0,
                module: 3.0,
            },
            Finder {
                x: 70.0,
                y: 10.0,
                module: 6.0,
            },
            Finder {
                x: 10.0,
                y: 70.0,
                module: 3.0,
            },
        ];
        assert!(codes(&fs, 200, 200, &p).is_empty());
    }

    #[test]
    fn defaults_validate_and_bad_values_fail() {
        let mut p = params();
        p.validate().unwrap();
        p.run_tolerance = 1.5;
        assert!(p.validate().is_err());
        let mut p = params();
        p.max_codes = 0;
        assert!(p.validate().is_err());
        let mut p = params();
        p.prior = f64::NAN;
        assert!(p.validate().is_err());
    }
}
