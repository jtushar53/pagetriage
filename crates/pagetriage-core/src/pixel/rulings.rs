//! Rulings, tables and checkbox grids (module M9), and fold lines told apart from printed rules.
//! See `docs/detectors/rulings.md`.
//!
//! Input is what the layout pass already found: the thin rulings it cut out of the ink mask, and
//! its glyph components. Pieces of one ruling split by a crossing stroke are merged into segments;
//! a segment is a printed rule if its core is dark against the paper beside it and a fold-line
//! candidate if it is faint. Horizontal and vertical segments that meet form tables (a framed box is
//! the smallest); the rest are reported together as ruled lines. Hollow squares with four straight
//! edges, aligned in a row or column, are checkbox grids. Everything is integer geometry; the only
//! floats are thresholds in x-heights and the confidence formulas.

use serde_json::Map;

use super::Detection;
use super::layout::{Layout, Rect};
use crate::error::{Error, Result};
use crate::fusion::scorer;
use crate::math;
use crate::params::{FusionParams, Params};
use crate::schema::{Evidence, Finding, Kind};
use serde::Deserialize;

pub const RULINGS: &str = "rulings";
const DETECTOR_ID: &str = "rulings.geometry";
const CITE_RULES: &str = "module M9: printed rules are dark thin straight runs; fold lines are faint ones (cf. M3 folds)";
const CITE_TABLE: &str =
    "module M9: a table is a connected set of horizontal and vertical rulings enclosing cells";
const CITE_BOXES: &str = "module M9: a checkbox is a hollow square with four straight edges";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RulingsParams {
    pub merge_across_xh: f64,
    pub merge_gap_xh: f64,
    pub rule_min_contrast: f64,
    pub fold_min_length_in: f64,
    pub join_tol_xh: f64,
    pub table_min_horizontal: usize,
    pub table_min_vertical: usize,
    pub line_cluster_xh: f64,
    pub table_prior: f64,
    pub table_cells_coef: f64,
    pub table_cells_cap: f64,
    pub table_junction_coef: f64,
    pub min_free_rules: usize,
    pub free_rules_prior: f64,
    pub free_rules_coef: f64,
    pub free_rules_cap: f64,
    pub box_min_xh: f64,
    pub box_min_in: f64,
    pub box_max_xh: f64,
    pub box_overshoot_fraction: f64,
    pub box_aspect_tol: f64,
    pub box_tolerance_px: usize,
    pub box_edge_coverage: f64,
    pub box_max_inner_ink: f64,
    pub box_checked_ink: f64,
    pub grid_align_xh: f64,
    pub grid_min_boxes: usize,
    pub grid_prior: f64,
    pub grid_coef: f64,
    pub grid_cap: f64,
}

impl RulingsParams {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParams(format!("rulings: {m}")));
        let all = [
            ("merge_across_xh", self.merge_across_xh),
            ("merge_gap_xh", self.merge_gap_xh),
            ("rule_min_contrast", self.rule_min_contrast),
            ("fold_min_length_in", self.fold_min_length_in),
            ("join_tol_xh", self.join_tol_xh),
            ("line_cluster_xh", self.line_cluster_xh),
            ("table_prior", self.table_prior),
            ("table_cells_coef", self.table_cells_coef),
            ("table_cells_cap", self.table_cells_cap),
            ("table_junction_coef", self.table_junction_coef),
            ("free_rules_prior", self.free_rules_prior),
            ("free_rules_coef", self.free_rules_coef),
            ("free_rules_cap", self.free_rules_cap),
            ("box_min_xh", self.box_min_xh),
            ("box_min_in", self.box_min_in),
            ("box_max_xh", self.box_max_xh),
            ("box_overshoot_fraction", self.box_overshoot_fraction),
            ("box_aspect_tol", self.box_aspect_tol),
            ("box_edge_coverage", self.box_edge_coverage),
            ("box_max_inner_ink", self.box_max_inner_ink),
            ("box_checked_ink", self.box_checked_ink),
            ("grid_align_xh", self.grid_align_xh),
            ("grid_prior", self.grid_prior),
            ("grid_coef", self.grid_coef),
            ("grid_cap", self.grid_cap),
        ];
        for (name, v) in all {
            if !v.is_finite() || v.abs() > 1.0e6 {
                return bad(&format!("{name} must be finite and within 1e6, got {v}"));
            }
        }
        for (name, v) in [
            ("merge_across_xh", self.merge_across_xh),
            ("merge_gap_xh", self.merge_gap_xh),
            ("join_tol_xh", self.join_tol_xh),
            ("line_cluster_xh", self.line_cluster_xh),
            ("box_min_xh", self.box_min_xh),
            ("box_min_in", self.box_min_in),
            ("grid_align_xh", self.grid_align_xh),
            ("fold_min_length_in", self.fold_min_length_in),
        ] {
            if v < 0.0 {
                return bad(&format!("{name} must be >= 0"));
            }
        }
        if self.box_max_xh < self.box_min_xh {
            return bad("box_max_xh must be at least box_min_xh");
        }
        if !(0.0..=1.0).contains(&self.box_edge_coverage)
            || !(0.0..=1.0).contains(&self.box_max_inner_ink)
            || !(0.0..=1.0).contains(&self.box_checked_ink)
            || !(0.0..=1.0).contains(&self.box_aspect_tol)
            || !(0.0..=1.0).contains(&self.box_overshoot_fraction)
            || self.box_tolerance_px > 3
        {
            return bad(
                "box_edge_coverage, box_max_inner_ink, box_checked_ink and box_aspect_tol must be in [0, 1]",
            );
        }
        if self.table_min_horizontal == 0 || self.table_min_vertical == 0 || self.grid_min_boxes < 2
        {
            return bad(
                "table_min_horizontal and table_min_vertical must be >= 1 and grid_min_boxes >= 2",
            );
        }
        Ok(())
    }
}

/// One ruling after merging the pieces of the layout pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub rect: Rect,
    pub horizontal: bool,
    /// Gray levels between the paper beside the segment and its darkest cross-section, averaged
    /// along it.
    pub contrast: f64,
}

impl Segment {
    pub fn length(&self) -> u32 {
        if self.horizontal {
            self.rect.width()
        } else {
            self.rect.height()
        }
    }

    fn across_centre2(&self) -> i64 {
        // Twice the centre, so that it stays an integer.
        if self.horizontal {
            i64::from(self.rect.y0 + self.rect.y1)
        } else {
            i64::from(self.rect.x0 + self.rect.x1)
        }
    }

    fn along(&self) -> (i64, i64) {
        if self.horizontal {
            (i64::from(self.rect.x0), i64::from(self.rect.x1))
        } else {
            (i64::from(self.rect.y0), i64::from(self.rect.y1))
        }
    }
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

fn union_rect(a: &Rect, b: &Rect) -> Rect {
    Rect {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    }
}

fn median_u8(v: &mut [u8]) -> f64 {
    if v.is_empty() {
        return 255.0;
    }
    v.sort_unstable();
    f64::from(v[v.len() / 2])
}

/// Merge the layout's ruling pieces into segments and measure each one's contrast on the levelled
/// gray page. A piece wider than tall is horizontal.
pub fn segments(layout: &Layout, levelled_gray: &[u8], p: &RulingsParams) -> Vec<Segment> {
    let (w, h) = (layout.width, layout.height);
    let xh = layout.xheight;
    let across_tol = (p.merge_across_xh * xh).round() as i64;
    let gap_tol = (p.merge_gap_xh * xh).round() as i64;
    let pieces: Vec<Segment> = layout
        .rules
        .iter()
        .map(|r| Segment {
            rect: *r,
            horizontal: r.width() >= r.height(),
            contrast: 0.0,
        })
        .collect();
    let mut parent: Vec<usize> = (0..pieces.len()).collect();
    for i in 0..pieces.len() {
        for j in i + 1..pieces.len() {
            let (a, b) = (&pieces[i], &pieces[j]);
            if a.horizontal != b.horizontal {
                continue;
            }
            // Centres are doubled, so compare against twice the tolerance.
            if (a.across_centre2() - b.across_centre2()).abs() > 2 * across_tol {
                continue;
            }
            let ((a0, a1), (b0, b1)) = (a.along(), b.along());
            if b0.max(a0) - a1.min(b1) - 1 <= gap_tol {
                union(&mut parent, i, j);
            }
        }
    }
    let mut merged: Vec<Option<Segment>> = vec![None; pieces.len()];
    for (i, piece) in pieces.iter().enumerate() {
        let root = find(&mut parent, i);
        merged[root] = Some(match merged[root] {
            None => *piece,
            Some(s) => Segment {
                rect: union_rect(&s.rect, &piece.rect),
                ..s
            },
        });
    }
    let mut out: Vec<Segment> = merged.into_iter().flatten().collect();
    for s in &mut out {
        s.contrast = contrast(levelled_gray, w, h, s);
    }
    out.sort_by_key(|s| (s.rect.y0, s.rect.x0, s.rect.y1, s.rect.x1));
    out
}

/// Paper beside the segment (median of two strips 3 to 6 px away on either side) minus the mean,
/// along the segment, of its darkest cross-section pixel.
fn contrast(gray: &[u8], w: usize, h: usize, s: &Segment) -> f64 {
    let r = &s.rect;
    let (x0, y0, x1, y1) = (r.x0 as usize, r.y0 as usize, r.x1 as usize, r.y1 as usize);
    let mut paper: Vec<u8> = Vec::new();
    let mut core: Vec<f64> = Vec::new();
    if s.horizontal {
        for x in x0..=x1 {
            let col_min = (y0..=y1).map(|y| gray[y * w + x]).min().unwrap_or(255);
            core.push(f64::from(col_min));
            for y in (y0.saturating_sub(6)..y0.saturating_sub(2)).chain(y1 + 3..(y1 + 7).min(h)) {
                paper.push(gray[y * w + x]);
            }
        }
    } else {
        for y in y0..=y1 {
            let row_min = (x0..=x1).map(|x| gray[y * w + x]).min().unwrap_or(255);
            core.push(f64::from(row_min));
            for x in (x0.saturating_sub(6)..x0.saturating_sub(2)).chain(x1 + 3..(x1 + 7).min(w)) {
                paper.push(gray[y * w + x]);
            }
        }
    }
    median_u8(&mut paper) - math::pairwise_sum(&core) / core.len() as f64
}

/// A table: a connected group of segments with enough of each orientation.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub bbox: Rect,
    pub rows: usize,
    pub cols: usize,
    pub segments: usize,
    /// Share of segment ends that touch a segment of the other orientation in the group.
    pub junction_share: f64,
}

fn cluster_count(mut positions: Vec<i64>, tol: i64) -> usize {
    positions.sort_unstable();
    let mut count = 0;
    let mut last = i64::MIN;
    for v in positions {
        if count == 0 || v - last > tol {
            count += 1;
        }
        last = v;
    }
    count
}

/// Group `rules` (segments that passed the contrast test) into tables; returns the tables and the
/// indices of the segments left over.
pub fn tables(rules: &[Segment], xheight: f64, p: &RulingsParams) -> (Vec<Table>, Vec<usize>) {
    let join = (p.join_tol_xh * xheight).round() as i64;
    let meet = |h: &Segment, v: &Segment| -> bool {
        // The vertical line's x lies within the horizontal's extent and the horizontal's y within
        // the vertical's extent, each widened by the tolerance.
        let vx2 = v.across_centre2();
        let hy2 = h.across_centre2();
        vx2 >= 2 * (i64::from(h.rect.x0) - join)
            && vx2 <= 2 * (i64::from(h.rect.x1) + join)
            && hy2 >= 2 * (i64::from(v.rect.y0) - join)
            && hy2 <= 2 * (i64::from(v.rect.y1) + join)
    };
    let mut parent: Vec<usize> = (0..rules.len()).collect();
    for i in 0..rules.len() {
        for j in i + 1..rules.len() {
            let (a, b) = (&rules[i], &rules[j]);
            if a.horizontal != b.horizontal {
                let (h, v) = if a.horizontal { (a, b) } else { (b, a) };
                if meet(h, v) {
                    union(&mut parent, i, j);
                }
            }
        }
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut group_of_root = vec![usize::MAX; rules.len()];
    for i in 0..rules.len() {
        let root = find(&mut parent, i);
        if group_of_root[root] == usize::MAX {
            group_of_root[root] = groups.len();
            groups.push(Vec::new());
        }
        groups[group_of_root[root]].push(i);
    }
    let cluster = (p.line_cluster_xh * xheight).round() as i64;
    let mut tables = Vec::new();
    let mut leftover = Vec::new();
    for g in groups {
        let hs: Vec<&Segment> = g
            .iter()
            .map(|&i| &rules[i])
            .filter(|s| s.horizontal)
            .collect();
        let vs: Vec<&Segment> = g
            .iter()
            .map(|&i| &rules[i])
            .filter(|s| !s.horizontal)
            .collect();
        if hs.len() < p.table_min_horizontal || vs.len() < p.table_min_vertical {
            leftover.extend(g);
            continue;
        }
        let row_lines = cluster_count(hs.iter().map(|s| s.across_centre2() / 2).collect(), cluster);
        let col_lines = cluster_count(vs.iter().map(|s| s.across_centre2() / 2).collect(), cluster);
        if row_lines < 2 || col_lines < 2 {
            leftover.extend(g);
            continue;
        }
        let bbox = g
            .iter()
            .map(|&i| rules[i].rect)
            .reduce(|a, b| union_rect(&a, &b))
            .expect("non-empty");
        // Ends that meet a segment of the other orientation.
        let (mut joined, mut ends) = (0usize, 0usize);
        for &i in &g {
            let s = &rules[i];
            let (a0, a1) = s.along();
            let across2 = s.across_centre2();
            for end in [a0, a1] {
                ends += 1;
                let hit = g
                    .iter()
                    .map(|&k| &rules[k])
                    .filter(|o| o.horizontal != s.horizontal)
                    .any(|o| {
                        let (o0, o1) = o.along();
                        let oc2 = o.across_centre2();
                        // The end lies at the other segment's across position, and that position lies
                        // within the other segment's extent.
                        (2 * end - oc2).abs() <= 2 * join + 2
                            && across2 >= 2 * (o0 - join)
                            && across2 <= 2 * (o1 + join)
                    });
                joined += usize::from(hit);
            }
        }
        tables.push(Table {
            bbox,
            rows: row_lines - 1,
            cols: col_lines - 1,
            segments: g.len(),
            junction_share: joined as f64 / ends as f64,
        });
    }
    (tables, leftover)
}

/// A hollow square with four straight edges.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CheckBox {
    pub bbox: Rect,
    pub checked: bool,
    pub edge_coverage: f64,
}

pub fn boxes(layout: &Layout, dpi: u16, p: &RulingsParams) -> Vec<CheckBox> {
    let w = layout.width;
    let xh = layout.xheight;
    let ink = |x: u32, y: u32| layout.ink[y as usize * w + x as usize] != 0;
    // A box is at least this many pixels on a side: round letters pass the other tests at 8 px.
    let min_px = p.box_min_in * f64::from(dpi);
    let mut out = Vec::new();
    for g in &layout.glyphs {
        let r = g.bbox;
        // The component may be bigger than the box (a tick that overshoots it), so the size test
        // on the component is loose and the real one is on the frame found inside.
        let slack = 1.6 * p.box_max_xh * xh;
        if f64::from(r.width()) < p.box_min_xh * xh
            || f64::from(r.height()) < p.box_min_xh * xh
            || f64::from(r.width()) > slack
            || f64::from(r.height()) > slack
        {
            continue;
        }
        // Frame: the component's box, less any outer rows and columns that are almost empty (the
        // tip of a tick poking out of the box). A letter's rounded cap is far too full to be
        // trimmed, and fails the straight-edge test below instead.
        let rows: Vec<u32> = (r.y0..=r.y1)
            .map(|y| (r.x0..=r.x1).filter(|&x| ink(x, y)).count() as u32)
            .collect();
        let cols: Vec<u32> = (r.x0..=r.x1)
            .map(|x| (r.y0..=r.y1).filter(|&y| ink(x, y)).count() as u32)
            .collect();
        let faint_row = |c: u32| f64::from(c) <= p.box_overshoot_fraction * f64::from(r.width());
        let faint_col = |c: u32| f64::from(c) <= p.box_overshoot_fraction * f64::from(r.height());
        let t = rows.iter().position(|&c| !faint_row(c));
        let bt = rows.iter().rposition(|&c| !faint_row(c));
        let l = cols.iter().position(|&c| !faint_col(c));
        let rt = cols.iter().rposition(|&c| !faint_col(c));
        let (Some(t), Some(bt), Some(l), Some(rt)) = (t, bt, l, rt) else {
            continue;
        };
        let f = Rect {
            x0: r.x0 + l as u32,
            y0: r.y0 + t as u32,
            x1: r.x0 + rt as u32,
            y1: r.y0 + bt as u32,
        };
        let (fw, fh) = (f64::from(f.width()), f64::from(f.height()));
        let min_side = (p.box_min_xh * xh).max(min_px);
        if fw < min_side || fh < min_side || fw > p.box_max_xh * xh || fh > p.box_max_xh * xh {
            continue;
        }
        if (fw - fh).abs() > p.box_aspect_tol * fw.max(fh) {
            continue;
        }
        // Four straight edges: along each side, the longest unbroken run of ink in a band of
        // `box_tolerance_px + 1` rows (columns) starting at the frame line, as a share of the side.
        // The band absorbs the staircase of a rotated raster line; unbroken is what a round letter
        // (O, D, Q, 0) lacks: its flat part is half the side or less.
        let tol = p.box_tolerance_px as u32;
        let longest = |len: u32, at: &dyn Fn(u32) -> bool| {
            let (mut best, mut run) = (0u32, 0u32);
            for i in 0..len {
                if at(i) {
                    run += 1;
                    best = best.max(run);
                } else {
                    run = 0;
                }
            }
            f64::from(best) / f64::from(len)
        };
        let (w_f, h_f) = (f.width(), f.height());
        let top = longest(w_f, &|i| {
            (0..=tol).any(|d| ink(f.x0 + i, (f.y0 + d).min(f.y1)))
        });
        let bottom = longest(w_f, &|i| {
            (0..=tol).any(|d| ink(f.x0 + i, f.y1.saturating_sub(d).max(f.y0)))
        });
        let left = longest(h_f, &|i| {
            (0..=tol).any(|d| ink((f.x0 + d).min(f.x1), f.y0 + i))
        });
        let right = longest(h_f, &|i| {
            (0..=tol).any(|d| ink(f.x1.saturating_sub(d).max(f.x0), f.y0 + i))
        });
        let coverage = [top, bottom, left, right]
            .into_iter()
            .fold(f64::INFINITY, f64::min);
        if coverage < p.box_edge_coverage {
            continue;
        }
        // Inner half of the frame.
        let (ix0, ix1) = (f.x0 + f.width() / 4, f.x1 - f.width() / 4);
        let (iy0, iy1) = (f.y0 + f.height() / 4, f.y1 - f.height() / 4);
        let (mut inked, mut total) = (0usize, 0usize);
        for y in iy0..=iy1 {
            for x in ix0..=ix1 {
                total += 1;
                inked += usize::from(ink(x, y));
            }
        }
        let inner = inked as f64 / total.max(1) as f64;
        if inner > p.box_max_inner_ink {
            continue;
        }
        out.push(CheckBox {
            bbox: f,
            checked: inner >= p.box_checked_ink,
            edge_coverage: coverage,
        });
    }
    out
}

/// Boxes aligned in a row or column, grouped.
pub fn box_grids(bs: &[CheckBox], xheight: f64, p: &RulingsParams) -> Vec<Vec<usize>> {
    let tol2 = (2.0 * p.grid_align_xh * xheight).round() as i64;
    let c2 = |b: &CheckBox| {
        (
            i64::from(b.bbox.x0 + b.bbox.x1),
            i64::from(b.bbox.y0 + b.bbox.y1),
        )
    };
    let mut parent: Vec<usize> = (0..bs.len()).collect();
    for i in 0..bs.len() {
        for j in i + 1..bs.len() {
            let ((xi, yi), (xj, yj)) = (c2(&bs[i]), c2(&bs[j]));
            let (si, sj) = (f64::from(bs[i].bbox.width()), f64::from(bs[j].bbox.width()));
            let similar = (si - sj).abs() <= 0.3 * si.max(sj);
            if similar && ((yi - yj).abs() <= tol2 || (xi - xj).abs() <= tol2) {
                union(&mut parent, i, j);
            }
        }
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut of_root = vec![usize::MAX; bs.len()];
    for i in 0..bs.len() {
        let root = find(&mut parent, i);
        if of_root[root] == usize::MAX {
            of_root[root] = groups.len();
            groups.push(Vec::new());
        }
        groups[of_root[root]].push(i);
    }
    groups.retain(|g| g.len() >= p.grid_min_boxes);
    groups
}

/// Run M9 on a segmented page. `gray` is the page's gray buffer (page frame), `dpi` its resolution.
pub fn detect(layout: &Layout, gray: &[u8], dpi: u16, params: &Params) -> Result<Detection> {
    detect_with(layout, gray, dpi, &params.rulings, &params.fusion)
}

pub fn detect_with(
    layout: &Layout,
    gray: &[u8],
    dpi: u16,
    p: &RulingsParams,
    fusion: &FusionParams,
) -> Result<Detection> {
    let levelled = layout.level_gray(gray);
    let segs = segments(layout, &levelled, p);
    let (printed, faint): (Vec<Segment>, Vec<Segment>) =
        segs.iter().partition(|s| s.contrast >= p.rule_min_contrast);
    let (tabs, leftover) = tables(&printed, layout.xheight, p);
    let bs = boxes(layout, dpi, p);
    let grids = box_grids(&bs, layout.xheight, p);

    let mut det = Detection::default();
    let f = &mut det.features;
    f.insert("segments".into(), segs.len() as f64);
    f.insert("printed_rules".into(), printed.len() as f64);
    f.insert("faint_segments".into(), faint.len() as f64);
    f.insert("tables".into(), tabs.len() as f64);
    f.insert("boxes".into(), bs.len() as f64);
    f.insert("box_grids".into(), grids.len() as f64);

    let ev =
        |feature: &str, value: f64, threshold: Option<f64>, llr: f64, note: String, cite: &str| {
            Evidence::new(
                DETECTOR_ID,
                feature,
                value,
                threshold,
                llr.clamp(-fusion.llr_clip, fusion.llr_clip),
                note,
                cite,
            )
        };
    let finding = |kind: Kind,
                   prior: f64,
                   evidence: Vec<Evidence>,
                   bbox: Rect,
                   attributes: Map<String, serde_json::Value>| {
        Finding {
            kind,
            confidence: math::round_output(scorer::confidence_capped(
                prior,
                &evidence,
                fusion.group_cap,
            )),
            bbox: Some(layout.rect_to_page(&bbox)),
            polygon: None,
            attributes,
            evidence,
        }
    };

    for t in &tabs {
        let cells = (t.rows * t.cols) as f64;
        let evidence = vec![
            ev(
                "cells",
                cells,
                Some(0.0),
                p.table_cells_coef * cells.min(p.table_cells_cap),
                format!("{} rows x {} columns", t.rows, t.cols),
                CITE_TABLE,
            )?,
            ev(
                "junction_share",
                t.junction_share,
                Some(0.5),
                p.table_junction_coef * (t.junction_share - 0.5),
                "share of ruling ends that meet a ruling of the other direction".into(),
                CITE_TABLE,
            )?,
        ];
        let mut a = Map::new();
        a.insert("rows".into(), (t.rows as u64).into());
        a.insert("cols".into(), (t.cols as u64).into());
        a.insert("segments".into(), (t.segments as u64).into());
        det.findings
            .push(finding(Kind::Table, p.table_prior, evidence, t.bbox, a));
    }

    if leftover.len() >= p.min_free_rules {
        let free: Vec<&Segment> = leftover.iter().map(|&i| &printed[i]).collect();
        let bbox = free
            .iter()
            .map(|s| s.rect)
            .reduce(|a, b| union_rect(&a, &b))
            .expect("non-empty");
        let n = free.len() as f64;
        let longest = free.iter().map(|s| s.length()).max().unwrap_or(0);
        let mean_contrast =
            math::pairwise_sum(&free.iter().map(|s| s.contrast).collect::<Vec<_>>()) / n;
        let evidence = vec![
            ev(
                "rules",
                n,
                Some(0.0),
                p.free_rules_coef * n.min(p.free_rules_cap),
                format!("{} printed rules outside tables", free.len()),
                CITE_RULES,
            )?,
            ev(
                "mean_contrast",
                mean_contrast,
                Some(p.rule_min_contrast),
                0.0,
                "gray levels darker than the paper beside the rule".into(),
                CITE_RULES,
            )?,
        ];
        let mut a = Map::new();
        a.insert(
            "horizontal".into(),
            (free.iter().filter(|s| s.horizontal).count() as u64).into(),
        );
        a.insert(
            "vertical".into(),
            (free.iter().filter(|s| !s.horizontal).count() as u64).into(),
        );
        a.insert("longest_px".into(), u64::from(longest).into());
        det.findings.push(finding(
            Kind::RulingLines,
            p.free_rules_prior,
            evidence,
            bbox,
            a,
        ));
    }

    for s in &faint {
        let inches = f64::from(s.length()) / f64::from(dpi);
        if inches < p.fold_min_length_in {
            continue;
        }
        let evidence = vec![ev(
            "contrast",
            s.contrast,
            Some(p.rule_min_contrast),
            (p.rule_min_contrast - s.contrast) / 10.0,
            format!(
                "{:.1} inches long, {:.0} gray levels darker than the paper: fainter than a printed rule",
                inches, s.contrast
            ),
            CITE_RULES,
        )?];
        let mut a = Map::new();
        a.insert(
            "orientation".into(),
            (if s.horizontal {
                "horizontal"
            } else {
                "vertical"
            })
            .into(),
        );
        a.insert("length_in".into(), math::round_output(inches).into());
        det.findings
            .push(finding(Kind::FoldLine, 0.0, evidence, s.rect, a));
    }

    for g in &grids {
        let members: Vec<&CheckBox> = g.iter().map(|&i| &bs[i]).collect();
        let n = members.len() as f64;
        let bbox = members
            .iter()
            .map(|b| b.bbox)
            .reduce(|a, b| union_rect(&a, &b))
            .expect("non-empty");
        let checked = members.iter().filter(|b| b.checked).count();
        let mean_edge =
            math::pairwise_sum(&members.iter().map(|b| b.edge_coverage).collect::<Vec<_>>()) / n;
        let evidence = vec![
            ev(
                "boxes",
                n,
                Some(p.grid_min_boxes as f64),
                p.grid_coef * n.min(p.grid_cap),
                format!("{} aligned hollow squares", members.len()),
                CITE_BOXES,
            )?,
            ev(
                "edge_coverage",
                mean_edge,
                Some(p.box_edge_coverage),
                0.0,
                "mean of each box's weakest edge".into(),
                CITE_BOXES,
            )?,
        ];
        let mut a = Map::new();
        a.insert("boxes".into(), (members.len() as u64).into());
        a.insert("checked".into(), (checked as u64).into());
        det.findings
            .push(finding(Kind::CheckboxGrid, p.grid_prior, evidence, bbox, a));
    }
    // Reading order, ties by kind name so the order is fixed.
    det.findings
        .sort_by_key(|f| (f.bbox.map(|b| (b[1], b[0])), f.kind as u8));
    Ok(det)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pixel::layout;

    fn paper(w: usize, h: usize) -> Vec<u8> {
        vec![240u8; w * h]
    }

    /// Fill `(x0, y0, x1, y1)` inclusive with `level`.
    fn fill(g: &mut [u8], w: usize, r: (usize, usize, usize, usize), level: u8) {
        for y in r.1..=r.3 {
            for x in r.0..=r.2 {
                g[y * w + x] = level;
            }
        }
    }

    /// A few lines of text-like stems so the page has an x-height of 8 px.
    fn text(g: &mut [u8], w: usize, x0: usize, y0: usize, lines: usize) {
        for l in 0..lines {
            for k in 0..14 {
                fill(
                    g,
                    w,
                    (x0 + k * 6, y0 + l * 17, x0 + k * 6 + 2, y0 + l * 17 + 7),
                    20,
                );
            }
        }
    }

    fn run(g: &[u8], w: usize, h: usize, dpi: u16) -> Detection {
        let p = Params::defaults().unwrap();
        let l = layout::segment(g, w, h, 0.0, &p).unwrap();
        detect(&l, g, dpi, &p).unwrap()
    }

    fn kinds(d: &Detection) -> Vec<Kind> {
        d.findings.iter().map(|f| f.kind).collect()
    }

    /// A 2 x 2 grid of cells: three horizontal and three vertical rules, 2 px thick, dark.
    fn grid(g: &mut [u8], w: usize) {
        for y in [20usize, 70, 120] {
            fill(g, w, (20, y, 220, y + 1), 30);
        }
        for x in [20usize, 120, 220] {
            fill(g, w, (x, 20, x + 1, 121), 30);
        }
    }

    #[test]
    fn a_ruled_grid_is_one_table_with_its_rows_and_columns() {
        let (w, h) = (260usize, 160usize);
        let mut g = paper(w, h);
        grid(&mut g, w);
        text(&mut g, w, 28, 30, 1);
        let d = run(&g, w, h, 150);
        let t: Vec<_> = d
            .findings
            .iter()
            .filter(|f| f.kind == Kind::Table)
            .collect();
        assert_eq!(t.len(), 1, "{:?}", d.features);
        assert_eq!(t[0].attributes["rows"], 2);
        assert_eq!(t[0].attributes["cols"], 2);
        assert_eq!(t[0].bbox, Some([20, 20, 221, 121]));
        assert!(t[0].confidence > 0.7, "{}", t[0].confidence);
        assert!(!kinds(&d).contains(&Kind::RulingLines));
    }

    #[test]
    fn a_lone_dark_rule_is_a_ruling_line_not_a_table() {
        let (w, h) = (260usize, 100usize);
        let mut g = paper(w, h);
        text(&mut g, w, 20, 20, 1);
        fill(&mut g, w, (20, 60, 240, 61), 30);
        let d = run(&g, w, h, 150);
        assert_eq!(kinds(&d), [Kind::RulingLines]);
        assert_eq!(d.findings[0].attributes["horizontal"], 1);
    }

    #[test]
    fn a_pieces_split_by_a_stroke_are_one_segment() {
        let (w, h) = (260usize, 100usize);
        let mut g = paper(w, h);
        text(&mut g, w, 20, 10, 1);
        fill(&mut g, w, (20, 60, 240, 61), 30);
        fill(&mut g, w, (100, 40, 102, 80), 30); // a stroke through the rule
        let p = Params::defaults().unwrap();
        let l = layout::segment(&g, w, h, 0.0, &p).unwrap();
        let segs = segments(&l, &l.level_gray(&g), &p.rulings);
        assert_eq!(segs.iter().filter(|s| s.horizontal).count(), 1, "{segs:?}");
        assert!(segs.iter().all(|s| s.contrast > 150.0));
    }

    #[test]
    fn a_faint_long_line_is_a_fold_candidate_and_a_short_one_is_dropped() {
        let (w, h) = (200usize, 220usize);
        let mut g = paper(w, h);
        text(&mut g, w, 20, 10, 2);
        fill(&mut g, w, (150, 5, 152, 214), 185); // 3 px wide, 55 levels deep: a crease
        fill(&mut g, w, (20, 150, 60, 151), 185); // too short to be a fold at 60 dpi (0.6 in)
        // 200 px at 60 dpi is 3.3 inches.
        let d = run(&g, w, h, 60);
        let folds: Vec<_> = d
            .findings
            .iter()
            .filter(|f| f.kind == Kind::FoldLine)
            .collect();
        assert_eq!(folds.len(), 1, "{:?}", d.features);
        assert_eq!(folds[0].attributes["orientation"], "vertical");
        assert!(
            !kinds(&d).contains(&Kind::RulingLines),
            "faint is not a printed rule"
        );
    }

    fn outline(g: &mut [u8], w: usize, x: usize, y: usize, s: usize) {
        fill(g, w, (x, y, x + s - 1, y + 1), 30);
        fill(g, w, (x, y + s - 2, x + s - 1, y + s - 1), 30);
        fill(g, w, (x, y, x + 1, y + s - 1), 30);
        fill(g, w, (x + s - 2, y, x + s - 1, y + s - 1), 30);
    }

    #[test]
    fn aligned_hollow_squares_are_a_checkbox_grid_and_ticks_are_counted() {
        let (w, h) = (260usize, 80usize);
        let mut g = paper(w, h);
        text(&mut g, w, 20, 10, 1);
        for i in 0..4 {
            outline(&mut g, w, 30 + i * 40, 40, 11);
        }
        // Tick the second box with a cross of two diagonals.
        for k in 0..7 {
            fill(&mut g, w, (73 + k, 44 + k, 74 + k, 44 + k), 30);
            fill(&mut g, w, (79 - k, 44 + k, 80 - k, 44 + k), 30);
        }
        let d = run(&g, w, h, 90);
        let grids: Vec<_> = d
            .findings
            .iter()
            .filter(|f| f.kind == Kind::CheckboxGrid)
            .collect();
        assert_eq!(grids.len(), 1, "{:?}", d.features);
        assert_eq!(grids[0].attributes["boxes"], 4);
        assert_eq!(grids[0].attributes["checked"], 1);
    }

    #[test]
    fn open_shapes_and_two_boxes_are_not_a_grid() {
        let (w, h) = (260usize, 80usize);
        let mut g = paper(w, h);
        text(&mut g, w, 20, 10, 1);
        // Three C shapes (a side missing) and two real boxes.
        for i in 0..3 {
            let x = 30 + i * 40;
            fill(&mut g, w, (x, 40, x + 10, 41), 30);
            fill(&mut g, w, (x, 49, x + 10, 50), 30);
            fill(&mut g, w, (x, 40, x + 1, 50), 30);
        }
        outline(&mut g, w, 160, 40, 11);
        outline(&mut g, w, 200, 40, 11);
        let d = run(&g, w, h, 90);
        assert!(!kinds(&d).contains(&Kind::CheckboxGrid), "{:?}", d.features);
    }

    #[test]
    fn blank_and_text_only_pages_report_nothing() {
        let (w, h) = (100usize, 100usize);
        assert!(run(&paper(w, h), w, h, 150).findings.is_empty());
        let mut g = paper(w, h);
        text(&mut g, w, 10, 10, 3);
        let d = run(&g, w, h, 150);
        assert!(d.findings.is_empty());
        assert_eq!(d.features["segments"], 0.0);
    }

    #[test]
    fn detection_is_repeatable_and_params_validate() {
        let (w, h) = (260usize, 160usize);
        let mut g = paper(w, h);
        grid(&mut g, w);
        assert_eq!(run(&g, w, h, 150).findings, run(&g, w, h, 150).findings);
        let mut p = Params::defaults().unwrap().rulings;
        assert!(p.validate().is_ok());
        p.box_edge_coverage = 1.5;
        assert!(p.validate().is_err());
        let mut p = Params::defaults().unwrap().rulings;
        p.grid_min_boxes = 1;
        assert!(p.validate().is_err());
    }
}
