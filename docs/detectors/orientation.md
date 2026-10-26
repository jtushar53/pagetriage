# `orientation` (module M14 input: is the page turned?)

Says whether a page has been scanned **sideways or upside down** and which quarter turn would fix it. It
reports `kind: "misoriented"` with `attributes.rotate_clockwise_deg` (90, 180 or 270) when it is
confident. The detector is **opt-in** (`"detectors": ["orientation"]`) and is added automatically when the
[page plan](plan.md) is asked for. It reads the gray page only, uses no model, and **abstains** (no
finding) when it cannot tell.

**Read this first**: checked only on synthetic pages (Pillow Latin text, procedural pseudo-glyphs for the
other nine scripts). No real scanned document has been looked at. Whatever else is said below, an answer
from this detector is a suggestion to be checked by the OCR run that follows, not a fact.

## Algorithm

1. **Lines along rows or columns.** The ink mask (Sauvola) is cut into strips; each strip's ink profile
   swings between line and gap when cut across the lines. The axis with the larger swing is the text axis.
2. **Levelling.** Lines tilted by up to 20 degrees are levelled by the shear (1 degree steps, refined by half a
   step) that makes the row profile swing most. Without this nothing above about 3 degrees of tilt was read.
3. **Up from down.** In Latin, ascenders and capitals put more ink above the x-height core than descenders
   put below; in headline scripts (Devanagari, Bengali, Gurmukhi) one sharp row of ink sits at the top of
   each line. A page-level headline cue (strongest row near the top or bottom of the bands, sharp enough)
   outranks the asymmetry. `up_z` is the up score over its standard error across bands.
4. Confidence = sigmoid(axis term) x sigmoid(up term) (`params/orientation.toml`, hand set). A finding is
   emitted only when it reaches `fusion.emit_min_confidence` and the rotation is not 0.

`debug.orientation` and the features: `rotation_deg`, `confidence`, `axis_ratio`, `up_score`, `up_z`,
`agreement`, `headline`, `bands`, `mean_sharpness`, `mean_peak_at`. Parameters: `params/orientation.toml`.

## Evaluation

`bench/orientation_synth_eval.py`: A4 at 150 dpi, 10 scripts, each page turned 0/90/180/270, four conditions;
12 turned pages per cell (3 seeds x 4 turns), dev seeds and test seeds. The result is what you get by **acting
on the finding** (no finding = left as is). Wrong ones have their mean confidence in brackets.

| Script | clean | tilt 3 deg | noise 8 | blur 1.5 px |
|---|---|---|---|---|
| Latin | 12/12, 12/12 | 10/12 (0.43), 12/12 | 12/12, 12/12 | 10/12 (0.47), 10/12 (0.47) |
| Devanagari, Bengali, Gurmukhi, Tamil, Telugu, Malayalam | 12/12 | 12/12 | 12/12 | 12/12 |
| Gujarati | 12/12, 11/12 (0.35) | 12/12 | 12/12, 11/12 | 12/12 |
| Kannada | 12/12, 10/12 (0.42) | 12/12 | 12/12, 10/12 | 12/12 |
| **Arabic (pseudo-script)** | **6/12, 4/12** (0.22, 0.16) | 12/12 | 6/12, 4/12 | 12/12 |

(dev, test where two numbers are given; one number: both splits.) The wrong cells are pages left as they
were because the detector abstained or was unsure, at low confidence; none was turned the wrong way with
high confidence. Look-alikes, **no misoriented finding on the blank, photo, table, form, cover and text
golden pages**.

How it got there: the first confidence came almost entirely from the axis ratio and was high on pages with
no usable up cue; the product of two confidences fixed it. Headline scripts were turned upside down by the
Latin asymmetry rule until the page-level headline cue was added. Tables fired until `up_z` required the
cue to be significant.

## Determinism

Integer profiles; floats only for the shear search, sigmoids and ratios, through `math::*` and
`pairwise_sum`. Checked on golden pages by the native, Python and WASM runners.

## Failure gallery

| Case | What happens | Where |
|---|---|---|
| **Sparse forms** | Few text bands, little up cue: abstains. In the plan benchmark 4 of 9 turned forms per split were left alone (none wrongly turned). | Measured (synthetic) |
| **Arabic-like text** | Joined letters have no clear headline and little asymmetry in the synthetic version: 4 to 6 of 12 clean pages left turned. Real Arabic or Urdu: not tested. | Measured (synthetic) |
| **Tilt over 20 degrees** | The shear search stops at 20; beyond that the axis reading fails. | By construction |
| **Mirrored page** (flipped, as seen through paper) | Not a turn: not detected, and text read through the paper looks like mirror writing. | Open |
| **Pages of a drawing, a map, a photo** | No text lines: abstains. Engineering sheets with title blocks in a corner are not read for orientation. | By construction |
| **Mixed orientations on one page** (rotated stamp text, side margins) | The strongest axis wins; the minority text is ignored. | By construction |
| **Real scans, real fonts, other scripts** | Never seen. Pseudo-glyph scripts are the author's drawing of a script, not the script. | Open |
