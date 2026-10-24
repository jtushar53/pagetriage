//! Bleed-through (show-through of the reverse side), module M3.
//!
//! The verso's ink shows through as faint strokes on the paper between and under the front text.
//! Two measures come from the same map, the drop of each pixel below the local paper level:
//!
//! * `mid_mass`: the share of pixels, away from front (strong) ink so that anti-aliased edges do
//!   not count, whose drop lies between "paper noise" and "ink": the third gray-histogram mode
//!   between ink and paper.
//! * `ghost_stroke_ratio`: the faint pixels that form thin, stroke-sized connected components,
//!   relative to the strong ink. Isolated speckle and smooth stains do not form such components.
//!
//! "Strong" ink is defined by the drop itself (a fraction of the ink/paper gap), not by the
//! binariser: a strong show-through is dark enough for Sauvola to call it ink.
//!
//! The "mirrored stroke" test in the module definition needs the reverse side to compare against,
//! which a single page does not have, so it is not implemented: a faint stroke is called ghost
//! ink by its contrast and size alone. This is a heuristic with no published basis; validate it on
//! pages you have looked at (see `docs/detectors/quality.md`).

use super::config::QualityParams;
use super::paper::BlockGrid;
use crate::pixel::components;
use crate::pixel::quality::stroke;

/// Local paper level per pixel, from a smoothed block grid.
struct PaperLevel {
    block: usize,
    blocks_x: usize,
    blocks_y: usize,
    level: Vec<i32>,
}

impl PaperLevel {
    fn new(grid: &BlockGrid, fallback: i32, p: &QualityParams) -> Self {
        Self {
            block: grid.block,
            blocks_x: grid.blocks_x,
            blocks_y: grid.blocks_y,
            level: grid.smoothed(p.bleed_bg_radius_blocks, fallback),
        }
    }

    fn at(&self, x: usize, y: usize) -> i32 {
        let i = (x / self.block).min(self.blocks_x - 1);
        let j = (y / self.block).min(self.blocks_y - 1);
        self.level[j * self.blocks_x + i]
    }
}

/// How far a gray level sits below the local paper level, per pixel (negative above it).
pub fn drops(
    gray: &[u8],
    grid: &BlockGrid,
    width: usize,
    height: usize,
    paper_level: f64,
    p: &QualityParams,
) -> Vec<i16> {
    let levels = PaperLevel::new(grid, paper_level.round() as i32, p);
    let mut drop = vec![0i16; gray.len()];
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            drop[i] = (levels.at(x, y) - i32::from(gray[i])) as i16;
        }
    }
    drop
}

/// Robust noise level of the paper's own gray values: 1.4826 times the median absolute deviation
/// of `drop` over the pixels where `eligible` is set (0 when there are none).
fn noise_sigma(drop: &[i16], eligible: impl Fn(usize) -> bool) -> f64 {
    let mut hist = [0u64; 511];
    let mut n = 0u64;
    for (i, &d) in drop.iter().enumerate() {
        if eligible(i) {
            hist[(i32::from(d) + 255).clamp(0, 510) as usize] += 1;
            n += 1;
        }
    }
    if n == 0 {
        return 0.0;
    }
    let median_of = |h: &[u64; 511]| -> usize {
        let mut acc = 0u64;
        for (v, &c) in h.iter().enumerate() {
            acc += c;
            if acc * 2 >= n {
                return v;
            }
        }
        0
    };
    let med = median_of(&hist) as i64;
    let mut dev = [0u64; 511];
    for (v, &c) in hist.iter().enumerate() {
        dev[(v as i64 - med).unsigned_abs() as usize] += c;
    }
    1.4826 * median_of(&dev) as f64
}

/// What the bleed-through measure found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bleed {
    /// Combined score in `[0, 1]`.
    pub score: f64,
    pub mid_mass: f64,
    pub ghost_stroke_ratio: f64,
    /// Faint pixels (any component size) over faint plus strong-ink pixels.
    pub faint_ink_ratio: f64,
    pub ghost_pixels: u64,
    pub strong_pixels: u64,
    /// Lower and upper gray-level drop between which a pixel is "faint".
    pub threshold_levels: f64,
    pub strong_levels: f64,
}

/// What the bleed-through measure needs from the rest of the quality pass.
pub struct BleedInput<'a> {
    pub gray: &'a [u8],
    pub grid: &'a BlockGrid,
    pub width: usize,
    pub height: usize,
    /// Tone measure: paper level in gray levels, and ink/paper gap in `[0, 1]`.
    pub paper_level: f64,
    pub gap: f64,
    /// The text's median stroke width in pixels, and the radius kept clear around strong ink.
    pub text_stroke_px: f64,
    pub exclude_px: usize,
}

pub fn measure(inp: &BleedInput<'_>, p: &QualityParams) -> Bleed {
    let BleedInput {
        gray,
        grid,
        width,
        height,
        paper_level,
        gap,
        text_stroke_px,
        exclude_px,
    } = *inp;
    let none = Bleed {
        score: 0.0,
        mid_mass: 0.0,
        ghost_stroke_ratio: 0.0,
        faint_ink_ratio: 0.0,
        ghost_pixels: 0,
        strong_pixels: 0,
        threshold_levels: 0.0,
        strong_levels: 0.0,
    };
    if gap <= 0.0 {
        return none;
    }
    let gap_levels = gap * 255.0;
    let drop = drops(gray, grid, width, height, paper_level, p);

    // Strong ink: dark enough to be front-side text whatever the binariser says (a strong show-
    // through can be dark enough for Sauvola to call it ink). Faint pixels are searched only
    // clear of it, so anti-aliased and blurred edges of real strokes do not count.
    let strong_levels = p.bleed_strong_frac * gap_levels;
    let strong: Vec<u8> = drop
        .iter()
        .map(|&d| u8::from(f64::from(d) >= strong_levels))
        .collect();
    let strong_pixels: u64 = strong.iter().map(|&v| u64::from(v)).sum();
    let near_strong = super::blur::dilate(&strong, width, height, exclude_px);
    let eligible = |i: usize| near_strong[i] == 0;

    let threshold = (p.bleed_ghost_min_frac * gap_levels)
        .max(p.bleed_ghost_floor_levels)
        .max(p.bleed_noise_sigmas * noise_sigma(&drop, eligible));
    if strong_pixels == 0 || threshold >= strong_levels {
        return Bleed {
            threshold_levels: threshold,
            strong_levels,
            strong_pixels,
            ..none
        };
    }
    let mut candidate = vec![0u8; gray.len()];
    let (mut n_eligible, mut faint) = (0u64, 0u64);
    for (i, slot) in candidate.iter_mut().enumerate() {
        if eligible(i) {
            n_eligible += 1;
            if f64::from(drop[i]) >= threshold {
                *slot = 1;
                faint += 1;
            }
        }
    }
    let lab = components::label_8(&candidate, width, height);
    // Ghost strokes are thin like text; blobs (stains, shadows) are as thick as they are wide.
    let dist = stroke::chamfer_distance(&candidate, width, height);
    let mut ridge_len = vec![0u32; lab.components.len()];
    for y in 0..height {
        for x in 0..width {
            let l = lab.labels[y * width + x];
            if l != 0 && stroke::is_ridge(&dist, width, height, x, y) {
                ridge_len[(l - 1) as usize] += 1;
            }
        }
    }
    let max_thickness = p.bleed_ghost_max_width_ratio * text_stroke_px.max(1.0);
    let ghost_pixels: u64 = lab
        .components
        .iter()
        .zip(&ridge_len)
        .filter(|(c, r)| {
            c.area >= p.bleed_ghost_min_area
                && f64::from(c.width()) < p.text_max_extent_frac * width as f64
                && f64::from(c.height()) < p.text_max_extent_frac * height as f64
                && f64::from(c.area) <= max_thickness * f64::from((**r).max(1))
        })
        .map(|(c, _)| u64::from(c.area))
        .sum();

    let mid_mass = faint as f64 / n_eligible.max(1) as f64;
    let ghost_stroke_ratio = ghost_pixels as f64 / (ghost_pixels + strong_pixels) as f64;
    let s_mid = (mid_mass / p.bleed_mid_mass_full).clamp(0.0, 1.0);
    let s_stroke = (ghost_stroke_ratio / p.bleed_ghost_ratio_full).clamp(0.0, 1.0);
    Bleed {
        score: p.bleed_weight_mid * s_mid + (1.0 - p.bleed_weight_mid) * s_stroke,
        mid_mass,
        ghost_stroke_ratio,
        faint_ink_ratio: faint as f64 / (faint + strong_pixels) as f64,
        ghost_pixels,
        strong_pixels,
        threshold_levels: threshold,
        strong_levels,
    }
}
