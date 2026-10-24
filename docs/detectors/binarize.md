# Binarisers (module M3 support): Otsu, Sauvola, Wolf-Jolion, Howe

Turn the gray page into a 0/1 ink mask (`1` = ink). Module M3 chooses among them from the page's
degradation; the other detectors then work on the mask. All four are deterministic: integer
arithmetic where the method allows it, otherwise IEEE `+ - * / sqrt` and the pure-Rust `libm` crate.
They are checked byte for byte across native, Python and WASM (see "Determinism").

```python
import pagetriage_py as pt
ink = pt.binarize(gray, width, height, "howe", '[howe]\nmode = "static"')   # bytes, 0/1
ink, c, t_hi = pt.binarize_howe(gray, width, height)                          # and what it chose
```

| Method | What | Source | Cost |
|---|---|---|---|
| `otsu` | One global threshold maximising between-class variance | Otsu 1979 | under 0.01 s/MP |
| `sauvola` | Local threshold `m (1 + k (s/R - 1))`, 75 px window, `k = 0.2` | Sauvola & Pietikäinen 2000 | under 0.01 s/MP |
| `wolf` | Local threshold mixing local mean, page minimum and relative local contrast | Wolf & Jolion 2004 (formula recalled, unverified) | under 0.01 s/MP |
| `howe` | Graph cut over Laplacian label costs and Canny-aware neighbour penalties, with per-image tuning of `c` and the high Canny threshold | Howe 2013 | 0.5 s/MP static, 8.5 s/MP tuned c, 23 s/MP dual |

Costs are one thread, native release build, Apple silicon. Howe's tuning solves 29 graph cuts per
tuned value, so it is a deliberate choice for pages that need it (bleed-through, low contrast, stains),
not something run on every page. `analyse_page` does not binarise with Howe.

## Howe: what is implemented

From the paper's text alone (the author's manuscript, CC-BY); his MATLAB code was not read, because
its licence was not checked. Source files: `pixel/binarize/{howe,maxflow,canny,filters}.rs`,
thresholds in `params/howe.toml` with the section or table each comes from.

1. **Label costs.** `L0 = lap(I)` (cost of paper), `L1 = -lap(I)` (cost of ink), from the exact
   integer 5-point Laplacian. Pixels more than 2 local standard deviations brighter than their
   Gaussian-weighted neighbourhood (sigma 20) are forced to paper.
2. **Neighbour penalty.** `c` for every neighbouring pair, except 0 between a Canny edge pixel and
   its brighter neighbour, which puts edge pixels on the ink side.
3. **Graph cut.** Boykov-Kolmogorov max-flow on the 4-connected pixel grid with integer capacities,
   written from the published algorithm and specialised to grids. It is checked against exhaustive
   search on 300 random small grids and for cut cost equal to flow on larger ones.
4. **Tuning.** Algorithm 1 (pick `c` at the stable point of the instability curve over 29 values,
   40 to 5120) and Algorithm 3 (choose the high Canny threshold from {0.25, 0.5} by comparing with
   the result at their midpoint). Mode `dual` runs both; `tune_c` and `static` are available.

### Where the paper's text had to be interpreted

Each of these is also noted in the source next to the code.

| Point | What the paper prints | What is done | Why |
|---|---|---|---|
| Bright-outlier cost (Eq. 4) | `L1 = phi` with `phi = -500` | The ink cost becomes `-phi` (large and positive) | A large negative ink cost would favour ink; the text says the purpose is to force background |
| Choosing `c` (Algorithm 1) | `arg min D'_q - 2 D'_r + D'_s` over `q < r < s` | `arg max` | The text says `c` is chosen at the minimum between the two peaks, which is the maximum of that second difference |
| Which Laplacian | Not stated | Plain integer 5-point sum (`cost_scale = 1.0`) | See below |
| Canny | MATLAB-style, thresholds relative to the largest gradient (assumption) | Derivative-of-Gaussian gradient, 4-sector non-maximum suppression, hysteresis | Interpolated suppression is not implemented |
| "Gaussian of radius r" | `r = 20` | Gaussian with sigma = 20 | The paper calls sigma the "radius" elsewhere (Canny, "radius 1" smoothing taps) |

**Laplacian convention.** Chosen on DIBCO 2009 and H-DIBCO 2010 only. Reading the paper's Laplacian as
MATLAB's `del2` (one quarter of the 5-point sum) makes the tuner collapse to an almost uniform page on
5 of the 20 training images (F between 4 and 44), because the scan range then reaches the regime where
the output stops changing and the instability curve has a second deep valley. With the plain sum, tuning
is sane on all 20.

## Results

Scored with the DIBCO F-measure (`bench/metrics.py`), pages and ground truth from `bench/fetch.py`.
Reproduce with `python -I bench/eval_binarize.py --method howe --howe-mode dual --sets dibco2011`.
Published numbers are Tables 1 and 3 of Howe 2013, copied by hand and checked against the paper's own
printed averages (`bench/tests/test_eval_binarize.py`).

**Training sets**, where the conventions above were chosen: the 10 DIBCO 2009 test images and the 10
H-DIBCO 2010 images (the paper's average covers 24; the four DIBCO 2009 training images are not in the
downloadable test sets, so "published" is the paper's per-image figures averaged over the same 20).

| Method | Ours | Published, same images |
|---|---|---|
| Otsu | 82.02 | |
| Sauvola | 81.28 | |
| Wolf | 82.89 | |
| Howe, static | 93.58 | 92.47 |
| Howe, tune `c` | 93.74 | 92.97 |
| Howe, dual | 94.01 | 93.84 |

**Held-out: DIBCO 2011, 16 images** (the set Howe himself used as a hands-off test):

| Method | Ours | Published |
|---|---|---|
| Otsu | 82.10 | |
| Sauvola | 81.64 | |
| Wolf | 84.67 | |
| Howe, static | 86.32 | 89.19 |
| Howe, tune `c` | 89.23 | 91.69 |
| **Howe, dual** | **89.59** | **91.90** (Table 4 prints 91.7 for the same algorithm) |

### Acceptance: missed

The Phase 1 target was "within 1.5 F-measure of the published DIBCO 2011 value (91.7)". Ours is 89.59,
**2.1 points short** (2.3 against Table 3's 91.9). The target is not met.

The shortfall is concentrated. On the 13 images other than the three below we score 92.91 against 92.47
published; the whole gap comes from three pages with large dark regions, where we label far too much
as ink (high recall, low precision). The three were identified after the fact, from the per-image
results, so this is a description of where the gap sits, not an independent finding.

| Page | Ours (dual) | Published (dual) | What is wrong (dual output viewed against ground truth) |
|---|---|---|---|
| P2011-6 | 70.2 | 86.3 | Dark, ribbed paper background: its texture comes out as blotches of ink across the page (precision 0.57, ink share 8.1% against 5.0%) |
| H2011-4 | 78.0 | 92.0 | A water stain touching the top border becomes a solid ink block; the handwriting is otherwise right (precision 0.68) |
| H2011-1 | 77.3 | 90.0 | A dark shadow band down the right edge (page gutter) becomes ink; the handwriting is right (precision 0.64) |

Per image, dual, ours / published: H2011-1 77.3 / 90.0, H2011-2 98.3 / 97.4, H2011-3 95.0 / 93.5,
H2011-4 78.0 / 92.0, H2011-5 97.0 / 96.2, H2011-6 93.0 / 92.1, H2011-7 94.1 / 89.8, H2011-8 96.6 / 95.4,
P2011-1 94.3 / 94.7, P2011-2 72.9 / 72.3, P2011-3 96.8 / 96.4, P2011-4 95.2 / 95.1, P2011-5 94.7 / 94.6,
P2011-6 70.2 / 86.3, P2011-7 91.1 / 93.1, P2011-8 88.9 / 91.5.

**Diagnosis, so far.** On H2011-4 the soft stain boundary is picked up as an edge through the low
hysteresis threshold (`t_lo = 0.1`) and linked to the strong text edges, which makes cutting along it
free. Raising `t_lo` to 0.2 lifted that page from 77.0 to 91.9 (static mode, measured with the earlier
gradient), but it costs about 1.7 F on the training sets (tune `c`: 93.74 to 92.06), so it was not
adopted. The other two pages were not diagnosed to a mechanism. Our Canny differs from the one
the paper presumably used (quantised instead of interpolated suppression, an assumed threshold scale),
and that is the most likely source of the difference; it has not been confirmed.

### How the held-out set was used

Stated plainly, since it limits what the held-out number means. The 2011 images were scored once with
the conventions chosen on the training sets. After the shortfall appeared, the worst pages were
inspected and one change was made, on the grounds of fidelity rather than score: the Canny gradient
became a true derivative of a Gaussian instead of blur-then-central-difference (a documented
approximation). It was first checked on the training sets, where it helped slightly (static 93.49 to
93.58, tune `c` 93.60 to 93.74, dual 93.61 to 94.01), and then the 2011 numbers above were produced. The
first 2011 run, before that change, gave dual 89.42. `t_lo` was not changed, although the 2011 pages would
have preferred it.

## Failure gallery

| Case | What happens | Where |
|---|---|---|
| Large dark stain or background region | Labelled ink, because a soft boundary linked to strong edges is cheap to cut | H2011-4, H2011-1, P2011-6 above; `t_lo` scan in `params/howe.toml` |
| Strokes come out about a pixel fatter at edges | Edge pixels are put on the ink side by design; recall is near-perfect, precision pays | `howe::tests::static_mode_recovers_strokes_on_a_clean_page` |
| Wrong Laplacian scale | Tuning picks a `c` in the regime where nothing changes and returns an almost uniform page | Training-set run with `cost_scale = 4.0`, recorded in `params/howe.toml` |
| Perfectly flat page | Relative Canny thresholds would turn rounding residue into edges everywhere; guarded (`FLAT_GRADIENT`) | `canny::tests::flat_image_has_no_edges` |
| Speed | 23 s/MP for the dual mode; a 150-dpi A4 page is about 2 MP | Not run in `analyse_page` |

## Determinism

`tests/golden/binarize_cases.json` lists six variants (the four methods, plus Howe in static, tuned-c and
dual modes); every golden page is binarised with each and the SHA-256 of the mask must match
`tests/golden/binarize_expected.json` on native, Python and WASM (`scripts/determinism.sh`). The
`ghosted` page (lighting ramp, soft strokes, a bleed-through ghost) separates all six variants, so the
check is not vacuous. A one-hex-digit corruption of the expected file fails all three runners. Adding
Howe took the WASM build to 173 KB gzipped; with the other Phase 1 additions merged it is 183 KB
(budget 1 MB).

## Benchmark line

DIBCO 2011 (16 images), F-measure, mean: Otsu 82.10, Sauvola 81.64, Wolf 84.67, Howe dual 89.59
(published 91.9). Training sets (20 images): Howe dual 94.01 (published 93.84).
