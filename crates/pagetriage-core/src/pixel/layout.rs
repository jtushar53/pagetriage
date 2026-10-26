//! Layout segmentation: glyph components, text lines, blocks, columns, rulings and graphics. The
//! shared groundwork for script identification (M4) and handwriting detection (M5), which need to
//! look at one line or one block of the page at a time. See `docs/detectors/layout.md`.
//!
//! Pipeline: Sauvola ink mask -> shear by the page's measured skew (module M3), so lines are level
//! -> 8-connected components -> classify each component as speck,
//! glyph, ruling or graphic against the page's x-height -> horizontal run-length smoothing of the
//! glyph mask (Wong, Casey and Wahl 1982) so each text line becomes one component -> lines grouped
//! into blocks by vertical gap and horizontal overlap -> columns from the horizontal projection of
//! the lines. Boxes are in the sheared frame; [`Layout::to_page`] maps a point back. Everything is
//! integer arithmetic on the pixel grid; the only floats are the
//! x-height-relative thresholds, which use IEEE operations only, so every target gives the same
//! segmentation.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::Detection;
use crate::error::{Error, Result};
use crate::math;
use crate::params::Params;
use crate::pixel::binarize::local;
use crate::pixel::components::{self, Component};

pub const LAYOUT: &str = "layout";

/// Parameters of the layout pass; defaults and sources in `params/layout.toml`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutParams {
    pub glyph_min_area: u32,
    pub glyph_max_extent_frac: f64,
    pub xheight_min_area: u32,
    pub xheight_fallback_px: f64,
    pub rule_min_len_xh: f64,
    pub rule_max_thick_xh: f64,
    pub rule_run_xh: f64,
    pub graphic_min_xh: f64,
    pub smear_xh: f64,
    pub line_min_glyphs: u32,
    pub line_min_height_xh: f64,
    pub line_max_height_xh: f64,
    pub block_min_overlap: f64,
    pub block_gap_margin_lines: f64,
    pub block_max_gap_lines: f64,
    pub column_gap_xh: f64,
    pub column_cover_frac: f64,
    pub shear_min_deg: f64,
}

impl LayoutParams {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParams(format!("layout: {m}")));
        let lengths = [
            ("glyph_max_extent_frac", self.glyph_max_extent_frac),
            ("xheight_fallback_px", self.xheight_fallback_px),
            ("rule_min_len_xh", self.rule_min_len_xh),
            ("rule_max_thick_xh", self.rule_max_thick_xh),
            ("rule_run_xh", self.rule_run_xh),
            ("graphic_min_xh", self.graphic_min_xh),
            ("smear_xh", self.smear_xh),
            ("line_min_height_xh", self.line_min_height_xh),
            ("line_max_height_xh", self.line_max_height_xh),
            ("block_min_overlap", self.block_min_overlap),
            ("block_gap_margin_lines", self.block_gap_margin_lines),
            ("block_max_gap_lines", self.block_max_gap_lines),
            ("column_gap_xh", self.column_gap_xh),
            ("column_cover_frac", self.column_cover_frac),
            ("shear_min_deg", self.shear_min_deg),
        ];
        for (name, v) in lengths {
            if !v.is_finite() || v < 0.0 || v > 1.0e4 {
                return bad(&format!("{name} must be finite and in [0, 10000], got {v}"));
            }
        }
        if !(self.glyph_max_extent_frac > 0.0 && self.glyph_max_extent_frac <= 1.0) {
            return bad("glyph_max_extent_frac must be in (0, 1]");
        }
        if self.xheight_fallback_px < 1.0 {
            return bad("xheight_fallback_px must be at least 1");
        }
        if self.block_min_overlap > 1.0 || self.column_cover_frac >= 1.0 {
            return bad("block_min_overlap must be in [0, 1] and column_cover_frac in [0, 1)");
        }
        if self.line_min_height_xh > self.line_max_height_xh {
            return bad("line_min_height_xh must not exceed line_max_height_xh");
        }
        if self.glyph_min_area == 0 || self.line_min_glyphs == 0 {
            return bad("glyph_min_area and line_min_glyphs must be at least 1");
        }
        Ok(())
    }
}

/// An axis-aligned box. Corners are inclusive pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl Rect {
    pub fn width(&self) -> u32 {
        self.x1 - self.x0 + 1
    }

    pub fn height(&self) -> u32 {
        self.y1 - self.y0 + 1
    }

    fn of(c: &Component) -> Self {
        Self {
            x0: c.x0,
            y0: c.y0,
            x1: c.x1,
            y1: c.y1,
        }
    }

    fn union(&self, o: &Rect) -> Rect {
        Rect {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }
}

/// One text line: the box of a smoothed component that holds glyphs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextLine {
    pub bbox: Rect,
    pub glyphs: u32,
    /// Ink pixels of the glyphs in the line.
    pub ink_area: u64,
    /// Median bounding-box height of the line's glyphs.
    pub glyph_height: u32,
    /// Index into [`Layout::blocks`].
    pub block: usize,
}

/// Lines that continue each other: a paragraph, a column of a table cell, a heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub bbox: Rect,
    /// Indices into [`Layout::lines`], top to bottom.
    pub lines: Vec<usize>,
}

/// One glyph-class component with the line it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlyphBox {
    pub bbox: Rect,
    pub area: u32,
    /// Index into [`Layout::lines`]; `None` when the glyph's smoothed component was not kept as a
    /// line (too few glyphs, wrong height).
    pub line: Option<usize>,
}

/// The segmentation of one page.
#[derive(Debug, Clone)]
pub struct Layout {
    /// Size of the levelled frame the boxes are in: the page size plus the rows and columns the two
    /// shears add. Equal to the page size when no shear was applied.
    pub width: usize,
    pub height: usize,
    /// Size of the page itself.
    pub page_width: usize,
    pub page_height: usize,
    /// The skew the page was levelled by, degrees, in the sense of `quality.skew_deg`; 0 when it
    /// was below `shear_min_deg`.
    pub skew_deg: f64,
    /// The page is levelled by two shears, which together turn it by `-skew_deg` (a rotation, up to
    /// terms of the order of the skew squared). First, rows: page pixel `(x, y)` moves to row
    /// `y + shear[x]`, which makes text lines horizontal.
    pub shear: Vec<i32>,
    /// Then, columns: a pixel on levelled row `r` moves to column `x + shear_x[r]`, which makes
    /// vertical rulings and column edges vertical.
    pub shear_x: Vec<i32>,
    /// Area-weighted median glyph height in pixels: the unit of every length in `LayoutParams`.
    pub xheight: f64,
    pub glyph_count: u32,
    pub glyph_ink_pixels: u64,
    pub ink_pixels: u64,
    /// Sorted top to bottom, then left to right.
    pub lines: Vec<TextLine>,
    /// Every glyph-class component, in raster order of its first pixel.
    pub glyphs: Vec<GlyphBox>,
    /// The ink mask of the sheared frame (`width * height`, 1 = ink), what the boxes index into.
    pub ink: Vec<u8>,
    pub blocks: Vec<Block>,
    pub rules: Vec<Rect>,
    pub graphics: Vec<Rect>,
    /// Horizontal runs of the page that hold text lines, left to right.
    pub columns: Vec<(u32, u32)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Speck,
    Glyph,
    Rule,
    Graphic,
}

fn median_u32(values: &mut [u32]) -> u32 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    values[values.len() / 2]
}

/// Area-weighted median of the heights of `(height, area)` pairs: the height at which half of the
/// ink area has been seen. The weighting keeps a crowd of small marks from dragging it down.
fn weighted_median_height(mut by_height: Vec<(u32, u32)>) -> Option<u32> {
    by_height.sort_unstable();
    let total: u64 = by_height.iter().map(|&(_, a)| u64::from(a)).sum();
    if total == 0 {
        return None;
    }
    let mut acc = 0u64;
    for &(height, area) in &by_height {
        acc += u64::from(area);
        if acc * 2 >= total {
            return Some(height);
        }
    }
    by_height.last().map(|&(h, _)| h)
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

fn union(parent: &mut [usize], a: usize, b: usize) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        parent[ra.max(rb)] = ra.min(rb);
    }
}

impl Layout {
    /// The gray page moved into the levelled frame, `width * height`, with white where the shears
    /// added pixels. `gray` is the page's own buffer (`page_width * page_height`).
    pub fn level_gray(&self, gray: &[u8]) -> Vec<u8> {
        let mut out = vec![255u8; self.width * self.height];
        for y in 0..self.page_height {
            for x in 0..self.page_width {
                let ny = (y as i64 + i64::from(self.shear[x])) as usize;
                let nx = (x as i64 + i64::from(self.shear_x[ny])) as usize;
                out[ny * self.width + nx] = gray[y * self.page_width + x];
            }
        }
        out
    }

    /// Map a point of the levelled frame back to page coordinates (not clamped: a point in the
    /// padding the shears added maps outside the page).
    pub fn to_page(&self, x: u32, y: u32) -> (i64, i64) {
        let px = i64::from(x) - i64::from(self.shear_x[y as usize]);
        let in_page = px.clamp(0, self.page_width as i64 - 1) as usize;
        (px, i64::from(y) - i64::from(self.shear[in_page]))
    }

    /// The page-coordinate bounding box `[x0, y0, x1, y1]` (inclusive, clamped to the page) of a
    /// box of the levelled frame: the box of its four corners mapped back.
    pub fn rect_to_page(&self, r: &Rect) -> [u32; 4] {
        let corners = [(r.x0, r.y0), (r.x1, r.y0), (r.x0, r.y1), (r.x1, r.y1)]
            .map(|(x, y)| self.to_page(x, y));
        let (max_x, max_y) = (self.page_width as i64 - 1, self.page_height as i64 - 1);
        let lo = |f: fn(&(i64, i64)) -> i64, m: i64| {
            corners
                .iter()
                .map(f)
                .min()
                .expect("four corners")
                .clamp(0, m) as u32
        };
        let hi = |f: fn(&(i64, i64)) -> i64, m: i64| {
            corners
                .iter()
                .map(f)
                .max()
                .expect("four corners")
                .clamp(0, m) as u32
        };
        [
            lo(|c| c.0, max_x),
            lo(|c| c.1, max_y),
            hi(|c| c.0, max_x),
            hi(|c| c.1, max_y),
        ]
    }
}

/// For each ink pixel, how many ink pixels there are in a straight line through it: `(dy, dx)` is
/// the step in the sweep direction. Returns `before + after + 1`, saturating.
fn extent_through(ink: &[u8], width: usize, height: usize, vertical: bool) -> Vec<u16> {
    let n = ink.len();
    let mut before = vec![0u16; n];
    let mut after = vec![0u16; n];
    let step = if vertical { width } else { 1 };
    for i in 0..n {
        if ink[i] != 0 {
            let first = if vertical { i < width } else { i % width == 0 };
            before[i] = if first {
                1
            } else {
                before[i - step].saturating_add(1)
            };
        }
    }
    for i in (0..n).rev() {
        if ink[i] != 0 {
            let last = if vertical {
                i + width >= n
            } else {
                i % width == width - 1
            };
            after[i] = if last {
                1
            } else {
                after[i + step].saturating_add(1)
            };
        }
    }
    let _ = height;
    before
        .iter()
        .zip(&after)
        .map(|(&b, &a)| (b.saturating_add(a)).saturating_sub(1))
        .collect()
}

/// Pixels of thin rulings, as two masks (horizontal runs, vertical runs): runs of ink at least
/// `run_min` long whose perpendicular extent at that pixel is at most `thick_max`. A pen stroke
/// that crosses a ruling makes the extent larger at its own columns, so those pixels stay and the
/// stroke stays whole. The masks are kept apart because at a crossing the end of one touches the
/// side of the other diagonally, and one labelling would join a whole grid into one component.
fn thin_ruling_pixels(
    ink: &[u8],
    width: usize,
    height: usize,
    run_min: usize,
    thick_max: usize,
) -> (Vec<u8>, Vec<u8>) {
    let mut horizontal = vec![0u8; ink.len()];
    let mut vertical_mask = vec![0u8; ink.len()];
    for vertical in [false, true] {
        let removed = if vertical {
            &mut vertical_mask
        } else {
            &mut horizontal
        };
        // Thickness across the run: vertical extent for a horizontal run, horizontal for a vertical.
        let thickness = extent_through(ink, width, height, !vertical);
        let (outer, inner) = if vertical {
            (width, height)
        } else {
            (height, width)
        };
        let at = |o: usize, i: usize| {
            if vertical {
                i * width + o
            } else {
                o * width + i
            }
        };
        // Runs are found on the mask widened by one pixel across the run, so that a ruling that
        // jogs by a pixel (a levelled rotated line wobbles) is still one run. Which pixels are
        // removed, and how thick they are, are decided on the original ink.
        let across = |o: usize, i: usize, d: isize| -> bool {
            let o2 = o as isize + d;
            o2 >= 0 && (o2 as usize) < outer && ink[at(o2 as usize, i)] != 0
        };
        let wide = |o: usize, i: usize| across(o, i, 0) || across(o, i, -1) || across(o, i, 1);
        for o in 0..outer {
            let mut i = 0;
            while i < inner {
                if !wide(o, i) {
                    i += 1;
                    continue;
                }
                let start = i;
                while i < inner && wide(o, i) {
                    i += 1;
                }
                if i - start >= run_min {
                    for k in start..i {
                        if ink[at(o, k)] != 0 && usize::from(thickness[at(o, k)]) <= thick_max {
                            removed[at(o, k)] = 1;
                        }
                    }
                }
            }
        }
    }
    (horizontal, vertical_mask)
}

/// Segment a `width * height` row-major gray page whose text lines are tilted by `skew_deg` (the
/// `quality.skew_deg` convention: positive is text rising to the right).
pub fn segment(
    gray: &[u8],
    width: usize,
    height: usize,
    skew_deg: f64,
    params: &Params,
) -> Result<Layout> {
    if width == 0 || height == 0 || gray.len() != width * height {
        return Err(Error::InvalidInput(format!(
            "gray buffer of {} bytes does not match {width}x{height}",
            gray.len()
        )));
    }
    let p = &params.layout;
    if !skew_deg.is_finite() || skew_deg.abs() > 45.0 {
        return Err(Error::InvalidInput(format!(
            "skew of {skew_deg} degrees is outside +-45"
        )));
    }
    let ink = local::sauvola(gray, width, height, &params.sauvola);

    // Level the page: move every column of the ink mask by the rows its text line has drifted,
    // x * tan(skew). The offsets are rounded once per column, so the result is fixed.
    let applied = if skew_deg.abs() >= p.shear_min_deg {
        skew_deg
    } else {
        0.0
    };
    let radians = applied * std::f64::consts::PI / 180.0;
    let slope = math::sin(radians) / math::cos(radians);
    let drift: Vec<i32> = (0..width)
        .map(|x| (x as f64 * slope).round() as i32)
        .collect();
    let base = -drift.iter().copied().min().unwrap_or(0).min(0);
    let shear: Vec<i32> = drift.iter().map(|d| d + base).collect();
    let rows_added = shear.iter().copied().max().unwrap_or(0).max(0) as usize;
    let full_h = height + rows_added;
    // Second shear: a pixel on levelled row r moves by -r * tan(skew), so vertical lines of a page
    // that was turned stand up again.
    let drift_x: Vec<i32> = (0..full_h)
        .map(|r| (-(r as f64) * slope).round() as i32)
        .collect();
    let base_x = -drift_x.iter().copied().min().unwrap_or(0).min(0);
    let shear_x: Vec<i32> = drift_x.iter().map(|d| d + base_x).collect();
    let cols_added = shear_x.iter().copied().max().unwrap_or(0).max(0) as usize;
    let full_w = width + cols_added;
    let ink = if rows_added == 0 && cols_added == 0 {
        ink
    } else {
        let mut sheared = vec![0u8; full_w * full_h];
        for y in 0..height {
            for x in 0..width {
                if ink[y * width + x] != 0 {
                    let ny = (y as i64 + i64::from(shear[x])) as usize;
                    let nx = (x as i64 + i64::from(shear_x[ny])) as usize;
                    sheared[ny * full_w + nx] = 1;
                }
            }
        }
        sheared
    };
    let (page_width, page_height) = (width, height);
    let (width, height) = (full_w, full_h);
    let mut lab = components::label_8(&ink, width, height);
    let ink_pixels: u64 = lab.components.iter().map(|c| u64::from(c.area)).sum();

    // Pass 1: x-height from everything that could be a glyph.
    let max_w = p.glyph_max_extent_frac * width as f64;
    let max_h = p.glyph_max_extent_frac * height as f64;
    let measure_xheight = |lab: &components::Labelling| {
        let candidates: Vec<(u32, u32)> = lab
            .components
            .iter()
            .filter(|c| {
                c.area >= p.xheight_min_area.max(p.glyph_min_area)
                    && f64::from(c.width()) < max_w
                    && f64::from(c.height()) < max_h
            })
            .map(|c| (c.height(), c.area))
            .collect();
        weighted_median_height(candidates)
            .map_or(p.xheight_fallback_px, f64::from)
            .max(1.0)
    };
    let mut xheight = measure_xheight(&lab);

    // Cut thin rulings out of the mask, so that text written across a ruling (a handwritten fill-in
    // on a printed line, a word touching a table edge) is not fused to it; then measure again.
    let run_min = ((p.rule_run_xh * xheight).round() as usize).max(2);
    let thick_max = ((p.rule_max_thick_xh * xheight).round() as usize).max(1);
    let (cut_h, cut_v) = thin_ruling_pixels(&ink, width, height, run_min, thick_max);
    let mut rules = Vec::new();
    let mut ink = ink;
    if cut_h.contains(&1) || cut_v.contains(&1) {
        for mask in [&cut_h, &cut_v] {
            for c in &components::label_8(mask, width, height).components {
                rules.push(Rect::of(c));
            }
        }
        for (v, (&a, &b)) in ink.iter_mut().zip(cut_h.iter().zip(&cut_v)) {
            if a != 0 || b != 0 {
                *v = 0;
            }
        }
        lab = components::label_8(&ink, width, height);
        xheight = measure_xheight(&lab);
    }

    // Pass 2: classify every component against it.
    let kinds: Vec<Kind> = lab
        .components
        .iter()
        .map(|c| {
            let (w, h) = (f64::from(c.width()), f64::from(c.height()));
            let long = p.rule_min_len_xh * xheight;
            let thin = p.rule_max_thick_xh * xheight;
            if (w >= long && h <= thin) || (h >= long && w <= thin) {
                Kind::Rule
            } else if (w >= p.graphic_min_xh * xheight && h >= p.graphic_min_xh * xheight)
                || w >= max_w
                || h >= max_h
            {
                Kind::Graphic
            } else if c.area < p.glyph_min_area {
                Kind::Speck
            } else {
                Kind::Glyph
            }
        })
        .collect();

    let mut graphics = Vec::new();
    let mut glyph_count = 0u32;
    let mut glyph_ink_pixels = 0u64;
    for (c, k) in lab.components.iter().zip(&kinds) {
        match k {
            Kind::Rule => rules.push(Rect::of(c)),
            Kind::Graphic => graphics.push(Rect::of(c)),
            Kind::Glyph => {
                glyph_count += 1;
                glyph_ink_pixels += u64::from(c.area);
            }
            Kind::Speck => {}
        }
    }

    // Glyph mask, smoothed horizontally. `first[k]` is a pixel of component k (its first in raster
    // order), used to look the component's smoothed label up afterwards.
    let gap = (p.smear_xh * xheight).round() as usize;
    let mut smeared = vec![0u8; width * height];
    let mut first = vec![usize::MAX; lab.components.len()];
    for y in 0..height {
        let mut last: Option<usize> = None;
        for x in 0..width {
            let i = y * width + x;
            let l = lab.labels[i];
            if l == 0 || kinds[(l - 1) as usize] != Kind::Glyph {
                continue;
            }
            let k = (l - 1) as usize;
            if first[k] == usize::MAX {
                first[k] = i;
            }
            if let Some(lx) = last.filter(|&lx| x - lx - 1 <= gap) {
                for v in &mut smeared[y * width + lx..i] {
                    *v = 1;
                }
            }
            smeared[i] = 1;
            last = Some(x);
        }
    }
    let rows = components::label_8(&smeared, width, height);

    // Collect the glyphs of each smoothed component.
    struct Acc {
        heights: Vec<u32>,
        area: u64,
    }
    let mut acc: Vec<Acc> = rows
        .components
        .iter()
        .map(|_| Acc {
            heights: Vec::new(),
            area: 0,
        })
        .collect();
    for (k, c) in lab.components.iter().enumerate() {
        if kinds[k] == Kind::Glyph {
            let l = rows.labels[first[k]];
            let a = &mut acc[(l - 1) as usize];
            a.heights.push(c.height());
            a.area += u64::from(c.area);
        }
    }

    let mut kept: Vec<(TextLine, u32)> = Vec::new();
    for (ri, (c, mut a)) in rows.components.iter().zip(acc).enumerate() {
        let h = f64::from(c.height());
        if (a.heights.len() as u32) < p.line_min_glyphs
            || h < p.line_min_height_xh * xheight
            || h > p.line_max_height_xh * xheight
        {
            continue;
        }
        kept.push((
            TextLine {
                bbox: Rect::of(c),
                glyphs: a.heights.len() as u32,
                ink_area: a.area,
                glyph_height: median_u32(&mut a.heights),
                block: 0,
            },
            ri as u32 + 1,
        ));
    }
    kept.sort_by_key(|(l, _)| (l.bbox.y0, l.bbox.x0, l.bbox.y1, l.bbox.x1));
    let mut line_of_label = vec![None; rows.components.len() + 1];
    for (idx, (_, label)) in kept.iter().enumerate() {
        line_of_label[*label as usize] = Some(idx);
    }
    let glyphs: Vec<GlyphBox> = lab
        .components
        .iter()
        .enumerate()
        .filter(|(k, _)| kinds[*k] == Kind::Glyph)
        .map(|(k, c)| GlyphBox {
            bbox: Rect::of(c),
            area: c.area,
            line: line_of_label[rows.labels[first[k]] as usize],
        })
        .collect();
    let mut lines: Vec<TextLine> = kept.into_iter().map(|(l, _)| l).collect();

    // Blocks. Candidate pairs are lines that overlap horizontally; the typical gap of the page is
    // the median, over lines, of the gap to the nearest candidate below.
    let mut line_heights: Vec<u32> = lines.iter().map(|l| l.bbox.height()).collect();
    let line_height = f64::from(median_u32(&mut line_heights));
    let hard_cap = (p.block_max_gap_lines * line_height).round() as i64;
    let mut edges: Vec<(usize, usize, i64)> = Vec::new();
    let mut nearest: Vec<u32> = Vec::new();
    for i in 0..lines.len() {
        let mut best: Option<i64> = None;
        for j in i + 1..lines.len() {
            let (a, b) = (&lines[i].bbox, &lines[j].bbox);
            let gap = i64::from(b.y0) - i64::from(a.y1) - 1;
            // Sorted by top edge, so once a line starts too far below `a` all later ones do too.
            if gap > hard_cap {
                break;
            }
            let overlap = i64::from(a.x1.min(b.x1)) - i64::from(a.x0.max(b.x0)) + 1;
            let narrow = a.width().min(b.width());
            if overlap > 0 && overlap as f64 >= p.block_min_overlap * f64::from(narrow) {
                edges.push((i, j, gap));
                best = Some(best.map_or(gap, |g| g.min(gap)));
            }
        }
        if let Some(g) = best {
            nearest.push(g.max(0) as u32);
        }
    }
    let typical = f64::from(median_u32(&mut nearest));
    let max_gap = (typical + p.block_gap_margin_lines * line_height).round() as i64;
    let mut parent: Vec<usize> = (0..lines.len()).collect();
    for &(i, j, gap) in &edges {
        if gap <= max_gap {
            union(&mut parent, i, j);
        }
    }
    let mut blocks: Vec<Block> = Vec::new();
    let mut block_of_root = vec![usize::MAX; lines.len()];
    for (i, line) in lines.iter_mut().enumerate() {
        let root = find(&mut parent, i);
        if block_of_root[root] == usize::MAX {
            block_of_root[root] = blocks.len();
            blocks.push(Block {
                bbox: line.bbox,
                lines: Vec::new(),
            });
        }
        let b = block_of_root[root];
        line.block = b;
        blocks[b].bbox = blocks[b].bbox.union(&line.bbox);
        blocks[b].lines.push(i);
    }

    // Columns: runs of x with enough lines over them, gaps shorter than the column gap closed.
    let mut cover = vec![0i32; width + 1];
    for l in &lines {
        cover[l.bbox.x0 as usize] += 1;
        cover[l.bbox.x1 as usize + 1] -= 1;
    }
    let col_gap = (p.column_gap_xh * xheight).round() as u32;
    let mut depths = Vec::with_capacity(width);
    let mut depth = 0i32;
    for d in cover.iter().take(width) {
        depth += d;
        depths.push(depth);
    }
    let busiest = depths.iter().copied().max().unwrap_or(0);
    let mut columns: Vec<(u32, u32)> = Vec::new();
    for (x, &depth) in depths.iter().enumerate() {
        if depth > 0 && f64::from(depth) > p.column_cover_frac * f64::from(busiest) {
            let x = x as u32;
            match columns.last_mut() {
                Some(last) if x - last.1 - 1 < col_gap => last.1 = x,
                _ => columns.push((x, x)),
            }
        }
    }

    Ok(Layout {
        width,
        height,
        page_width,
        page_height,
        skew_deg: applied,
        shear,
        shear_x,
        xheight,
        glyph_count,
        glyph_ink_pixels,
        ink_pixels,
        lines,
        glyphs,
        ink,
        blocks,
        rules,
        graphics,
        columns,
    })
}

/// Run the segmentation and report its summary numbers as features. It produces no finding: the
/// segmentation itself is consumed by the detectors built on it.
pub fn detect(
    gray: &[u8],
    width: usize,
    height: usize,
    skew_deg: f64,
    params: &Params,
) -> Result<Detection> {
    Ok(summarize(&segment(gray, width, height, skew_deg, params)?))
}

/// The summary numbers of a segmentation, as [`detect`] reports them.
pub fn summarize(layout: &Layout) -> Detection {
    let (width, height) = (layout.width, layout.height);
    let xh = layout.xheight;
    let page_px = (width * height) as f64;

    // Line pitch: distance between the vertical centres of consecutive lines of one block.
    let mut pitches: Vec<u32> = Vec::new();
    for b in &layout.blocks {
        for w in b.lines.windows(2) {
            let (a, c) = (&layout.lines[w[0]].bbox, &layout.lines[w[1]].bbox);
            let centre = |r: &Rect| r.y0 + r.y1;
            pitches.push(centre(c).saturating_sub(centre(a)));
        }
    }
    let pitch_xh = if pitches.is_empty() {
        0.0
    } else {
        f64::from(median_u32(&mut pitches)) / 2.0 / xh
    };
    let mut heights: Vec<u32> = layout.lines.iter().map(|l| l.glyph_height).collect();

    let mut features = BTreeMap::new();
    let mut put = |k: &str, v: f64| {
        features.insert(k.to_owned(), v);
    };
    put("xheight_px", xh);
    put("skew_deg_applied", layout.skew_deg);
    put("glyphs", f64::from(layout.glyph_count));
    put("lines", layout.lines.len() as f64);
    put("blocks", layout.blocks.len() as f64);
    put("columns", layout.columns.len() as f64);
    put("rules", layout.rules.len() as f64);
    put("graphics", layout.graphics.len() as f64);
    put(
        "glyph_ink_fraction",
        layout.glyph_ink_pixels as f64 / page_px,
    );
    put(
        "text_share_of_ink",
        if layout.ink_pixels == 0 {
            0.0
        } else {
            layout.glyph_ink_pixels as f64 / layout.ink_pixels as f64
        },
    );
    put("median_line_pitch_xh", pitch_xh);
    put(
        "median_glyph_height_xh",
        f64::from(median_u32(&mut heights)) / xh,
    );
    Detection {
        findings: Vec::new(),
        features,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paper with rectangles of ink: `(x0, y0, x1, y1)` inclusive.
    fn page(w: usize, h: usize, ink: &[(usize, usize, usize, usize)]) -> Vec<u8> {
        let mut g = vec![240u8; w * h];
        for &(x0, y0, x1, y1) in ink {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    g[y * w + x] = 20;
                }
            }
        }
        g
    }

    /// `n` words per line, each 4 glyphs of 3x8 with 3-px gaps (word space 12 px), `lines` lines
    /// 17 px apart starting at `(x, y)`.
    fn text_block(
        x: usize,
        y: usize,
        lines: usize,
        words: usize,
    ) -> Vec<(usize, usize, usize, usize)> {
        let mut out = Vec::new();
        for l in 0..lines {
            let mut cx = x;
            for _ in 0..words {
                for _ in 0..4 {
                    out.push((cx, y + l * 17, cx + 2, y + l * 17 + 7));
                    cx += 6;
                }
                cx += 6;
            }
        }
        out
    }

    fn params() -> Params {
        Params::defaults().unwrap()
    }

    #[test]
    fn one_paragraph_is_one_block_of_n_lines() {
        let ink = text_block(20, 20, 5, 4);
        let g = page(200, 140, &ink);
        let l = segment(&g, 200, 140, 0.0, &params()).unwrap();
        assert_eq!(l.lines.len(), 5);
        assert_eq!(l.blocks.len(), 1);
        assert_eq!(l.blocks[0].lines, vec![0, 1, 2, 3, 4]);
        assert_eq!(l.columns.len(), 1);
        assert_eq!(l.xheight, 8.0);
        assert!(l.rules.is_empty() && l.graphics.is_empty());
        assert_eq!(l.lines[0].glyphs, 16);
        assert_eq!(l.lines[0].glyph_height, 8);
    }

    #[test]
    fn two_columns_are_two_blocks_and_two_columns() {
        let mut ink = text_block(10, 20, 4, 2);
        ink.extend(text_block(130, 20, 4, 2));
        let g = page(250, 100, &ink);
        let l = segment(&g, 250, 100, 0.0, &params()).unwrap();
        assert_eq!(l.blocks.len(), 2);
        assert_eq!(l.columns.len(), 2);
        assert_eq!(l.lines.len(), 8);
    }

    #[test]
    fn paragraph_gap_splits_blocks() {
        let mut ink = text_block(20, 10, 2, 4);
        ink.extend(text_block(20, 70, 2, 4));
        let g = page(200, 120, &ink);
        let l = segment(&g, 200, 120, 0.0, &params()).unwrap();
        assert_eq!(l.lines.len(), 4);
        assert_eq!(l.blocks.len(), 2);
    }

    #[test]
    fn a_ruling_is_not_text_and_does_not_join_lines() {
        let mut ink = text_block(20, 10, 1, 4);
        ink.push((10, 30, 190, 31));
        ink.extend(text_block(20, 36, 1, 4));
        let g = page(200, 70, &ink);
        let l = segment(&g, 200, 70, 0.0, &params()).unwrap();
        assert_eq!(l.rules.len(), 1);
        assert_eq!(l.lines.len(), 2);
    }

    #[test]
    fn a_filled_frame_is_a_graphic() {
        let mut ink = text_block(20, 10, 3, 3);
        ink.push((120, 10, 190, 70));
        let g = page(200, 90, &ink);
        let l = segment(&g, 200, 90, 0.0, &params()).unwrap();
        assert_eq!(l.graphics.len(), 1);
        assert_eq!(l.lines.len(), 3);
    }

    #[test]
    fn specks_and_blank_pages_give_an_empty_layout() {
        let g = page(100, 100, &[(10, 10, 10, 10), (50, 60, 51, 60)]);
        let l = segment(&g, 100, 100, 0.0, &params()).unwrap();
        assert!(l.lines.is_empty() && l.blocks.is_empty() && l.columns.is_empty());
        assert_eq!(l.xheight, params().layout.xheight_fallback_px);
        let d = detect(&[240u8; 64], 8, 8, 0.0, &params()).unwrap();
        assert_eq!(d.features["lines"], 0.0);
        assert!(d.findings.is_empty());
    }

    #[test]
    fn detect_reports_pitch_in_xheights() {
        let ink = text_block(20, 20, 5, 4);
        let g = page(200, 140, &ink);
        let d = detect(&g, 200, 140, 0.0, &params()).unwrap();
        assert_eq!(d.features["median_line_pitch_xh"], 17.0 / 8.0);
        assert_eq!(d.features["lines"], 5.0);
        assert_eq!(d.features["median_glyph_height_xh"], 1.0);
    }

    #[test]
    fn segmentation_is_repeatable() {
        let g = page(200, 140, &text_block(20, 20, 5, 4));
        let a = detect(&g, 200, 140, 0.0, &params()).unwrap().features;
        let b = detect(&g, 200, 140, 0.0, &params()).unwrap().features;
        assert_eq!(a, b);
    }

    #[test]
    fn a_stroke_crossing_a_ruling_stays_text_and_the_ruling_is_reported() {
        // Text, a 1-px rule 200 px long below it, and a descender that crosses the rule.
        let mut ink = text_block(20, 10, 1, 4);
        ink.push((10, 36, 209, 36));
        ink.push((50, 20, 52, 44)); // a tall stroke through the rule
        ink.extend(text_block(20, 50, 1, 4));
        let g = page(220, 80, &ink);
        let l = segment(&g, 220, 80, 0.0, &params()).unwrap();
        // The crossing stroke splits the ruling in two pieces; together they span the rule.
        assert_eq!(l.rules.len(), 2, "{:?}", l.rules);
        assert!(l.rules.iter().map(Rect::width).sum::<u32>() >= 150);
        assert_eq!(l.lines.len(), 2);
        // The rule no longer fuses the two text lines or the stroke to the rule.
        assert!(l.graphics.is_empty());
    }

    /// Lines of 4-px-wide stems along a slope: `rise` rows per 100 columns, text rising to the right.
    fn tilted_page(rise: i32) -> Vec<u8> {
        let (w, h) = (400usize, 200usize);
        let mut g = vec![240u8; w * h];
        for line in 0..4usize {
            for stem in 0..60usize {
                let x = 20 + stem * 6;
                let y = 150 - line * 34 - (x as i32 * rise / 100) as usize;
                for dy in 0..9 {
                    for dx in 0..3 {
                        g[(y + dy) * w + x + dx] = 20;
                    }
                }
            }
        }
        g
    }

    #[test]
    fn a_tilted_page_is_levelled_by_the_measured_skew() {
        // Rising 5 rows per 100 columns is atan(0.05) = 2.862 degrees.
        let g = tilted_page(5);
        let skew = 2.862;
        let level = segment(&g, 400, 200, skew, &params()).unwrap();
        assert_eq!(level.lines.len(), 4, "levelled");
        assert_eq!(level.skew_deg, skew);
        assert!(level.lines.iter().all(|l| l.bbox.height() <= 12));
        // Without the correction each line's box is a slanted strip, about three times as tall.
        let tilted = segment(&g, 400, 200, 0.0, &params()).unwrap();
        assert!(tilted.lines.iter().all(|l| l.bbox.height() > 20));
        // Mapping a corner back lands inside the page.
        let l = &level.lines[0].bbox;
        let (x, y) = level.to_page(l.x0, l.y0);
        assert!((0..400).contains(&x) && (0..200).contains(&y), "({x}, {y})");
    }

    #[test]
    fn skew_below_the_threshold_changes_nothing() {
        let g = page(200, 140, &text_block(20, 20, 5, 4));
        let a = segment(&g, 200, 140, 0.0, &params()).unwrap();
        let b = segment(&g, 200, 140, 0.1, &params()).unwrap();
        assert_eq!((b.skew_deg, b.height), (0.0, 140));
        assert_eq!(a.lines, b.lines);
        assert!(segment(&g, 200, 140, f64::NAN, &params()).is_err());
        assert!(segment(&g, 200, 140, 60.0, &params()).is_err());
    }

    #[test]
    fn bad_buffers_are_rejected() {
        assert!(segment(&[0; 5], 2, 2, 0.0, &params()).is_err());
        assert!(segment(&[], 0, 0, 0.0, &params()).is_err());
    }

    #[test]
    fn params_validation_rejects_nonsense() {
        let mut p = params().layout;
        assert!(p.validate().is_ok());
        p.smear_xh = f64::NAN;
        assert!(p.validate().is_err());
        let mut p = params().layout;
        p.block_min_overlap = 1.5;
        assert!(p.validate().is_err());
        let mut p = params().layout;
        p.line_min_glyphs = 0;
        assert!(p.validate().is_err());
    }
}
