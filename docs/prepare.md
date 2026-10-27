# `prepare` (module M14: apply the page plan)

`pagetriage prepare PAGE -o DIR` turns a scan into what an OCR engine or a vision-language model reads
best: **upright, level, cropped to its content, at a sensible size, with the regions cut out and routed**.
It is the [page plan](detectors/plan.md) carried out. No model; the same input gives the same files and the
same manifest, byte for byte, on every machine (integer pixel arithmetic; the one rotation takes its sine
and cosine from libm).

**Read this first**: checked only on synthetic forms. Whether the images actually make a downstream model
read better is untested: no OCR or vision model has been run on them.

```sh
pagetriage prepare scan.png -o out/                  # dpi from the file header, else 150 (said on stderr)
pagetriage prepare scan.png -o out/ --rotate 90      # you know it was scanned sideways: turn first
```

```
out/
  manifest.json            what was done and what each file is (also printed on stdout)
  page.png                 the content box of the turned, levelled page, scaled
  tiles/tile_r0_c1.png     instead of page.png, when the content is over 2048 px on a side
  regions/region_003_handwriting.png ...   one per plan region, in plan order
```

Options: `--dpi`, `--max-pixels`, `--detectors` (default: everything that feeds the plan, see
`params/prepare.toml`), `--params`, `--no-gate`, `--rotate DEG`.

## What it does

1. Analyses the page with the plan (and the layout, orientation, script, handwriting, rulings, QR, ...).
2. If the plan says the page is **sideways or upside down**, turns it a quarter turn from the original
   pixels and analyses again. If it was already levelled on the sideways page, that levelling is thrown away.
3. If the measured **skew** is between 0.5 and 20 degrees, levels the page about its middle onto a canvas big
   enough to keep every corner (bilinear, paper colour around it) and analyses again.
4. With the final plan, writes: `page.png` (content box, scaled so the x-height is about 24 px, within
   0.5x to 4x and 8192 px; a factor within 10% of 1 is not applied), or the tiles when the content is over
   2048 px on a side; and one image per region whose route is not `decoded` or `note`, cut with 0.02 in of
   margin and scaled the same way.
5. A blank page gets no images; the manifest says `skip: blank`.

At most 4 analyses per page (`max_analyses`). About 2 s for an A4 form at 150 dpi.

## The manifest

`manifest.json` (schema id `pagetriage-prepare/0.1`, canonical JSON): `source_sha256` and `frame_sha256`
(cache keys for the page as given and the prepared page), `dpi`, `steps` (the turn and the levelling that
were applied, in order), `analyses`, `skip`, `notes`, the final `plan`, and for each image in `outputs`:
`file`, `kind` (`page`, `tile`, `region`), `route`, `region_kind`, `priority`, `bbox`, `scale`, `dpi` (the
page's dpi times the scale), `width`, `height`, `sha256` of the pixels.

All boxes are in the **prepared frame**: the page after the turn and the levelling, before cropping and scaling.
`frame` gives how that frame relates to the page as given (`rotate_cw_deg`, `deskew_cw_deg`, the sizes),
and `pagetriage_core::prepare::Frame::to_source(x, y)` maps a point back, so a word an OCR run found
in `region_003` can be put at its place on the original scan: point in the region image, divide by `scale`,
add the box corner, then `to_source`. A unit test checks the mapping against the pixels.

## Evaluation

`bench/prepare_synth_eval.py`: A4 at 150 dpi forms with printed labels and handwritten fill-ins, seeds 1 to 3
(dev) and 11 to 13 (test); each tilted by 0, 3, 6 or 10 degrees (Pillow) and turned 0, 90, 180 or 270, then
prepared by the release CLI. "Turn right" is the quarter turn that restores the page (or none, for pages not
turned); "abstained" is no turn although the page was turned; "wrong" is another turn. The level columns are
the skew `analyse` measures on `page.png` for pages turned right, and the skew it measures on the input.

| Tilt | Split | Turn right | Abstained | Wrong | Level (< 0.5 deg) | Median skew left | Skew of input | Page crop |
|---|---|---|---|---|---|---|---|---|
| 0 | dev, test | 8/12, 8/12 | 4, 4 | 0, 0 | 1.00 | 0.00 | 0.00 | 0.33, 0.34 |
| 3 | dev, test | 10/12, 7/12 | 2, 5 | 0, 0 | 1.00 | 0.00 | 3.00, 2.95 | 0.30, 0.31 |
| 6 | dev, test | 10/12, 9/12 | 2, 3 | 0, 0 | 1.00 | 0.00 | 6.00 | 0.27, 0.28 |
| 10 | dev, test | 12/12, 10/12 | 0, 2 | 0, 0 | 1.00 | 0.00 | 10.00 | 0.25 |

"Page crop" is the area of `page.png` (before scaling) over the input's area: the forms fill about the top third
of the page, so a model is given a third of the pixels (a tilted page is measured against its larger canvas).
**No page was turned the wrong way**; every page turned right came out level; about 3 in 10 of the turned pages were
left turned because the orientation detector abstained (22 of the 72 turned pages, about 3 in 10).

The skew is read by the same estimator that chose the angle, so the near-zero residual shows the levelling is
consistent with it; that the estimator is right comes from the input baselines matching the tilt applied
(3.00, 6.00, 10.00) and from `bench/quality_synth_eval.py`.

Tests: `tests/prepare.rs` (turned pages come back with the right step and the original pixels, a tilted page
levels, a blank page is skipped, outputs are described and reproducible), the unit tests in
`src/prepare.rs` (rotations, resizing, the frame against the pixels) and four CLI tests.

## Failure gallery

| Case | What happens | Where |
|---|---|---|
| **Turned page, orientation unsure** | The detector abstains (4 of 9 turned forms per split with no tilt; 22 of 72 over all tilts), the page is prepared as it is, and the manifest says `the page may be turned N degrees clockwise (confidence c, too low to act on)` when it read a turn but too weakly. Pass `--rotate`. | Measured (synthetic) |
| **Skew measured on a sideways page** | If the first reading is of a sideways page the skew is meaningless; the page is levelled, then turned when the second reading is sure, and the levelling is redone. Costs analyses; if the budget runs out the page may be left unlevelled. | By construction |
| **Skew over 20 degrees** | Not levelled; a note says so. | By construction |
| **Handwriting regions** | Only as good as the handwriting detector (about half of handwritten words in synthetic forms): the rest are cut as `ocr` text blocks. | Measured, see [plan](detectors/plan.md) |
| **Scale from a wrong x-height** | A page of capitals or digits, or a photo, gets a wrong factor. | Open |
| **Colour** | Colour pages stay colour (RGB PNG); the paper colour for the levelling canvas is the mean colour of the most common gray. Not tested on real colour scans. | Open |
| **Levelling blurs a little** | Bilinear resampling softens thin strokes slightly; below 0.5 degrees nothing is resampled. Not measured against an OCR engine. | Open |
| **Mirrored pages, folds, curved pages** | Not corrected; a book curve is not a tilt. | Out of scope |
| **Python and WASM** | Only the Rust API and the CLI; the bindings do not expose `prepare` yet. | Open |
| **Does it help a model?** | Untested. The images are what the plan says a reader needs, not a measured improvement. | Open |
