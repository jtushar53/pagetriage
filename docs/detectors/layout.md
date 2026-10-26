# `layout` (shared by M4 `script` and M5 `handwriting`, Phase 2)

Splits a page into **text lines, blocks and columns**, and sets aside **rulings** and **graphics**
(frames, stamps, photos). It produces no `finding`: it is the groundwork the Phase 2 detectors stand
on, because script and handwriting are decided one line or one block at a time, not per page. The
summary numbers are in the report under `debug.layout`. The segmentation itself
(`pixel::layout::segment`) is a Rust API for the detectors built on it.

The detector is **opt-in** (`"detectors": ["layout"]`): it does not change the default report.

**Read this first.** It was built and checked on synthetic pages (clean renderings with controlled
damage) and on 3 hand-drawn fixtures. It has not seen a real deed. The thresholds are placeholders
(`params/layout.toml` says so key by key). The numbers below say the method is sound on pages that
are laid out like text; they do not say how it behaves on real scans. See the
[failure gallery](#failure-gallery).

## Algorithm

All steps are integer arithmetic on the pixel grid except the thresholds that are multiples of the
x-height, which use IEEE operations only, so native, Python and WASM give the same segmentation.

1. **Ink mask.** Sauvola (the M3 binariser, `params/sauvola.toml`).
2. **Level the page.** M3's `quality.skew_deg` (positive = text rising to the right) turns the page
   by two shears, which together make a rotation (up to terms of the order of the skew squared):
   column `x` moves down `round(x * tan(skew))` rows, which levels the text lines; then a pixel on
   levelled row `r` moves `round(-r * tan(skew))` columns, which stands vertical rules and column
   edges up again. (Only the first shear was done at first: tables then lost their vertical rules at
   2 degrees.) Skew below `shear_min_deg` (0.25) applies no shear, so an upright page is segmented as
   it is. Boxes are in the levelled frame; `Layout::to_page` and `rect_to_page` map back to the page.
3. **Components.** 8-connected labelling (`pixel/components.rs`).
3b. **Cut thin rulings.** After a first labelling gives the x-height, every horizontal or vertical
   run of ink at least `rule_run_xh` (6) x-heights long is removed wherever it is at most
   `rule_max_thick_xh` (0.5) x-heights thick across, and the page is labelled again. A run may jog by
   one pixel (it is found on the mask widened by one pixel across), because a levelled rotated line
   wobbles; horizontal and vertical cuts are labelled apart, because at a crossing the end of one
   touches the side of the other diagonally and a whole grid would be one component. A pen stroke
   crossing the ruling is thicker at its own columns, so those pixels stay and the stroke stays
   whole. The removed pieces are the page's `rules`. Without this, text written across or touching a
   printed line (a handwritten fill-in on a form line, a word on a table edge) was fused to it.
4. **Unit.** The page's *x-height* is the area-weighted median height of the glyph-sized
   components. Every length below is a multiple of it. For scripts with a headline (Devanagari,
   Bengali, Gurmukhi) a component is a whole word, so this "x-height" is about the height of a word
   (23 px against 13 px for Latin at 11 pt and 150 dpi). The unit is a scale, not a typographic
   x-height.
5. **Classify** each component: *ruling* (at least `rule_min_len_xh` long and at most
   `rule_max_thick_xh` thick, either way), *graphic* (at least `graphic_min_xh` in both directions,
   or spanning `glyph_max_extent_frac` of the page), *speck* (under `glyph_min_area` pixels), else
   *glyph*.
6. **Lines.** Horizontal run-length smoothing of the glyph mask (Wong, Casey and Wahl 1982): a
   background run of at most `smear_xh` x-heights between two glyph pixels in one row is filled.
   Each smoothed component with at least `line_min_glyphs` glyphs and a height between
   `line_min_height_xh` and `line_max_height_xh` is a line.
7. **Blocks.** Two lines join if they overlap horizontally by `block_min_overlap` of the narrower
   and the gap between them is at most *the page's typical gap* (median, over lines, of the gap to
   the nearest line below) plus `block_gap_margin_lines` median line heights, and never more than
   `block_max_gap_lines` line heights. Tying the gap to the page's own spacing, not to the x-height,
   is what makes paragraphs split in both Latin and headline scripts: with a fixed 1.5 x-heights the
   Devanagari and Bengali paragraphs all merged (0 of 3 pages right, then 5 of 5).
8. **Columns.** Project the lines onto the x axis; a position counts when more than
   `column_cover_frac` of the busiest position's lines cover it, and gaps shorter than
   `column_gap_xh` are closed. The threshold keeps a full-width heading from hiding the gutter.

## Output (`debug.layout`)

| Feature | Meaning |
|---|---|
| `xheight_px` | The unit (step 4) |
| `skew_deg_applied` | The skew the page was levelled by (0 under `shear_min_deg`) |
| `glyphs`, `lines`, `blocks`, `columns` | Counts |
| `rules`, `graphics` | Components set aside (step 5) |
| `glyph_ink_fraction` | Glyph ink pixels over page pixels |
| `text_share_of_ink` | Glyph ink pixels over all ink pixels: low on forms, stamps, photos |
| `median_line_pitch_xh` | Median distance between the centres of consecutive lines in a block |
| `median_glyph_height_xh` | Median over lines of the line's median glyph height |

## Parameters

`crates/pagetriage-core/params/layout.toml`. **Every value is a placeholder** (the file says
"Source: placeholder" per key); the only published values are the method itself (Wong et al.) and
the Sauvola constants M3 already uses. None is tuned on real data.

## Evaluation

`bench/layout_synth_eval.py`: A4 pages at 150 dpi from `bench/synth` (Pillow text in Latin;
procedural pseudo-glyphs with a headline for Devanagari and Bengali), 6 paragraphs, truth taken from
what was drawn. 5 seeds per row; "lines" = exact line count, "blocks" = exact paragraph count,
"cols" = exact column count.

| Page | lines | blocks | cols |
|---|---|---|---|
| Latin, 1 column | 5/5 | 5/5 | 5/5 |
| Devanagari, 1 column | 5/5 | 5/5 | 5/5 |
| Bengali, 1 column | 5/5 | 5/5 | 5/5 |
| Latin, 2 columns | 5/5 | 5/5 | 5/5 |
| Devanagari, 2 columns | 5/5 | 5/5 | 5/5 |
| Latin, skew 1.5 deg | 5/5 | 5/5 | 5/5 |
| Latin, skew 4 deg | 5/5 | 5/5 | 5/5 |
| Latin, speckle 0.2% | 5/5 | 5/5 | 5/5 |
| Latin, blur 1.2 px | 5/5 | 5/5 | 5/5 |

Extra probes, 1 seed each (not in the script's default run): skew 8 and 12 degrees, 1% and 3%
speckle and sensor noise of 12 levels were exact in Latin and Devanagari; blur of 2.5 px lost one
Devanagari line (16 of 17). **These pages are easy**: one font, even leading, no tables, no
handwriting, no stamps. Perfect scores here mean the method does what it is meant to on clean
structure, not that it will on a deed.

Skew was the one thing the first version got wrong: without the shear, the 4-degree page scored 0/3
on lines because a slanted line's box is taller than `line_max_height_xh`.

Cost on a 1240 x 1754 page (A4 at 150 dpi), one run in a cloud container, release build, median of
5: the quality pass alone 212 ms, quality plus layout 310 ms, so layout adds about 100 ms. The
layout pass labels the page twice (ink, then smoothed lines).

## Determinism

No transcendental function is called except `math::sin`/`math::cos` (the `libm` crate) for the
shear slope. Component labelling, smoothing, grouping and projection are integer. Lines are sorted
by `(top, left, bottom, right)` before grouping, and the union-find always keeps the smaller index
as root, so block numbering does not depend on traversal order. Checked on the golden fixtures
below by the native, Python and WASM runners.

## Failure gallery

Known wrong answers. The first is pinned by a fixture; the rest were measured on synthetic pages
(numbers above and below) and are not fixtures.

| Case | What happens | Where |
|---|---|---|
| **Text touching a ruling** | Before the ruling cut was added, text that touched a table rule joined it: in `layout_form` a line whose last row sat directly above a rule was lost (8 lines instead of 9), and a handwritten fill-in crossing a form line fused with it into one 280-px "glyph". The cut (step 3b) fixes both. What is left: a ruling thicker than 0.5 x-heights (a heavy bar, a ruling blurred to 3 px at 10 pt) is not cut, and a ruling shorter than 6 x-heights is not cut. | `layout_form.pgm`, `layout_hand_form.pgm`; unit test `a_stroke_crossing_a_ruling_stays_text_and_the_ruling_is_reported` |
| **A frame is rules, not a table** | The cut turns a frame into its separate horizontal and vertical rules (pieces, split at crossings). Grouping them into tables and cells is [`rulings`](rulings.md) (M9). | `layout_form.pgm`: 1 graphic (the stamp) |
| **Narrow gutter in a headline script** | `smear_xh` (2.0) is two word heights in Devanagari, about 46 px at 11 pt and 150 dpi. A gutter narrower than that fuses the two columns' lines: at a 40 px gutter 52 true lines came out as 47, one column, 8 blocks; at 25 px, 42 lines. At 60 px it was exact. Latin was exact down to a 25 px gutter. | Measured on synthetic pages, see Evaluation |
| **Sparse column** | A column holding fewer lines than `column_cover_frac` (20%) of the busiest position's count is not counted. A narrow sidebar of 2 lines beside a 20-line column is missed. | By construction |
| **Pages of single-line paragraphs** | "Typical gap" is the median over lines, so a list of one-line items (or a form of evenly spaced fields) has a large typical gap and merges into one block. | By construction |
| **Fused or touching lines** | Two lines joined by descenders and ascenders, or by a fold shadow, are one smoothed component. If the result is taller than `line_max_height_xh` (4) it is dropped as a blob; if not, it is one line. | By construction; the `quality_fold` golden is a clean case |
| **Heavy blur in a headline script** | At 2.5 px blur one Devanagari line of 17 was lost. | Measured, 1 seed |
| **Skew estimate wrong** | The page is levelled by M3's skew. When that is wrong (see `skew_peak_ratio` in `quality.md`), lines are cut or fused. Not tested here. | Depends on M3 |
| **Handwriting, curved lines, rotated text, vertical text, mixed font sizes** | Not tried. Mixed sizes shift the x-height unit, which moves every threshold. | Open |

## Golden fixtures

`tests/golden/layout_inputs/` (generated by `tests/golden/gen_inputs.py`, synthetic):

| File | Drawn | Pinned by `tests/layout_golden.rs` |
|---|---|---|
| `layout_one_column.pgm` | A 10-line paragraph and a 3-line paragraph | 13 lines, 2 blocks, 1 column |
| `layout_two_columns.pgm` | A heading over two 9-line columns | 19 lines, 3 blocks, 2 columns |
| `layout_form.pgm` | Text, a framed box with text inside, a filled stamp, two free-standing rules | 9 lines, 1 graphic (M9 reads the rules: one 2 x 1 table, two ruling lines) |

The `quality_inputs/` pages are run through `layout` as well. Expected reports are in
`tests/golden/layout_expected/`; the Python and WASM runners compare against the same files.
Refresh after an intentional change: `UPDATE_GOLDEN=1 cargo test -p pagetriage-core --test
layout_golden`, then review the diff.
