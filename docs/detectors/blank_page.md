# `blank_page` (module M10, Phase 0 skeleton)

Reports `kind: "blank_page"` when a page has almost no ink. In Phase 0 it exists to drive the whole
path (pixels, evidence, fusion, finding) with real float arithmetic, so the determinism gate has
something to compare. It is not the final detector.

## Algorithm

1. Histogram the gray page. The **paper level** is the most common gray value (ties go to the
   darker value, so the choice is fixed).
2. A pixel is **ink** when it is darker than `paper - ink_delta`.
3. `ink_coverage = ink pixels / all pixels`.
4. Evidence `ink_coverage`, with `llr = clip(llr_per_ln_unit * ln(blank_ink_fraction / max(coverage, coverage_floor)))`.
5. Confidence is `sigmoid(prior_logit + llr)`, rounded to 6 decimals. A finding is emitted when it is
   at least `emit_min_confidence`.

Every number is in `crates/pagetriage-core/params/blank_page.toml` and `fusion.toml`. With
`{"debug": true}` the report carries `ink_coverage`, `ink_pixels` and `paper_level` whether or not a
finding was emitted.

## Failure gallery

Known wrong answers, kept honest here and pinned in `tests/golden/` so a fix shows up as a reviewed
diff.

| Case | What happens | Golden input |
|---|---|---|
| **Faded scan**: strokes less than `ink_delta` (60) below the paper | Counted as no ink, so the page is reported blank at 0.9997 confidence. This is the main weakness: faded photocopies are exactly the target documents. | `faint_strokes.pgm` |
| Uniformly dark page (black scan, covered lens) | Paper level is taken as the dark mode, so nothing is "darker than paper": reported blank (checked by hand; `dark_fill.pgm` pins the same mechanism). | `dark_fill.pgm` |
| Dust, scanner specks, punch holes | No despeckle step, so a few large specks count as ink. Small isolated specks under 0.2% are fine. | `blank_speckle.pgm` (stays blank) |
| Ink exactly at the 0.2% threshold | Confidence sits near 0.5 and flips on a single pixel. Expected; the llr is smooth around the threshold. | `borderline.pgm` |
| Solid dark region covering most of the page (photo, black border, heavy stamp ink) | The dark region becomes the paper level, so the clean paper beside it is lighter than "paper", not darker, and nothing counts as ink: the whole page is reported blank at 0.9997 confidence. A mid-gray fill with dark marks inside it is handled correctly (checked by hand). | `dark_fill.pgm` |

### Seen on real data

Run over the DIBCO benchmark pages (`bench/run.py --debug`, 2026-10-24). The Phase 0 detector is a
placeholder, so these are recorded as observations, not as a score.

| Set | Result |
|---|---|
| H-DIBCO 2014, DIBCO 2013, DIBCO 2011 handwritten (34 pages of ink on paper) | None reported blank; ink coverage 0.61% to 33.35% (the lowest is three times the 0.2% threshold). Correct. |
| DIBCO 2019 track B (papyri, 10 pages, dark textured background) | Wrong in both directions. Pages 15 and 19 are reported blank (confidence 0.9997 and 0.9963) because the dark papyrus is the most common gray level. Seven more report 49% to 77% "ink" because the papyrus is darker than the white mount around it. This is the dark-region case above, on real scans. |

Papyri are not the target documents, but the same mechanism will hit any scan with a dark border or
a scanner-bed background, so Phase 1 must not rely on a global "most common gray level".

## What replaces it

Phase 1 adds the binariser choice from M3 (so faded strokes are found by a local-contrast method
rather than a global delta) and a despeckle step before the coverage is measured; Phase 2 adds the
connected-component statistics that separate specks from text.
