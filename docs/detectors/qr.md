# `qr` (module M11, e-stamp QR)

Finds **QR codes** from their three finder patterns and **reads** them. It reports `kind: "qr_code"` with
a page-coordinate `bbox` and `attributes`: `modules` (symbol side, estimated), `version_estimate`,
`module_px`, `decoded` and, when the code reads, `version`, `ec_level`, `corrected_codewords`,
`payload_bytes`, `text` (up to 256 bytes; `text_truncated`). The detector is **opt-in**
(`"detectors": ["qr"]`) and reads the gray page only.

Reading a code gives its text; it does not check that an e-stamp is genuine (that needs the issuer's
register), so a QR that says it is a stamp certificate is reported as that and no more. The other M11
items (stamp-paper header, registrar endorsement, embossed seal) are separate, see
[modules](../modules.md). **Read this first**: checked on synthetic pages only; the codes themselves
are real (made with the `qrcode` Python package) but no real e-stamp or scan has been looked at.

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
6. **Read** (`pixel/qr_decode.rs`, integer apart from the sampling positions). The corner finder is the
   top-left; of the other two, the top-right is the one that puts the bottom-left a quarter turn clockwise
   (so codes turned by any angle are read; a mirrored one is not). The size is the finder spacing over the
   module (corrected for the turn) plus 7, snapped to `17 + 4 * version`, with versions one and two either
   side tried when it fails. The module grid is sampled with the affine map the three finder centres fix
   (no perspective or alignment-pattern correction), each module as the centre pixel (modules under 3.5 px)
   or the 3 x 3 majority, the other as the fallback. The format information (both copies, BCH(15,5),
   within 3 bit errors) gives the error-correction level and mask; the data modules are read in the zigzag
   order, unmasked and de-interleaved; each Reed-Solomon block over GF(256) is corrected (Berlekamp-
   Massey, Chien, Forney) and **checked again**; numeric, alphanumeric and byte segments are read (ECI
   headers are skipped; kanji and structured append are not read). The block tables are ISO/IEC 18004's,
   written by `scripts/gen_qr_tables.py` from the `qrcode` package.
7. **Score** `sigmoid(prior + leg + hypotenuse + module terms + 4 if it decoded)`, at most `max_codes` (8)
   per page; overlapping boxes keep the most confident. A decode that passes every block's check is strong
   evidence that the triple is a code.

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
12 / 12; noise 25 12 / 12; blur 1.5 px 10 / 12; **blur 2 px 7 / 12** (finder patterns only, the
random-module symbols).

**Reading.** `tests/golden/qr_vectors.json` holds 22 real codes (versions 1 to 27, all four levels, all
eight masks, numeric, alphanumeric and byte segments, one of them UTF-8) made with the `qrcode` package:
**22 / 22 read** on a page through the whole detector, and rotated by 90, 180 and 270 degrees (versions 1,
4, 7, 14) all read. Twelve flipped modules are repaired (`corrected_codewords` above 0), a version 7
level H code with 250 modules flipped still reads and one with every module flipped does not (the
finder patterns are still found, `decoded: false`); a mirrored code is found, not read. In the benchmark
(`real qr` rows: version 9, 10 or 12, level M, Q or H, a 65-character URL):

| Condition | Found, dev / test | Text read, dev / test |
|---|---|---|
| clean | 6 / 6, 6 / 6 | 6 / 6, 6 / 6 |
| rotated 3 degrees | 6 / 6, 6 / 6 | 6 / 6, 6 / 6 |
| sensor noise 8 | 6 / 6, 6 / 6 | 6 / 6, 6 / 6 |
| blur 1 px | 6 / 6, 6 / 6 | **4 / 6, 4 / 6** |
| 2 px per module | 6 / 6, 6 / 6 | 6 / 6, 6 / 6 |

Pooled (12 seeds): rotated 10 and 20 degrees read 12 / 12; **rotated 45 degrees read 10 / 12** (the
first version, which took the module size from the row runs, read 0 / 12: a code turned by 45 degrees
looks 1.41 times too large along a row, so the version was wrong; the size is now corrected by the
angle, and versions up to two away are tried); noise 25 read 12 / 12; **blur 1.5 px read 7 / 12**;
**3 px modules with blur 1 px read 3 / 12**.

How it got there: with `run_tolerance` 0.5 the **blur 1 px** row was 2 / 6 dev and 5 / 6 test, and
symbols of 3 to 4 px per module were found **0 of 12** (5 and 6 px: 12 of 12). The binarised dark runs
grow and the light runs shrink under blur (6:2:13:4:5 for 4:4:12:4:4), which a tolerance of two pixels
barely misses. 0.75 fixed it and the look-alike rows stayed at zero. The look-alikes are few and
simple, so this says little about false codes on real pages.

## Determinism

Integer run lengths and decoding; module sizes, the triangle test and the sampling positions are `f64`
sums (`math::pairwise_sum`), IEEE `+ - * /` and sqrt, with `math::atan2/cos` for the turn. Checked on the
golden page (a real version 2 code that reads as `HTTPS://EXAMPLE.INVALID/STAMP/IN-DL123`) by the
native, Python and WASM runners.

## Failure gallery

| Case | What happens | Where |
|---|---|---|
| **Heavy blur** | Blur 2 px: 7 / 12; at 1.5 px 10 / 12. The runs no longer fit 1:1:3:1:1. | Measured (synthetic) |
| **Below 2 px per module** | Not looked for (`min_module_px`). A QR on a 100 dpi scan is often there. | By construction |
| **Real e-stamp QRs** | Never seen. Printed on hatched stamp paper, smeared by a photocopy, with fused modules; none simulated. | Open |
| **Damaged or covered finder pattern** | Two finders are not a code; a stamp over one hides the symbol. | By construction |
| **Inverted or mirrored codes** | Light-on-dark codes are not looked for; mirrored ones would need the same test on the mirrored layout, not done. | Open |
| **Other 2-D codes** (Data Matrix, Aztec, PDF417) | Not found or read; they have no QR finder patterns. | By construction |
| **Blurred or tiny modules** | Reading needs sharper modules than finding: blur 1 px read 4 / 6, blur 1.5 px 7 / 12, 3 px modules with blur 1 px 3 / 12 (the codes were still found). | Measured (synthetic) |
| **Perspective, curl, creases** | The grid is sampled with an affine map from the finder centres only; a photographed or curled code drifts across the grid. The alignment patterns are not used. | By construction |
| **Segments not read** | Kanji, structured append and FNC1 modes end the read (`decoded: false`). ECI is skipped and bytes are shown as UTF-8 with replacement. | By construction |
| **Genuine or not** | A decoded e-stamp text is a string; nothing is looked up. A forged stamp with a well-formed code reads like a real one. | By construction |
| **Reading is not the same as understanding** | `text` can hold personal data (certificate numbers, names, amounts). It is copied into the report (up to `max_text_bytes`); set `decode = false` in `params/qr.toml` to leave it out. | By construction |
| **Look-alike patterns** | A deliberately drawn nest of squares in the right ratio (a logo) three times in a right triangle would be reported. Not tested. | Open |
