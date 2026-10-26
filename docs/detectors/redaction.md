# `redaction` (module M12, pixel part)

Finds **redaction boxes**: solid, straight-edged dark rectangles in or against running text, as when a
name or a number is blacked out. It reports `kind: "redaction_box"` with a page-coordinate `bbox` and
`attributes`: `sides_with_text`, `height_xh`, `width_xh`, `fill`. The detector is **opt-in**
(`"detectors": ["redaction"]`) and runs on the [`layout`](layout.md) pass plus the gray page.

Only the **pixel** part of M12's "redaction boxes" is here. The PDF side (a filled rectangle drawn over
text that is still in the content stream, which is the common failed redaction) is a structural check
that is **not built**.

**White-out cannot be found**: a white box is paper. **Read this first**: checked on synthetic
rectangles drawn on clean text pages by the author of the detector (black, gray, rotated 2 degrees,
blurred 1 px). Real redactions are marker strokes, bars drawn in an editor and printed, or ragged
scans.

## Algorithm

1. **Dark mask.** Pixels at least `dark_delta` (120) gray levels below the paper (the most common
   bright gray) in the layout's levelled frame, and its 8-connected components. The layout's own
   Sauvola ink mask is **not** used: inside a large solid black block the local variance is zero, so
   Sauvola's threshold is zero and the interior is called background. The first version used it and
   missed every block wider than the 75 px window (the full-width blocks, whatever the condition).
2. **Rectangles.** A component is a candidate when it is between `min_height_xh` (0.8) and
   `max_height_xh` (14) x-heights tall, at least `min_width_xh` (2.2) wide, at most `max_aspect` (40)
   long over short, at least `min_fill` (0.9) ink in its box, and at least `min_edge` (0.9) of each
   edge's outer two rows or columns inked (two, so a rotated or blurred edge a pixel deep passes).
   2.2 x-heights excludes a filled checkbox (about 1.8) and also a one- or two-letter word.
3. **Context.** Glyphs of the layout's lines within `neighbour_xh` (3) x-heights to the left or right
   (overlapping the box vertically by half a glyph) or above or below (overlapping it horizontally)
   count as running text on that side. `sides_with_text` is 0 to 4.
4. **Score** `sigmoid(prior + sides term + solidity term)`; emitted at the fusion threshold. A box with
   text on two sides (a block across a column) is just above 0.5; with three or four sides, a word
   blacked out in a line, around 0.8 to 0.95.

The confidence is a hand-set sigmoid and is **not calibrated**.

`debug.redaction`: `candidates`, `boxes`.

## Parameters

`crates/pagetriage-core/params/redaction.toml`, placeholders. One was set from the benchmark: the prior
(-2.0 to -1.5), because full-width blocks have text above and below only and scored 0.485.

## Evaluation

`bench/redaction_synth_eval.py`: A4 pages at 150 dpi, Latin text, 1 to 4 words blacked out (a rectangle a
little bigger than the word) and, half the time, a block of 2 to 4 lines. Words under 2.2 x-heights are
not blacked out in the test (the detector does not search for them). A box is found at IoU 0.5. 6 dev
and 6 test seeds, 18 and 20 truth boxes per condition and split.

| Condition | Found, dev | Found, test |
|---|---|---|
| clean | 18 / 18 | 20 / 20 |
| skew 2 degrees | 18 / 18 | 20 / 20 |
| sensor noise 8 | 18 / 18 | 20 / 20 |
| blur 1 px | 18 / 18 | 20 / 20 |
| gray box (60), not black | 18 / 18 | 20 / 20 |

Spurious findings per page on look-alikes: filled checkboxes, a solid logo, a thin black rule, a
reversed-out heading (white on black), a table header bar with white column titles, a plain page, a
12 px full-width separator bar: **0 on all**, on both splits.

How it got there: the first version found 16 of 18 and 18 of 20 clean and **1 of 74 under skew and blur**
(single-row edge test, and the skew rows compared boxes drawn before the page was rotated against boxes
found after: a benchmark bug, fixed by rotating the truth). Filled checkboxes were 5 spurious findings
per page until the minimum width was raised past them. The Sauvola problem above cost every
full-width block.

The perfect rows are the shape of the test, not a property of real redactions: clean rectangles at
exactly the angle the layout levels, on a clean page.

## Determinism

Integer throughout, apart from the sigmoid and the fractions, which are IEEE operations and `libm`.
Checked on the golden page by the native, Python and WASM runners.

## Failure gallery

| Case | What happens | Where |
|---|---|---|
| **White-out** | Not found. | By construction |
| **PDF redactions** | The structural check (a filled rectangle over live text) is not built. | Open |
| **Ragged or hand-drawn marker bars** | The edge test (0.9 of the two outer rows) fails on a bar with a torn edge. Not tested. | Open |
| **Boxes touching text** | A redaction pressed against letters merges with them into a component that is not a rectangle. The golden and unit-test boxes had their neighbours cleared for this reason; the synthetic pages have a gap of a few pixels. | Found building the unit tests |
| **One or two letters** | Under 2.2 x-heights wide: not found (the same limit that excludes filled checkboxes). | By construction |
| **A solid bar between paragraphs** | A 3 to 10 px rule is under 0.8 x-heights or over aspect 40; a 12 px bar is over aspect 40. A 30 px bar 300 px long, with text above and below, is a redaction by this detector's definition. | Look-alike rows; not tested at that size |
| **A black signature box, a filled table cell** | Solid rectangles with text beside them: reported. | By construction |
| **Dark photos and logos** | Not rectangles (edges or fill): rejected; a dark rectangular photo with text beside it would be reported. | Not tested |
| **Real redactions** | Never seen. | Open |

## Golden fixture

`tests/golden/layout_inputs/redaction_page.pgm`: eleven lines of text-like words with a word-sized
box and a two-line block blacked out (the text beside each is cleared a few pixels back).
`tests/layout_golden.rs` pins two boxes on it and none on the other pages. Candidates with every
measurement: `cargo run --release -p pagetriage-core --example dump_redaction -- page.pgm`.
