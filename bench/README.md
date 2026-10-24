# bench

Benchmark harness. Python, but **no algorithm lives here**: it fetches datasets, decodes images,
calls the Rust core through `pagetriage_py`, and scores the output.

| File | Role |
|---|---|
| `datasets.toml` | The datasets: HTTPS URLs, sizes, SHA-256 pins, citation text |
| `fetch.py` | Download, verify and unpack into `bench/data/` (gitignored) |
| `run.py` | Run the analyser over a folder of images, write one JSON line per image |
| `metrics.py` | F-measure, PSNR, F1, IoU, box matching, ECE, Brier. pFM and DRD are not implemented yet (Phase 1) |
| `synth/make_pdf_corpus.py` | Synthetic PDF corpus with ground truth, for scoring the PDF structural layer (see below) |
| `structure_eval.py` | Scores the PDF structural layer (M1, M2, M12 structure) against the synthetic corpus: page-level text-layer accuracy and confusion matrix, script, legacy-font detection, incremental updates, producer, JPEG quality. `--split dev|heldout|all` by document number (see the protocol in `docs/detectors/text_layer.md`). Results in `docs/detectors/text_layer.md`, `font_script.md`, `provenance.md` |
| `compare_pdf_inspector.py` | Sets `firecrawl/pdf-inspector`'s per-page verdicts beside ours on the corpus (needs `bench/tools/pdf_inspector_compare`, a standalone crate: `cargo build --release --manifest-path bench/tools/pdf_inspector_compare/Cargo.toml`) |
| `tools/legacy_probe.py`, `tools/structure_gallery.py` | The legacy-font statistics on approximate Kruti Dev text, and the structural layer's known wrong answers, rebuilt on demand |
| `phash_eval.py` | Perceptual-hash evaluation: perturbed copies and distinct pairs of the 136 DIBCO pages, plus a synthetic typed-page arm. Writes `bench/results/phash_eval.json`; results in `docs/detectors/phash.md` |
| `tests/` | Offline unit tests (`python -I -m unittest discover -s bench/tests`) |

## Setup

```sh
python3 -m venv .venv && source .venv/bin/activate
pip install maturin -r bench/requirements.txt
maturin develop --release -m crates/pagetriage-py/Cargo.toml
```

## Datasets

```sh
python -I bench/fetch.py --list               # what there is, and sizes
python -I bench/fetch.py                       # everything (about 240 MB of downloads, 1.2 GB unpacked)
python -I bench/fetch.py --only dibco2014      # one set
python -I bench/fetch.py --verify              # re-check downloaded archives, no network
```

Run it with `python -I` so nothing in the data folder can be imported. The downloads are untrusted:
HTTPS only (redirects to HTTP are refused), size and SHA-256 checked before unpacking, no absolute
paths, `..` or symlinks, and a cap on unpacked size. `.rar` and `.7z` archives need `bsdtar`
(libarchive; ships with macOS, `apt install libarchive-tools`) or `7z`.

| Key | Source | Status |
|---|---|---|
| `dibco2009` ... `dibco2018`, `dibco2019_track_a`, `dibco2019_track_b` | DIBCO / H-DIBCO organisers (users.iit.demokritos.gr, utopia.duth.gr, vc.ee.duth.gr) | Downloaded and verified 2026-10-24. 136 images across 11 sets. There was no 2015 edition |
| `tobacco800` | IAPR TC11 | **Unverified.** The host `tc11.cvc.uab.es` timed out on 2026-10-24, so the live download, its contents and whether it needs a login are all unconfirmed. The URL comes from a Wayback copy of the page. `fetch.py` fails with a clear message if the host is down. Once it works, check the size and tar contents and pin the hash |

None of these hosts states a licence, so nothing here is redistributed from this repository. The
host asks Tobacco800 users to cite the references listed under it in `datasets.toml`.

Things that bite, all from the original archives:

- Folder names keep the organisers' typos (`DIBC02009_...` with a zero, `DIPCO2016_dataset`).
- 2011 and 2012 keep images and ground truth in the same folder (`*_GT.tiff`, `*_GT.tif`), so give
  `run.py` a `--glob` for the images.
- Image names repeat across editions (`H01`, `1.bmp`): always keep the edition in the path.
- The 2018 ground-truth archive also contains 20 `weights/` files, which are not images.
- Track B (papyri) carries an `info.txt` crediting the source collections; third-party image
  rights are not stated.

## Running the analyser over a set

`run.py` takes an explicit folder (`--images`) because the extracted layouts differ by year:

| Set | `--images` (under `bench/data/`) | `--glob` |
|---|---|---|
| `dibco2009` | `dibco2009/DIBC02009_Test_images-handwritten/DIBC02009_Test_images-handwritten` and `dibco2009/DIBCO2009_Test_images-printed/DIBCO2009_Test_images-printed` | `*` |
| `dibco2010` | `dibco2010/H_DIBCO2010_test_images` | `*` |
| `dibco2011` | `dibco2011/DIBCO11-handwritten`, `dibco2011/DIBCO11-machine_printed` | `*.png` |
| `dibco2012` | `dibco2012/H-DIBCO2012-dataset/H-DIBCO2012-dataset/H-DIBCO2012-dataset` | `*.png` |
| `dibco2013` | `dibco2013/DIBCO2013-dataset/OriginalImages` | `*` |
| `dibco2014` | `dibco2014/original_images` | `*` |
| `dibco2016` | `dibco2016/DIBCO2016_dataset-original/DIPCO2016_dataset` | `*` |
| `dibco2017` | `dibco2017/DIBCO2017_Dataset/Dataset` | `*` |
| `dibco2018` | `dibco2018/dibco2018_Dataset/dataset` | `*` |
| `dibco2019_track_a` | `dibco2019_track_a/dibco2019_dataset_trackA/Dataset` | `*` |
| `dibco2019_track_b` | `dibco2019_track_b/dibco2019_dataset_trackB/Dataset` | `*` (skips `info.txt`) |

```sh
python -I bench/run.py --images bench/data/dibco2014/original_images --debug --out bench/results/dibco2014.jsonl
```

Ground truth sits beside each set (for example `dibco2014/GT/H01_estGT.tiff`, `dibco2018/dibco2018-GT/gt`).
DIBCO ground truth stores ink as black, so use `metrics.ink_mask(...)` before scoring.

Nothing under `bench/data/` or `bench/results/` is committed. Never put real deed pages in either.

## Synthetic PDF corpus

`synth/make_pdf_corpus.py` writes about 60 PDFs totalling exactly 300 pages, plus ground-truth labels,
so the PDF structural layer can be scored. It needs only Pillow and numpy (qpdf and poppler are used by
the tests, not by the generator).

```sh
python -I bench/synth/make_pdf_corpus.py                                  # -> bench/data/synth_pdf (gitignored)
python -I bench/synth/make_pdf_corpus.py --pages 60 --seed 7 --out /tmp/c --force
python -I -m unittest discover -s bench/tests -p 'test_pdf_corpus.py'     # about 15 s
```

The same `--seed` and `--pages` give byte-identical files and labels (checked across separate
processes). That holds on one machine and library set: Pillow's JPEG and zlib's Flate output can change
with the library version. The generated PDFs are never committed.

Layout of `synth/`: `make_pdf_corpus.py` (plan, assembly, incremental updates, CLI), `pagekinds.py` (one
builder per page kind), `pdfparts.py` (fonts, text layout), `scanimg.py` (page images and encoders),
`synthtext.py` (synthetic text in each script), `pdfio.py` (object, xref and update writer).

### Output

`labels.json`, the schema agreed with the structural layer:

```json
{"files": {"doc_001.pdf": {"producer": "OCRmyPDF 15.4.4 / Tesseract OCR-PDF 5.3.4", "incremental_updates": 1,
  "pages": [{"text_layer": "invisible_ocr_under_image", "script": "latin", "legacy_font": false}]}}}
```

`text_layer` is one of `native_text`, `raster_only`, `invisible_ocr_under_image`, `mixed`, `vector_only`,
`empty`. `script` is one of `latin`, `devanagari`, `bengali`, `gurmukhi`, `gujarati`, `tamil`, `telugu`,
`kannada`, `malayalam`, `arabic`, `none`.

Label rules (decisions the schema leaves open):

- `producer` is the `/Producer` of the final `/Info` (after incremental updates); `""` if there is none
  or it is empty. Labels describe the final state of every page.
- `script` is the script of the page's *text layer*, or the script a legacy font stands for. Pages
  with no text layer (`raster_only`, `vector_only`, `empty`) are `none`, because a structural reader
  cannot know the script of pixels. The script of image content is in the sidecar.
- `invisible_ocr_under_image` is a full-page image plus a text layer not meant to be seen. In the default
  corpus 56 of 63 such pages use `3 Tr` (text after the image, or before it); the other 7 paint visible text
  (`0 Tr`) under an opaque image (sidecar variant `occluded`). Filter those out for a pure render-mode-3 test.
- `mixed` is visible native text with a partial-page image. A full-page scan with a visible text
  stamp is not generated, because its label would be a judgement call.
- `legacy_font` pages use Latin code points under a font named like a pre-Unicode Indian font (Kruti Dev
  010, DV-TTYogesh, Shree-Lipi, Chanakya, APS-Dev, Walkman-Chanakya) and read as gibberish. They are
  `devanagari`, and `native_text` or `invisible_ocr_under_image`.

`labels_detail.json` is a sidecar outside the agreed schema: per file the persona, `xref` style (table or
stream), original and final producer, and each revision (annotation or content replacement, with the pages
touched); per page the expected extracted text, variant, image encoding and size, skew, page size and
rotation. Only the generator and tests rely on it.

### What is in it (default seed 0, 300 pages)

| Aspect | Content |
|---|---|
| Page kinds | 136 native_text, 63 invisible_ocr_under_image, 43 raster_only, 24 mixed, 18 empty, 16 vector_only |
| Scripts on pages with a text layer | latin 119, devanagari 60 (17 of them legacy), bengali 13, telugu 8, tamil 7, gujarati 7, kannada 3, gurmukhi 3, arabic 2, malayalam 1. Small corpora (under about 40 pages) may miss some |
| Native text | Base-14 and non-embedded TrueType/Type1 fonts with WinAnsi; Type0 / Identity-H fonts with a `/ToUnicode` CMap and arbitrary glyph ids for the Indic scripts and Arabic (RTL stored in visual order); Indic conjuncts as one glyph that maps to three code points; one or two columns; `TJ` kerning; one or two content streams |
| Raster | A page-filling image (or 2-4 stacked strips): DCT (grey and RGB, quality 20-92), Flate (grey, RGB, with PNG-Up predictor), 1-bit Flate, CCITT G4; 60-200 dpi; skew, blur, shading, noise, speckle; blank and photo-like pages; some `/Rotate` |
| Invisible OCR | Tesseract-style GlyphLess Type0 font (one `Tj` per line, `Tz` fitted) and per-word text in named fonts; text before or after the image; one or two content streams |
| Mixed | Logo header, side image, watermark under text, inline figure |
| Vector | Ruled grids, charts, plan sketches with curves and clipping, text outlined into filled rectangles |
| Empty | No `/Contents`, empty stream, whitespace, `q Q`, `BT ET`, `[]`, graphics state only |
| Producers | 12 personas, 28 distinct `/Producer` values including empty (OCRmyPDF, ABBYY, Adobe Paper Capture, pdfTeX, Word, Chrome/Skia, iText/ReportLab, Ghostscript, scanner firmware strings, print drivers); some with XMP; some with non-ASCII text strings (PDFDocEncoding or UTF-16BE) |
| Structure | PDF 1.3-1.7; classic xref tables (52 files) and xref streams (8); nested page trees; inherited `/MediaBox`; mixed page sizes; `/CropBox`; non-zero MediaBox origin |
| Incremental updates | 30 files with 0, 12 with 1, 12 with 2, 6 with 3 appended revisions; each has a valid xref and `/Prev` chain and its own `%%EOF`. Revisions add annotations (Text, Link, Highlight, FreeText, Stamp), replace page content streams (same kind, or a kind change such as raster to OCR, native to empty), and sometimes replace `/Info` with an editor's producer string |

### What the tests check

`bench/tests/test_pdf_corpus.py` (offline; the qpdf and poppler tests skip when the tools are missing, and
one test proves it by running them with an empty `PATH`): same seed gives the same bytes in-process and across
two interpreters; page and file counts; label schema and coverage of every kind, script, legacy flag, update
count; `qpdf --check` clean (no warnings); `pdfinfo` clean with matching page counts; `%%EOF`, `startxref`
and `/Prev` structure; and `pdftotext` agreement: raster_only, vector_only and empty pages extract nothing,
native_text, mixed and legacy pages extract the expected string (word for word; two-column pages and
OCR pages may come back reordered, Arabic is compared character by character because bidi reordering moves
punctuation, and hidden OCR words may be merged by poppler). Both the 60-page test corpus and the default
300-page corpus pass.

Agreement with `pdftotext` shows the labels match what poppler extracts. It does not show that they match
what real documents look like.

### What the corpus does NOT cover

This corpus was written by the same team as the detector, to that team's idea of what scans and producers
look like. Treat accuracy on it as a regression and smoke signal, not as a measure of real-world accuracy.

| Not covered | Why it matters |
|---|---|
| Real scanner and OCR output | Real JPEG tables and colour profiles, JBIG2, JPX, image masks, `/SMask`, CMYK, Indexed and ICC colour spaces, scanner tiling, 300-600 dpi images (these are at most 200 dpi and 3.2 megapixels) |
| Real OCR engines' quirks | Embedded GlyphLess font programs, ABBYY and Adobe text clipped to its image, Adobe ClearScan (Type3 glyph substitution), hOCR renderer details, OCR text that disagrees with the picture. Producer strings are imitations written from memory and unverified |
| Encrypted PDFs | No RC4 or AES, no owner or user password, no permission flags |
| Form XObject nesting | Page text and images are drawn directly in the page content stream. Content reached through nested `/Do` forms (common in real producers) is absent, and annotation appearances are graphics only |
| Object streams and hybrid xref files | Only classic xref tables and xref streams without object streams |
| Font and text pathologies | No embedded font programs, Type3 fonts, missing or wrong `/ToUnicode`, `/ActualText`, ligature glyphs, text hidden by white colour, zero size or clipping (`7 Tr`), OCG layers |
| Real Indian-deed layouts | No stamp-paper margins, seals, handwriting, tables, mixed-script lines, Bates stamps, or multi-language pages. Indic scans use procedural pseudo-glyphs, not real shaped text. Indic text is syllable soup, not language |
| Real legacy-font text | The gibberish is Kruti-Dev-style Latin soup, not a verified mapping of real Hindi. Legacy fonts are not embedded, so font-program heuristics cannot be tested |
| Right-to-left OCR | Arabic appears only as native text, without digits |
| Damaged files | Every file is well-formed: no broken xref, truncation, wrong `/Length`, missing `endobj`, linearisation or repair paths |
| Page-tree and sharing oddities | No shared content streams or resources between pages, inherited `/Resources`, XFA or AcroForm, signatures, portfolios |
| Stamped scans | A full-page scan with visible text added on top (Bates numbering) is not generated, because its label is a judgement call |

The text-layer labels follow the generator's intent and are checked against poppler only, so a page whose
"correct" label is arguable (for example a scan with a very small figure) is built to avoid the argument.
