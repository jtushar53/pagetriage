# `photo` (module M10, region typing)

Finds **pictures**: areas of continuous tone, such as an identity photograph on a deed or a pasted
image. It reports `kind: "photo"` with a page-coordinate `bbox` and `attributes` `area_sq_in`, `fill`.
The detector is **opt-in** (`"detectors": ["photo"]`) and reads the gray page only. The page typing
([page_type](page_type.md)) counts photo regions in its graphic feature. Together with the layout
blocks, the tables and rulings, handwriting, stamps and the other findings this is the page's **region
map**; there is no separate region type beyond the findings.

**Read this first**: checked only on synthetic pictures drawn by numpy (smooth noise, an oval portrait
with texture, a passport-sized one, a busy one with sharp blobs, a dithered one). No real photograph has
been looked at.

## Algorithm

1. The paper is the most common gray. A pixel is **mid-gray** when it is between `paper - ink_depth`
   (150) and `paper - paper_band` (20), and **gentle** when the 3 x 3-smoothed gray changes by at most
   `flat_gradient` (6 levels per pixel, over x and y across four pixels). Mid-gray pixels on a steep
   edge (the edge of blurred text) do not count.
2. The page is cut into `tile_in` (0.15 in) tiles; a tile is **tone** when at least `min_mid_share`
   (40%) of its pixels are gentle mid-gray.
3. Tone tiles are closed (dilate then erode, radius 1 tile) so a white sky or a black shadow inside a
   picture does not split it, and joined into 4-connected regions.
4. A region is kept with at least `min_area_sq_in` (0.8) of tiles and `min_fill` (50%) of its box in
   tiles. Confidence `sigmoid(-1 + 0.8 * (area - 1.5) + 5 * (mid_share - 0.4))`, hand set, not
   calibrated.

`debug.photo`: `candidates`, `photos`. Parameters: `params/photo.toml`.

## Evaluation

`bench/photo_synth_eval.py`: A4 at 150 dpi, text page with a picture of 0.9 to 4 square inches
(`passport`: 1.4 x 1.8 in) pasted in. A picture is found at IoU 0.5; 6 dev and 6 test seeds each.

| Picture | clean | sensor noise 8 | blur 1 px | Spurious per page |
|---|---|---|---|---|
| smooth noise | 6 / 6, 6 / 6 | 6 / 6, 6 / 6 | 6 / 6, 6 / 6 | 0 |
| portrait with sky | 6 / 6, 6 / 6 | 6 / 6, 6 / 6 | 6 / 6, 6 / 6 | 0 |
| passport-sized | 6 / 6, 6 / 6 | 6 / 6, 6 / 6 | 6 / 6, 6 / 6 | 0 |
| busy (sharp blobs, grain) | 6 / 6, 6 / 6 | 6 / 6, 6 / 6 | 6 / 6, 6 / 6 | 0 |
| **dithered, black and white** | **0 / 6, 0 / 6** | **0 / 6, 0 / 6** | 6 / 6, 6 / 6 | 0 |

(dev, test in each cell.) Look-alikes, spurious per page, 0 on both splits: a text page, a ruled table,
a solid black disc, an inked ring stamp, dense 7 pt text, a fingerprint-like ridge oval, blank paper.

How it got there: the first version counted any mid-gray pixel and gave **2.0 to 2.3 spurious findings
per page at blur 1 px**, because blurred text is mid-gray along its edges and its blocks passed the
tile test. Requiring the mid-gray to be gentle removed them (and the clean and noisy rows were
unchanged). The perfect rows are the shape of the test: smooth synthetic pictures against clean text.

## Determinism

Integer throughout, apart from the sigmoid and area. Checked on a golden page (`photo_page`) by the
native, Python and WASM runners.

## Failure gallery

| Case | What happens | Where |
|---|---|---|
| **Photocopied, dithered or thresholded photos** | A black-and-white picture has no mid-gray: not found (0 / 12 clean, 0 / 12 noisy). Blur 1 px turns the dither into gray and it is found. Most photos on photocopied deeds look like this. | Measured (synthetic) |
| **Real photographs** | Never seen. Faces, glare, low contrast prints, tinted paper. | Open |
| **Low-contrast prints** | A picture whose gray stays within 20 levels of the paper is paper to this detector. | By construction |
| **Pictures with a dark border or solid regions** | Solid ink is not mid-gray; the closing joins a few tiles but a mostly solid image (a logo block) is not found. | By construction |
| **Small photos** | Under 0.8 square inch: not found. A thumbnail photo is 0.5 to 1 square inch. | By construction |
| **Gray washes and stains** | A smooth gray stain (coffee, a fold shadow) over 0.8 square inch looks like a picture. Not tested. | Open |
| **Colour** | Reads gray only; a coloured picture is judged by its luma. | By construction |
