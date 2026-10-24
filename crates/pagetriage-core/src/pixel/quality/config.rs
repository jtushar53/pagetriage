//! Parameters of the quality pass (module M3). Defaults live in `params/quality.toml`; every key
//! there carries its source or "tuned on <set>".

use serde::Deserialize;

use crate::error::{Error, Result};

/// What the skew sweep maximises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkewCriterion {
    /// Sum of squared bin counts (equivalent to the profile variance at a fixed bin count).
    Variance,
    /// Sum of squared differences between adjacent bins (rewards sharp edges, ignores envelope).
    Gradient,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualityParams {
    // --- text mask ---
    pub text_min_area: u32,
    pub text_max_extent_frac: f64,
    pub xheight_min_area: u32,
    pub xheight_fallback_px: f64,
    // --- tone ---
    pub ink_level_percentile: f64,
    // --- speckle ---
    pub speck_xheight_frac: f64,
    pub speck_min_side_px: u32,
    // --- blur ---
    pub blur_dilate_px: usize,
    // --- bleed-through ---
    pub bleed_bg_block_px: usize,
    pub bleed_bg_radius_blocks: usize,
    pub bleed_exclude_px: usize,
    pub bleed_ghost_min_frac: f64,
    pub bleed_ghost_floor_levels: f64,
    pub bleed_noise_sigmas: f64,
    pub bleed_ghost_min_area: u32,
    pub bleed_mid_mass_full: f64,
    pub bleed_ghost_ratio_full: f64,
    pub bleed_weight_mid: f64,
    // --- skew ---
    pub skew_range_deg: f64,
    pub skew_coarse_step_deg: f64,
    pub skew_fine_step_deg: f64,
    pub skew_fine_halfwidth_deg: f64,
    pub skew_refine_candidates: usize,
    pub skew_criterion: SkewCriterion,
    pub skew_max_stroke_ratio: f64,
    pub skew_bin_scale: f64,
    pub skew_min_points: usize,
    pub skew_max_points: usize,
}

impl QualityParams {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParams(format!("quality: {m}")));
        let finite = [
            ("text_max_extent_frac", self.text_max_extent_frac),
            ("xheight_fallback_px", self.xheight_fallback_px),
            ("ink_level_percentile", self.ink_level_percentile),
            ("speck_xheight_frac", self.speck_xheight_frac),
            ("bleed_ghost_min_frac", self.bleed_ghost_min_frac),
            ("bleed_ghost_floor_levels", self.bleed_ghost_floor_levels),
            ("bleed_noise_sigmas", self.bleed_noise_sigmas),
            ("bleed_mid_mass_full", self.bleed_mid_mass_full),
            ("bleed_ghost_ratio_full", self.bleed_ghost_ratio_full),
            ("bleed_weight_mid", self.bleed_weight_mid),
            ("skew_range_deg", self.skew_range_deg),
            ("skew_coarse_step_deg", self.skew_coarse_step_deg),
            ("skew_fine_step_deg", self.skew_fine_step_deg),
            ("skew_fine_halfwidth_deg", self.skew_fine_halfwidth_deg),
            ("skew_max_stroke_ratio", self.skew_max_stroke_ratio),
            ("skew_bin_scale", self.skew_bin_scale),
        ];
        for (name, v) in finite {
            if !v.is_finite() {
                return bad(&format!("{name} must be finite, got {v}"));
            }
        }
        if !(self.ink_level_percentile > 0.0 && self.ink_level_percentile <= 1.0) {
            return bad("ink_level_percentile must be in (0, 1]");
        }
        if self.speck_xheight_frac < 0.0 || self.blur_dilate_px > 10 {
            return bad("speck_xheight_frac must be >= 0 and blur_dilate_px <= 10");
        }
        if self.bleed_bg_block_px == 0
            || self.bleed_bg_block_px > 64
            || self.bleed_bg_radius_blocks > 64
        {
            return bad("bleed_bg_block_px must be in 1..=64 and bleed_bg_radius_blocks <= 64");
        }
        if !(self.bleed_ghost_min_frac > 0.0 && self.bleed_ghost_min_frac < 1.0)
            || self.bleed_ghost_floor_levels < 0.0
            || self.bleed_noise_sigmas < 0.0
            || self.bleed_mid_mass_full <= 0.0
            || self.bleed_ghost_ratio_full <= 0.0
            || !(0.0..=1.0).contains(&self.bleed_weight_mid)
        {
            return bad("bleed_* thresholds out of range");
        }
        if !(self.text_max_extent_frac > 0.0 && self.text_max_extent_frac <= 1.0) {
            return bad("text_max_extent_frac must be in (0, 1]");
        }
        if self.xheight_fallback_px <= 0.0 {
            return bad("xheight_fallback_px must be > 0");
        }
        if !(self.skew_range_deg > 0.0 && self.skew_range_deg <= 45.0) {
            return bad("skew_range_deg must be in (0, 45]");
        }
        if !(self.skew_coarse_step_deg >= 0.01 && self.skew_coarse_step_deg <= self.skew_range_deg)
        {
            return bad("skew_coarse_step_deg must be in [0.01, skew_range_deg]");
        }
        if !(self.skew_fine_step_deg >= 0.001
            && self.skew_fine_step_deg <= self.skew_coarse_step_deg)
        {
            return bad("skew_fine_step_deg must be in [0.001, skew_coarse_step_deg]");
        }
        if !(self.skew_fine_halfwidth_deg >= 0.0
            && self.skew_fine_halfwidth_deg <= 5.0 * self.skew_coarse_step_deg)
        {
            return bad("skew_fine_halfwidth_deg must be in [0, 5 * skew_coarse_step_deg]");
        }
        if self.skew_max_stroke_ratio < 1.0 {
            return bad("skew_max_stroke_ratio must be >= 1");
        }
        if self.skew_max_points < 100 {
            return bad("skew_max_points must be >= 100");
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn test_params() -> QualityParams {
    crate::params::Params::defaults()
        .expect("embedded defaults parse")
        .quality
}
