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
use crate::schema::{GenerationEstimate, Quality};

pub mod bleed;
pub mod blur;
pub mod config;
pub mod folds;
pub mod generation;
pub mod grade;
pub mod paper;
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

/// Gray-level (or ridge-value) at which a histogram's cumulative count first reaches half.
fn hist_median(hist: &[u64]) -> usize {
    let n: u64 = hist.iter().sum();
    let mut acc = 0u64;
    for (v, &c) in hist.iter().enumerate() {
        acc += c;
        if acc * 2 >= n && n > 0 {
            return v;
        }
    }
    0
}

/// Run the quality pass on a `width * height` row-major gray page scanned at `dpi`.
pub fn assess(
    gray: &[u8],
    width: usize,
    height: usize,
    dpi: u16,
    params: &Params,
) -> Result<QualityAssessment> {
    if width == 0 || height == 0 || gray.len() != width * height || dpi == 0 {
        return Err(Error::InvalidInput(format!(
            "gray buffer of {} bytes does not match {width}x{height} at {dpi} dpi",
            gray.len()
        )));
    }
    let p = &params.quality;
    let page_px = (width * height) as f64;
    let mut features: BTreeMap<String, f64> = BTreeMap::new();

    let ink = local::sauvola(gray, width, height, &params.sauvola);
    let lab = components::label_8(&ink, width, height);
    let text = text_mask(&lab, width, height, p);
    let is_text_ink = |i: usize| {
        let l = lab.labels[i];
        l != 0 && text.is_text[(l - 1) as usize]
    };

    // Stroke centre lines of text components: their distance values, and how many per component.
    let dist = stroke::chamfer_distance(&ink, width, height);
    let mut ridge_hist = [0u64; 256];
    let mut comp_ridge = vec![0u32; lab.components.len()];
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            if is_text_ink(i) && stroke::is_ridge(&dist, width, height, x, y) {
                ridge_hist[usize::from(dist[i]).min(255)] += 1;
                comp_ridge[(lab.labels[i] - 1) as usize] += 1;
            }
        }
    }
    let median_ridge = hist_median(&ridge_hist) as u16;
    let text_stroke_px = stroke::width_px(median_ridge).max(1.0);

    // Skew from the ink pixels of components that are not blobs. A component's mean stroke width
    // is its area over its centre-line length; much thicker than the page's median is a blob.
    let limit_px = p.skew_max_stroke_ratio * text_stroke_px;
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
    let tone = tone::measure(gray, &ink, &is_text_ink, p.ink_level_percentile);
    let text_areas: Vec<u32> = lab
        .components
        .iter()
        .zip(&text.is_text)
        .filter(|(_, t)| **t)
        .map(|(c, _)| c.area)
        .collect();
    let text_ink_pixels: u64 = text_areas.iter().map(|&a| u64::from(a)).sum();
    let ink_fraction = text_ink_pixels as f64 / page_px;

    // Speckle: components no bigger than a fraction of the text height, per page pixel.
    let speck_side = (p.speck_xheight_frac * text.xheight).max(f64::from(p.speck_min_side_px));
    let specks = lab
        .components
        .iter()
        .filter(|c| f64::from(c.width()) <= speck_side && f64::from(c.height()) <= speck_side)
        .count();
    let speckle_density = specks as f64 / page_px;

    // Blur: Laplacian statistics on the text mask (text ink and a thin ring around it).
    let text_pixels: Vec<u8> = (0..ink.len()).map(|i| u8::from(is_text_ink(i))).collect();
    let text_mask_px = blur::dilate(&text_pixels, width, height, p.blur_dilate_px);
    let focus = blur::focus_stats(gray, width, height, &text_mask_px);
    // Blur radius implied by the edge sharpness (a Gaussian step edge has S = 1 / (sqrt 2 sigma)),
    // in pixels, capped so a page with no gradient gives a finite value.
    let blur_sigma_px = if focus.sharpness > 0.0 {
        (1.0 / (std::f64::consts::SQRT_2 * focus.sharpness)).min(p.blur_sigma_cap_px)
    } else if focus.pixels > 0 {
        p.blur_sigma_cap_px
    } else {
        0.0
    };
    let blur_sigma_mm = blur_sigma_px * 25.4 / f64::from(dpi);

    // Paper level grid, bleed-through and folds.
    let exclude_px = p
        .bleed_exclude_px
        .max((p.bleed_exclude_stroke_mult * text_stroke_px).ceil() as usize);
    let near_ink = blur::dilate(&ink, width, height, exclude_px);
    let clear: Vec<u8> = near_ink.iter().map(|&v| 1 - v).collect();
    let grid = paper::BlockGrid::new(gray, &clear, width, height, p.paper_block_px);
    let bleed = bleed::measure(
        &bleed::BleedInput {
            gray,
            near_ink: &near_ink,
            grid: &grid,
            width,
            height,
            paper_level: tone.paper_level,
            gap: tone.gap,
            text_ink_pixels,
            text_stroke_px,
        },
        p,
    );
    let folds = folds::detect(&grid, tone.gap * 255.0, dpi, p);

    // Photocopy generation (experimental).
    let median_area = {
        let mut a = text_areas.clone();
        a.sort_unstable();
        a.get(a.len() / 2).copied().unwrap_or(0)
    };
    let fragments = text_areas
        .iter()
        .filter(|&&a| f64::from(a) < p.gen_fragment_area_frac * f64::from(median_area))
        .count();
    let fragment_rate = if text_areas.is_empty() {
        0.0
    } else {
        fragments as f64 / text_areas.len() as f64
    };
    let generation = generation::estimate(
        &generation::GenerationInput {
            ridge_hist,
            xheight: text.xheight,
            fragment_rate,
            speckle_density,
            gap: tone.gap,
            components: text_areas.len(),
        },
        p,
    );

    // Grade and binariser.
    let graded = grade::assess(
        &grade::GradeInput {
            skew_deg: skew.angle_deg,
            blur_sigma_mm,
            gap: tone.gap,
            speckle_density,
            bleed: bleed.score,
            folds: folds.count(),
            ink_fraction,
        },
        p,
    );

    let mut put = |name: &str, value: f64| {
        features.insert(name.to_owned(), value);
    };
    put("components", lab.components.len() as f64);
    put("text_components", text_areas.len() as f64);
    put("xheight_px", text.xheight);
    put("skew_peak_ratio", skew.peak_ratio);
    put("skew_points", skew.points as f64);
    put("ridge_median_units", f64::from(median_ridge));
    put("text_stroke_px", text_stroke_px);
    put("paper_level", tone.paper_level);
    put("ink_level", tone.ink_level);
    put("peak_gap", tone.gap);
    put("ink_fraction", ink_fraction);
    put("specks", specks as f64);
    put("speck_side_px", speck_side);
    put("blur_pixels", focus.pixels as f64);
    put("blur_sharpness", focus.sharpness);
    put("blur_sigma_px", blur_sigma_px);
    put("blur_sigma_mm", blur_sigma_mm);
    put("bleed_mid_mass", bleed.mid_mass);
    put("bleed_ghost_stroke_ratio", bleed.ghost_stroke_ratio);
    put("bleed_faint_ink_ratio", bleed.faint_ink_ratio);
    put("bleed_ghost_pixels", bleed.ghost_pixels as f64);
    put("bleed_threshold_levels", bleed.threshold_levels);
    put("fold_horizontal", f64::from(folds.horizontal));
    put("fold_vertical", f64::from(folds.vertical));
    put("fold_candidates", f64::from(folds.candidates));
    put("fold_max_depth", folds.max_depth);
    put("fold_shading_ratio", folds.shading_ratio);
    put("fragment_rate", fragment_rate);
    if let Some((mean_w, cv)) = generation::width_stats(&ridge_hist) {
        put("stroke_width_mean_px", mean_w);
        put("stroke_width_cv", cv);
    }
    if let Some((g, sev)) = &generation {
        put("generation_confidence", g.confidence);
        for (name, v) in ["cv", "width", "fragment", "speckle", "gap"]
            .iter()
            .zip(sev)
        {
            put(&format!("generation_sev_{name}"), *v);
        }
    }
    put("grade_badness", graded.badness);
    for (name, v) in ["skew", "blur", "gap", "speckle", "bleed", "folds"]
        .iter()
        .zip(&graded.severities)
    {
        put(&format!("grade_sev_{name}"), *v);
    }

    let quality = Quality {
        skew_deg: math::finite(skew.angle_deg, "quality.skew_deg")?,
        blur_laplacian_var: math::finite(focus.variance, "quality.blur_laplacian_var")?,
        contrast_rms: math::finite(tone.contrast_rms, "quality.contrast_rms")?,
        speckle_density: math::finite(speckle_density, "quality.speckle_density")?,
        bleed_through: math::finite(bleed.score, "quality.bleed_through")?,
        fold_lines: folds.count(),
        photocopy_generation_est: match generation {
            Some((g, _)) => Some(GenerationEstimate {
                value: g.value,
                confidence: math::finite(g.confidence, "quality.photocopy_generation_est")?,
            }),
            None => None,
        },
        recommended_binarizer: graded.binarizer,
        grade: graded.grade.to_owned(),
    };
    Ok(QualityAssessment { quality, features })
}

/// Fold lines found on a page as `(horizontal, position in pixels, depth in gray levels)`: the
/// row (for horizontal folds) or column (vertical) of the line. A diagnostic that repeats the front
/// end of [`assess`]; the report itself only carries the count.
pub fn fold_positions(
    gray: &[u8],
    width: usize,
    height: usize,
    dpi: u16,
    params: &Params,
) -> Result<Vec<(bool, usize, f64)>> {
    if width == 0 || height == 0 || gray.len() != width * height {
        return Err(Error::InvalidInput(format!(
            "gray buffer of {} bytes does not match {width}x{height}",
            gray.len()
        )));
    }
    let p = &params.quality;
    let ink = local::sauvola(gray, width, height, &params.sauvola);
    let lab = components::label_8(&ink, width, height);
    let text = text_mask(&lab, width, height, p);
    let is_text_ink = |i: usize| {
        let l = lab.labels[i];
        l != 0 && text.is_text[(l - 1) as usize]
    };
    let dist = stroke::chamfer_distance(&ink, width, height);
    let mut hist = [0u64; 256];
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            if is_text_ink(i) && stroke::is_ridge(&dist, width, height, x, y) {
                hist[usize::from(dist[i]).min(255)] += 1;
            }
        }
    }
    let stroke_px = stroke::width_px(hist_median(&hist) as u16).max(1.0);
    let exclude_px = p
        .bleed_exclude_px
        .max((p.bleed_exclude_stroke_mult * stroke_px).ceil() as usize);
    let near_ink = blur::dilate(&ink, width, height, exclude_px);
    let clear: Vec<u8> = near_ink.iter().map(|&v| 1 - v).collect();
    let grid = paper::BlockGrid::new(gray, &clear, width, height, p.paper_block_px);
    let tone = tone::measure(gray, &ink, &is_text_ink, p.ink_level_percentile);
    let (lines, _) = folds::locate(&grid, tone.gap * 255.0, dpi, p);
    Ok(lines
        .into_iter()
        .map(|(horizontal, j, depth, _)| (horizontal, j * grid.block + grid.block / 2, depth))
        .collect())
}
