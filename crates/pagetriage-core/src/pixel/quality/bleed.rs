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
use crate::pixel::components;

/// Local paper level on a coarse grid: the mean gray of paper pixels in a window of blocks.
struct PaperLevel {
    block: usize,
    blocks_x: usize,
    blocks_y: usize,
    /// Rounded mean gray per block; falls back to the page paper level where no paper is near.
    level: Vec<i32>,
}

impl PaperLevel {
    fn new(
        gray: &[u8],
        paper: &[u8],
        width: usize,
        height: usize,
        fallback: i32,
        p: &QualityParams,
    ) -> Self {
        let block = p.bleed_bg_block_px.max(1);
        let bx = width.div_ceil(block);
        let by = height.div_ceil(block);
        // Per-block sums of paper gray values and paper pixel counts, as integral images.
        let stride = bx + 1;
        let mut sum = vec![0u64; stride * (by + 1)];
        let mut cnt = vec![0u64; stride * (by + 1)];
        for j in 0..by {
            let mut row_sum = 0u64;
            let mut row_cnt = 0u64;
            for i in 0..bx {
                let (mut s, mut c) = (0u64, 0u64);
                for y in j * block..((j + 1) * block).min(height) {
                    for x in i * block..((i + 1) * block).min(width) {
                        if paper[y * width + x] != 0 {
                            s += u64::from(gray[y * width + x]);
                            c += 1;
                        }
                    }
                }
                row_sum += s;
                row_cnt += c;
                sum[(j + 1) * stride + i + 1] = sum[j * stride + i + 1] + row_sum;
                cnt[(j + 1) * stride + i + 1] = cnt[j * stride + i + 1] + row_cnt;
            }
        }
        let r = p.bleed_bg_radius_blocks;
        let mut level = vec![fallback; bx * by];
        for j in 0..by {
            for i in 0..bx {
                let (i0, i1) = (i.saturating_sub(r), (i + r + 1).min(bx));
                let (j0, j1) = (j.saturating_sub(r), (j + r + 1).min(by));
                let at = |t: &[u64], x: usize, y: usize| t[y * stride + x];
                let s = at(&sum, i1, j1) + at(&sum, i0, j0) - at(&sum, i1, j0) - at(&sum, i0, j1);
                let c = at(&cnt, i1, j1) + at(&cnt, i0, j0) - at(&cnt, i1, j0) - at(&cnt, i0, j1);
                if let Some(mean) = (s + c / 2).checked_div(c) {
                    level[j * bx + i] = mean as i32;
                }
            }
        }
        Self {
            block,
            blocks_x: bx,
            blocks_y: by,
            level,
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
    ink: &[u8],
    near_ink: &[u8],
    width: usize,
    height: usize,
    paper_level: f64,
    p: &QualityParams,
) -> Drops {
    let paper: Vec<u8> = ink.iter().map(|&v| u8::from(v == 0)).collect();
    let levels = PaperLevel::new(gray, &paper, width, height, paper_level.round() as i32, p);
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

/// `ink` is the Sauvola mask (1 = ink), `near_ink` is that mask dilated by the exclusion radius,
/// `paper_level` and `gap` come from the tone measure (gap in `[0, 1]`), `text_ink_pixels` counts
/// ink pixels of text components.
#[allow(clippy::too_many_arguments)]
pub fn measure(
    gray: &[u8],
    ink: &[u8],
    near_ink: &[u8],
    width: usize,
    height: usize,
    paper_level: f64,
    gap: f64,
    text_ink_pixels: u64,
    p: &QualityParams,
) -> Bleed {
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
    let d = drops(gray, ink, near_ink, width, height, paper_level, p);
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
    let ghost_pixels: u64 = lab
        .components
        .iter()
        .filter(|c| {
            c.area >= p.bleed_ghost_min_area
                && f64::from(c.width()) < p.text_max_extent_frac * width as f64
                && f64::from(c.height()) < p.text_max_extent_frac * height as f64
        })
        .map(|c| u64::from(c.area))
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
