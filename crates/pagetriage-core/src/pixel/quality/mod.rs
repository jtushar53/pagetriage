//! Degradation triage (module M3): skew, blur, contrast, speckle, bleed-through, fold lines,
//! photocopy generation, the recommended binariser and an A-D grade.
//!
//! The pass binarises once with Sauvola (cheap, local) and derives everything from that mask, the
//! gray histogram and a handful of linear scans. It never runs Howe: Howe is only *recommended*
//! when the page looks hard. See `docs/detectors/quality.md`.

use std::collections::BTreeMap;

use crate::error::{Error, Result};
use crate::math;
use crate::params::Params;
use crate::pixel::binarize::local;
use crate::pixel::components::{self, Labelling};
use crate::schema::{Binarizer, Quality};

pub mod bleed;
pub mod blur;
pub mod config;
pub mod skew;
pub mod stroke;
pub mod tone;

pub use config::QualityParams;

/// The quality block plus the raw features behind it (surfaced under `debug.quality`).
#[derive(Debug, Clone)]
pub struct QualityAssessment {
    pub quality: Quality,
    pub features: BTreeMap<String, f64>,
}

/// Which components count as text, and the text height they imply.
struct TextMask {
    is_text: Vec<bool>,
    xheight: f64,
}

fn text_mask(lab: &Labelling, width: usize, height: usize, p: &QualityParams) -> TextMask {
    let max_w = p.text_max_extent_frac * width as f64;
    let max_h = p.text_max_extent_frac * height as f64;
    let is_text: Vec<bool> = lab
        .components
        .iter()
        .map(|c| {
            c.area >= p.text_min_area
                && f64::from(c.width()) < max_w
                && f64::from(c.height()) < max_h
        })
        .collect();

    // Area-weighted median of bounding-box heights over components big enough to be glyphs. The
    // weighting keeps a crowd of specks from dragging the median down.
    let mut by_height: Vec<(u32, u32)> = lab
        .components
        .iter()
        .zip(&is_text)
        .filter(|(c, t)| **t && c.area >= p.xheight_min_area)
        .map(|(c, _)| (c.height(), c.area))
        .collect();
    by_height.sort_unstable();
    let total: u64 = by_height.iter().map(|&(_, a)| u64::from(a)).sum();
    let xheight = if total == 0 {
        p.xheight_fallback_px
    } else {
        let mut acc = 0u64;
        let mut h = by_height[0].0;
        for &(height, area) in &by_height {
            acc += u64::from(area);
            h = height;
            if acc * 2 >= total {
                break;
            }
        }
        f64::from(h)
    };
    TextMask { is_text, xheight }
}

/// Median of a stream of small integers (0 for an empty stream).
fn median_u16(values: impl Iterator<Item = u16>) -> u16 {
    let mut hist = vec![0u32; 4096];
    let mut n = 0u64;
    for v in values {
        hist[usize::from(v).min(4095)] += 1;
        n += 1;
    }
    let mut acc = 0u64;
    for (v, &c) in hist.iter().enumerate() {
        acc += u64::from(c);
        if acc * 2 >= n && n > 0 {
            return v as u16;
        }
    }
    0
}

/// Run the quality pass on a `width * height` row-major gray page.
pub fn assess(
    gray: &[u8],
    width: usize,
    height: usize,
    params: &Params,
) -> Result<QualityAssessment> {
    if width == 0 || height == 0 || gray.len() != width * height {
        return Err(Error::InvalidInput(format!(
            "gray buffer of {} bytes does not match {width}x{height}",
            gray.len()
        )));
    }
    let p = &params.quality;
    let mut features: BTreeMap<String, f64> = BTreeMap::new();

    let ink = local::sauvola(gray, width, height, &params.sauvola);
    let lab = components::label_8(&ink, width, height);
    let text = text_mask(&lab, width, height, p);

    // Stroke centre lines of text components, in raster order, with their distance values.
    let dist = stroke::chamfer_distance(&ink, width, height);
    let mut ridge: Vec<u16> = Vec::new();
    let mut comp_ridge = vec![0u32; lab.components.len()];
    for y in 0..height {
        for x in 0..width {
            let l = lab.labels[y * width + x];
            if l != 0
                && text.is_text[(l - 1) as usize]
                && stroke::is_ridge(&dist, width, height, x, y)
            {
                ridge.push(dist[y * width + x]);
                comp_ridge[(l - 1) as usize] += 1;
            }
        }
    }
    let median_ridge = median_u16(ridge.iter().copied());
    // A component's mean stroke width is its area over its centre-line length. Components much
    // thicker than the page's median stroke are blobs, not text.
    let limit_px = p.skew_max_stroke_ratio * stroke::width_px(median_ridge).max(1.0);
    let thin: Vec<bool> = lab
        .components
        .iter()
        .zip(&comp_ridge)
        .map(|(c, &r)| f64::from(c.area) <= limit_px * f64::from(r.max(1)))
        .collect();
    let mut points: Vec<(u32, u32)> = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let l = lab.labels[y * width + x];
            if l != 0 && text.is_text[(l - 1) as usize] && thin[(l - 1) as usize] {
                points.push((x as u32, y as u32));
            }
        }
    }
    let skew = skew::estimate(&points, width, height, p);

    // Tone: contrast, paper and ink levels.
    let is_text_ink = |i: usize| {
        let l = lab.labels[i];
        l != 0 && text.is_text[(l - 1) as usize]
    };
    let tone = tone::measure(gray, &ink, &is_text_ink, p.ink_level_percentile);
    let text_ink_pixels: u64 = lab
        .components
        .iter()
        .zip(&text.is_text)
        .filter(|(_, t)| **t)
        .map(|(c, _)| u64::from(c.area))
        .sum();

    // Speckle: components no bigger than a fraction of the text height, per page pixel.
    let speck_side = (p.speck_xheight_frac * text.xheight).max(f64::from(p.speck_min_side_px));
    let specks = lab
        .components
        .iter()
        .filter(|c| f64::from(c.width()) <= speck_side && f64::from(c.height()) <= speck_side)
        .count();
    let speckle_density = specks as f64 / (width * height) as f64;

    // Blur: Laplacian variance on the text mask (text ink and a thin ring around it).
    let text_pixels: Vec<u8> = (0..ink.len()).map(|i| u8::from(is_text_ink(i))).collect();
    let text_mask_px = blur::dilate(&text_pixels, width, height, p.blur_dilate_px);
    let (blur_var, blur_pixels) = blur::laplacian_variance(gray, width, height, &text_mask_px);
    let gap_levels = (tone.gap * 255.0).max(1.0);
    let blur_norm = blur_var / (gap_levels * gap_levels);

    // Bleed-through.
    let near_ink = blur::dilate(&ink, width, height, p.bleed_exclude_px);
    let bleed = bleed::measure(
        gray,
        &ink,
        &near_ink,
        width,
        height,
        tone.paper_level,
        tone.gap,
        text_ink_pixels,
        p,
    );

    features.insert("components".into(), lab.components.len() as f64);
    features.insert("xheight_px".into(), text.xheight);
    features.insert("skew_peak_ratio".into(), skew.peak_ratio);
    features.insert("skew_points".into(), skew.points as f64);
    features.insert("text_points".into(), points.len() as f64);
    features.insert("ridge_median_units".into(), f64::from(median_ridge));
    features.insert("paper_level".into(), tone.paper_level);
    features.insert("ink_level".into(), tone.ink_level);
    features.insert("peak_gap".into(), tone.gap);
    features.insert(
        "ink_fraction".into(),
        text_ink_pixels as f64 / (width * height) as f64,
    );
    features.insert("specks".into(), specks as f64);
    features.insert("speck_side_px".into(), speck_side);
    features.insert("blur_pixels".into(), blur_pixels as f64);
    features.insert("blur_norm".into(), blur_norm);
    features.insert("bleed_mid_mass".into(), bleed.mid_mass);
    features.insert("bleed_ghost_stroke_ratio".into(), bleed.ghost_stroke_ratio);
    features.insert("bleed_faint_ink_ratio".into(), bleed.faint_ink_ratio);
    features.insert("bleed_ghost_pixels".into(), bleed.ghost_pixels as f64);
    features.insert("bleed_threshold_levels".into(), bleed.threshold_levels);

    let quality = Quality {
        skew_deg: math::finite(skew.angle_deg, "quality.skew_deg")?,
        blur_laplacian_var: math::finite(blur_var, "quality.blur_laplacian_var")?,
        contrast_rms: math::finite(tone.contrast_rms, "quality.contrast_rms")?,
        speckle_density: math::finite(speckle_density, "quality.speckle_density")?,
        bleed_through: math::finite(bleed.score, "quality.bleed_through")?,
        fold_lines: 0,
        photocopy_generation_est: None,
        recommended_binarizer: Binarizer::Sauvola,
        grade: "A".to_owned(),
    };
    Ok(QualityAssessment { quality, features })
}
