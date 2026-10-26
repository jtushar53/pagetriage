# `handwriting` (module M5, `pixel.handwriting`)

Finds regions of **handwriting** among printed text, including handwritten fill-ins on printed forms,
from the pixels alone, with no model. It reports one `kind: "handwriting"` finding per region (a run
of handwritten words on one line) with a page-coordinate `bbox`, and the share of the page's ink
that is handwritten in `debug.handwriting.hand_ink_share`.

The detector is **opt-in** (`"detectors": ["handwriting"]`) and runs on the [`layout`](layout.md)
pass.

**Read this first. This is the weakest detector in the project, and the numbers below must not be
read as "it finds handwriting".** It finds *round, joined, cursive-looking strokes* written by a
generator its thresholds were set on. On the other two kinds of synthetic handwriting it was tested
on, it found **none of it** (0% recall), and it is not known what it does on real handwriting. The
form-fill case is partly working (about two thirds of the fill-ins) only because those fill-ins
were drawn in the style it knows. It also flags pseudo-Arabic print, and blurred Devanagari print.
All checks are synthetic. See the [results](#evaluation) and the [failure gallery](#failure-gallery).

## Algorithm

1. **Layout.** Lines and glyph components come from `layout` (levelled by M3's skew, thin rulings cut
   out so that writing across a form line stays separate from it).
2. **Words.** Consecutive glyphs of a line with a horizontal gap of at most `word_gap_xh` (0.35)
   x-heights form a word. A joined handwritten word is often a single component.
3. **Four measurements per word:**
   - `baseline_dev`: mean distance of the word's tall glyphs' bottoms from the line's median bottom,
     in x-heights (print rests on one baseline, handwriting wanders);
   - `axis_share`: share of 3x3 Sobel edge strength of the ink mask within about 22 degrees of an
     axis (print is built of verticals and horizontals; round, slanted writing is not);
   - `stroke_cv`: coefficient of variation of the distance transform along stroke centre lines (a
     print pen is constant, a hand's is not);
   - `height_cv`: coefficient of variation of the tall glyphs' heights, for words with at least
     three (`min_glyphs_for_height_cv`).
4. **Score.** The sum of four terms `coef * (feature - threshold)`, each clipped to `+-term_clip`,
   plus `prior_logit` (0).
5. **Neighbours.** A word's score is its own plus `neighbour_weight` (1.0) times the mean score of
   its neighbours on the line, so a short word borrows from the words around it. This follows the
   neighbour propagation of Belaid et al. (arXiv 1303.4614); the features follow Garlapati and
   Chalamala (UKSim 2017), see `docs/REFERENCES.md`. The probability is the sigmoid of the result.
6. **Regions.** Runs of words with probability at least 0.5 on one line, with gaps of at most
   `region_gap_xh` (2.0) x-heights. A region is a finding if its mean probability is at least
   `emit_min_probability` (0.85) and it has at least `min_region_glyphs` (2) glyphs. Its `bbox` is
   mapped back to page coordinates; its evidence is the mean of each term.

The confidence is the mean word probability. **It is not calibrated.**

`debug.handwriting`: `words`, `hand_words`, `hand_ink_share`.

## Parameters

`crates/pagetriage-core/params/handwriting.toml`. Thresholds and coefficients are placeholders set
by looking at feature medians of the synthetic dev pages. Two were swept against the evaluation:
`neighbour_weight` (0 to 1.5: 1.0 chosen) and `emit_min_probability` (0.6 to 0.85: 0.85), on the
30 printed dev pages (Latin and pseudo-Devanagari, clean, noise and blur). With the neighbour weight
at 0.5, 17 of the 30 had a stray region at 0.6 and 6 at 0.75; with 1.0 and 0.85, 2 of 30. The price
is recall: about three to five points on forms.

## Evaluation

`bench/handwriting_synth_eval.py`: A4 pages at 150 dpi, 5 dev seeds (1 to 5; the thresholds were
looked at on seeds 1 to 3 and the two parameters above swept on all five) and 5 test seeds (11 to
15). "Pages with a region" counts pages with at least one finding; recall is the share of truth
handwriting boxes whose centre is inside a region; precision is the share of region area inside a
truth box widened by the gap a region may bridge. Printed pages have no truth boxes, so they only
have the first number. Clean and blur 1 px shown; noise 8 gave the same numbers as clean.

| Pages | Cond. | Dev: pages with region | Test: pages with region | Recall (dev / test) | Precision (dev / test) |
|---|---|---|---|---|---|
| Latin print | clean | 0/5 | 0/5 | | |
| Latin print | blur 1 px | 0/5 | 0/5 | | |
| Devanagari print (pseudo) | clean | 0/5 | **1/5** | | |
| Devanagari print (pseudo) | blur 1 px | **2/5** | **5/5** | | |
| Arabic print (pseudo) | clean | **5/5** | **5/5** | | |
| Arabic print (pseudo) | blur 1 px | **5/5** | **5/5** | | |
| Handwriting, cursive (tuned on) | clean | 5/5 | 5/5 | 97% / 97% | 90% / 90% |
| Handwriting, cursive | blur 1 px | 5/5 | 5/5 | 100% / 100% | 89% / 89% |
| Handwriting, scribble (never tuned on) | clean | 0/5 | 1/5 | **0% / 0%** | |
| Handwriting, scribble | blur 1 px | 5/5 | 5/5 | 97% / 96% | 100% / 100% |
| Handwriting, block capitals (never tuned on) | clean | 0/5 | 0/5 | **0% / 0%** | |
| Handwriting, block capitals | blur 1 px | 0/5 | 0/5 | **0% / 0%** | |
| Form with fill-ins | clean | 5/5 | 5/5 | 70% / 67% | 94% / 94% |
| Form with fill-ins | blur 1 px | 5/5 | 5/5 | 73% / 75% | 84% / 84% |

How to read it. The row that works is the one it was tuned on. The scribble row shows the
generalisation problem: unblurred, the spiky strokes are as axis-aligned and uniform as print and
get 0%; blurred, the same strokes become round and are found at 97%, which says the detector reads
*roundness*, not handwriting. Block capitals, which are what many people write on forms, never score.
The Arabic confuser is a joined, round, baseline-wandering script, so it looks like cursive to every
feature; every pseudo-Arabic page gets regions.

Cost: on a form page at A4 and 150 dpi (release build, median of 5, one container), the layout pass
alone took 261 ms and layout plus handwriting 268 ms. The quality pass (186 ms there) comes first
because the layout needs its skew.

Tried and removed: a per-word profile feature (the share of columns whose lowest or highest ink row
strays from the word's median), meant to catch joined words where the glyph-based measures are
blind. Cursive handwriting scored *lower* on it than printed Latin (median 0.15 against 0.24) and block
capitals about the same as Latin (0.26), and adding it dropped cursive recall, so it was taken out.

## Determinism

Counts, Sobel sums and distance-transform values are integers; the coefficients of variation, term
sums, the sigmoid and the region means use IEEE operations, `math::exp` through `math::sigmoid`, and
`math::pairwise_sum`. Regions are sorted by ink, then by reading order. Checked on the golden pages by
the native, Python and WASM runners.

## Failure gallery

| Case | What happens | Where |
|---|---|---|
| **Hand-printed block capitals** | 0% recall on synthetic block capitals, which look more like print than print does (`axis_share` 0.66 against 0.60, `stroke_cv` 0.15 against 0.21). No feature here tells a neat hand from a typeface. This is the common way to fill in a form. | `bench/handwriting_synth_eval.py`, "block" rows |
| **Spiky or angular writing** | 0% on the random-segment scribble style when sharp, 97% when blurred. | "scribble" rows |
| **Joined, round print** | Pseudo-Arabic print is flagged on every page; it is round, joined and wanders about its baseline. Real Arabic or Urdu print is untested. A cross-check with M4's `cursive` family (M13) is the planned remedy. | "Arabic (confuser)" rows |
| **Blur** | Blurring rounds print: `quality_blurred` (3x3 box blur of a text page) gets six handwriting regions, and 5 of 5 blurred pseudo-Devanagari test pages get one. M3 measures blur, so M13 can discount M5 on blurred pages. | `quality_blurred.pgm` golden; Devanagari blur rows |
| **Fill-ins that are one short glyph** | A fill-in that is a single short component has only `axis_share` and `stroke_cv` to go on, and sits near their thresholds: 30% to 33% of the synthetic form's fill-ins are missed. | "Form" rows |
| **Writing that touches or crosses a long ruling** | Handled for thin rules (6 or more x-heights long, up to 0.5 x-heights thick), by the layout's ruling cut. A thick or short ruling, or a signature line drawn as a heavy bar, is not cut and fuses with the writing. | `layout` gallery |
| **Handwriting touching printed text** | A word that touches printed letters is one component with them: its features are an average and it gets one verdict. The plan's overlay case (a signature over print) is not addressed. | Not tested |
| **Lines that touch each other** | Closely spaced handwriting, with descenders reaching the next line, fuses lines; at 1.6 em line spacing the synthetic page lost two thirds of its lines in the layout. The evaluation uses 2.1 em. | Found while building the generator |
| **Real handwriting** | Never seen. Everything above is the author's generator. | Open |

## Golden fixtures

`tests/golden/layout_inputs/layout_hand_form.pgm`: three printed lines, then two form rows (a printed
label, a rule, and two scribbled words) and one scribbled line. Drawn by `tests/golden/gen_inputs.py`
with integer-only code (a jittered random walk with square pens, not cursive: the detector flags it,
which says nothing about real handwriting). `tests/layout_golden.rs` pins three regions on it
and none on the printed pages. `quality_blurred.pgm` gives six regions in the golden (the blur
failure above), which is pinned by the expected report, not by an assertion.

The per-word numbers can be dumped with
`cargo run --release -p pagetriage-core --example dump_words -- page.pgm`.
