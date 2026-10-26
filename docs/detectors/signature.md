# `signature` (module M7, `pixel.signature`)

Finds **wet signatures**: isolated, tangled, curvy pen marks bigger than text. It reports
`kind: "signature"` with a page-coordinate `bbox` and `attributes`: `width_in`, `height_in`, `loops`,
`tortuosity`, `fill`, `max_hole`. The detector is **opt-in** (`"detectors": ["signature"]`) and runs on
the [`layout`](layout.md) pass. No model.

**Read this first.** Checked on synthetic flourishes drawn by the author of the detector, from curves
that have exactly the properties it measures (loops, a long underswing, tangle), on clean pages. Real
signatures range from a legible name to a near-straight line to a lone initial, and a pen scribble or a
cross-out is a tangle too. It cannot tell a signature from handwriting that happens to be tangled and
tall (see the gallery); M13's cross-detector rules are meant to settle that.

## Algorithm

1. **Candidates.** The layout pass's *graphics* (components big in both directions) and its *glyphs*
   far taller than their own line (at least `min_height_xh` = 2.2 times the line's glyph height) and at
   least `min_width_xh` (3) x-heights wide. Boxes within `merge_in` (0.25 in) of each other are one
   candidate (a pen lift, a dot).
2. **Size and fill window.** Width 0.5 to 4 in, height 0.15 to 1.6 in, and 2% to 35% of the box inked.
3. **Measurements** on the ink in the box:
   - `tortuosity`: stroke path length (ink pixels over the stroke width from the distance transform)
     over the box diagonal. A tangle is long for its size;
   - `loops`: closed holes, from the Euler number (Gray's bit-quads, 8-connectivity) and the component
     count, per inch of width and capped;
   - `axis_share`: the share of edge strength along the axes (as in `handwriting`);
   - `height_xh`: the box height in x-heights;
   - `max_hole`: the biggest enclosed background region as a share of the box.
4. **Score** `sigmoid(prior + terms)`, each term `coef * (value - threshold)` clipped to `+-term_clip`:
   tortuosity and loops and height raise it, a high axis share and a big hole lower it.
5. **Veto.** A candidate whose biggest hole is at least `veto_hole` (0.25) of its box is a ring or a
   frame and is dropped whatever its score: a stamp's text is as tangled as a flourish, and its terms
   add up to more than the hole term takes away.

The confidence is a hand-set sigmoid and is **not calibrated**.

`debug.signature`: `candidates`, `signatures`.

## Parameters

`crates/pagetriage-core/params/signature.toml`, all placeholders. The thresholds were set from the
feature ranges of the synthetic dev pages (flourishes: tortuosity 3.2 to 10.5, 9 to 33 loops per inch,
axis share 0.44 to 0.55, 4.5 to 9.3 x-heights tall; ringed stamps with text: 4 to 9 loops per inch,
11 to 18 x-heights, holes 0.3 to 0.5). `veto_hole` and the loops threshold exist because of the
black stamp (below).

## Evaluation

`bench/signature_synth_eval.py`: A4 pages at 150 dpi, a Latin text page with 1 or 2 signatures (flourishes
of 0.9 to 2.6 in from `bench/synth/signgen.py`) below the text, 60% over a signature rule with a
caption. Found means a finding with IoU of at least 0.5 against the drawn box. 6 dev seeds (thresholds
set looking at them) and 6 test seeds, 8 signatures per split and condition.

| Condition | Found, dev | Found, test |
|---|---|---|
| clean | 8 / 8 | 8 / 8 |
| skew 2 degrees | 8 / 8 | 8 / 8 |
| sensor noise 8 | 8 / 8 | 8 / 8 |
| blur 1 px | 8 / 8 | **6 / 8** |

Spurious findings per page on the look-alike pages (6 seeds per split):

| Page | Spurious per page |
|---|---|
| plain text, checkbox list, ruled box frame, solid logo shapes, zigzag cross-out of a line | 0 on every condition and split |
| black circular double-ring stamp with a word | 0 (it was 1.0 on every page before the hole veto) |
| a page of handwriting (the style M5 is tuned on) | **0.33 on dev (clean), 0 on test**, 0.17 blurred |

The handwriting row is the honest one: tall, tangled cursive words are signatures to every feature
here. It happened on 2 of 6 dev pages and none of the test pages; with other handwriting it could be
all of them.

How it got there: the first version read every black ring stamp as a signature (1.0 per page: a ring
with text has tortuosity 6 to 11 and is taller than text). Raising the loops threshold and adding a
hole term were not enough, because the clipped tortuosity and height terms (4 each) outweighed them;
only the veto worked. The signature golden page then found no candidate at all: the layout's x-height
was 45 px, because it was area-weighted and a flourish and a ring hold most of the page's ink. That
was a layout bug (see its change log), not a detector one.

Cost: single runs in this container varied by more than the detector's cost (a layout pass took 209
to 354 ms on the same page), so no number is given; it measures a few hundred components at most.

## Determinism

Counts, the Euler number, the distance transform and the Sobel sums are integer; the tortuosity, terms
and sigmoid use IEEE operations and `libm`. Findings are sorted by position. Checked on the golden
page by the native, Python and WASM runners.

## Failure gallery

| Case | What happens | Where |
|---|---|---|
| **Tangled, tall handwriting** | Scored as a signature (2 of 6 synthetic handwriting dev pages). M5 and M7 overlap; M13 is meant to resolve it. | `bench/signature_synth_eval.py`, "handwriting" rows |
| **Blur** | Blur 1 px smooths the small loops, lowering the loops term: 6 of 8 on test. | "blur 1 px" row |
| **A straight underline in the flourish** | The layout cuts a thin straight run of 6 or more x-heights out of the mask as a *ruling*, so a signature's long underswing is removed from the signature and counted as a rule. The loops remain and are found, with a smaller box. Seen on the golden page (the 170 px underswing became ruling pieces). | `signature_page.pgm` |
| **Cross-outs and scribbles** | A dense scribble over text is tangled. A regular zigzag was not found (its holes and tortuosity are low); a messy one would be. Not tested. | Open |
| **Initials, a short name, a flat signature** | Under 0.5 in wide or low in loops and tortuosity: not found. | By construction |
| **Signature over text or a stamp** | The ink in the box includes whatever it overlaps. A signature across a printed line merges with it; one across a stamp is rejected by the hole veto or merged into the stamp. Not tested. | Open |
| **Colour** | Ink colour is not used (blue and black ink score alike). | By construction |
| **Pages with large non-text marks** | Photos, logos and stamps pass through `graphics` and are judged by the same measures; solid ones fail the fill window, rings the hole veto, anything else is a candidate. | Look-alike rows |
| **Real signatures** | Never seen. | Open |

## Golden fixture

`tests/golden/layout_inputs/signature_page.pgm`, drawn by `tests/golden/gen_inputs.py` with integer
code: three lines of text, a flourish of six loops with an underswing, and a black double-ringed stamp
with marks inside. `tests/layout_golden.rs` pins one signature on it (the flourish, not the ring) and
none on the other pages. Candidates with every measurement:
`cargo run --release -p pagetriage-core --example dump_signatures -- page.pgm` (`LAYOUT=1` also lists
the layout's graphics and tall glyphs).
