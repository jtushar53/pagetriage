//! Skew by projection-profile variance maximisation (module M3).
//!
//! Ink pixels are projected onto the axis perpendicular to a candidate text-line direction:
//! `bin = y cos(t) + x sin(t)`. When the candidate matches the true skew, text lines (and, for
//! Devanagari and Bangla, the long shirorekha headlines) fall into few bins, so the profile is
//! spiky and its variance is large. The page is never resampled; pixels are accumulated into row
//! bins directly. A coarse sweep over the whole range is followed by a fine sweep around its best
//! few local maxima.
//!
//! Convention: positive `t` is a counter-clockwise tilt on screen (text rises to the right), the
//! same sense as Pillow's `Image.rotate(t)`. To deskew, rotate by `-t`.
//!
//! Sources: W. Postl, "Detection of linear oblique structures and skew in digitized documents",
//! ICPR 1986 (projection-profile variance; recalled from memory, unverified). Headline-based skew
//! for Indian scripts: B. B. Chaudhuri and U. Pal, IEEE TPAMI 19(2), 1997 (see docs/REFERENCES.md).

use super::config::{QualityParams, SkewCriterion};
use crate::math;

/// One skew estimate and how sharp its peak was.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkewEstimate {
    pub angle_deg: f64,
    /// Best fine-sweep score divided by the median coarse-sweep score. 1 means no preferred
    /// direction; text pages with clear lines give several times that.
    pub peak_ratio: f64,
    /// Points that went into the profile (after subsampling).
    pub points: usize,
}

impl SkewEstimate {
    fn none(points: usize) -> Self {
        Self {
            angle_deg: 0.0,
            peak_ratio: 1.0,
            points,
        }
    }
}

const DEG_TO_RAD: f64 = std::f64::consts::PI / 180.0;

/// Weight resolution of linear binning (each point carries this much weight).
const SUBBIN: f64 = 16.0;

struct Profile<'a> {
    points: &'a [(u32, u32)],
    width: usize,
    criterion: SkewCriterion,
    scale: f64,
    bins: Vec<u32>,
}

impl Profile<'_> {
    fn score(&mut self, angle_deg: f64) -> u64 {
        let t = angle_deg * DEG_TO_RAD;
        let (a, b) = (math::cos(t) * self.scale, math::sin(t) * self.scale);
        let offset = if b < 0.0 { -b * self.width as f64 } else { 0.0 };
        self.bins.fill(0);
        for &(x, y) in self.points {
            // Linear binning: split the point between the two nearest bins in sixteenths, so the
            // integer pixel lattice cannot make some angles look sharper than others.
            let v = f64::from(y) * a + f64::from(x) * b + offset;
            let bin = v as usize;
            let frac = ((v - bin as f64) * SUBBIN) as u32;
            self.bins[bin] += SUBBIN as u32 - frac;
            self.bins[bin + 1] += frac;
        }
        match self.criterion {
            SkewCriterion::Variance => self.bins.iter().map(|&c| u64::from(c) * u64::from(c)).sum(),
            SkewCriterion::Gradient => self
                .bins
                .windows(2)
                .map(|w| {
                    let d = i64::from(w[1]) - i64::from(w[0]);
                    (d * d) as u64
                })
                .sum(),
        }
    }
}

/// Best of `angles` by score; ties go to the smaller `|angle - prefer|`, then the earlier angle.
fn argmax(angles: &[f64], scores: &[u64], prefer: f64) -> usize {
    let mut best = 0;
    for i in 1..angles.len() {
        let better = scores[i] > scores[best]
            || (scores[i] == scores[best]
                && (angles[i] - prefer).abs() < (angles[best] - prefer).abs());
        if better {
            best = i;
        }
    }
    best
}

/// Score of the profile at each of `angles_deg` (diagnostic and test helper).
pub fn sweep(
    points: &[(u32, u32)],
    width: usize,
    height: usize,
    angles_deg: &[f64],
    criterion: SkewCriterion,
    scale: f64,
) -> Vec<u64> {
    let mut profile = Profile {
        points,
        width,
        criterion,
        scale,
        bins: vec![0; ((height + width) as f64 * scale) as usize + 2],
    };
    angles_deg.iter().map(|&a| profile.score(a)).collect()
}

/// Estimate the skew of `points` (ink pixel coordinates) on a `width * height` page.
pub fn estimate(
    points: &[(u32, u32)],
    width: usize,
    height: usize,
    p: &QualityParams,
) -> SkewEstimate {
    if points.len() < p.skew_min_points {
        return SkewEstimate::none(points.len());
    }
    // Deterministic thinning: every k-th point in raster order keeps the profile unbiased.
    let stride = points.len().div_ceil(p.skew_max_points.max(1)).max(1);
    let thinned: Vec<(u32, u32)>;
    let used: &[(u32, u32)] = if stride > 1 {
        thinned = points.iter().copied().step_by(stride).collect();
        &thinned
    } else {
        points
    };

    let mut profile = Profile {
        points: used,
        width,
        criterion: p.skew_criterion,
        scale: p.skew_bin_scale,
        bins: vec![0; ((height + width) as f64 * p.skew_bin_scale) as usize + 2],
    };

    let range = p.skew_range_deg;
    let steps = (2.0 * range / p.skew_coarse_step_deg).round() as usize;
    let angles: Vec<f64> = (0..=steps)
        .map(|i| (-range + i as f64 * p.skew_coarse_step_deg).clamp(-range, range))
        .collect();
    let scores: Vec<u64> = angles.iter().map(|&a| profile.score(a)).collect();

    let mut sorted = scores.clone();
    sorted.sort_unstable();
    let median = sorted[sorted.len() / 2].max(1);

    // Candidates: the best local maxima of the coarse sweep, highest first.
    let mut maxima: Vec<usize> = (0..angles.len())
        .filter(|&i| {
            (i == 0 || scores[i] >= scores[i - 1])
                && (i + 1 == angles.len() || scores[i] >= scores[i + 1])
        })
        .collect();
    maxima.sort_by(|&i, &j| {
        scores[j]
            .cmp(&scores[i])
            .then((angles[i].abs()).total_cmp(&angles[j].abs()))
            .then(i.cmp(&j))
    });
    maxima.truncate(p.skew_refine_candidates.max(1));

    let mut best: Option<(f64, u64)> = None;
    for &m in &maxima {
        let centre = angles[m];
        let n = (p.skew_fine_halfwidth_deg / p.skew_fine_step_deg).round() as i64;
        let fine: Vec<f64> = (-n..=n)
            .map(|k| (centre + k as f64 * p.skew_fine_step_deg).clamp(-range, range))
            .collect();
        let fine_scores: Vec<u64> = fine.iter().map(|&a| profile.score(a)).collect();
        let i = argmax(&fine, &fine_scores, centre);
        let candidate = (fine[i], fine_scores[i]);
        let better = match best {
            None => true,
            Some((angle, score)) => {
                candidate.1 > score || (candidate.1 == score && candidate.0.abs() < angle.abs())
            }
        };
        if better {
            best = Some(candidate);
        }
    }
    let (angle_deg, score) = best.unwrap_or((0.0, 0));
    SkewEstimate {
        angle_deg,
        peak_ratio: score as f64 / median as f64,
        points: used.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pixel::quality::config::test_params;

    /// Horizontal text-like lines (3 px thick) tilted by `deg` counter-clockwise.
    fn tilted_lines(w: u32, h: u32, deg: f64, pitch: u32, thickness: u32) -> Vec<(u32, u32)> {
        let slope = math::sin(deg * DEG_TO_RAD) / math::cos(deg * DEG_TO_RAD);
        let mut pts = Vec::new();
        for y in 0..h {
            for x in 0..w {
                // Line k sits at y0_k - x * slope (text rises to the right for positive deg).
                let yy = f64::from(y) + f64::from(x) * slope;
                let rel = yy - 20.0;
                if rel >= 0.0 && (rel as u32 % pitch) < thickness {
                    pts.push((x, y));
                }
            }
        }
        pts
    }

    #[test]
    fn recovers_known_tilts_within_the_fine_step() {
        let p = test_params();
        for deg in [-9.3, -2.0, 0.0, 0.7, 4.45, 11.0] {
            let pts = tilted_lines(600, 500, deg, 24, 3);
            let est = estimate(&pts, 600, 500, &p);
            assert!(
                (est.angle_deg - deg).abs() <= 0.15,
                "true {deg}, got {}",
                est.angle_deg
            );
            assert!(est.peak_ratio > 1.5, "ratio {}", est.peak_ratio);
        }
    }

    #[test]
    fn too_few_points_means_zero_skew() {
        let p = test_params();
        let est = estimate(&[(1, 1), (5, 5)], 10, 10, &p);
        assert_eq!(est.angle_deg, 0.0);
        assert_eq!(est.peak_ratio, 1.0);
    }

    #[test]
    fn uniform_noise_has_no_preferred_direction() {
        let p = test_params();
        let mut pts = Vec::new();
        let mut s = 12_345u32;
        for _ in 0..4000 {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let x = (s >> 16) % 400;
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let y = (s >> 16) % 300;
            pts.push((x, y));
        }
        pts.sort_by_key(|&(x, y)| (y, x));
        pts.dedup();
        let est = estimate(&pts, 400, 300, &p);
        assert!(est.peak_ratio < 1.5, "ratio {}", est.peak_ratio);
    }

    #[test]
    fn estimate_is_repeatable() {
        let p = test_params();
        let pts = tilted_lines(300, 300, 3.3, 20, 2);
        let a = estimate(&pts, 300, 300, &p);
        let b = estimate(&pts, 300, 300, &p);
        assert_eq!(a, b);
    }
}
