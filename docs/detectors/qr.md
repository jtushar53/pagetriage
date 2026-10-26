# `qr` (module M11, e-stamp QR)

Finds **QR codes** from their three finder patterns. It reports `kind: "qr_code"` with a page-coordinate
`bbox` and `attributes`: `modules` (symbol side, estimated), `version_estimate`, `module_px`,
`decoded: false`. The detector is **opt-in** (`"detectors": ["qr"]`) and reads the gray page only.

**The code is not decoded**, so an e-stamp QR cannot be told from any other QR, and nothing here checks
that a stamp is genuine. The other M11 items (stamp-paper header, registrar endorsement, embossed seal)
are separate and listed in [modules](../modules.md). **Read this first**: checked only on synthetic
symbols (`bench/synth/qrgen.py`: finder, separator and timing patterns drawn as in ISO/IEC 18004, random
data modules) pasted on synthetic text pages. No real e-stamp or scan has been looked at.

## Algorithm

1. **Mask.** The Sauvola mask of the page.
2. **Rows.** Each row's runs are scanned for five consecutive runs, dark first, in the ratio 1:1:3:1:1
   (module = total over 7, each run within `run_tolerance` of its ideal times the module). The module
   must be between `min_module_px` (2) and `max_module_px` (20).
3. **Cross-check.** The column through the run's centre must show the same ratio, with a module within
   `module_ratio` (1.4) of the row's.
4. **Merge.** Hits within three modules of each other are one finder pattern; a pattern needs at
   least three hits (rows).
5. **Triangles.** Three finder patterns with modules within `module_ratio`, two legs equal within
   `leg_tolerance` (0.2), the hypotenuse equal to a leg times the square root of two within
   `hypotenuse_tolerance` (0.15), and legs of at least `min_leg_modules` (12) form a code. The box is the
   four corners plus 3.5 modules.
6. **Score** `sigmoid(prior + leg + hypotenuse + module terms)`, at most `max_codes` (8) per page;
   overlapping boxes keep the most confident.

The ratio holds along any scan direction, so rotated codes are found as well. Confidence is a hand-set
sigmoid and is **not calibrated**.

`debug.qr`: `finder_patterns`, `codes`.

## Parameters

`crates/pagetriage-core/params/qr.toml`, placeholders. One was set from the benchmark:
`run_tolerance` 0.5 to 0.75 (see below).

## Evaluation

`bench/qr_synth_eval.py`: A4 at 150 dpi, one 25 to 57 module symbol of 3 to 5 px per module per page, 6
dev and 6 test seeds. A symbol is found at IoU 0.5.

| Condition | Found, dev | Found, test |
|---|---|---|
| clean | 6 / 6 | 6 / 6 |
| rotated 3 degrees | 6 / 6 | 6 / 6 |
| sensor noise 8 | 6 / 6 | 6 / 6 |
| blur 1 px | 6 / 6 | 6 / 6 |
| 2 px per module | 6 / 6 | 6 / 6 |

Spurious findings per page on look-alikes (a plain page, a grid of small boxes, a table, a 1-D barcode, a
random-module block without finders): **0 on all**, on both splits.

Beyond the table (all 12 seeds pooled, run once while writing this page): rotated 10, 20 and 45 degrees
12 / 12; noise 25 12 / 12; blur 1.5 px 10 / 12; **blur 2 px 7 / 12**.

How it got there: with `run_tolerance` 0.5 the **blur 1 px** row was 2 / 6 dev and 5 / 6 test, and
symbols of 3 to 4 px per module were found **0 of 12** (5 and 6 px: 12 of 12). The binarised dark runs
grow and the light runs shrink under blur (6:2:13:4:5 for 4:4:12:4:4), which a tolerance of two pixels
barely misses. 0.75 fixed it and the look-alike rows stayed at zero. The look-alikes are few and
simple, so this says little about false codes on real pages.

## Determinism

Integer run lengths; module sizes and the triangle test are `f64` sums (`math::pairwise_sum`) and
IEEE sqrt, with no libm calls. Checked on the golden page by the native, Python and WASM runners.

## Failure gallery

| Case | What happens | Where |
|---|---|---|
| **Heavy blur** | Blur 2 px: 7 / 12; at 1.5 px 10 / 12. The runs no longer fit 1:1:3:1:1. | Measured (synthetic) |
| **Below 2 px per module** | Not looked for (`min_module_px`). A QR on a 100 dpi scan is often there. | By construction |
| **Real e-stamp QRs** | Never seen. Printed on hatched stamp paper, smeared by a photocopy, with fused modules; none simulated. | Open |
| **Damaged or covered finder pattern** | Two finders are not a code; a stamp over one hides the symbol. | By construction |
| **Inverted or mirrored codes** | Light-on-dark codes are not looked for; mirrored ones would need the same test on the mirrored layout, not done. | Open |
| **Other 2-D codes** (Data Matrix, Aztec, PDF417) | Not found; they have no QR finder patterns. | By construction |
| **Decoding** | None. The finding says a QR is there, not what it says. | By construction |
| **Look-alike patterns** | A deliberately drawn nest of squares in the right ratio (a logo) three times in a right triangle would be reported. Not tested. | Open |
