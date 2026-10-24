# `structural.text_layer` (module M1)

For each page of a PDF, decides whether the page has real text, is a picture of a page, carries an
invisible OCR layer under a scan, or something in between. It reads the page's content stream; it
renders nothing. It needs the `pdf` cargo feature of `pagetriage-core` (off by default).

```sh
pagetriage structure FILE.pdf --pretty              # CLI
python -c "import pagetriage_py as p; print(p.analyse_pdf_structure(open('f.pdf','rb').read()))"
```

Output is the canonical JSON of a `StructureReport` (`schema/pagetriage-structure-0.1.schema.json`);
`pages[i]` is exactly what `analyse_page`'s `structural_json` argument accepts, and its
`text_layer` becomes the page report's `text_layer`. The same bytes come out of the native, Python
and WASM builds (`tests/golden/pdf*`, `scripts/determinism.sh`).

## States

| State | Meaning | Typical source |
|---|---|---|
| `native_text` | Visible text, no substantial image | Word, LaTeX, any export |
| `raster_only` | Images cover the page and there is no text | Scanner, phone app, `img2pdf` |
| `invisible_ocr_under_image` | A text layer that is not shown (render mode 3, or drawn under a later opaque image) over a page-covering image | OCRmyPDF, ABBYY, Acrobat "searchable image" |
| `mixed` | Visible text together with an image of at least 1% of the page, or part of the text hidden | Letterhead logo, photo, stamp image, scan with visible OCR text on top |
| `vector_only` | Painted paths, no text, no image. `features.outlined_text_suspected` marks many small filled outlines (text converted to curves) | Charts, ruled forms, outlined fonts |
| `empty` | Nothing visible | Blank page, a lone border or rule |

`mixed` is deliberately broad. Any raster content on a text page can hold a stamp, signature or
photo that the text layer does not describe, so the page should go to the pixel detectors; a text
page with a small logo is `mixed`, not `native_text`. This is a choice, not a fact about PDFs, and it
is the one threshold (`mixed_image_cover_fraction`) that was tuned on corpus labels (see below).

## Algorithm

1. **Load** with `lopdf` (no default features), with a cap on the size any stream may decompress to
   and a guard that turns a parser panic into an error. Encrypted files that the empty password does
   not open give a degraded report (pages listed, `text_layer: null`).
2. **Page tree** walked iteratively with a visited set and depth/page-count limits; `Resources`,
   `MediaBox`, `CropBox` and `Rotate` inherited. If the tree gives no pages, every `/Type /Page`
   object is used and the report is marked degraded.
3. **Interpret** each page's content streams (and the Form XObjects they draw, with depth, count and
   cycle guards) with a small tokenizer and graphics model. Tracked: the CTM, text state, text render
   mode, the position of each text-show operation, every image `Do` and inline image with its
   rectangle, and path painting. Nothing is drawn.
4. **Count** non-space glyphs (by font code width: one per byte for simple fonts, per code for
   composite ones), glyphs shown in render mode 3 or 7, visible glyphs that a *later* opaque image
   covers (an image with no soft mask, covering at least `occluding_image_cover_fraction` of the
   page), glyphs whose origin lies inside an image box, image coverage (union of image rectangles on
   a `coverage_grid` x `coverage_grid` occupancy grid, or the largest single image if larger), and
   painted / small-filled paths.
5. **Decide** with the rules below, then build the evidence list and `confidence = sigmoid(base_logit
   + sum(llr))`.

Rules (all thresholds in `params/text_layer.toml`):

* fewer than `min_glyphs` glyphs: no text layer. Image cover >= `raster_min_cover_fraction` is
  `raster_only`; at least `vector_min_paint_ops` painting operators (or many small filled outlines) is
  `vector_only`; otherwise `empty`.
* hidden glyphs >= `hidden_glyph_fraction` (0.90): `invisible_ocr_under_image` if images cover at
  least `raster_min_cover_fraction`; with nothing under it the page is `vector_only` or `empty`.
* visible glyphs >= 0.90: `mixed` if images cover at least `mixed_image_cover_fraction`, else
  `native_text`.
* anything between: `mixed`.

Evidence on the golden OCR page (`tests/golden/pdf/scanned_ocr.pdf`, page 2):

```
invisible_ocr_under_image  0.9997
  glyph_count            106      thr 3     llr +1.5
  hidden_glyph_fraction  1.0      thr 0.9   llr +2.5   106 of 106 glyphs are hidden: 106 in render mode 3/7, 0 under a later opaque image
  image_cover            1.0      thr 0.85  llr +2.0   images cover 100.0% of the page box (full at 85%, floor 30%)
  glyph_image_overlap    1.0      thr 0.9   llr +1.0   100% of glyph origins lie inside an image box
  producer_ocr           1.0      thr 1.0   llr +1.0   producer or creator names an OCR tool
```

The producer fingerprint is positive evidence only: absence costs nothing, because many OCR tools
leave no trace. The same fingerprint on a page that has *no* text layer is recorded with llr 0 and the
note "the document's producer is an OCR tool but this page has no text layer" (OCR skipped or failed
on that page).

`text_layer.reliable` is filled by module M2 for pages that carry text: `false` for legacy-encoded
or undecodable text, see [`font_script`](font_script.md).

## Decision: why not `pdf-inspector`

The project rule is to depend on `pdf-inspector` upstream instead of forking it, and the module plan
called for building M1 on it. After trying it, `pagetriage-core` does **not** depend on it. It is
not forked either: it is simply not used, and nothing was copied from it. What was measured
(2026-10-24, `pdf-inspector` 1.25.2, `lopdf` 0.45/0.46, Apple-silicon macOS, Rust 1.98):

| | `pdf-inspector` 1.25.2 | `lopdf` alone (what we use) |
|---|---|---|
| Clean release build, wall clock (CPU) | 19.7 s (119 s) | 10.2 s (36 s) |
| `wasm32-unknown-unknown` | Compiles (`lopdf` needs its `wasm_js` feature on wasm in both cases) | Compiles |
| WASM size, one call, `-Oz`, gzip -9 | **1,310,479 bytes** | **278,754 bytes** (`lopdf` 0.45); 283,486 (0.46) |
| Our whole pixel-only WASM, for scale | 183,143 bytes on `main` | |
| What it pulls in | `regex`, `unicode-bidi`, `unicode-normalization`, `ttf-parser`, `include_dir` (embedded CMap data), `log`; on native also `rayon`, `env_logger` | `lopdf`'s own crypto/compression crates |
| Threads | Enables `lopdf`'s `rayon` feature on native targets only (a target-specific dependency, not a feature), so native would parse in parallel and WASM would not | None |
| Licence | MIT | MIT |
| Internals | `HashMap`/`HashSet` and `f32` throughout (aggregates only reach the output, so this is probably harmless, but unverified) | `BTreeMap` objects, insertion-ordered dictionaries |

And the API does not fit the evidence format. `detect_pdf_type_mem_with_config(.., Full)` returns a
document type, an `f32` confidence and, per page, only whether the page *needs OCR* with a reason
(`scanned`, `no_text`, `vector_text`, `invisible_text_layer`, `suspected_garbled_text`). There are no
glyph counts, no render-mode fraction, no image-coverage number and no glyph/image overlap, so the
`value` and `threshold` fields of our `Evidence` could not be filled from it; and it has no page-level
`native_text` versus `mixed`. We would still have needed our own content-stream interpreter for
everything the module specifies, with `pdf-inspector` as a second opinion at 4.7 times the WASM size.

How it fares on the synthetic corpus, as a comparison (not a benchmark of `pdf-inspector`, which is
built for a different question: "does this need OCR?"). Coarse classes, `native_text` and `mixed`
folded into `text`; 300 pages; `bench/compare_pdf_inspector.py` with
`bench/tools/pdf_inspector_compare` (a standalone crate that is not a workspace member):

| | pages agreeing with the label |
|---|---|
| `pdf-inspector` (`ScanStrategy::Full`) | 263 / 300 = 87.7% |
| this module | 300 / 300 |

Its misses: 7 of 63 OCR-layer pages (the 7 where visible text sits under an opaque image: six called
`text`, one `raster_only`), and 30 vector or empty pages (29 reported as plain text, one vector page as empty) because it
only flags pages that need OCR. Where its question applies (scanned, OCR layer, text) it agrees on 259
of 266 pages.

Revisit this if upstream exposes per-page counts and a target-independent `lopdf` setup. Not
depending on it is the exception to project rule 5 that this page justifies; the lead should confirm
the exception.

## Parameters

`crates/pagetriage-core/params/text_layer.toml` (thresholds and weights),
`structural_limits.toml` (resource limits). Each value there names its source. Two were tuned on the
corpus dev split (odd-numbered documents), and say so:

| Parameter | Initial | Now | Why it moved |
|---|---|---|---|
| `mixed_image_cover_fraction` | 0.25 | 0.01 | the corpus labels a page with text and a logo of about 2% as `mixed` |
| `vector_min_paint_ops` | 8 | 3 | a labelled chart page paints 6 paths; one or two paths stay `empty` |

Before those two changes the dev split scored 136 / 141 = 96.5%; after, 141 / 141.

## Robustness

Input is untrusted. Limits in `structural_limits.toml`: input size, decompressed bytes per stream and
per document, objects, pages, page-tree depth, form nesting and count, operators and operands per
page, graphics-state depth, fonts and images per page. Hitting one gives `degraded: true` and a
warning, with counts that are lower bounds, never a hang. Checked by:

* tests that truncate small PDFs at every byte and flip bytes in them (`tests/structure.rs`);
* 60,000 mutated copies of the corpus PDFs (byte flips, truncation, deleted, duplicated and zeroed
  chunks) through the Python binding: 0 panics, 0 parser-panic errors, slowest call 25 ms; about
  21% are refused as unparseable (mostly "failed parsing cross reference table"), the rest analyse;
* a page tree whose kid is its own parent, a `/Contents` array that contains itself, a form that
  draws itself, 300 nested `/Pages` nodes with `/Count 4000000000`: all terminate;
* a page content stream larger than `max_decompressed_bytes` is refused and the page marked truncated;
* size: a 41 MB single page with 400,000 text operators takes 0.23 s; a 62 MB, 2,000-page file 1.4 s.

On WASM the build uses `panic = "abort"`, so the panic guard cannot catch a parser panic there; none
was seen in the fuzzing above, which ran natively.

## Failure gallery

`python -I bench/tools/structure_gallery.py` builds each case and prints the verdict (golden fixtures
are in `tests/golden/pdf/`). These are wrong or doubtful answers, kept visible.

| Case | What happens | Why |
|---|---|---|
| White text (`1 g`) drawn over a scan | `mixed`, 0.99 | Fill colour is not tracked, so the text counts as visible. A hidden layer made with white text, rather than render mode 3, is missed. |
| Text clipped away (`W n` with an empty rectangle) or drawn at x = -5000 | `native_text`, 0.996 | Clipping and the page box are not applied to text. |
| A scan painted with a tiling pattern or shading fill instead of `Do` | `empty`, 0.98 | Only `Do` and inline images count as images. |
| Hidden layer plus visible stamp text on a scan (hidden fraction 77%) | `mixed`, 0.92 | By the 90% rule. Arguably right; flagged because a page like this is a scan whose real content needs OCR. |
| Optional-content layers switched off | Not looked at | The text in a hidden layer counts as visible. |
| Text in a Type3 bitmap font (old scanners and OCR tools) | `native_text` | If the font has `/Differences` names it decodes as ordinary text; with none, the glyphs are counted but marked undecodable and `reliable` becomes `false`. A visible Type3 OCR layer is not recognised as a scan. |
| Scan inside margins, covering 66% or 81% of the page | still `invisible_ocr_under_image` (0.996 and 0.999) | The 85% threshold is evidence, not a gate (`raster_min_cover_fraction` is the gate, 30%); a scan covering less is rarely a pure scan, but the verdict does not change until 30%. |
| Text outlined into curves | `vector_only` with `features.outlined_text_suspected` | Right per the state list, but the consumer must treat it as "needs OCR". Needs at least 4 segments per small filled path and 150 such paths. |
| Page image only in an annotation appearance | Not seen | Annotations are not interpreted. |
| Damaged cross-reference table | Error, or `degraded` with a repair note | `lopdf` repairs some damage; the rest is refused. |

## Measured results

Corpus: `bench/synth/make_pdf_corpus.py` (60 documents, 300 pages, written by us; seed 0);
scored by `bench/structure_eval.py`. **A synthetic corpus written by the same team is weak evidence
of accuracy on real PDFs.** The numbers below mostly show that the interpreter implements the rules
as intended and that the generator and the analyser agree about what the states mean.

Which labels were seen while building: the corpus is split by document number. The *dev* split (odd
numbers, 30 documents, 141 pages) was scored repeatedly, its mismatches inspected, and its
sidecar (`labels_detail.json`: variants, image qualities) read; two parameters were tuned on it (above).
The *held-out* split (even numbers, 30 documents, 159 pages) was scored once, after the analyser was
finished; nothing in the analyser was changed after seeing it (the only follow-up was a scoring-script
fix: the corpus writes "no producer" as `""`, the report says `null`). The generator's aggregate
summary (page counts per label across all 300 pages) was visible from the start, as was the
generator's README. The generator source was not read. All 17 legacy-font pages happen to lie in the
held-out split, so M2's legacy detector had no corpus data during development and was checked on
`bench/tools/legacy_probe.py` instead (see `font_script.md`).

| Split | Pages | State accuracy | Notes |
|---|---|---|---|
| dev (in-sample for 2 parameters) | 141 | 141 / 141 = 100.0% (96.5% before tuning) | |
| held-out (one run) | 159 | 159 / 159 = 100.0% | |
| all | 300 | 300 / 300 = 100.0% | target >= 99% |

Per state over all 300 pages: `native_text` 136, `raster_only` 43, `invisible_ocr_under_image` 63,
`mixed` 24, `vector_only` 16, `empty` 18; every one recalled and predicted without error. The 7 pages
where visible `0 Tr` text lies under an opaque page image (generator variant `occluded`) are all
`invisible_ocr_under_image`: the text is covered by a later opaque image, so the page shows the scan
only. They behave the same as render-mode-3 layers here because that is how the module treats a
hidden layer. `pdf-inspector` gets six of the seven wrong.

Further checks that do not use the labels:

* the same 60 PDFs rewritten by `qpdf` with object streams and cross-reference streams, linearised,
  and fully uncompressed (`--qdf`): per-page results identical in all 180 files; cross-reference style
  and linearisation read correctly; `incremental_updates` 0 for the linearised files;
* the reports for all 60 corpus PDFs are byte-identical between the native CLI, the Python binding
  and the WASM build with the `pdf` feature (SHA-256 of each report compared), beyond the 7 golden
  fixtures the determinism gate checks;
* `pdftotext` and `qpdf --check` agree with the golden fixtures' contents (the generator, not the
  analyser, is what they check).

## WASM size and the `pdf` feature

`lopdf` is behind the off-by-default `pdf` feature of `pagetriage-core` (and of `pagetriage-wasm`), so
the pixel-only build does not carry a PDF parser. `cargo build -p pagetriage-core --features pdf
--target wasm32-unknown-unknown` compiles; on that target `lopdf` needs its `wasm_js` feature for
`getrandom`, which adds one JS import (`crypto.getRandomValues`, never called by the structural
analyser) that the host must provide (`globalThis.crypto`, present in browsers and Node 19+).
`scripts/build-wasm.sh` with `PAGETRIAGE_WASM_FEATURES=pdf WASM_OUT=dist/wasm-pdf`:

| Build | gzip -9 |
|---|---|
| `main` before this branch (pixel-only) | 183,143 bytes |
| pixel-only with this branch | 199,131 bytes (+15,988: parsing `structural_json` in `analyse_page`) |
| with the `pdf` feature | 581,025 bytes (+381,894 over pixel-only) |

Both are inside the 1,000,000-byte Phase 0 budget, and `scripts/determinism.sh` checks the second
against it.

## What is not covered

Real scanner or OCR output (OCRmyPDF, ABBYY, Acrobat, Tesseract, phone apps, multifunction printers),
JBIG2 and JPEG 2000 scans, ImageMask text over a background image, Type3 OCR fonts, files from
Indian court and registry portals, password-protected files with a known password, broken or
partial `ToUnicode` maps, optional content, tagged PDFs, PDF/A and PDF 2.0 files, very large real files,
and anything that was printed and re-scanned. The producer-name list was written from memory.
