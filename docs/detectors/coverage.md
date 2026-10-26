# `coverage` (extraction coverage)

Says how much of a page's text a **text layer or an OCR run actually covers**. It takes text boxes (where
the extracted text sits on the page) and the page's pixels, and reports `kind: "extraction_gap"` when text
on the page is not under any box: the PDF has a text layer but it is incomplete, or the OCR missed a block.
The detector is **opt-in** (`"detectors": ["coverage"]`) and runs on the [`layout`](layout.md) pass.

It does not read text and it does not say what is missing, only where and how much ink is not covered.

## Where the boxes come from

- `Options.text_boxes` (`"text_boxes": [[x0, y0, x1, y1], ...]`): boxes in page pixels at the analysis
  DPI, from an OCR run or any other source. `[]` means "no text"; leaving the option out means "unknown".
- Otherwise the PDF's own: `analyse_pdf_structure` returns `text_boxes_pt` for each page (every
  text-show operation of any render mode, so a **hidden OCR layer under a scan counts**, as points from
  the top-left of the page box, estimated with a flat 0.5 em per glyph). Pass that page structure as
  `structural_json` and the boxes are scaled to pixels. The structure has `null` boxes on a rotated
  page, or when the content was cut short by a limit, and the detector then knows nothing.

## Algorithm

1. Rasterise the boxes (grown by `box_pad_in`, 0.03 in, on every side) into a page-sized mask.
2. For each glyph of the layout pass, take its centre in the page frame; it is covered if the mask is set.
3. A text line is covered when at least `covered_line_fraction` (50%) of its glyphs are covered.
4. `ink_covered_share` is the covered glyph ink area over all glyph ink. With at least `min_lines` (3)
   lines, `uncovered = 1 - ink_covered_share` gives one evidence entry; confidence
   `sigmoid(prior + clip(6 * (uncovered - 0.2)))`, so a page with more than 20% of its ink uncovered is
   reported. The finding's bbox is the union of the uncovered lines; attributes: `lines`,
   `lines_uncovered`, `ink_covered_share`, `boxes`.
5. Boxes with no glyph in them (a layer for text that is not on the page) are counted in
   `debug.coverage.boxes_without_ink`.

`debug.coverage`: `have_boxes`, `lines`, `lines_covered`, `ink_covered_share`, `boxes`, `boxes_without_ink`.
Parameters: `params/coverage.toml`, hand set.

## Evaluation

`bench/coverage_synth_eval.py`: A4 at 150 dpi, 24 to 30 lines of Latin text; the boxes are the true
word boxes of a chosen share of the lines. 6 dev and 6 test seeds.

| Boxes over | Mean `ink_covered_share`, dev / test | True share of lines | Largest difference | Gap found, dev / test |
|---|---|---|---|---|
| all lines | 1.00 / 1.00 | 1.00 | 0.00 | 0 / 6, 0 / 6 |
| 70% of lines | 0.70 / 0.68 | 0.70 | 0.03 | 6 / 6, 6 / 6 |
| 40% of lines | 0.41 / 0.39 | 0.40 | 0.04 | 6 / 6, 6 / 6 |
| none | 0.00 / 0.00 | 0.00 | 0.00 | 6 / 6, 6 / 6 |
| all lines, boxes a third of a line too low | 1.00 / 1.00 | 1.00 | 0.00 | 0 / 6, 0 / 6 |

The boxes here are exact. A PDF's boxes use a flat glyph width and are rougher; no real PDF or scan
has been looked at, and the PDF fixture has hidden OCR boxes only as a structure test (`scanned_ocr.pdf`
gives two boxes per page). The first version used a prior of -1, which let 70% coverage (30% of the ink
uncovered) through at confidence 0.40; the prior is 0 now, which is "more than 20% uncovered".

## Determinism

Integer masks and sums; one `f64` sigmoid. Checked on a golden report (`tests/golden/coverage_expected/`)
by the native, Python and WASM runners.

## Failure gallery

| Case | What happens | Where |
|---|---|---|
| **Rough PDF boxes** | Flat glyph widths make boxes too short or long; the padding (0.03 in) and the 50% line rule absorb some. A line of wide capitals may fall outside its box. | By construction; not measured |
| **Rotated PDF pages, truncated content** | No boxes, so no verdict. | By construction |
| **Pictures with text** | Text in a picture (a stamp, a logo) is a glyph to the layout and is reported as uncovered if no box sits on it. Often right (OCR did miss it), sometimes not what the caller means. | By construction |
| **Handwriting, signatures** | Layout calls some of it text; uncovered handwriting is reported. | By construction |
| **Skewed scans** | The layout is levelled by the quality skew and glyph centres are mapped back, but the supplied boxes are taken as already in page coordinates. Not measured. | Not tested |
| **Few lines** | Under three lines: no verdict. | By construction |
| **Boxes of the wrong page size** | Taken in page pixels at the analysis DPI; a mismatch looks like a gap. | By construction |
