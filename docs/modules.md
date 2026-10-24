# Modules

The analyser is split into modules. Code that implements one cites its number (for example
"module M10") in comments and in the `citation` field of evidence. Status is as of the end of Phase 1 work (2026-10-24).

| Module | Name | What it decides | Phase | Status |
|---|---|---|---|---|
| M1 | `structural.text_layer` | Native text, raster only, invisible OCR layer under a scan, mixed, vector only, or empty | 1 | implemented: [`text_layer`](detectors/text_layer.md) (cargo feature `pdf`) |
| M2 | `structural.font_script` | Script from PDF font internals, including legacy non-Unicode Devanagari fonts | 1 | implemented: [`font_script`](detectors/font_script.md) (cargo feature `pdf`) |
| M3 | `pixel.quality` | Skew, blur, contrast, speckle, bleed-through, fold lines, photocopy generation; picks the binariser | 1 | implemented: [`quality`](detectors/quality.md); binarisers (Otsu, Sauvola, Wolf, Howe): [`binarize`](detectors/binarize.md) |
| M4 | `pixel.script` | Script identification from pixels, without ML (headline, connected-component and texture tiers) | 2 | planned |
| M5 | `pixel.handwriting` | Handwritten vs machine-printed text, including filled-in blanks on printed forms | 2 | planned |
| M6 | `pixel.stamp_seal` | Rubber stamps and seals, by colour clustering and by shape | 3 | planned |
| M7 | `pixel.signature` | Wet signatures | 3 | planned |
| M8 | `pixel.thumb_impression` | Thumb and finger impressions, via ridge coherence and frequency | 3 | planned |
| M9 | `pixel.rulings` | Rulings, tables, checkbox grids, and fold lines vs rules | 3 | planned |
| M10 | `pixel.content_type` | Page and region typing (text, form, table, photo, blank, ...) | 0 (blank skeleton), later phases | skeleton: [`blank_page`](detectors/blank_page.md) |
| M11 | `india.*` | Indian-deed detectors: stamp-paper header, e-stamp QR, registrar endorsement, embossed seal | 4 | planned |
| M12 | `provenance.*` | JPEG tables, PDF producer, incremental updates, redaction boxes, near-duplicate pages | 1 and later | near-duplicate pages (perceptual hashes): [`phash`](detectors/phash.md); PDF structure, incremental updates and JPEG tables: [`provenance`](detectors/provenance.md) (cargo feature `pdf`); redaction boxes planned |
| M13 | `fusion` | Log-odds evidence sum, calibration, abstention and cross-detector consistency rules | 0 (sum only), 5 | skeleton |

## Phases

| Phase | Scope |
|---|---|
| 0 Skeleton | Workspace, schema and JSON Schema export, params, CLI, Python and WASM builds, determinism gate, bench harness |
| 1 Text layer and quality | M1, M2, M3, Howe binarisation, perceptual hash, incremental-update detection |
| 2 Script and handwriting | M4, M5, layout segmentation |
| 3 Stamps, signatures, thumbs | M6, M7, M8, M9 |
| 4 Indian deeds | M11, registrar-endorsement repetition across pages; needs a small in-house annotated set |
| 5 Fusion, calibration, release | M13 calibration, npm and PyPI packages, docs site, offline browser demo |
