//! Fold lines (module M3): long, low-contrast lines that run across (nearly) the whole page,
//! left by a crease in the paper.
//!
//! The paper level is averaged on a block grid (ink and its halo excluded), so text, thin rules and
//! anti-aliasing drop out and what is left is the shading of the sheet. A crease then shows up as a
//! narrow valley in that shading: along a line of blocks, the value is lower than the average of
//! the two values `arm` blocks away on either side. A valley counts as a fold when
//!
//! * it is deep enough to see but shallow enough to be low-contrast (a fraction of the page's
//!   ink/paper gap), and
//! * it is long (a few inches), narrow (a crease is a line, not a broad shadow), away from the page
//!   edge, and present in nearly every column (row, for vertical folds) where the paper can be
//!   measured at all, with enough such columns, so a stain, a stamp or a shadow that covers only
//!   part of the page does not count.
//!
//! Valleys are insensitive to a smooth lighting gradient (a linear ramp has no valley). A page
//! with many equally strong valleys in one direction (ruled paper) is treated as having none: real
//! pages have a handful of creases at most. A valley must also show the one-sided shading of a
//! crease (`shading_ratio`, between a minimum and a maximum), which separates it from printed rules
//! (no asymmetry) and from region edges (too much). A bare step in shading with no valley is not
//! counted. There is no published method behind this: it is a heuristic
//! that has been exercised on synthetic creases and checked for false alarms on DIBCO pages (which
//! contain very few folds), not validated on real folded deeds.

use super::config::QualityParams;
use super::paper::BlockGrid;

/// Result of the fold search.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Folds {
    pub horizontal: u32,
    pub vertical: u32,
    /// Depth of the deepest accepted valley, in gray levels (0 when none).
    pub max_depth: f64,
    /// For the deepest accepted valley: shoulder difference over depth (0 when none).
    pub shading_ratio: f64,
    /// Full-width narrow valleys before the shading test and the ruled-paper guard, both directions.
    pub candidates: u32,
}

impl Folds {
    pub fn count(&self) -> u32 {
        self.horizontal + self.vertical
    }
}

/// Valleys found in one direction: those that also show crease shading, and how many full-width
/// narrow valleys there were before that test.
#[derive(Default)]
struct Search {
    found: Vec<(usize, f64, f64)>,
    candidates: u32,
}

struct Series {
    /// Values per line, `None` for a line with too little paper.
    value: Vec<Option<f64>>,
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

/// Valley depth of `s` at `j` with arms `a`: mean of the two outer values minus the centre.
fn dip(s: &[Option<f64>], j: usize, a: usize) -> Option<f64> {
    Some(0.5 * (s.get(j.checked_sub(a)?).copied()?? + s.get(j + a).copied()??) - s[j]?)
}

/// Shoulder asymmetry at `j`: |mean of the `a` values after - mean of the `a` values before|.
fn shoulders(s: &[Option<f64>], j: usize, a: usize) -> Option<f64> {
    let mut before = 0.0;
    let mut after = 0.0;
    for k in 1..=a {
        before += s.get(j.checked_sub(k)?).copied()??;
        after += s.get(j + k).copied()??;
    }
    Some(((after - before) / a as f64).abs())
}

/// Search one direction. `cell(line, along)` is the paper mean of one block.
fn search(
    lines: usize,
    along: usize,
    cell: &dyn Fn(usize, usize) -> Option<f64>,
    gap_levels: f64,
    min_along: usize,
    p: &QualityParams,
) -> Search {
    let a = p.fold_arm_blocks.max(1);
    if lines < 2 * a + 1 || along < min_along.max(4) {
        return Search::default();
    }
    // Per-line profile: median over the line's valid blocks.
    let series = Series {
        value: (0..lines)
            .map(|l| {
                let mut v: Vec<f64> = (0..along).filter_map(|i| cell(l, i)).collect();
                if (v.len() as f64) < p.fold_min_valid_frac * along as f64 {
                    None
                } else {
                    Some(median(&mut v))
                }
            })
            .collect(),
    };
    let max_depth = p.fold_max_depth_gap_frac * gap_levels;
    let mut found = Vec::new();
    let mut candidates = 0u32;
    let margin = p
        .fold_edge_margin_blocks
        .max((p.fold_edge_margin_frac * lines as f64).ceil() as usize)
        .max(a);
    if lines <= 2 * margin {
        return Search::default();
    }
    for j in margin..lines - margin {
        let Some(d) = dip(&series.value, j, a) else {
            continue;
        };
        if d < p.fold_min_depth_levels || d > max_depth {
            continue;
        }
        // A crease is narrow: the valley is already (almost) as deep two blocks out as at the full
        // arm. A broad shadow or a stain edge is not.
        let narrow = dip(&series.value, j, 2.min(a)).unwrap_or(0.0);
        if narrow < p.fold_min_narrowness * d {
            continue;
        }
        // Strongest valley within the arm distance only.
        let local_max = (j.saturating_sub(a)..=(j + a).min(lines - 1))
            .filter_map(|k| dip(&series.value, k, a))
            .all(|other| other <= d);
        if !local_max {
            continue;
        }
        // Coverage: of the columns where the valley can be measured at all (text and stamps hide
        // the paper in others), the share that show at least half of it, allowing one block of
        // tilt. Enough columns must be measurable, or a short valley between two text lines
        // would pass.
        let (mut have, mut hit) = (0usize, 0usize);
        for i in 0..along {
            let col: Vec<Option<f64>> = (0..lines).map(|l| cell(l, i)).collect();
            let best = (j.saturating_sub(1)..=(j + 1).min(lines - 1))
                .filter_map(|k| dip(&col, k, a))
                .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |x| x.max(v))));
            if let Some(v) = best {
                have += 1;
                if v >= 0.5 * d {
                    hit += 1;
                }
            }
        }
        if have as f64 >= p.fold_min_measurable * along as f64
            && hit as f64 >= p.fold_min_coverage * have as f64
        {
            candidates += 1;
            // One-sided shading: the two shoulders differ, but by less than the valley is deep. No
            // difference is a printed or pencilled rule; a difference larger than the valley is the
            // edge of a region (dark binding, scanner bed), not a crease.
            let shading = shoulders(&series.value, j, a).unwrap_or(0.0) / d;
            if shading >= p.fold_min_shading && shading <= p.fold_max_shading {
                found.push((j, d, shading));
            }
        }
    }
    Search { found, candidates }
}

/// Fold lines found on the paper grid, as `(horizontal, block index, depth)` after the ruled-paper
/// guard, plus the number of candidates before it. Block indices count blocks from the top (for
/// horizontal folds) or the left (vertical).
pub fn locate(
    grid: &BlockGrid,
    gap_levels: f64,
    dpi: u16,
    p: &QualityParams,
) -> (Vec<(bool, usize, f64, f64)>, u32) {
    let min_px = p.fold_min_block_pixels;
    let (bx, by) = (grid.blocks_x, grid.blocks_y);
    let horizontal = |l: usize, i: usize| grid.mean(i, l, min_px);
    let vertical = |l: usize, i: usize| grid.mean(l, i, min_px);
    // A fold is long: the line must be at least this many blocks.
    let min_along = (p.fold_min_length_in * f64::from(dpi) / grid.block as f64).ceil() as usize;
    let (h, v) = (
        search(by, bx, &horizontal, gap_levels, min_along, p),
        search(bx, by, &vertical, gap_levels, min_along, p),
    );
    let mut out = Vec::new();
    for (is_horizontal, direction) in [(true, &h), (false, &v)] {
        // Many full-width narrow valleys in one direction are rulings, not creases.
        if direction.candidates as usize <= p.fold_max_lines {
            out.extend(
                direction
                    .found
                    .iter()
                    .map(|&(j, d, s)| (is_horizontal, j, d, s)),
            );
        }
    }
    (out, h.candidates + v.candidates)
}

/// Count the fold lines of a page from its paper block grid. `gap_levels` is the ink/paper gap in
/// gray levels and `dpi` the page's resolution (a fold must be a few inches long).
pub fn detect(grid: &BlockGrid, gap_levels: f64, dpi: u16, p: &QualityParams) -> Folds {
    let (lines, candidates) = locate(grid, gap_levels, dpi, p);
    let deepest = lines.iter().fold(
        (0.0f64, 0.0f64),
        |best, &(_, _, d, s)| if d > best.0 { (d, s) } else { best },
    );
    Folds {
        horizontal: lines.iter().filter(|l| l.0).count() as u32,
        vertical: lines.iter().filter(|l| !l.0).count() as u32,
        max_depth: deepest.0,
        shading_ratio: deepest.1,
        candidates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pixel::quality::config::test_params;

    /// Paper with optional horizontal crease: a valley `depth` deep and 2 px wide at row `at`, plus
    /// a one-sided shade (`shade` levels darker over the 20 rows below it).
    fn page(w: usize, h: usize, creases: &[(usize, u8, u8)], vertical: bool) -> Vec<u8> {
        let mut g = vec![220u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let pos = if vertical { x } else { y };
                for &(at, depth, shade) in creases {
                    if pos == at || pos == at + 1 {
                        g[y * w + x] -= depth;
                    } else if pos > at + 1 && pos < at + 22 {
                        g[y * w + x] -= shade;
                    }
                }
            }
        }
        g
    }

    fn run(g: &[u8], w: usize, h: usize, gap: f64) -> Folds {
        let p = test_params();
        let grid = BlockGrid::new(g, &vec![1u8; w * h], w, h, p.paper_block_px);
        detect(&grid, gap, 72, &p)
    }

    #[test]
    fn finds_one_horizontal_crease_with_a_one_sided_shade() {
        let (w, h) = (300, 240);
        let g = page(w, h, &[(120, 25, 6)], false);
        let f = run(&g, w, h, 180.0);
        assert_eq!((f.horizontal, f.vertical), (1, 0), "{f:?}");
        assert!(f.max_depth > 10.0);
        assert!(f.shading_ratio > 0.0);
    }

    #[test]
    fn finds_a_vertical_crease_and_two_of_them_separately() {
        let (w, h) = (300, 240);
        let g = page(w, h, &[(100, 25, 6), (200, 25, 6)], true);
        let f = run(&g, w, h, 180.0);
        assert_eq!((f.horizontal, f.vertical), (0, 2), "{f:?}");
    }

    #[test]
    fn clean_paper_and_a_smooth_lighting_ramp_have_no_folds() {
        let (w, h) = (300, 240);
        assert_eq!(run(&vec![220u8; w * h], w, h, 180.0).count(), 0);
        let ramp: Vec<u8> = (0..w * h).map(|i| (150 + (i / w) * 70 / h) as u8).collect();
        assert_eq!(run(&ramp, w, h, 180.0).count(), 0);
    }

    /// Text-like pages: a crease that crosses lines of text must still be found.
    fn text_page(w: usize, h: usize, crease_at: usize, horizontal: bool) -> Vec<u8> {
        let mut g = page(w, h, &[(crease_at, 25, 6)], !horizontal);
        for line in 0..h / 24 {
            for y in line * 24 + 6..line * 24 + 17 {
                for x in 60..w - 60 {
                    // Dense glyph-like ink: 70% coverage on a 3-px comb.
                    if (x / 3) % 10 < 7 {
                        g[y * w + x] = 40;
                    }
                }
            }
        }
        g
    }

    fn run_with_ink(g: &[u8], w: usize, h: usize) -> Folds {
        use crate::pixel::binarize::local::sauvola;
        use crate::pixel::quality::blur::dilate;
        let params = crate::params::Params::defaults().unwrap();
        let ink = sauvola(g, w, h, &params.sauvola);
        let near = dilate(&ink, w, h, 3);
        let clear: Vec<u8> = near.iter().map(|&v| 1 - v).collect();
        let grid = BlockGrid::new(g, &clear, w, h, params.quality.paper_block_px);
        detect(&grid, 180.0, 72, &params.quality)
    }

    #[test]
    fn a_crease_that_crosses_text_lines_is_still_found() {
        let (w, h) = (600, 480);
        // Inside a text band (row 6..17 of line 10 = 240 + 10) and in a gap between lines.
        for at in [250usize, 262] {
            let f = run_with_ink(&text_page(w, h, at, true), w, h);
            assert_eq!(
                (f.horizontal, f.vertical),
                (1, 0),
                "horizontal at {at}: {f:?}"
            );
        }
        for at in [200usize, 307] {
            let f = run_with_ink(&text_page(w, h, at, false), w, h);
            assert_eq!(
                (f.horizontal, f.vertical),
                (0, 1),
                "vertical at {at}: {f:?}"
            );
        }
    }

    #[test]
    fn a_partial_width_shadow_is_not_a_fold() {
        let (w, h) = (300, 240);
        let mut g = vec![220u8; w * h];
        for y in 118..122 {
            for x in 0..120 {
                g[y * w + x] -= 25;
            }
        }
        assert_eq!(run(&g, w, h, 180.0).count(), 0);
    }

    #[test]
    fn ruled_paper_with_many_valleys_is_not_folded() {
        let (w, h) = (300, 240);
        let creases: Vec<(usize, u8, u8)> = (1..8).map(|k| (k * 28, 25, 0)).collect();
        let g = page(w, h, &creases, false);
        let f = run(&g, w, h, 180.0);
        assert_eq!(f.count(), 0);
        assert!(f.candidates >= 5, "candidates {}", f.candidates);
    }

    #[test]
    fn too_shallow_or_too_deep_valleys_are_ignored() {
        let (w, h) = (300, 240);
        assert_eq!(
            run(&page(w, h, &[(120, 2, 0)], false), w, h, 180.0).count(),
            0
        );
        assert_eq!(
            run(&page(w, h, &[(120, 150, 0)], false), w, h, 180.0).count(),
            0
        );
    }
}
