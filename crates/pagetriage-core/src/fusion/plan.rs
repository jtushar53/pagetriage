//! Page plan (module M14). See `docs/detectors/plan.md`.
//!
//! Turns what the other detectors found into instructions for whatever reads the page next: how to
//! turn and scale it, what box holds the content, which pieces to send to which kind of reader, which
//! pages to skip, how to tile a page too big to send whole, and a key to cache the work under. All
//! arithmetic on existing findings; nothing is guessed from pixels except the ink profile that tile
//! cuts follow.

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::hash::sha256_hex;
use crate::math;
use crate::pixel::layout::Layout;
use crate::schema::{Finding, Kind, PagePlan, PlanRegion, PlanScale, PlanSkip, PlanTile, Quality};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanParams {
    pub target_xheight_px: f64,
    pub min_scale: f64,
    pub max_scale: f64,
    pub margin_in: f64,
    pub handwriting_overlap: f64,
    pub covered_overlap: f64,
    pub blank_ink_share: f64,
    pub tile_max_side_px: u32,
    pub tile_overlap: f64,
    pub tile_search: f64,
    pub tile_ink_delta: u8,
    pub handwriting_bonus: f64,
    pub table_bonus: f64,
    pub turn_bonus: f64,
    pub small_text_bonus: f64,
    pub small_xheight_px: f64,
}

impl PlanParams {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParams(format!("plan: {m}")));
        let all = [
            self.target_xheight_px,
            self.min_scale,
            self.max_scale,
            self.margin_in,
            self.handwriting_overlap,
            self.covered_overlap,
            self.blank_ink_share,
            self.tile_overlap,
            self.tile_search,
            self.handwriting_bonus,
            self.table_bonus,
            self.turn_bonus,
            self.small_text_bonus,
            self.small_xheight_px,
        ];
        if all.iter().any(|v| !v.is_finite() || v.abs() > 1.0e6) {
            return bad("every value must be finite and within 1e6");
        }
        if self.target_xheight_px <= 0.0 || self.min_scale <= 0.0 || self.max_scale < self.min_scale
        {
            return bad("target_xheight_px > 0 and 0 < min_scale <= max_scale");
        }
        if self.tile_max_side_px < 64
            || !(0.0..0.5).contains(&self.tile_overlap)
            || !(0.0..0.5).contains(&self.tile_search)
        {
            return bad("tile_max_side_px >= 64, tile_overlap and tile_search in [0, 0.5)");
        }
        for (name, v) in [
            ("handwriting_overlap", self.handwriting_overlap),
            ("covered_overlap", self.covered_overlap),
        ] {
            if !(v > 0.0 && v <= 1.0) {
                return bad(&format!("{name} must be in (0, 1]"));
            }
        }
        Ok(())
    }
}

/// What the plan reads.
pub struct PlanInput<'a> {
    pub gray: &'a [u8],
    pub width: usize,
    pub height: usize,
    pub dpi: u16,
    pub layout: &'a Layout,
    pub findings: &'a [Finding],
    pub quality: &'a Quality,
    /// The quality grade's score in [0, 1] (0 clean).
    pub badness: f64,
    /// The text-line detectors were not run because the page is too degraded: the layout is not
    /// to be trusted, so the plan sends the whole page to a vision model.
    pub gated: bool,
}

fn area(b: [u32; 4]) -> f64 {
    f64::from(b[2].saturating_sub(b[0]) + 1) * f64::from(b[3].saturating_sub(b[1]) + 1)
}

/// Share of `a` that lies inside `b`.
fn inside(a: [u32; 4], b: [u32; 4]) -> f64 {
    let ix = i64::from(a[2].min(b[2])) - i64::from(a[0].max(b[0])) + 1;
    let iy = i64::from(a[3].min(b[3])) - i64::from(a[1].max(b[1])) + 1;
    if ix <= 0 || iy <= 0 {
        0.0
    } else {
        (ix * iy) as f64 / area(a)
    }
}

fn union(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

fn kind_name(k: Kind) -> String {
    match serde_json::to_value(k) {
        Ok(serde_json::Value::String(s)) => s,
        _ => unreachable!("Kind serialises as a string"),
    }
}

/// (route, priority) for a finding kind that becomes a region; `None` for kinds that do not.
fn route_of(f: &Finding) -> Option<(&'static str, u32)> {
    Some(match f.kind {
        Kind::Table => ("table", 0),
        Kind::Handwriting => ("handwriting", 0),
        Kind::Signature | Kind::Stamp | Kind::Seal | Kind::EmbossedSeal | Kind::ThumbImpression => {
            ("mark", 1)
        }
        Kind::Photo => ("vision", 1),
        Kind::QrCode => {
            if f.attributes.get("decoded").and_then(|v| v.as_bool()) == Some(true) {
                ("decoded", 2)
            } else {
                ("decode", 2)
            }
        }
        Kind::CheckboxGrid => ("fields", 1),
        Kind::RedactionBox => ("note", 2),
        _ => return None,
    })
}

pub fn build(input: &PlanInput<'_>, p: &PlanParams) -> PagePlan {
    let (w, h) = (input.width as u32, input.height as u32);
    let page = [0, 0, w.saturating_sub(1), h.saturating_sub(1)];
    let dpi = f64::from(input.dpi);
    let mut notes: Vec<String> = Vec::new();

    // Turn: the misoriented finding, if there is one.
    let rotate = input
        .findings
        .iter()
        .find(|f| f.kind == Kind::Misoriented)
        .and_then(|f| {
            f.attributes
                .get("rotate_clockwise_deg")
                .and_then(|v| v.as_u64())
        })
        .map_or(0, |d| d as u32);
    let needs_rerun = rotate != 0;
    if needs_rerun {
        notes.push(
            "the page was read sideways or upside down: turn it, then analyse it again for regions"
                .into(),
        );
    }
    let skew = input.quality.skew_deg;
    let deskew = (!needs_rerun).then(|| {
        if skew.abs() >= 0.1 {
            math::round_output(skew)
        } else {
            0.0
        }
    });

    // Scale.
    let xh = input.layout.xheight;
    let factor = if xh > 0.0 {
        (p.target_xheight_px / xh).clamp(p.min_scale, p.max_scale)
    } else {
        1.0
    };
    let scale = PlanScale {
        xheight_px: math::round_output(xh),
        target_xheight_px: p.target_xheight_px,
        factor: math::round_output(factor),
        suggested_dpi: (dpi * factor).round() as u32,
    };

    // Regions. When the page is on its side the layout is meaningless, so there are none.
    let mut regions: Vec<PlanRegion> = Vec::new();
    if input.gated {
        notes.push(
            "the page is too degraded for line-level analysis: send the whole page to a vision model"
                .into(),
        );
        regions.push(PlanRegion {
            bbox: page,
            kind: "page".into(),
            route: "vision".into(),
            priority: 0,
        });
    } else if !needs_rerun {
        let boxes_of = |kinds: &[Kind]| -> Vec<[u32; 4]> {
            input
                .findings
                .iter()
                .filter(|f| kinds.contains(&f.kind))
                .filter_map(|f| f.bbox)
                .collect()
        };
        let hand = boxes_of(&[Kind::Handwriting]);
        let covering = boxes_of(&[
            Kind::Table,
            Kind::Signature,
            Kind::Stamp,
            Kind::Seal,
            Kind::EmbossedSeal,
            Kind::ThumbImpression,
            Kind::Photo,
            Kind::QrCode,
        ]);
        for b in &input.layout.blocks {
            let bb = input.layout.rect_to_page(&b.bbox);
            if covering.iter().any(|c| inside(bb, *c) >= p.covered_overlap) {
                continue;
            }
            let is_hand = hand.iter().any(|c| inside(bb, *c) >= p.handwriting_overlap);
            regions.push(PlanRegion {
                bbox: bb,
                kind: if is_hand { "handwriting" } else { "text_block" }.into(),
                route: if is_hand { "handwriting" } else { "ocr" }.into(),
                priority: 0,
            });
        }
        for f in input.findings {
            if let (Some((route, priority)), Some(bbox)) = (route_of(f), f.bbox) {
                // Handwriting findings overlapping a text block already turned that block into a
                // handwriting region; an extra region only for those no block covers.
                if f.kind == Kind::Handwriting
                    && regions.iter().any(|r| {
                        r.route == "handwriting" && inside(bbox, r.bbox) >= p.handwriting_overlap
                    })
                {
                    continue;
                }
                regions.push(PlanRegion {
                    bbox,
                    kind: kind_name(f.kind),
                    route: route.into(),
                    priority,
                });
            }
        }
        regions.sort_by_key(|r| (r.priority, r.bbox[1], r.bbox[0]));
    }

    // Content box.
    let margin = (p.margin_in * dpi).round() as u32;
    let content_bbox = regions
        .iter()
        .map(|r| r.bbox)
        .reduce(union)
        .map_or(page, |u| {
            [
                u[0].saturating_sub(margin),
                u[1].saturating_sub(margin),
                (u[2] + margin).min(page[2]),
                (u[3] + margin).min(page[3]),
            ]
        });

    // Skip: blank pages (document-level duplicates are marked later by the document pass).
    let ink_share =
        input.layout.ink_pixels as f64 / (input.layout.width * input.layout.height) as f64;
    let has_blank = input.findings.iter().any(|f| f.kind == Kind::BlankPage);
    let skip = (!needs_rerun && regions.is_empty() && (has_blank || ink_share < p.blank_ink_share))
        .then(|| PlanSkip {
            reason: "blank".into(),
            of: None,
        });

    // Difficulty.
    let has = |k: Kind| input.findings.iter().any(|f| f.kind == k);
    let mut difficulty = input.badness;
    if has(Kind::Handwriting) {
        difficulty += p.handwriting_bonus;
    }
    if has(Kind::Table) {
        difficulty += p.table_bonus;
    }
    if needs_rerun {
        difficulty += p.turn_bonus;
    }
    if xh > 0.0 && xh < p.small_xheight_px {
        difficulty += p.small_text_bonus;
    }
    let difficulty = math::round_output(difficulty.clamp(0.0, 1.0));

    // Scripts, from the script finding.
    let scripts: Vec<String> = input
        .findings
        .iter()
        .find(|f| f.kind == Kind::Script)
        .and_then(|f| f.attributes.get("scripts"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();

    let tiles = tiles(input, p);
    if !tiles.is_empty() {
        notes.push(format!(
            "the page is larger than {} px on a side: send the tiles, not the whole page",
            p.tile_max_side_px
        ));
    }

    // Cache key: the pixels, the size and the resolution.
    let mut keyed = format!(
        "pagetriage-plan|{}x{}|{}|",
        input.width, input.height, input.dpi
    )
    .into_bytes();
    keyed.extend_from_slice(input.gray);

    PagePlan {
        rotate_clockwise_deg: rotate,
        deskew_clockwise_deg: deskew,
        needs_rerun,
        scale,
        content_bbox,
        binarizer: match serde_json::to_value(input.quality.recommended_binarizer) {
            Ok(serde_json::Value::String(s)) => s,
            _ => unreachable!("Binarizer serialises as a string"),
        },
        difficulty,
        skip,
        scripts,
        regions,
        tiles,
        input_sha256: sha256_hex(&keyed),
        notes,
    }
}

/// Tiles for a page with a side over `tile_max_side_px`: a grid whose cuts fall on the emptiest rows
/// and columns near an even split, each tile reaching `tile_overlap` into its neighbours.
fn tiles(input: &PlanInput<'_>, p: &PlanParams) -> Vec<PlanTile> {
    let (w, h) = (input.width, input.height);
    let max_side = p.tile_max_side_px as usize;
    if w <= max_side && h <= max_side {
        return Vec::new();
    }
    let mut histogram = [0u64; 256];
    for &v in input.gray {
        histogram[usize::from(v)] += 1;
    }
    let mut paper = 0usize;
    for (level, &count) in histogram.iter().enumerate() {
        if count > histogram[paper] {
            paper = level;
        }
    }
    let dark = paper.saturating_sub(usize::from(p.tile_ink_delta));
    let mut rows = vec![0u32; h];
    let mut cols = vec![0u32; w];
    for (y, row) in input.gray.chunks_exact(w).enumerate() {
        for (x, &v) in row.iter().enumerate() {
            if usize::from(v) < dark {
                rows[y] += 1;
                cols[x] += 1;
            }
        }
    }
    let cuts = |len: usize, profile: &[u32]| -> Vec<usize> {
        let n = len.div_ceil(max_side).max(1);
        let mut out = vec![0usize];
        for i in 1..n {
            let ideal = i * len / n;
            let reach = ((len / n) as f64 * p.tile_search) as usize;
            let (lo, hi) = (
                ideal.saturating_sub(reach).max(1),
                (ideal + reach).min(len - 1),
            );
            // Least ink in a 3-wide window; ties go to the cut nearest the even split.
            let best = (lo..=hi)
                .min_by_key(|&c| {
                    let s: u32 = profile[c.saturating_sub(1)..(c + 2).min(len)].iter().sum();
                    (s, c.abs_diff(ideal))
                })
                .unwrap_or(ideal);
            out.push(best);
        }
        out.push(len);
        out
    };
    let (xc, yc) = (cuts(w, &cols), cuts(h, &rows));
    let grow = |len_side: usize| (len_side as f64 * p.tile_overlap).round() as usize;
    let mut out = Vec::new();
    for (r, ys) in yc.windows(2).enumerate() {
        for (c, xs) in xc.windows(2).enumerate() {
            let (ox, oy) = (grow(xs[1] - xs[0]), grow(ys[1] - ys[0]));
            out.push(PlanTile {
                bbox: [
                    xs[0].saturating_sub(ox) as u32,
                    ys[0].saturating_sub(oy) as u32,
                    ((xs[1] + ox).min(w) - 1) as u32,
                    ((ys[1] + oy).min(h) - 1) as u32,
                ],
                row: r as u32,
                column: c as u32,
            });
        }
    }
    out
}
