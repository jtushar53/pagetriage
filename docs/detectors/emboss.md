# `emboss` (module M11, embossed seal)

Finds **blind-embossed seals**: a circular relief pressed into the paper with no ink, which a scan
shows only as faint shading. It reports `kind: "embossed_seal"` with a page-coordinate `bbox` (the
circle's square) and `attributes`: `radius_px`, `contrast`. The detector is **opt-in**
(`"detectors": ["emboss"]`) and reads the gray page only. It does not read what the seal says.

This is a **basic** detector. **Read this first**: it has been tried only on shading rendered by the
author of the detector (bench/synth/embossgen.py: a height field of rims, a ring and some arcs, lit
from one side); no real embossing has been scanned for it. Real blind embossings are subtler, are often
under text, are flattened by photocopying, and depend on the scanner's lighting.

## Algorithm

1. **Reduce** the page in 4 x 4 blocks and smooth it (a 3 x 3 box), so that sensor noise averages out
   and the relief, a few reduced pixels wide, stays. Take the Sobel gradient.
2. **Vote.** Each pixel with a gradient between `min_gradient` (0.6 gray levels per reduced pixel) and
   `max_gradient` (12; stronger is ink) votes, for every radius from 10 to 30 mm, for the two circle
   centres along its gradient direction. A centre's vote share at a radius is the votes within one
   reduced pixel of it over the circumference; peaks of at least `min_vote_share` (0.25) are looked at,
   strongest first, one per neighbourhood, at most `max_tries` (30).
3. **Antisymmetry.** A raised ring is symmetric, but its shading under one-sided light is not: turned
   through 180 degrees about the centre it changes sign. The page is high-passed (a 7 x 7 box mean
   removed) and, in the annulus between 0.5 and 1.1 radii, *minus* the correlation of the shading with
   itself turned through 180 degrees is measured (about 1 for a lit relief, negative for an inked or
   stained ring, which looks the same turned round). Pixels more than `max_shading` (12 gray levels)
   from their local mean are ink and left out. The centre is searched for within 3 reduced pixels of the
   vote peak and the radius over the whole range, taking the most antisymmetric; a circle is kept at
   `antisymmetry_min` (0.4) and an rms shading of at least `min_contrast` (0.8 gray level).
4. **Size.** The seal's radius is the outermost radius at which the mean gradient magnitude is at least
   half its largest value. A seal has several concentric edges; only the outermost circle is kept.
5. **Score** `sigmoid(prior + vote term + antisymmetry term)`, emitted at the fusion threshold.

The confidence is a hand-set sigmoid and is **not calibrated**. `debug.emboss`: `candidates`, `seals`.

## Parameters

`crates/pagetriage-core/params/emboss.toml`, placeholders. Set from the benchmark: the first-harmonic
test was replaced by the antisymmetry test (below), and `antisymmetry_min` went from 0.5 to 0.4 (the
golden page scored 0.49).

## Evaluation

`bench/emboss_synth_eval.py`: A4 at 150 dpi, a Latin text page with one embossed seal of 30 to 50 mm
(diameter 177 to 295 px) over or beside the text, lit from a random side. A is the amplitude of the
shading in gray levels around the paper (about 240). A seal is found at IoU 0.5; seeds 1 to 6 dev,
11 to 16 test.

| Condition | Found, dev | Found, test | Spurious per page, dev / test |
|---|---|---|---|
| A 12 | 4 / 6 | 5 / 6 | 0.00 / 0.17 |
| A 8 | 4 / 6 | 5 / 6 | 0.00 / 0.17 |
| A 5 | 3 / 6 | 3 / 6 | 0.00 / 0.17 |
| A 8, sensor noise 4 | 2 / 6 | **0 / 6** | 0.00 / 0.17 |
| A 8, blur 1 px | 4 / 6 | 5 / 6 | 0.00 / 0.17 |

Look-alikes, spurious findings per page (dev / test): plain page 0 / 0, inked ring stamp 0 / 0, solid
disc 0 / 0, faint dark-ring stain **0.17** / 0, a round hill of shading 0 / 0, a linear lighting
gradient 0 / 0.

How it got there: the first version judged a circle by the first harmonic of the radial gradient
around it (lit half positive, shaded half negative) and found 1 to 4 of 6 with **0.3 to 0.7 spurious
per page and about one false seal per page on the stain look-alike**. A ring sampled a few pixels
off-centre looks like a dipole too, so the stain and the seals' own inner rings fooled it. The
band-polarity variant had the same flaw. Antisymmetry of the shading about the centre does not, because
a stain looks the same turned round. Then the vote peak was found to lean towards the lit and shaded
edges (centre off by 10 px: antisymmetry 0), so the centre is searched for; the radius it picked was the
inner ring or far too large, so the size is taken from the edge profile. Seals overlapping text
(dev seeds 4 and 6 at A 12) are still missed, because the pixels of text are left out of the
correlation and too few pairs remain; the one test miss at A 12 was not looked into.

## Determinism

Integer block sums and Sobel; the vote, correlation and edge profile are `f64` with `math::sin/cos`
and `math::pairwise_sum`, `sqrt` only otherwise. Checked on the golden page by the native, Python and
WASM runners.

## Failure gallery

| Case | What happens | Where |
|---|---|---|
| **Real embossings** | Never seen. | Open |
| **Sensor noise** | 4 gray levels of noise on a shading of 8: 2 / 6 dev and 0 / 6 test. The antisymmetry falls as noise energy is added; the first threshold, 0.5, found none. | Measured (synthetic) |
| **Seal over text** | Ink pixels are left out of the correlation; with a lot of text the seal is lost (the two dev seeds that overlap text at A 12). | Measured (synthetic) |
| **Faint dark-ring stain** | 1 of 6 dev pages gave a false seal. | Measured (synthetic) |
| **Faint relief (A 5)** | 3 / 6. | Measured (synthetic) |
| **Not round** | Oval, rectangular and multi-lobed embossings are not looked for. | By construction |
| **Pages with a large print or crease** | A crease is a straight antisymmetric shading, not a circle; a coffee ring that is lit unevenly could score. Not tested. | Open |
| **Cost** | About 0.06 s for the detector on one A4 page at 150 dpi, release build, one run, including reading the file. | Measured once |
