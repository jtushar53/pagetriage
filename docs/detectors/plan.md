# `plan` (module M14: page plan for the model that reads next)

A deterministic **work order for a page**: what to do before a downstream reader (OCR engine, vision-language
model) looks at it, and where to point it, so the reader spends less effort and sees an easier input. It
is arithmetic on what the other detectors found; no model. Opt-in: `"detectors": ["plan"]` (the layout,
quality, orientation and the other detectors it uses are run for it). The result is `PageReport.plan`, `null`
when not asked for.

**Read this first**: the thresholds are hand set from the author's idea of what an OCR engine or a
vision-language model needs. Nothing has been checked against a real downstream reader or a real document.
The routes are only as good as the detectors behind them (handwriting is weak, see below).

## What is in the plan

| Field | Meaning |
|---|---|
| `rotate_clockwise_deg`, `needs_rerun` | Quarter turn to apply, from the [orientation](orientation.md) finding. When non-zero the page's other findings were measured sideways: **turn, then analyse again** (`needs_rerun`). |
| `deskew_clockwise_deg` | The M3 skew, or `null` when a turn is needed first. |
| `scale` | Measured x-height in px, the target (24 px), the factor (kept in 0.5 to 4) and the `suggested_dpi` to render at. |
| `content_bbox` | Ink box plus 0.1 in margin: crop to it. |
| `binarizer` | The M3 recommendation. |
| `regions` | Boxes with a `route` (`ocr`, `handwriting`, `table`, `mark`, `vision`, `decoded`, `fields`, `note`), a `kind` and a `priority`, sorted by priority then y, x. Text blocks lying 60% inside a table, mark, photo or QR are dropped; a block 50% inside a handwriting finding is routed to `handwriting`. A decoded QR is `decoded` (nothing for a model to do). |
| `tiles` | When the content box (not the whole canvas) exceeds 2048 px on a side: overlapping (8%) tiles of it whose cuts go through the least-ink row or column. |
| `skip` | `blank`, or `near_duplicate` of an earlier page (set by the document pass; cleared if no longer a duplicate). |
| `difficulty` | 0 to 1: quality badness plus bonuses (handwriting, table, a turn, small text). |
| gated page | On a badly blurred page (the [quality gate](quality.md#the-quality-gate)) the line detectors did not run: one `vision` region over the whole page and a note. |
| `scripts` | Script families found, for choosing a recogniser. |
| `input_sha256` | Hash of the pixels and size: a **cache key** for the downstream result. Same input, same key, on every platform. |
| `notes` | Plain-text reasons. |

## Evaluation

`bench/plan_synth_eval.py`: A4 at 150 dpi synthetic forms with printed labels and handwritten fill-ins whose
boxes are known; 6 dev and 6 test seeds, four conditions. A word counts as routed when 60% of its box is in a
region with the right route.

| Condition | Handwritten words to `handwriting` (dev, test) | Printed words to `ocr` | Share of page sent |
|---|---|---|---|
| clean | 70/98, 63/101 | 108/108, 108/108 | 0.31, 0.32 |
| tilt 3 deg | 42/98, 47/101 | 108/108, 108/108 | 0.35, 0.35 |
| noise 8 | 70/98, 63/101 | 108/108, 108/108 | 0.31, 0.32 |
| blur 1 px | 72/98, 77/101 | 108/108, 108/108 | 0.32, 0.32 |

The handwriting figures are the [handwriting detector's](handwriting.md) recall (the plan adds none of its
own): a missed word is sent to OCR as print. The share sent is what a downstream model would be given in
place of the whole page: about a third on these forms. **Turned pages** (3 forms x 3 turns per split): the
right turn 5 and 5, abstained 4 and 4, **wrong 0 and 0**.

Mechanical checks (`tests/plan.rs`, 7): routes, decoded QR, blank skip, not planned unless asked, cache key,
a sideways page gets `needs_rerun`, tiles avoid a dark bar.

## Determinism

Integer boxes and hashes; scale and difficulty by `math::*`. Golden pages carry plans (`layout_*`,
`qr_page`, ...) and the document golden marks the duplicate page's plan as skipped; native, Python and WASM
agree.

## Failure gallery

| Case | What happens | Where |
|---|---|---|
| **Handwriting missed** | 42 to 77 of about 100 handwritten words routed correctly; the rest go to OCR, which misreads them. Tilt hurts most. | Measured (synthetic) |
| **Turned page, no confident cue** | `rotate_clockwise_deg` 0, no rerun flagged; a sideways form is sent as is. | Measured (synthetic) |
| **Scale from a wrong x-height** | On a page of mostly capitals or digits, or a photo, the measured x-height is off and so is the factor. | Open |
| **Drawings** | Symbols, dimension lines and title blocks are not regions; a sheet becomes tiles and text blocks. | By construction |
| **Two-column and table pages** | Region order is priority then y, x, not reading order. | By construction |
| **Near-duplicate skip** | Rests on the perceptual hash; a page moved by 3% or turned is not seen as a copy (see [phash](phash.md)). A skip is advice. | By construction |
| **Difficulty** | Hand-set sum; not calibrated to any reader's error rate. | Open |
| **Cache key** | Pixels only: the same page analysed with other options has the same key. | By construction |
