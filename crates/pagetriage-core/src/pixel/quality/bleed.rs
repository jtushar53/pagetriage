//! Bleed-through (show-through of the reverse side), module M3.
//!
//! The verso's ink shows through as faint strokes on the paper between and under the front text.
//! Two measures come from the same map, the drop of each paper pixel below the local paper level:
//!
//! * `mid_mass`: the share of paper pixels (away from front ink, so anti-aliased edges do not
//!   count) whose drop lies between "paper noise" and "ink": the third gray-histogram mode between
//!   ink and paper.
//! * `ghost_stroke_ratio`: the faint pixels that form stroke-sized connected components, relative
//!   to the front ink. Isolated speckle and smooth stains do not form such components.
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

/// Drop of every pixel below the local paper level, for pixels clear of front ink (others get
/// `None`), plus the robust noise level of those drops (1.4826 times their median absolute
/// deviation, in gray levels).
pub struct Drops {
    pub drop: Vec<i16>,
    pub eligible: Vec<bool>,
    pub noise_sigma: f64,
}

pub fn drops(
    gray: &[u8],
    near_ink: &[u8],
    grid: &BlockGrid,
    width: usize,
    height: usize,
    paper_level: f64,
    p: &QualityParams,
) -> Drops {
    let levels = PaperLevel::new(grid, paper_level.round() as i32, p);
    let mut drop = vec![0i16; gray.len()];
    let mut eligible = vec![false; gray.len()];
    let mut hist = [0u64; 511];
    let mut n = 0u64;
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            if near_ink[i] != 0 {
                continue;
            }
            let d = levels.at(x, y) - i32::from(gray[i]);
            drop[i] = d as i16;
            eligible[i] = true;
            hist[(d + 255) as usize] += 1;
            n += 1;
        }
    }
    let median_of = |h: &[u64; 511], n: u64| -> usize {
        let mut acc = 0u64;
        for (v, &c) in h.iter().enumerate() {
            acc += c;
            if acc * 2 >= n {
                return v;
            }
        }
        0
    };
    let noise_sigma = if n == 0 {
        0.0
    } else {
        let med = median_of(&hist, n) as i64;
        let mut dev = [0u64; 511];
        for (v, &c) in hist.iter().enumerate() {
            dev[(v as i64 - med).unsigned_abs() as usize] += c;
        }
        1.4826 * median_of(&dev, n) as f64
    };
    Drops {
        drop,
        eligible,
        noise_sigma,
    }
}

/// What the bleed-through measure found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bleed {
    /// Combined score in `[0, 1]`.
    pub score: f64,
    pub mid_mass: f64,
    pub ghost_stroke_ratio: f64,
    /// Faint pixels (any component size) over faint plus front-ink pixels.
    pub faint_ink_ratio: f64,
    pub ghost_pixels: u64,
    pub threshold_levels: f64,
}

/// What the bleed-through measure needs from the rest of the quality pass.
pub struct BleedInput<'a> {
    pub gray: &'a [u8],
    /// 1 where a pixel is ink or close to ink (halo): such pixels are not "paper".
    pub near_ink: &'a [u8],
    pub grid: &'a BlockGrid,
    pub width: usize,
    pub height: usize,
    /// Tone measure: paper level in gray levels, and ink/paper gap in `[0, 1]`.
    pub paper_level: f64,
    pub gap: f64,
    /// Ink pixels of text components, and the text's median stroke width in pixels.
    pub text_ink_pixels: u64,
    pub text_stroke_px: f64,
}

pub fn measure(inp: &BleedInput<'_>, p: &QualityParams) -> Bleed {
    let BleedInput {
        gray,
        near_ink,
        grid,
        width,
        height,
        paper_level,
        gap,
        text_ink_pixels,
        text_stroke_px,
    } = *inp;
    let none = Bleed {
        score: 0.0,
        mid_mass: 0.0,
        ghost_stroke_ratio: 0.0,
        faint_ink_ratio: 0.0,
        ghost_pixels: 0,
        threshold_levels: 0.0,
    };
    if text_ink_pixels == 0 || gap <= 0.0 {
        return none;
    }
    let d = drops(gray, near_ink, grid, width, height, paper_level, p);
    let threshold = (p.bleed_ghost_min_frac * gap * 255.0)
        .max(p.bleed_ghost_floor_levels)
        .max(p.bleed_noise_sigmas * d.noise_sigma);
    let mut candidate = vec![0u8; gray.len()];
    let (mut eligible, mut faint) = (0u64, 0u64);
    for (i, slot) in candidate.iter_mut().enumerate() {
        if d.eligible[i] {
            eligible += 1;
            if f64::from(d.drop[i]) >= threshold {
                *slot = 1;
                faint += 1;
            }
        }
    }
    if eligible == 0 {
        return none;
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

    let mid_mass = faint as f64 / eligible as f64;
    let ghost_stroke_ratio = ghost_pixels as f64 / (ghost_pixels + text_ink_pixels) as f64;
    let s_mid = (mid_mass / p.bleed_mid_mass_full).clamp(0.0, 1.0);
    let s_stroke = (ghost_stroke_ratio / p.bleed_ghost_ratio_full).clamp(0.0, 1.0);
    Bleed {
        score: p.bleed_weight_mid * s_mid + (1.0 - p.bleed_weight_mid) * s_stroke,
        mid_mass,
        ghost_stroke_ratio,
        faint_ink_ratio: faint as f64 / (faint + text_ink_pixels) as f64,
        ghost_pixels,
        threshold_levels: threshold,
    }
}
