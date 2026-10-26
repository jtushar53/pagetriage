# `quality` (module M3, `pixel.quality`)

Degradation triage. It fills `PageReport.quality` for every page, whichever detectors are selected,
and picks the binariser the rest of the pipeline should use. It is a set of hand-designed
measurements with fixed thresholds: no model file, no learned coefficients.

| Field | Meaning | Unit and convention |
|---|---|---|
| `skew_deg` | Tilt of the text lines | Degrees, searched over +-15. **Positive is counter-clockwise on screen (text rises to the right), the sense of Pillow's `Image.rotate`.** To deskew, rotate by `-skew_deg`. |
| `blur_laplacian_var` | Variance of the 4-neighbour Laplacian over the text mask (Pech-Pacheco style focus measure) | Gray levels squared. Raw: it scales with contrast squared and with scan resolution. The grade uses an edge-based blur index instead (below). |
| `contrast_rms` | Standard deviation of the page's gray values over 255 | 0 to about 0.5. |
| `speckle_density` | Connected components smaller than a fraction of the text height, per page pixel | Specks per pixel. |
| `bleed_through` | Show-through of the reverse side | Score in 0..1. A heuristic, see below. |
| `fold_lines` | Number of fold lines | Count (horizontal plus vertical). |
| `photocopy_generation_est` | Estimated copier generation. **Experimental** | `value` 0..5, `confidence` at most 0.6; `null` when there is too little text. |
| `recommended_binarizer` | `sauvola` or `howe` | Howe when bleed-through or low contrast is flagged and there is ink to binarise. |
| `grade` | `A` to `D` | See "Grade". |

With `{"debug": true}` the raw features behind every field are in `debug.quality` (peak ratio of the
skew sweep, blur radius, ink/paper levels, ghost-stroke counts, fold depths, per-measure severities,
...). `bench/` tuning reads those.

## Algorithm

Everything is derived from one Sauvola binarisation (`sauvola.toml`; window 75, k 0.2), its
8-connected components (`pixel/components.rs`: two-pass labelling with a union-find that keeps the
smaller label as root, so labels are in raster order of first pixel), the gray histogram and a few
linear scans. The pass never runs Howe: Howe is only *recommended*. All arithmetic on pixels is
integer; the float work is IEEE `+ - * /`, `sqrt` and `math::sin/cos/exp`, so the report is
byte-identical on native, Python and WASM (`scripts/determinism.sh`).

**Text components.** A component is text when it has at least 3 pixels and its bounding box spans
less than 40% of the page width and height (frames, rules, fold shadows and the edge of a rotated
scan are not text). The **text height** is the area-weighted median bounding-box height of text
components of at least 6 pixels (weighting stops a crowd of specks from dragging the median down).
It stands in for the x-height: for cursive handwriting and for Devanagari words, whose headline
joins the letters, it is larger than a true x-height.

**Skew** (`skew.rs`). Projection-profile variance maximisation. Ink pixels of text components are
accumulated into bins of `y cos t + x sin t`; at the right angle text lines, and the shirorekha
headlines of Devanagari, Bengali and Gurmukhi, fall into few bins and the profile is spiky. The
criterion is the sum of squared bin counts (equal to the variance at a fixed bin count). A coarse
sweep (0.5 degree steps over +-15) is followed by a fine sweep (0.05 degree steps over +-0.5)
around the three best coarse local maxima. The image is never resampled. Points use linear binning
in sixteenths so the integer pixel lattice cannot favour some angles. Components much thicker than
the page's median stroke (blobs, the dark band a local threshold leaves beside a scan edge) are
dropped, and more than 60 000 points are thinned by a fixed stride. `debug.quality.skew_peak_ratio`
is the best fine score over the median coarse score: below about 1.5 the profile has no clear peak
and the angle should not be trusted (see the results).

**Blur** (`blur.rs`). The Laplacian variance over the text mask (ink of text components plus a
1-px ring) is reported as `blur_laplacian_var`. The grade uses a contrast-independent *blur index*:
`S = 2 sqrt(sum L^2 / sum ((E-W)^2 + (S-N)^2))`, the ratio of Laplacian to gradient over the mask,
which for a Gaussian-blurred step edge is `1 / (sqrt(2) s)` for blur radius `s`. The noise floor is
measured on clear paper (mean `L^2` and mean gradient power) and subtracted first. When the text's
Laplacian power is less than 2x the noise floor the edges are no stronger than the noise and the
page is read as very blurred (radius capped at 10 px). The index is converted to millimetres at the
page's dpi. On thin strokes it reads about 0.6 of the true Gaussian radius (table below).

**Contrast** (`tone.rs`). RMS contrast from the exact histogram. The paper level is the median of
the pixels the binariser called paper; the ink level is the 10th percentile of text ink pixels;
their difference over 255 is the ink/paper **peak gap**, used by the grade and the binariser rule.

**Speckle**. A component is a speck when both sides of its bounding box are at most
`max(2 px, 0.15 x text height)`. The count over the number of page pixels is `speckle_density`.

**Bleed-through** (`bleed.rs`, `paper.rs`). A show-through is faint ink on the paper. The paper
level is averaged on a grid of 4 px blocks over pixels that are neither ink nor next to ink, then
smoothed over 6 blocks. The *drop* of a pixel is how far its gray sits below the local paper level.
"Strong" ink is a drop of at least half the ink/paper gap, whatever the binariser says (a strong
show-through is dark enough for Sauvola to call it ink). Clear of strong ink (by 2 stroke widths),
pixels with a drop above a threshold are "faint". The threshold is the largest of 10% of the gap, 6
gray levels and 4 robust standard deviations of the paper's own noise. Two numbers result:
`mid_mass`, the share of clear pixels that are faint (the third mode between ink and paper), and
`ghost_stroke_ratio`, the faint pixels that form thin (mean width at most 2x the text's), stroke-
sized components, relative to the strong ink. The score is a weighted mean of the two, each scaled to
0..1. The "mirrored-stroke" test in the module definition needs the other side of the sheet to
compare with, so it is **not implemented**: a faint stroke is called ghost ink by contrast and size
alone. There is no published basis for this heuristic.

**Fold lines** (`folds.rs`). The paper block grid also gives, for every row (and column) of blocks,
the median paper level. A crease is a narrow valley in that profile: the level is lower than the
mean of the two levels 3 blocks to either side, by at least 5 gray levels and at most 35% of the
ink/paper gap ("low contrast"), the valley is almost as deep two blocks out (narrow), is at least
10 blocks and 10% of the page from the edge, shows in 80% of the columns where it can be measured
(and at least 25% of all columns must be measurable, because text hides the paper), and has
one-sided shading: the two shoulders differ by 0.15 to 1 times the valley depth (no difference is a
printed or pencilled rule, more is the edge of a region). The line must be at least 3 inches long at
the page's dpi. More than 4 full-width valleys in one direction are ruled paper, not creases. See
the failure gallery for what this misses.

**Photocopy generation** (`generation.rs`, experimental). The stroke width comes from a 3-4
chamfer distance transform of the ink (`stroke.rs`); twice the value on the centre line of each
stroke, less one pixel, is its width. Five measurements are turned into severities and averaged:
coefficient of variation of stroke width, mean width over the text height (falls with each
generation), the share of text components that are fragments (smaller than a quarter of the median
component), speckle density and the ink/paper gap. The average is scaled to 0..5. **No published
method was found and none of this has been validated on real photocopies**; the confidence is
`0.6 x min(1, text components / 300)`, never above 0.6.

**Binariser choice and grade** (`grade.rs`). Sauvola by default; Howe when `bleed_through >= 0.6` or
the peak gap is below 0.25, and only when at least 0.2% of the page is text ink. Each of skew, blur
radius (mm), peak gap, speckle density, bleed-through score and fold count becomes a severity in
0..1 (0 at its "good" value, 1 at its "bad" value, linear between; `params/quality.toml`).
`badness = 0.5 x worst severity + 0.5 x weighted mean severity` (weights: skew 0.5, blur 1, gap 1,
speckle 0.7, bleed 0.7, folds 0.5). **A** below 0.2, **B** below 0.4, **C** below 0.6, **D**
otherwise. Every number is a placeholder (see "What was tuned on what").

**Degenerate pages.** A blank, black or one-pixel page gives finite values: skew 0, contrast 0,
no generation estimate, Sauvola recommended (`tests/quality.rs`).

## Parameters

All in `crates/pagetriage-core/params/quality.toml` (flat keys, one comment each naming its source,
"tuned on <set>" or "placeholder"), loaded through `params.rs` and overridable per call, for
example `{"params": "[quality]\nskew_criterion = \"gradient\""}`. Most are placeholders; the ones
that came from the module definition (search range, 0.5 and 0.05 degree steps, the 0.15 x text
height speck size, the 0.6 cap on generation confidence, Howe on bleed-through or low contrast) say
so.

## Cost

Measured on an Apple M-series laptop, native release build.

| Page | Time |
|---|---|
| Synthetic A4 at 150 dpi (1240 x 1754, 2.2 MP, dense text), quality pass only | 54 ms in the Rust core; 60 ms through the Python call (JSON included) |
| Same page, WASM build (size-optimised profile), Node 22 | about 360 ms |
| 136 DIBCO pages (0.05 to 9 MP), Python call | median 29 ms, 90th percentile 79 ms, largest (8.9 MP) 319 ms; about 30 ms per megapixel |

Rough split for the A4 page (native): Sauvola 11 ms, labelling 3, distance transform and ridges 5,
skew 10, paper grid, dilations, blur and bleed-through 20, folds 1. The target was well under
100 ms natively for A4 at 150 dpi; it is met. The WASM size profile is about 6 times slower.

## Results

Honesty notes first. DIBCO is **not Indian-deed material**: it is Latin (and some Greek) ink on aged
paper, mostly handwriting, cropped to a few lines. It has no ground truth for degradation, so most
measures are checked against synthetic pages with known damage and against the author's own
eyeball. Parameters were tuned on a **tuning split by edition** (DIBCO 2009, 2012, 2016, 2018 and
2019 track A: 54 pages) and the rest (2010, 2011, 2013, 2014, 2017, 2019 track B: 82 pages) was
held out. See "What was tuned on what" for exactly what was looked at.

### Skew (acceptance: median error at most 0.3 degrees)

`bench/skew_eval.py`: each DIBCO page is rotated by 12 fixed angles in [-10, 10]; the error is
`est(rotated) - est(original) - angle`, so a page's own tilt cancels. 136 pages, 1632 measurements.
"Fail" is a measurement more than 1 degree off; "pages failing" are pages with at least one.

| Rotation | Median abs error | 90th percentile | Maximum | Measurements over 0.3 deg | over 1 deg | Pages failing |
|---|---|---|---|---|---|---|
| Pillow, **white fill** (the specified test) | 0.05 | 0.80 | 26.5 | 292 | 120 (7.4%) | 18 of 136 |
| Pillow, paper-coloured fill | 0.00 | 0.20 | 26.6 | 139 | 40 (2.5%) | 8 of 136 |
| Augraphy `Geometric`, whole degrees, black corners | 0.00 | 0.20 | 13.4 | 127 | 45 (2.8%) | 9 of 136 |

**The median target is met (0.05 deg against 0.3). The tail is not good:** 7.4% of white-fill
measurements are off by more than a degree.

White-fill result by split and edition (median / 90th percentile / maximum of the absolute error;
measurements over 1 degree):

| Set | Median | p90 | Max | over 1 deg |
|---|---|---|---|---|
| 2009 (tune) | 0.05 | 0.25 | 0.80 | 0 |
| 2012 (tune) | 0.05 | 1.25 | 2.00 | 25 |
| 2016 (tune) | 0.00 | 0.05 | 1.05 | 1 |
| 2018 (tune) | 0.30 | 1.55 | 1.65 | 22 |
| 2019 A (tune) | 0.05 | 0.60 | 1.25 | 12 |
| **Tune sets, 54 pages** | **0.05** | **0.85** | **2.00** | **60** |
| 2010 | 0.05 | 0.95 | 4.80 | 9 |
| 2011 | 0.05 | 3.00 | 4.15 | 24 |
| 2013 | 0.00 | 0.50 | 0.80 | 0 |
| 2014 | 0.00 | 0.05 | 0.10 | 0 |
| 2017 | 0.05 | 0.20 | 0.50 | 0 |
| 2019 B (papyri) | 0.25 | 6.00 | 26.5 | 27 |
| **Held-out sets, 82 pages** | **0.05** | **0.75** | **26.5** | **60** |

`skew_peak_ratio` tells which estimates to trust. Over all white-fill measurements whose original
and rotated pages both have a ratio of at least 1.8, 1.3% are off by more than a degree (and none at
2.2 or more); below 1.5 the failure rate is far higher. With the paper-coloured fill it is 0% at
1.8. A caller that wants a safe deskew should skip pages with a low peak ratio.

Synthetic pages (`bench/quality_synth_eval.py`, A4 at 150 dpi, error in degrees; rotations are off
the 0.05-degree grid): Latin text, Devanagari and Bengali pseudo-glyphs with a headline all stay
within **0.03 degrees** over -9.07 to 8.58 degrees.

### Bleed-through, against the author's labels

`bench/quality_eval.py`. The author labelled DIBCO pages by eye from thumbnails: 31 "show-through
plainly visible" and 56 "none" (the label lists are in the script). AUC, 1.0 perfect, 0.5 chance:

| Measure | Tune sets (10 vs 29 pages) | Held-out sets (21 vs 27) | All (31 vs 56) |
|---|---|---|---|
| `bleed_through` (the reported score) | 0.864 | **0.705** | 0.744 |
| `mid_mass` | 0.762 | 0.785 | 0.743 |
| `ghost_stroke_ratio` | 0.838 | 0.624 | 0.686 |

The held-out number is the honest one, and it is mediocre: the tuning data are few and the labels
are subjective. On the synthetic show-through sweep (`quality_synth_eval.py`, a mirrored second page
at strength alpha): alpha 0 gives 0.00, 0.08 gives 0.00, 0.15 gives 0.28, 0.25 gives 0.95, 0.40
gives 0.62, and 0.6 stays near 0.6. It is **not monotone**: once the ghost is dark enough to count
as strong ink the ghost ratio falls. Howe is recommended at alpha 0.25 and 0.4 but not at 0.15.

### Other measures, on synthetic pages with known damage

| Measure | What was done | Result |
|---|---|---|
| Blur index | Gaussian blur of radius 0, 0.5, 1, 1.5, 2, 3 px on a noisy (sigma 2) page | estimated radius 0.35, 0.41, 0.62, 0.84, 0.90, "very blurred" (cap 10); grades A, A, A, A, B, D. It tracks the blur but reads low, and saturates. |
| Speckle | 0.0002 to 0.003 of the pixels set to dark | measured / known = 0.78 to 0.81 (specks that merge with text or each other are lost) |
| Folds | one crease (2 px, 22 gray levels deep, 6 levels of shade) at 1/3, 0.4, 0.45, 0.5, 0.55, 2/3, 0.75 and 0.8 of the height, then of the width; also two horizontal, and letter fold plus vertical | 16 of 16 single creases found, none on the clean page, `fold_lines` right in the composite cases. These are clean synthetic creases through real-sized text; real ones are not this regular. |
| Photocopy generation | 0 to 5 simulated copies (blur, noise, clipping, toner loss) | estimate 1, 1, 1, 2, 2, 2: a weak upward trend only, at the 0.6 confidence cap. Not validated on real copies. |

### Folds and other measures on DIBCO

19 of the 136 pages get `fold_lines >= 1` (5 of the 54 tune pages, 14 of the 82 held-out). The five
tune flags were looked at: 2009/H03 and 2012/H06 sit on a visible tonal change that is plausibly a
crease; 2016/1's flag is a vertical line 10% from the left edge that looks like the card's printed
border on the thumbnail (false alarm); 2018/6's flag is a vertical line in the text area with
nothing visible there (false alarm, and see the gallery for the crease it misses); 2018/1's flag is
a horizontal line through the text with nothing obvious (probably false). The held-out flags were
not inspected, and there is no ground truth, so no false-alarm rate can be given. Over all pages the grade is A 33, B 47, C 38,
D 18, Howe is recommended for 64 and Sauvola for 72, and the generation estimate is 2 or 3 for 119
pages.

## The quality gate

On a page graded D **whose blur severity is at least 0.9** (`gate_min_blur_severity`; blur of 3 px and
more at 150 dpi) the detectors that read text lines (`script`, `handwriting`, `signature`, `coverage`,
`redaction`) are **not run**; each leaves `debug.<name>.gated = 1`, the page type is not given, and one
finding `kind: "degraded"` (attribute `grade`, confidence from the blur severity, evidence
`quality.gate`) says why. The [page plan](plan.md) then routes the whole page to a vision model
(`kind: "page"`, `route: "vision"`). `"no_gate": true` (CLI `--no-gate`) switches the gate off. The
finding appears only when a skipped detector was asked for.

Why: on a stress run of synthetic pages the layout invented **262 lines** for a page of 41 at blur 4 px, and
read "headline" script on it. A page whose text has merged into blobs has nothing for them to read.

`bench/quality_stress.py` (one 41-line synthetic Latin page, each damage alone; lines found with the gate off):

| Case | Grade | Blur severity | Lines found (truth 41) | Default run |
|---|---|---|---|---|
| clean, blur 2 px | A, B | 0.00, 0.32 | 41, 41 | script |
| **blur 3, 4, 6 px** | D | 1.00 | 64, **262**, 22 | **degraded** (gated) |
| noise sigma 20 | C | 0.00 | 41 | script |
| **noise sigma 40, 60** | D | 0.00 | 55, **3** | script: **not gated, wrong** |
| speckle 3%, bleed-through 0.4 | C, A | 0.00 | 41, 47 | script |
| photocopied x5 | A | 0.00 | 45 | script (grade A despite the damage) |
| photocopied x10 | B | 0.00 | 0 | nothing: text gone, nothing invented |
| skew 25 degrees | C | 0.00 | 0 | nothing: skew read wrongly |
| blank | D | 0.00 | 0 | nothing (gap measure alone makes it D) |

So the gate covers blur only. Heavy noise (grade D from speckle) is not gated: the layout reads noise as
text and `script` still answers. The bleed-through golden (`quality_bleed`, grade D, bleed severity 1.0) keeps
its 16 lines.

The first version gated on grade D alone and fired on the blank golden and the bleed-through golden, whose
text is fine; blur severity is what separates them. The threshold is hand set from this one stress test.

## Failure gallery

Each row is something that goes wrong, with the fixture, test or evaluation number that pins it.
A fixture or test failing after a change here means a behaviour changed: review it.

| Case | What happens | Pinned by |
|---|---|---|
| **Handwriting with a few loopy lines** (DIBCO crops of 2 to 4 lines) | The sweep has a broad plateau, not a peak; the angle is arbitrary (peak ratio 1.1 to 1.9). Errors of several degrees. | `bench/skew_eval.py`: 2018/2 and 2018/4 fail at 9 of 12 angles; 2011/HW7 at 12 of 12 (peak ratio 1.2); 2019 B papyri (ratio about 1.1) |
| **Exposed corners add a straight ink band** | A rotated scan with white (or black) corners makes a local threshold mark the paper beside the edge as ink, in a band along the new boundary. The first version of the estimator locked onto it and returned exactly the rotation angle on 2018/2. The 40% extent filter removes the long components of it, and the thick-blob filter (weak at 10 times the median stroke) the rest of the thickest ones; some of the band can survive. | White vs paper fill: over 1 deg in 120 vs 40 measurements (table above) |
| **Pixel lattice** | Projection bins finer than a pixel (scale 2) make the integer pixel grid look like structure: median error 2.7 degrees. Non-integer scales are also worse. Bins are one pixel, linear binning. | `bench/skew_eval.py --params '[quality]\nskew_bin_scale = 2.0'` |
| **Gradient criterion** | The sum of squared adjacent differences was far worse than the variance (tune sets: median 0.35 vs 0.15 deg at the time, 90th percentile 6.35 vs 1.55). | `--params '[quality]\nskew_criterion = "gradient"'` |
| **Strong show-through** | Once the ghost is dark enough to count as strong ink (about half the ink contrast) the ghost ratio falls: 0.95 at alpha 0.25 but 0.62 at 0.40. | `bench/quality_synth_eval.py --only bleed` |
| **Stains, grid paper, faded ink, halos called show-through** | Faint structure of any origin looks like a ghost stroke. Pages labelled clean score high: 2016/1 and 2016/4 (stains and shadow), 2019 A 4 (graph paper), 2018/7 (faded ink). 2018/10 had to be dropped from the clean list: its faded front ink is not distinguishable from show-through without the reverse side. | `bench/quality_eval.py --sets tune --table` |
| **Show-through fragments counted as specks** | A show-through that binarises as ink breaks into small components: the show-through fixture has a speckle density of 0.0018, the clean page it came from 0. | `tests/quality.rs::known_weakness_show_through_fragments_count_as_speckle`, fixture `quality_bleed` |
| **Noisy or faint pages read as very blurred** | When text edges are weaker than 2x the paper noise, the blur index gives up and reports the cap (10 px). 4 DIBCO pages hit it (2010/H01, 2011/PR6, 2012/H03, 2019 B 17), and a heavily noisy blurred synthetic page too. | `quality_synth_eval.py --only blur` (3 px blur gives the cap) |
| **Blur index reads low** | A thin stroke blurred by a Gaussian of radius s reads as about 0.6 s: the step-edge model does not hold for strokes narrower than the blur. The grade thresholds absorb this. | `quality_synth_eval.py --only blur` |
| **A dark crease is invisible** | A crease dark enough to binarise as ink is excluded from the paper grid, so the valley search never sees it; DIBCO 2018/6 has an obvious one (dark line, lighter paper below) and gets none. A bare step in the paper level is not counted: a step detector flagged 17 of 54 tune pages and was dropped. | `folds.rs::too_shallow_or_too_deep_valleys_are_ignored`; DIBCO 2018/6 |
| **A printed border or margin rule called a fold** | A long thin line with some shading asymmetry passes the fold test. Margin lines near the edge are excluded by position, but 2016/1's border line (10.2% from the edge) is not. | DIBCO 2016/1 |
| **A crease through dense text** | Text hides the paper, so the valley is measured only between words; below 25% measurable columns it is missed. The synthetic creases are all found, but they are regular and the text is sparse by comparison with a dense deed. | `folds.rs::a_crease_that_crosses_text_lines_is_still_found`, `quality_synth_eval.py --only folds` |
| **Strips and crops** | Nothing under 3 inches long can be a fold, so DIBCO crops never get fold lines, vertical or horizontal. | `fold_min_length_in` in `quality.toml` |
| **Speckle size** | Dots over i and j, full stops and hyphens are 2 to 4 pixels and count as specks at 150 dpi (about 8 pixels of text height). Measured / known is 0.8 on synthetic specks. | `quality_synth_eval.py --only speckle` |
| **Text-height proxy** | The area-weighted median component height overstates the x-height for cursive handwriting and for Devanagari headline-joined words, so the speck size grows with it. | `xheight_px` in the debug features |
| **Photocopy generation** | Clean synthetic pages read 1, five simulated copies read 2. The scale between 0 and 5 means nothing until real copies are measured. | `quality_synth_eval.py --only generation` |
| **Papyri and dark backgrounds** | DIBCO 2019 track B (dark papyrus on a white mount, up to 9 MP): skew fails (peak ratio 1.0 to 1.3) and 6 of 10 pages grade D. | `bench/skew_eval.py` and `bench/quality_eval.py --table` rows for `dibco2019_track_b` |
| **Blank and degenerate pages** | Finite values, no NaN: 1x1, 2x2, strips, white, black, mid-gray and noise pages. A blank white page reads skew 0, no generation estimate and Sauvola. | `tests/quality.rs::degenerate_pages_give_finite_sensible_values` |

## Fixtures and tests

`tests/golden/quality_inputs/` holds seven synthetic pages from `tests/golden/gen_inputs.py` (pure
integer arithmetic, so the bytes are the same everywhere): `quality_clean`; `quality_skew_p3`
(the same layout rotated counter-clockwise by atan(1/19) = 3.01 degrees); `quality_deva_skew_m2`
(words with a headline bar, clockwise by 1.98 degrees); `quality_blurred` (two 3x3 mean filters);
`quality_speckled` (100 specks); `quality_bleed` (a mirrored faint second layer);
`quality_fold` (a 460-pixel-wide page with one crease between two lines of text). They are kept
apart from `inputs/` so the binariser goldens, which run Howe on every page there, stay fast. The
byte-exact reports are in `tests/golden/expected/` and are checked on native, Python and WASM by
`scripts/determinism.sh`; `crates/pagetriage-core/tests/quality.rs` asserts what the numbers mean
(skew within 0.3 degrees of the known angle, blur and speckle ordering, show-through flagged and
Howe recommended, one crease counted once, finite output on degenerate pages).

## What was tuned on what

Look-ups against held-out data are the thing to be careful about, so here is the full list.

* **Examined in detail while choosing thresholds**: tune sets only. Images looked at: skew 2018/2
  (including the white-fill band), 2018/5, 2012/H09; show-through 2016/3, 2016/5, 2018/10, 2016/8
  (candidate maps); folds 2009/H02, 2009/H03, 2009/P01, 2012/H06, 2018/1, 2018/4, 2018/6, 2019 A 1.
  Looked at as numbers (fold positions) but not as images: 2009/P02, 2009/P04, 2016/1, 2019 A 4,
  2019 A 10. The labelled tune pages were also used as a group for the bleed-through AUC.
* **Skew parameters** (criterion, bin scale, linear binning, the blob filter, extent filter, refine
  candidates, point cap) were chosen on the tune sets with `bench/skew_eval.py --sets tune`. The
  held-out sets were first run through it at the end, once with each of the three rotations.
* **Bleed-through parameters** and the **fold parameters** were chosen on the tune pages (the
  fold thresholds also against synthetic creases). The labels were written before tuning from
  thumbnail contact sheets of all 136 pages, so the author *saw* every page at thumbnail size,
  held-out ones included; no held-out number was computed until the end. One label was changed
  after tuning started (2018/10 moved out of the clean list).
* **Grade, binariser and blur-SNR thresholds** were changed once after the first all-set run, when
  the grade came out harsh (about 30% D) in both splits. They were adjusted on the tune
  distribution (`blur_min_snr` 4 to 2, `howe_bleed_min` to 0.6 from the tune-set balanced accuracy,
  bleed severity range and weight, grade cutoffs 0.2 / 0.4 / 0.6), but the author had seen the
  held-out summary, so the held-out grade distribution is **not** a clean out-of-sample result.
  The held-out skew and bleed-through AUC numbers above are unaffected: they were computed before
  and are independent of those thresholds.
* **Evaluated only, never tuned on**: DIBCO 2010, 2011, 2013, 2014, 2017 and 2019 track B (82 pages),
  and every synthetic sweep is a check, not a tuning set (the blur SNR rule and the grade's blur
  range were set from the synthetic blur sweep, so that one is in-sample).

## What replaces it

Rulings (M9) will own thin printed lines and the fold-versus-rule distinction; the fold search here
should then be re-run against real folded deeds, which need a small annotated set (Phase 4). The
bleed-through score wants a verso-aware version (mirrored-stroke test) when a two-sided scan is
available, and a fold, bleed and grade calibration against Indian-deed pages instead of DIBCO.
Fusion (M13) can then use the severities as evidence rather than a fixed grade.
