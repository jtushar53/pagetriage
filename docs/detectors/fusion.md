# Fusion (module M13): consistency rules and calibration

Fusion is a sum of log-odds: every detector turns its measurements into evidence entries (`llr`) and
`sigmoid(prior + sum of llr)` is the confidence, so any confidence can be explained exactly. Phase 0
built the sum and the per-evidence clip (`llr_clip`). This page covers what was added: cross-detector
rules run on every page, the two Indian-deed composites built on them, and the calibration machinery.
The page typing is [page_type](page_type.md) and the pass across pages is
[endorsement](endorsement.md). **Read this first**: every number is a hand-set placeholder; no real
document has been looked at.

## Consistency rules (`fusion::consistency`)

Run on each page after the detectors, and the same on every runtime. Each rule that moves a confidence
adds an evidence entry (`detector: "fusion.consistency"`) saying why.

1. **Overlap.** Of two overlapping findings of different kinds among stamp, seal, thumb impression and
   signature (the intersection covers at least 50% of the smaller box), the less confident loses 1.5
   log-odds. A thumb impression inside a stamp ring, or a signature under a seal, is more likely to be
   one thing seen twice.
2. **Blank page with content.** A `blank_page` finding loses 12 log-odds when the page also has a
   confident table, ruling, QR, handwriting, stamp, seal, signature, thumb or redaction finding. The
   skeleton blank detector saturates at 0.9997 and cannot see faint ink or a dark fill; this rule only
   fixes the case where another detector disagrees.
3. **e-stamp QR** (`estamp_qr`, module M11). A QR code at least 5% of the page wide whose centre is in
   the top 35% of the page is repeated as an `estamp_qr` finding, with the QR's attributes (including `decoded` and `text`),
   confidence `sigmoid(logit(QR) - 1.5 + depth term)`. The text is not matched against anything, so an
   e-stamp cannot be told from any other QR; the finding says only "a QR near the top of the page".
4. **Stamp-paper header** (`stamp_paper_header`, module M11). `sigmoid(-3 + 3 * (ratio - 1.6) + 2 if a
   QR sits in the top band + 1 if a table or ruling does)`, where `ratio` is the tallest line's glyph
   height in the top 35% of the page over the median glyph height. A bbox covers the heading lines and
   the QR; `validated: false` is in its attributes. **No stamp paper has been seen**: the heading
   size and QR position are the author's idea of Indian stamp paper.

## Calibration (`fusion::calibration`)

An optional monotone piecewise-linear map from raw to calibrated confidence per finding kind
(`params/calibration.toml`, `maps = { stamp = [[0.0, 0.0], ...] }`), validated (knots in [0, 1], raw
strictly increasing, calibrated not decreasing) and applied last. **No map ships**: calibrating needs
findings labelled on real documents, which this project does not have, so every confidence is "how much
hand-set evidence", not "how often right". The machinery is tested (interpolation, flat ends, kinds
without a map untouched, bad maps rejected).

## Evidence group caps (`fusion.group_cap`)

Each evidence term is clipped to `llr_clip` (8). Terms from one detector are also summed and the sum
clamped to `group_cap` (12) before the groups are added (`scorer::confidence_capped`, used by every
detector that scores with a sum), so several correlated features of one detector cannot add up to
more than 1.5 clipped terms on their own. Terms of different detectors (the groups are the evidence
entries' `detector` strings) add without a cap. With the shipped parameters no existing golden
finding reaches the cap, so the goldens did not change; it is a guard, not a measured improvement.
Where two detectors measure nearly the same thing (handwriting and signature on a flourish), rule 1
above is the only guard.

## Fitting calibration maps

`bench/fit_calibration.py labelled.csv` fits the maps by isotonic regression (pool-adjacent-violators)
from rows `kind,confidence,label` and prints the `[calibration]` override. Its `--selftest` draws
synthetic data from a known miscalibration (true rate = confidence squared), checks the fit is monotone
and close to the truth, and that the core accepts the output and rejects a non-monotone map. That is a
test of the tool; **no labelled real findings exist, so no map ships**.

## What is not built

- **Learned weights and a confidence reliability table.** No labelled data.
- **Decoding of any code** beyond what `qr` says (see its page).

## Evaluation

Unit tests for each rule (the weaker mark loses and the stronger one is untouched; a weak mark under a
strong one is dropped; marks of one kind or far apart are left alone; a blank page with a table is
not blank and on its own it is; the QR at the top, below the top and too small; a heading with a QR,
without one and of ordinary size) and the synthetic cover page in `bench/page_type_synth_eval.py`
(a large heading and a QR at the top). Over its 12 cover pages per condition (dev and test pooled),
`estamp_qr` was found on 12 of 12 in every condition and `stamp_paper_header` on 11 of 12 clean, 11 of
12 with noise 8 and 12 of 12 with blur 1 px. On the other seven page types (12 pages each) there were
no `estamp_qr` and **2 `stamp_paper_header` on the 12 form pages** (the cause was not looked into). That is a check that
the code does what it was written to, not that stamp paper looks like this.

## Failure gallery

| Case | What happens | Where |
|---|---|---|
| **Any QR near the top** | Becomes an `estamp_qr` at about 0.9 (decoded text carried over). A letterhead QR, a payment code, a document-ID code. | By construction |
| **Plain headings** | A large title on any page with a QR (a certificate) becomes a stamp-paper header; 2 of 12 synthetic forms got one with no QR at all. | Measured (synthetic) |
| **A real stamp paper** | Never seen; the heading may be small, in an ornamental band, or in Hindi, and the QR can be elsewhere. | Open |
| **Overlap rule on a signature across a stamp** | A real signature written over a stamp (very common) loses 1.5 log-odds as the weaker of the two. | By construction |
| **Blank rule** | Needs another detector to have found something; a faint page that none sees stays blank. | By construction |
| **Calibration** | None shipped (the tool exists, the data does not). | Open |
