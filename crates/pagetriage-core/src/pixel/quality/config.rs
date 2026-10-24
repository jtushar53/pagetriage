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
    pub blur_sigma_cap_px: f64,
    // --- paper level grid ---
    pub paper_block_px: usize,
    // --- bleed-through ---
    pub bleed_bg_radius_blocks: usize,
    pub bleed_exclude_px: usize,
    pub bleed_ghost_min_frac: f64,
    pub bleed_ghost_floor_levels: f64,
    pub bleed_noise_sigmas: f64,
    pub bleed_ghost_min_area: u32,
    pub bleed_ghost_max_width_ratio: f64,
    pub bleed_exclude_stroke_mult: f64,
    pub bleed_mid_mass_full: f64,
    pub bleed_ghost_ratio_full: f64,
    pub bleed_weight_mid: f64,
    // --- fold lines ---
    pub fold_arm_blocks: usize,
    pub fold_min_block_pixels: u64,
    pub fold_min_valid_frac: f64,
    pub fold_min_depth_levels: f64,
    pub fold_max_depth_gap_frac: f64,
    pub fold_min_coverage: f64,
    pub fold_max_lines: usize,
    pub fold_min_narrowness: f64,
    pub fold_edge_margin_blocks: usize,
    pub fold_edge_margin_frac: f64,
    pub fold_min_shading: f64,
    pub fold_max_shading: f64,
    // --- photocopy generation ---
    pub gen_min_components: usize,
    pub gen_full_confidence_components: usize,
    pub gen_max_confidence: f64,
    pub gen_max_generation: u32,
    pub gen_fragment_area_frac: f64,
    pub gen_cv_good: f64,
    pub gen_cv_bad: f64,
    pub gen_width_good: f64,
    pub gen_width_bad: f64,
    pub gen_fragment_good: f64,
    pub gen_fragment_bad: f64,
    pub gen_speckle_good: f64,
    pub gen_speckle_bad: f64,
    pub gen_gap_good: f64,
    pub gen_gap_bad: f64,
    pub gen_w_cv: f64,
    pub gen_w_width: f64,
    pub gen_w_fragment: f64,
    pub gen_w_speckle: f64,
    pub gen_w_gap: f64,
    // --- grade and binariser ---
    pub grade_skew_good: f64,
    pub grade_skew_bad: f64,
    pub grade_blur_good: f64,
    pub grade_blur_bad: f64,
    pub grade_gap_good: f64,
    pub grade_gap_bad: f64,
    pub grade_speckle_good: f64,
    pub grade_speckle_bad: f64,
    pub grade_bleed_good: f64,
    pub grade_bleed_bad: f64,
    pub grade_folds_good: f64,
    pub grade_folds_bad: f64,
    pub grade_w_skew: f64,
    pub grade_w_blur: f64,
    pub grade_w_gap: f64,
    pub grade_w_speckle: f64,
    pub grade_w_bleed: f64,
    pub grade_w_folds: f64,
    pub grade_w_max: f64,
    pub grade_a_max: f64,
    pub grade_b_max: f64,
    pub grade_c_max: f64,
    pub howe_bleed_min: f64,
    pub howe_gap_max: f64,
    pub howe_min_ink_fraction: f64,
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
            (
                "bleed_ghost_max_width_ratio",
                self.bleed_ghost_max_width_ratio,
            ),
            ("bleed_exclude_stroke_mult", self.bleed_exclude_stroke_mult),
            ("bleed_mid_mass_full", self.bleed_mid_mass_full),
            ("bleed_ghost_ratio_full", self.bleed_ghost_ratio_full),
            ("bleed_weight_mid", self.bleed_weight_mid),
            ("fold_min_valid_frac", self.fold_min_valid_frac),
            ("fold_min_depth_levels", self.fold_min_depth_levels),
            ("fold_max_depth_gap_frac", self.fold_max_depth_gap_frac),
            ("fold_min_coverage", self.fold_min_coverage),
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
        if self.paper_block_px == 0 || self.paper_block_px > 64 || self.bleed_bg_radius_blocks > 64
        {
            return bad("paper_block_px must be in 1..=64 and bleed_bg_radius_blocks <= 64");
        }
        if !(self.bleed_ghost_min_frac > 0.0 && self.bleed_ghost_min_frac < 1.0)
            || self.bleed_ghost_floor_levels < 0.0
            || self.bleed_noise_sigmas < 0.0
            || self.bleed_ghost_max_width_ratio <= 0.0
            || self.bleed_exclude_stroke_mult < 0.0
            || self.bleed_mid_mass_full <= 0.0
            || self.bleed_ghost_ratio_full <= 0.0
            || !(0.0..=1.0).contains(&self.bleed_weight_mid)
        {
            return bad("bleed_* thresholds out of range");
        }
        if self.fold_arm_blocks == 0
            || !(0.0..=1.0).contains(&self.fold_min_valid_frac)
            || self.fold_min_depth_levels <= 0.0
            || self.fold_max_depth_gap_frac <= 0.0
            || !(self.fold_min_coverage > 0.0 && self.fold_min_coverage <= 1.0)
            || !(0.0..=1.0).contains(&self.fold_min_narrowness)
            || !(0.0..0.5).contains(&self.fold_edge_margin_frac)
            || !(self.fold_min_shading >= 0.0 && self.fold_min_shading < self.fold_max_shading)
        {
            return bad("fold_* thresholds out of range");
        }
        // Good/bad ranges: finite and distinct, so a severity never divides by zero.
        let ranges = [
            ("gen_cv", self.gen_cv_good, self.gen_cv_bad),
            ("gen_width", self.gen_width_good, self.gen_width_bad),
            (
                "gen_fragment",
                self.gen_fragment_good,
                self.gen_fragment_bad,
            ),
            ("gen_speckle", self.gen_speckle_good, self.gen_speckle_bad),
            ("gen_gap", self.gen_gap_good, self.gen_gap_bad),
            ("grade_skew", self.grade_skew_good, self.grade_skew_bad),
            ("grade_blur", self.grade_blur_good, self.grade_blur_bad),
            ("grade_gap", self.grade_gap_good, self.grade_gap_bad),
            (
                "grade_speckle",
                self.grade_speckle_good,
                self.grade_speckle_bad,
            ),
            ("grade_bleed", self.grade_bleed_good, self.grade_bleed_bad),
            ("grade_folds", self.grade_folds_good, self.grade_folds_bad),
        ];
        for (name, good, bad_value) in ranges {
            if !good.is_finite() || !bad_value.is_finite() || good == bad_value {
                return bad(&format!(
                    "{name}_good and {name}_bad must be finite and different"
                ));
            }
        }
        let gen_w = [
            self.gen_w_cv,
            self.gen_w_width,
            self.gen_w_fragment,
            self.gen_w_speckle,
            self.gen_w_gap,
        ];
        let grade_w = [
            self.grade_w_skew,
            self.grade_w_blur,
            self.grade_w_gap,
            self.grade_w_speckle,
            self.grade_w_bleed,
            self.grade_w_folds,
        ];
        for ws in [&gen_w[..], &grade_w[..]] {
            if ws.iter().any(|w| !w.is_finite() || *w < 0.0) || ws.iter().sum::<f64>() <= 0.0 {
                return bad("weights must be finite, non-negative and not all zero");
            }
        }
        let others = [
            self.gen_max_confidence,
            self.gen_fragment_area_frac,
            self.blur_sigma_cap_px,
            self.grade_w_max,
            self.grade_a_max,
            self.grade_b_max,
            self.grade_c_max,
            self.howe_bleed_min,
            self.howe_gap_max,
            self.howe_min_ink_fraction,
        ];
        if others.iter().any(|v| !v.is_finite()) {
            return bad("gen_*, grade_*, howe_* and blur_sigma_cap_px values must be finite");
        }
        if !(0.0..=1.0).contains(&self.gen_max_confidence) || self.gen_max_generation == 0 {
            return bad("gen_max_confidence must be in [0, 1] and gen_max_generation >= 1");
        }
        if self.gen_full_confidence_components == 0 || self.blur_sigma_cap_px <= 0.0 {
            return bad("gen_full_confidence_components and blur_sigma_cap_px must be > 0");
        }
        if !(0.0..=1.0).contains(&self.grade_w_max)
            || !(0.0 < self.grade_a_max
                && self.grade_a_max < self.grade_b_max
                && self.grade_b_max < self.grade_c_max)
        {
            return bad(
                "grade_w_max must be in [0, 1] and 0 < grade_a_max < grade_b_max < grade_c_max",
            );
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
