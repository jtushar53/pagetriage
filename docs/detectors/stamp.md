# `stamp` (module M6, `pixel.stamp_seal`)

Finds **rubber stamps and seals** on a colour page: single-hue ink regions with a circular ring or a
rectangular border and something inside. It reports `kind: "stamp"` or `"seal"` with a page-coordinate
`bbox`, and `attributes`: `shape` (`circle` or `rectangle`), `colour` (the ink's hue name), `diameter_in`,
`ring_sectors`, `inner_ring`, `interior_share`, `pixels`. A circle with a second concentric ring is a
`seal`; every other framed region is a `stamp`.

The detector is **opt-in** (`"detectors": ["stamp"]`).

**It needs a colour page.** Without `rgb` it reports nothing and sets `debug.stamp.coloured` to 0.
A gray-only version was tried and removed: with text behind and inside the ring, a circle cannot be
told from a seal; the ring's gaps drop it under the coverage threshold; and a rectangular border cannot
be told from a bordered paragraph. It found 0 of 24 stamps; keeping code that never fires was worse
than saying so.

**Read this first.** Checked on synthetic stamps only (drawn by the author of the detector, on clean
pages) and on 3 hand-built fixtures. Real stamps smear, overlap text, fade and are pressed
unevenly; the 100% below is for ink at the opacity the generator draws. See the
[failure gallery](#failure-gallery).

## Algorithm

1. **Coloured ink.** A pixel is ink when `max(R,G,B) - min(R,G,B) >= chroma_min` (45). Tinted paper
   (10 to 30) and black text (under 15) are not.
2. **Regions.** The mask is widened by `merge_in` (0.07 in) and cut into connected regions, taken
   biggest first. One whose longer side is between `min_in` (0.5) and `max_in` (4.5) inches, and whose
   shorter side is at least 0.4 of `min_in`, starts a candidate. Any smaller region whose centre lies
   inside a candidate's box is added to it: the text inside a ring, a seal's inner ring. (Without that
   the ring and its text were two regions, the ring looked empty and the text looked unframed.)
3. **Filters.** At least `min_pixels` (150) ink pixels, and at most `max_fill` (0.45) of the box inked:
   a solid block (logo, highlighter bar) is not made of strokes.
4. **Hue.** A 36-bin hue histogram of the region's ink; `hue_concentration` is the share of ink in the
   best window of 7 bins (70 degrees); the colour name is the window's weighted centre.
5. **Frame.**
   - *Ring*: radii and angles about the box centre; the radius is the one holding 98% of the ink; the
     ring score is the share of 36 angle sectors with at least 3 ink pixels in the outer band (0.82 to
     1.0 of the radius); the inner ring is the same over 0.5 to 0.76. The box must be round, within
     `circle_aspect_tol` (0.3).
   - *Rectangle*: for tilts from -20 to 20 degrees in steps of 4, rotate the pixels, take the
     bounding box and score the weakest of its four sides (the share of its length with ink within a
     band of `border_band` (0.1) of the shorter side); take the best tilt.
   - A frame counts when its score is at least `frame_threshold` (0.7). A ring beats a rectangle.
6. **Interior.** A framed region is a bare frame (a coloured table border) unless at least
   `min_interior_share` (0.08) of its ink is inside the frame.
7. **Score.** `sigmoid(prior + hue_concentration term + mean_chroma term + frame_coverage term)`,
   each term `coef * (value - threshold)` clipped to `+-term_clip`. Emitted at the fusion threshold
   (0.5), framed regions only.

**The confidences are hand-set sigmoids and are not calibrated.**

`debug.stamp`: `coloured`, `candidates`, `stamps`.

## Parameters

`crates/pagetriage-core/params/stamp.toml`. All placeholders; one was set from the benchmark:
`min_interior_share`. The first guess, 0.25, missed a third of the synthetic stamps (their text is
small against a thick ring: 0.15 to 0.6 of the ink is inside); a bare frame has about 0, so 0.08
separates them.

## Evaluation

`bench/stamp_synth_eval.py`: A4 pages at 150 dpi, Latin text on cream paper with one stamp multiplied
on (blue, violet, red or green ink; a circle, a double-ringed seal or a rectangle; 1.0 to 2.4 in;
tilted up to 20 degrees; ink dropout; opacity 0.55 to 0.95), 6 seeds x 3 kinds per split. Found means a
stamp or seal finding with IoU of at least 0.5 against the drawn box; kind, shape and colour are
checked on the found ones. Dev seeds 1-6, test seeds 11-16.

| Condition | Found, dev | Found, test | Kind, shape and colour right |
|---|---|---|---|
| colour, opacity 0.55-0.95 | 18 / 18 | 18 / 18 | all found |
| faint, opacity 0.4 | 13 / 18 | 10 / 18 | all found |
| blur 1 px | 16 / 18 | 17 / 18 | all found |
| gray only | 0 / 12 | 0 / 12 | (unsupported: reports nothing) |

False stamps on pages that are not stamped, 12 pages each (6 dev and 6 test seeds): a text page with
no stamp, a blue heading block, blue ballpoint handwriting, a red bare table frame, a yellow
highlighter bar, a four-colour logo: **0 of 12 on each**.

The faint row is chroma: at opacity 0.4 the ink's chroma is about 44, under `chroma_min` (45), so the
pixels are not coloured ink at all. Lowering it makes cream paper and photos start to count; a paper
chroma baseline (median over the page plus a margin) would be the next thing to try. Not done.

How it got there, because each step was a failure of the one before: the first version found 5 of 18
(the ring and its text were separate regions: fixed by absorbing regions inside a candidate's box);
then 11 of 18 on test (the interior threshold, above); a four-colour block of solid rectangles was a
false rectangle stamp until `max_fill` was added.

Cost on a 1240 x 1754 page, release build, median of 5: quality alone 171 ms, with stamp 204 ms
(33 ms for M6).

## Determinism

The chroma test and hue are integer; the polar and rotated coordinates use `math::atan2`, `math::sin`
and `math::cos` (the `libm` crate) and IEEE arithmetic; sums go through `math::pairwise_sum` where
they are floats. Regions are sorted by ink, then by position. Checked on the three golden colour pages
by the native, Python and WASM runners (the Python runner converts to gray with the library's own
`rgb_to_gray`).

## Failure gallery

| Case | What happens | Where |
|---|---|---|
| **Gray-only scans** | Nothing is reported. Most deeds are scanned in gray or black and white; a stamp there is invisible to M6. This is the largest gap. | `bench/stamp_synth_eval.py`, "gray only" |
| **Faint ink** | Below a chroma of 45 the ink is not coloured: 13 of 18 and 10 of 18 at opacity 0.4. | "faint" row |
| **Black or dark-blue-black ink** | A black stamp has no chroma and is invisible. | Unit test `bare_frames_solid_blocks_and_black_ink_are_not_stamps` |
| **Unframed stamps** | A stamp that is only text ("RECEIVED" in red) has no ring or border and is not reported, whatever its colour. Coloured text cannot be told from a stamp here. | By construction |
| **Open or broken frames** | A ring with a quarter missing, or a rectangle open on one side, falls under `frame_threshold`. A heavily dropped-out stamp too. | By construction |
| **Two hues** | A stamp whose ring and text are different colours has a split hue histogram; the confidence drops (unit test `a_stamp_in_two_hues_is_not_one`). | Unit test |
| **A stamp over a coloured area** | A stamp pressed over a colour letterhead or a photo merges with it into a region that is too large or too filled. | Not tested |
| **A stamp touching another** | Two stamps in each other's merge distance are one region and are reported as one or not at all. | Not tested |
| **A stamp bigger than 4.5 in or smaller than 0.5 in** | Not found. | By construction |
| **Real stamps** | Never seen. | Open |

## Golden fixtures

`tests/golden/stamp_inputs/` (PPM, P6; drawn by `tests/golden/gen_inputs.py` with integer code):

| File | Drawn | Pinned by `tests/stamp_golden.rs` |
|---|---|---|
| `stamp_circle_over_text.ppm` | Lines of black text with a blue ring stamp and two lines of words over them | one blue circular stamp |
| `stamp_violet_seal_and_red_rectangle.ppm` | A violet double-ringed seal and a red rectangle stamp tilted 11 degrees | a violet seal and a red rectangular stamp |
| `stamp_confusers.ppm` | A bare red frame, a solid green block, a black ring with words, a block of two hues | nothing |

Expected reports are in `tests/golden/stamp_expected/`. Candidate regions with every measurement can
be dumped with `cargo run --release -p pagetriage-core --example dump_stamps -- page.ppm`.
