# `script` (module M4, `pixel.script`)

Says which **family** of script a page is written in, from the pixels alone, with no model:

| Family | Stands for | What the detector sees |
|---|---|---|
| `headline` | Devanagari, Bengali, Gurmukhi | a continuous bar along the top of each word (the shirorekha) |
| `cursive` | Arabic script (Urdu, Persian) | letters joined along a baseline, a dense low row, dots |
| `discrete` | Latin, Gujarati, Odia, Tamil, Telugu, Kannada, Malayalam | separate letters, no bar, no continuous baseline |

It reports `kind: "script"` with `attributes.family`. It **cannot tell Devanagari from Bengali, or
Latin from Tamil**: nothing in it looks at letter shape. For PDFs with a text layer, M2
(`font_script`) is the better source; M4 is for scans.

The detector is **opt-in** (`"detectors": ["script"]`) and runs on the [`layout`](layout.md) pass.

**Read this first.** It was built and checked only on synthetic pages, drawn by the same author as the
detector. The Indic and Arabic-script pages are *pseudo-glyphs* made to have a headline or a joined
baseline, so a high score here means the code does what it was designed to do, not that it works on
real print. Nobody has run it on a real deed, real Devanagari type, real Urdu or a real Latin font
other than Pillow's default. See the [failure gallery](#failure-gallery).

## Algorithm

1. **Layout.** Lines come from the `layout` pass (levelled by M3's skew).
2. **Four numbers per line**, from the levelled ink mask inside the line's box:
   - `top_peak`: ink in the densest row of the top half of the line, over the line's width. A
     headline bar reads about 0.85; the tops of separate letters about 0.3 to 0.55.
   - `low_peak`: the same for the bottom half. A joined baseline reads about 0.84; separate letters
     0.35 to 0.6.
   - `comp_density`: glyph components per x-height of width. Joined scripts have a component per
     word or sub-word, separate scripts one per letter.
   - `small_share`: share of the line's glyphs under `small_height_fraction` of the median glyph
     height (dots and marks).
3. **Page value** of each: the median over lines weighted by line width, so a short heading cannot
   outvote a paragraph.
4. **One linear score per family**, a sum of terms `coef * (feature - threshold)` each clipped to
   `+-term_clip`:
   - headline = `top_peak` high, `comp_density` low, `small_share` low (half weight);
   - cursive = `top_peak` low, `low_peak` high, `comp_density` low, `small_share` high;
   - discrete = `top_peak` low, `comp_density` high, `small_share` low (half weight).
5. **Softmax** over the three scores. The winner is reported if its probability is at least
   `emit_min_probability` and the page has `min_lines` lines. The evidence is the winner's terms.
6. **Mixed pages.** Each line gets its own family the same way. Any other family with at least
   `min_lines` lines and `secondary_line_share` of all lines is reported as a second finding
   (`attributes.secondary: true`, confidence = mean line probability).

The confidence is a softmax of hand-set scores. **It is not calibrated.**

The plan also lists a **texture tier** (Gabor filter banks). It is not built: it would be there to
separate scripts of the same family (Tamil from Latin), and nothing here can check it.

## Parameters

`crates/pagetriage-core/params/script.toml`: 14 numbers, all in the file with a comment. The
thresholds and coefficients were set by looking at the feature ranges of the dev split below
(`bench/script_synth_eval.py --features`), so they are "tuned on synthetic dev seeds 1-5" and
nothing else. The method (headline bar, component and profile features) is from Pal and Chaudhuri,
ICDAR 1999, and Pal, Sinha and Chaudhuri, ICDAR 2003 (`docs/REFERENCES.md`); their numeric
thresholds were not used.

Debug features (`debug.script`): `top_peak`, `low_peak`, `comp_density`, `small_share` (page
medians), `p_headline`, `p_cursive`, `p_discrete`, `lines`, `lines_headline`, `lines_cursive`,
`lines_discrete`.

## Evaluation

`bench/script_synth_eval.py`: A4 pages at 150 dpi, 6 paragraphs, nine scripts (Latin from Pillow's
font; the others pseudo-glyphs, with a headline for the first three Indic ones and a joined-letter
generator with dots for Arabic), seven conditions (clean 11 pt, 9 pt, 14 pt, skew 3 degrees, sensor
noise 8, speckle 0.3%, blur 1 px), 5 seeds each. Dev seeds 1-5, test seeds 11-15.

| Split | Pages | Correct family | Abstained |
|---|---|---|---|
| dev (thresholds tuned here) | 315 | 315 | 0 |
| test (seeds 11-15, looked at once) | 315 | 315 | 0 |

Every cell of the confusion matrix off the diagonal is zero, and so is every condition. **That is a
sign the test is easy, not that the detector is good**: the pseudo-scripts share the detector's
assumptions. The history, so the 100% is not read as more than it is. The first thresholds were a guess and got the
first three seeds of every script right, with the Arabic generator drawing some letters as flat 2-px
lines. That generator was wrong (x-height 5 px, baselines read as rulings, 356 "lines"), so letters
were given bodies; with that, one Arabic page read `discrete` and another `cursive` at 0.64, because the
dots had shrunk to specks. The thresholds were then re-set from the feature ranges (above). The
fix to the generator, then the re-set of the thresholds, are both tuning on the dev split.

Mixed pages (upper 45% of one page over the lower 45% of another), 10 pages per pair: Latin over
Devanagari, Devanagari over Latin, Latin over Arabic, Devanagari over Arabic all reported both
families on 10 of 10 pages.

Other probes, 3 seeds each: all-capitals Latin and Latin lines underlined at the baseline stayed
`discrete` (`top_peak` 0.53 to 0.55, against the 0.7 threshold).

Cost: on a 1240 x 1754 page, release build, median of 5, quality alone 186 ms, plus layout 233 ms,
plus script 253 ms (about 20 ms for M4 on top of the layout it shares).

## Determinism

Row counts and component counts are integers; the page medians, the clipped terms and the softmax use
IEEE operations and `math::exp` (the `libm` crate). Sums go through `math::pairwise_sum`. Checked on
the golden pages by the native, Python and WASM runners.

## Failure gallery

| Case | What happens | Where |
|---|---|---|
| **Blurred text** | When blur merges neighbouring letters of separate-letter text into word blobs, `comp_density` drops and the page reads as joined text. `quality_blurred` (3x3 box blur of an 8-px-tall text page): `cursive` at 0.548 plus a secondary `headline` at 0.522, `lines_cursive` 10 of 13. The low confidences are the only warning. | `tests/golden/layout_inputs` via `quality_inputs/quality_blurred.pgm` |
| **Scripts inside a family** | Devanagari, Bengali and Gurmukhi are one family and Latin and Tamil are one. By design. | By construction |
| **Real fonts** | The thresholds come from Pillow's default Latin font and pseudo-glyphs. Serif fonts, bold type, condensed type, or a Devanagari font whose headline has gaps may fall on the wrong side of 0.7. The margin on synthetic Latin is 0.15 below the threshold, on synthetic headline 0.14 above. Unknown on real print. | Unmeasured |
| **Handwriting** | A handwritten line has neither a clean bar nor a clean baseline. The family it gets is arbitrary. M5 should be run first. | Unmeasured |
| **Rulings through text** | A ruling touching the line (an underline at the baseline, a table cell edge) adds a dense row at the top or low part of the box. `layout` sets free-standing rulings aside, but one that touches text joins its component. A low-part ruling could push `low_peak` over 0.7 and tip `discrete` to `cursive`. Not tested. | Unmeasured |
| **Few lines** | Under `min_lines` (3) it abstains, so a stamp, a letterhead or a form with a few fields gets no family. | By construction |
| **Digits and numbers** | A page of figures has no letters to join. It reads `discrete`, whatever the script. | By construction |
| **Mixed line** | One line of Hindi with an English word in it gets one family. Only the page-level count of lines shows the mix. | By construction |
| **Layout fails** | A bad skew estimate, or columns merged by a narrow gutter (see `layout`), spoils the lines and with them every number here. | Depends on `layout` and M3 |

## Golden fixtures

`tests/golden/layout_inputs/script_cursive.pgm` (joined pseudo-letters with dots) and
`script_mixed.pgm` (headline words over separate letters), drawn by `tests/golden/gen_inputs.py`; the
other layout and quality pages are run through `script` too. `tests/layout_golden.rs` pins the
families of four of them. Expected reports are in `tests/golden/layout_expected/`.
