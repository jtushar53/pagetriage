# `rulings` (module M9, `pixel.rulings`)

Finds **printed rulings, tables and checkbox grids**, and tells printed rules from **fold lines**,
from the pixels alone, with no model. It reports:

| Finding | Meaning |
|---|---|
| `table` | A connected group of horizontal and vertical rules enclosing cells: `rows`, `cols`, `segments`. A framed box is a 1 x 1 table. |
| `ruling_lines` | The page's other printed rules (underlines, form lines, ruled paper), one finding for all of them: counts by direction, `longest_px`. |
| `checkbox_grid` | Three or more hollow squares in a row or column: `boxes`, `checked`. |
| `fold_line` | A long, faint straight line: a crease, not a printed rule. |

The detector is **opt-in** (`"detectors": ["rulings"]`) and runs on the [`layout`](layout.md) pass.

**Read this first.** Checked on synthetic pages only (clean renderings with controlled damage, drawn
by the author of the detector) and on 2 hand-built fixtures. Every threshold is a placeholder tuned on
the synthetic dev split. It has not seen a scanned form. In particular the fold-versus-rule call rests
on one number (contrast) that a faded real rule can fall on the wrong side of; see the
[failure gallery](#failure-gallery).

## Algorithm

1. **Rulings from layout.** The layout pass cuts thin runs of ink out of the levelled ink mask
   (`rule_run_xh`, `rule_max_thick_xh`) and reports the pieces as `rules`.
2. **Segments.** Pieces on one line (centre lines within `merge_across_xh`, gap along at most
   `merge_gap_xh`, so a stroke crossing a rule does not split it) are merged. Each segment gets a
   **contrast**: the median gray of two strips 3 to 6 px either side, minus the mean along the segment
   of its darkest cross-section pixel, on the levelled gray page.
3. **Printed or faint.** Contrast at least `rule_min_contrast` (70) is a printed rule. A fainter
   segment at least `fold_min_length_in` (3 in, as in M3) long is a `fold_line` finding; a shorter one
   is dropped.
4. **Tables.** A horizontal and a vertical printed segment meet when the vertical's x is within the
   horizontal's span and the horizontal's y within the vertical's span, widened by `join_tol_xh`.
   Connected groups with at least 2 horizontal and 2 vertical segments, at 2 or more distinct row
   lines and column lines (positions within `line_cluster_xh` are one line), are tables:
   `rows = row lines - 1`, `cols = column lines - 1`. Confidence is
   `sigmoid(table_prior + table_cells_coef * min(cells, cap) + table_junction_coef * (share of segment
   ends that meet a segment - 0.5))`.
5. **Free rules.** Printed segments not in a table are reported together as `ruling_lines`.
6. **Checkboxes.** A glyph-class component is a box when, after trimming outer rows and columns that
   carry at most `box_overshoot_fraction` (0.25) of the side in ink (the tip of a tick that pokes
   out), the rest is a square of side at least `max(box_min_xh * x-height, box_min_in * dpi)`
   (1.0 x-height, 0.11 in) and at most `box_max_xh`, with each of its four edges holding an unbroken
   run of ink at least `box_edge_coverage` (0.8) of the side in a band of 2 pixels from the frame, and
   at most `box_max_inner_ink` (0.65) of its inner half inked. It is ticked when the inner half is at
   least `box_checked_ink` (0.08) inked. Boxes of similar size aligned in a row or column (centres
   within `grid_align_xh`) form a grid of at least `grid_min_boxes` (3).

The confidences are sigmoids of hand-set sums. **They are not calibrated.**

`debug.rulings`: `segments`, `printed_rules`, `faint_segments`, `tables`, `boxes`, `box_grids`.

## Parameters

`crates/pagetriage-core/params/rulings.toml` and, for the cut, `layout.toml` (`rule_run_xh`,
`rule_max_thick_xh`). Placeholders, with two set from the benchmark: `box_min_in` (0.11 in; at
100 dpi round letters `a`, `o` passed every other box test, 8 px wide) and `rule_run_xh` (6 x-heights;
12 missed the vertical rules of a three-row table, 114 px long).

## Evaluation

`bench/rulings_synth_eval.py`: A4 pages at 150 dpi (and 100 dpi), Pillow font, 5 dev seeds (the
checkbox and run-length settings were looked at on them) and 5 test seeds.

- **table**: a ruled grid of 2-8 rows and 2-5 columns, 1-3 px lines, text in cells.
- **checks**: 6-14 hollow squares of 1-2 px edge with labels, about 40% ticked (crosses or check marks
  that overshoot the box).
- **mixed**: a table, a checkbox list and an underline.
- **text**: paragraphs, and a line of `0 O D Q 10 100 OOO DDD 000 [ ] ( )` meant to look like boxes.

Conditions: clean, skew 2 degrees, sensor noise 8, blur 1 px, 100 dpi. Exact means: the tables found
have exactly the drawn rows and columns; the boxes found and the ticked ones are exactly the drawn
counts; nothing found on a text page.

| Pages | Exact on dev (5 conditions x 5 seeds) | Exact on test | Spurious per page |
|---|---|---|---|
| table | 25 / 25 | 25 / 25 | 0 |
| checks | 25 / 25 | 25 / 25 | 0 |
| mixed | 25 / 25 | 25 / 25 | 0 |
| text | no finding on 25 / 25 | no finding on 25 / 25 | 0 |

The path to those numbers is the useful part, because each step was a failure of the previous
version, found by this benchmark and not by the unit tests:

1. Tables under 2 degrees of skew found none of their vertical rules, because the layout levelled
   only the text lines (one shear) and left vertical lines tilted. A second shear (a rotation, in
   effect) fixed it: tables 0 of 5 to 2 to 3 of 5.
2. Levelled vertical rules wobble by a pixel every 10 to 20 rows, which broke runs below the minimum
   length; runs are now found on a mask widened by one pixel across (tables 3 of 5 to 5 of 5).
3. Ticked boxes were missed when the tick overshot the box: the component's box was not square. The
   frame is now the component's box less its nearly empty outer rows and columns.
4. Capital `O`, `D`, `Q` in the confuser line were taken for boxes (5 false boxes on every clean text
   page) when edges were tested at points and with tolerance, and then when the frame was picked by
   row density (the rounded caps were left outside it). Requiring an unbroken run along each edge,
   on the box's own bounding box, removed them.
5. At 100 dpi the round letters `a`, `o` and `e` (8 px) were taken for boxes, 60 per text page; the
   absolute minimum of 0.11 in removed them.
6. Short vertical rules (3-row tables) needed the cut to start at 6 x-heights, not 12.

Cost: on a 1240 x 1754 mixed page, release build, median of 5: quality alone 127 ms, with layout
209 ms, with layout and rulings 261 ms (about 50 ms for M9, most of it the contrast and the box tests).

The first four findings above also changed the layout, so the layout and script benchmarks were
rerun: layout 45 / 45 exact (9 rows x 5 seeds), script 315 / 315 on both splits, unchanged.

## Determinism

Geometry is integer. The contrast uses means through `math::pairwise_sum`, the confidence the
`math::sigmoid` (`libm`). Findings are sorted by position and kind. Checked on the golden pages by
the native, Python and WASM runners.

## Failure gallery

| Case | What happens | Where |
|---|---|---|
| **Faded rules read as folds** | The only difference between a printed rule and a fold line here is contrast against the paper (70 gray levels). A faded photocopy's rules, or any rule scanned lightly, fall under it and are reported as `fold_line` if 3 inches long, and dropped otherwise (and the table they belong to is not reported). M3's own fold detector (`quality.fold_lines`) measures broad shading and is independent: the two can disagree. | By construction; threshold not checked on real data |
| **A crease must binarise** | A fold line is found only if it comes out of the Sauvola mask as a thin dark line; a soft shading dip does not. That is M3's job. The synthetic crease is 3 px wide and 55 levels deep. | `rulings_table.pgm` |
| **Borderless tables** | A table drawn with alignment and no lines is not found. | By construction |
| **Merged cells, irregular grids** | `rows` and `cols` count distinct line positions: a cell spanning two columns is invisible, and an irregular table gets the count of its busiest lines. | By construction |
| **Double rules** | Two rules closer than `line_cluster_xh` (0.5 x-heights, about 6 px) are one line. | By construction |
| **Dotted and dashed rules** | Leader dots and dashed lines are not runs and are not cut. Not tested. | Open |
| **Thick or short rules** | A rule thicker than 0.5 x-heights (a bold bar, a heavy underline) or shorter than 6 x-heights is not cut and stays a graphic or a glyph. | `layout` gallery |
| **Small boxes** | A box under 0.11 in (2.8 mm) or under one x-height is ignored; a box with a heavy mark (over 65% of its inner half inked, as when the box is filled in) is ignored. Round or circled options are not boxes. | By construction |
| **Two boxes are not a grid** | `grid_min_boxes` is 3. | By construction |
| **Boxes in table cells** | Not specially handled: a cell small enough to look like a box would be one. Not tested. | Open |
| **Ruled paper** | A page of 30 ruled lines is one `ruling_lines` finding, which cannot tell it from a form. | By construction |
| **Skew estimate wrong** | The page is levelled by M3's skew; if that is wrong by more than a degree or so, long rulings are no longer straight runs and are not cut. Not tested. | Depends on M3 |
| **Real scans** | Never seen. | Open |

## Golden fixtures

`tests/golden/layout_inputs/rulings_table.pgm`, drawn by `tests/golden/gen_inputs.py`: a 4 x 3 ruled
table with words in the top row, a row of five 18-px checkboxes (two ticked), an underline, and a
faint vertical crease 3 px wide and about 55 levels deep. `tests/layout_golden.rs` pins what was drawn:
a 4 x 3 table, a grid of 5 boxes with 2 ticked, one ruling-lines finding and one fold line, and a
2 x 1 table plus 2 ruling lines on `layout_form.pgm`, and nothing on a printed text page. The boxes
are 18 px because a box under 0.11 in (16.5 px at 150 dpi) is ignored.

Debugging helpers: `cargo run --release -p pagetriage-core --example dump_boxes -- page.pgm`
(`ASCII=1` draws glyphs, `REGION=x,y,w,h` draws a region of the levelled mask and lists the rules).
