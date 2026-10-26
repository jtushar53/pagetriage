# `thumb` (module M8, `pixel.thumb_impression`)

Finds **thumb and finger impressions**: compact, dark ovals of ridges. It reports
`kind: "thumb_impression"` with a page-coordinate `bbox` and `attributes`: `width_in`, `height_in`,
`ridged_blocks`, `coherence`, `orientation_spread`, `dark_fraction`. The detector is **opt-in**
(`"detectors": ["thumb"]`). It works on the gray page (no layout, no colour). No model.

**Read this first.** Checked on synthetic prints drawn by the author of the detector: neat, concentric,
elliptical ridges, which is the easiest thing a ridge detector can be asked about. Real inked
impressions on deeds are smudged, partial, rolled, over- or under-inked, and often scanned at 150 dpi,
where a ridge is 3 pixels across. It finds **none** of the synthetic prints that were faint or
blurred to fusion, and the real ones are likely to be harder than those. See the
[failure gallery](#failure-gallery).

## Algorithm

1. **Blocks.** The page is cut into blocks of 0.1 in (15 px at 150 dpi). In each, the Sobel gradient's
   structure tensor `(Gxx, Gyy, Gxy)` gives the energy `(Gxx + Gyy)` per pixel and the coherence
   `|(Gxx - Gyy, 2 Gxy)| / (Gxx + Gyy)` (the measure used to segment fingerprints by Bazen and Gerez,
   IEEE TPAMI 24(7), 2002; their thresholds were not used).
2. **Ridge test.** A block is *ridged* when the energy is at least `min_energy` (4000), the coherence
   at least `min_coherence` (0.35), and the ridge period is in range. The period is
   `2 * line length / zero crossings` of the mean-removed gray along a line of two block sides through
   the block centre in the gradient direction, between `min_period_in` (0.014 in) and `max_period_in`
   (0.05 in); a finger's ridges are about 0.02 in apart.
3. **Regions.** Ridged blocks that touch (8 neighbours) form a region. Its sides must be between 0.45
   and 1.6 in, its aspect at most 1.8, and ridged blocks must fill between 45% and 98% of its box (an
   ellipse fills 79%).
4. **Texture of the region.**
   - `orientation_spread`: one minus the length of the mean of the doubled gradient angle's unit vector
     over the blocks (circular variance). Ruling, hatching and a barcode have one direction and score
     near 0; loops and whorls are well above. At least `min_orientation_spread` (0.12).
   - `dark_fraction`: the share of box pixels at least `dark_delta` (60) below the paper level
     (the most common bright gray). A print is about half ink, a text block about a tenth. At least 0.3.
5. **Score** `sigmoid(prior + terms)` over coherence, orientation spread, dark fraction and fill, each
   `coef * (value - threshold)` clipped to `+-term_clip`; emitted at the fusion threshold.

The confidence is a hand-set sigmoid and is **not calibrated**.

`debug.thumb`: `candidates`, `impressions`.

## Parameters

`crates/pagetriage-core/params/thumb.toml`, all placeholders, set from looking at the synthetic dev
pages. Only the period range has a physical basis (ridge spacing of about half a millimetre, widened).

## Evaluation

`bench/thumb_synth_eval.py`: A4 pages at 150 dpi, a Latin text page with one print (0.7 to 1.1 in wide,
0.85 to 1.3 in tall, ridge period 3 to 5 px, `bench/synth/printgen.py`) in the lower half; a print is
found when a finding overlaps the drawn box with IoU of at least 0.5. 6 dev seeds (thresholds set
looking at them) and 6 test seeds.

| Condition | Found, dev | Found, test |
|---|---|---|
| clean | 5 / 6 | 6 / 6 |
| blur 0.8 px | 6 / 6 | 6 / 6 |
| faint (ink pressure 0.45) | **1 / 6** | **1 / 6** |
| smudged (ridges blurred by 1.8 px) | **0 / 6** | **0 / 6** |

Spurious findings on look-alike pages (a block of text, parallel-line hatching, a barcode, a halftone
dot grid, a solid ink blob, diagonal hatching, a plain page): **0 per page on every one**, on both
splits.

The look-alike result is easier than it sounds: the detector got there on the first run, with no
tuning against them, because each fails one of the checks it already had (text and halftones are not
dark enough or coherent, hatching and barcodes have one orientation, a solid blob has no gradient
inside). A texture that is dark, coherent *and* has varied orientation and the right ridge period
(a coarse fingerprint-like pattern, a woodgrain stamp, a fingerprint pressed on a different form) would
pass.

The faint row fails on gradient energy: at pressure 0.45 the ridge contrast is below `min_energy`.
Lowering it would let faded text and shading through. The smudged row is the real worry: an impression
whose ridges have fused into an ink blob has none of the features, and "an inked blob where the
thumb goes" is probably what many real ones look like.

Cost: on a 1240 x 1754 page, release build, medians of 7 runs: quality alone 168 and 179 ms, with `thumb`
191 and 185 ms (so roughly 10 to 25 ms).

## Determinism

The gradient sums, energy test and block counts are integer; coherence, the zero-crossing period, the
doubled-angle sums and the sigmoid use IEEE operations, `sqrt` and `math::pairwise_sum`. Regions are
sorted by size, then by position. Checked on the golden page by the native, Python and WASM runners.

## Failure gallery

| Case | What happens | Where |
|---|---|---|
| **Smudged or blob impressions** | Not found (0 of 12): no ridges left to measure. | "smudged" row |
| **Faint impressions** | 2 of 12: under the gradient energy floor. | "faint" row |
| **Partial prints** | A print missing a third of its area fails the fill window (0.45 to 0.98) or the size window if cut to a strip. Not measured. | By construction |
| **Prints at 100 dpi or less** | A ridge is 2 px or less: the period test fails. Not measured. | Open |
| **A print over text or a ruling** | Text strokes in the box add dark, incoherent blocks and shift the box; a ruling adds one orientation. Not measured. | Open |
| **Other ridged patterns** | A coarse hologram, a guilloche, a woodgrain texture of the right period and size could pass. Not tried. | Open |
| **Very large or very small prints** | Outside 0.45 to 1.6 in: not found. A palm print is out of range on purpose. | By construction |
| **Colour** | Not used: a red or blue ink print is judged in grays. | By construction |
| **Real impressions** | Never seen. | Open |

## Golden fixture

`tests/golden/layout_inputs/thumb_page.pgm`, drawn by `tests/golden/gen_inputs.py` with integer
arithmetic and `math.isqrt`: three lines of text, a 120 x 150 px ellipse of concentric ridges (period 4),
and a block of horizontal hatching with the same period. `tests/layout_golden.rs` pins one impression
on it (on the ridged oval, not the hatching) and none on the other pages.
