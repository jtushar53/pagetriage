# `page_type` (module M10)

Gives each page one **type**: `text`, `text_with_annotations`, `form`, `table`, `photo`, `mixed`,
`blank` or `cover_stamp_paper`, with a confidence and the other labels that scored at least 5%
(`alternatives`). It fills the report's `page_type` field. It is not a detector with findings of its
own: it reads the layout and the findings the other detectors made on the page.

It runs when the opt-in `layout` detector is named (`"detectors": ["layout", ...]`), because it needs
the layout pass, and otherwise leaves `page_type` as `null`. It is only as good as the detectors that
ran: with `layout` alone it can tell text from a picture or an empty page and little else; handwriting,
rulings, signature, thumb, redaction, qr and stamp feed the rest. **Read this first**: the pages used
to set the coefficients and to check them are synthetic and drawn by the author of the typing.

## Algorithm

1. **Eight features** in [0, 1], each a count or share over a scale in `params/page_type.toml`:
   `text` (layout lines), `rules` (printed rules: the horizontal and vertical counts in the
   `ruling_lines` findings, each weighted by how long the longest rule is, plus checkbox grids),
   `table` (the largest table's share of the page), `annot` (handwriting, signature, stamp, seal, thumb
   and redaction findings), `graphic` (the larger of the graphic components' boxes, the dark area
   that is not text counted on the gray page, and the area of `photo` findings), `qr_top` (a QR code in the top band), `header` (a
   `stamp_paper_header` finding, see [fusion](fusion.md)) and `empty` (`1 - ink / 0.2%`).
2. **A linear score per label**: a bias plus the features times that label's coefficients (eight
   labels, 9 numbers each, most zero; 49 non-zero in all, just under the limit of about 50).
3. **Softmax** over the eight scores (`math::exp`, temperature 1). The winner and its probability are
   reported; the others at 5% or more are `alternatives`.

The confidence is the winner's softmax probability: a hand-set sigmoid-like number, **not
calibrated**. With `{"debug": true}` the features appear under `debug.page_type` (`lines`,
`rules`, `table_share`, `annotations`, `graphic_share`, `ink_fraction`, `qr_top`, `header`).

## Parameters

`crates/pagetriage-core/params/page_type.toml`: every number is hand set; the feature scales
(`table_full`, `graphic_full`, ...) and the coefficients were adjusted on the dev seeds below.

## Evaluation

`bench/page_type_synth_eval.py`: A4 at 150 dpi, eight types (text; text with handwritten lines and a
signature; a printed form with handwriting; a ruled table; a smooth dark picture; blank paper; a
stamp-paper-like cover with a large heading and a QR-like symbol; text above a picture). 6 dev seeds
(1 to 6) and 6 test seeds (11 to 16) per type and condition (clean, noise 8, blur 1 px). Detectors on:
layout, handwriting, rulings, signature, thumb, redaction, qr.

| Condition | Correct, dev | Correct, test | Misses |
|---|---|---|---|
| clean | 47 / 48 | 48 / 48 | one cover typed as text |
| sensor noise 8 | 47 / 48 | 48 / 48 | the same cover |
| blur 1 px | 48 / 48 | 48 / 48 | |

How it got there: the first coefficients typed **form 0 / 6** (all as text with annotations), **photo
2 of 6, table 4 of 6, mixed 0 of 6**. The causes were not the weights alone: handwriting was counted
by finding (8 findings on a page saturated the annotation feature), a form's field rules and a
signature's stray strokes counted as the same "rules" (fixed by weighting rules by length), a table
page's text lines outvoted the table (fixed by a less negative table bias), and a picture is mostly
smooth gray, which Sauvola's ink mask does not see, so the graphic share came to about 0 (fixed by also
counting dark area on the gray page). The test seeds were run after the dev adjustments and were not
used to change anything.

The 48 / 48 is the shape of the test: pages drawn to be of the type, one type each, the same
generators for dev and test, and detectors whose own thresholds were tuned on similar pages. Typing the
golden fixtures (320 x 260 px, `tests/golden/layout_inputs/`) gives sensible labels for most and a
`photo` at 0.35 for the thumb impression page and a `table` for the small form page, which shows how
page size and layout shift the features.

## Determinism

`f64` sums by `math::pairwise_sum`, `math::exp` for the softmax, outputs rounded to 6 decimals. Checked
on the golden pages by the native, Python and WASM runners.

## Failure gallery

| Case | What happens | Where |
|---|---|---|
| **Real documents** | Never seen. Real forms, mixed pages and photographs differ from the drawn ones in every feature. | Open |
| **Detector false positives** | A picture produces handwriting and signature findings (four `signature` findings on one dark picture), which count as annotations; the graphic share outweighs them here, but a small picture would not. | Dev run of the eval |
| **Cover pages** | Needs a header (a tall heading in the top band) and a QR; one cover in 12 clean ones had no header finding and was typed as text. Stamp-paper headers have not been checked against any stamp paper. | Dev seed 4 |
| **Small pages** | The scales are shares of the page; the 320 x 260 golden pages type differently from A4 pages of the same content. | Golden fixtures |
| **Photograph of a page, curled or dark** | Not tested; the dark-area feature would call a dark scan a picture. | Open |
| **Table versus form** | Told apart by the table's share of the page (12%) and by long rules; a form with a big table in it is typed as a table. | By construction |
| **Mixed** | Needs both a picture and text; text with a seal or a stamp is `text_with_annotations`, not `mixed`. | By construction |
| **Without `layout`** | No page type at all. | By construction |
