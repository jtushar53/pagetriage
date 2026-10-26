# Modules

The analyser is split into modules. Code that implements one cites its number (for example
"module M10") in comments and in the `citation` field of evidence. Status is as of the start of Phase 2 work (2026-10-25).

| Module | Name | What it decides | Phase | Status |
|---|---|---|---|---|
| M1 | `structural.text_layer` | Native text, raster only, invisible OCR layer under a scan, mixed, vector only, or empty | 1 | implemented: [`text_layer`](detectors/text_layer.md) (cargo feature `pdf`) |
| M2 | `structural.font_script` | Script from PDF font internals, including legacy non-Unicode Devanagari fonts | 1 | implemented: [`font_script`](detectors/font_script.md) (cargo feature `pdf`) |
| M3 | `pixel.quality` | Skew, blur, contrast, speckle, bleed-through, fold lines, photocopy generation; picks the binariser | 1 | implemented: [`quality`](detectors/quality.md); binarisers (Otsu, Sauvola, Wolf, Howe): [`binarize`](detectors/binarize.md) |
| M4 | `pixel.script` | Script identification from pixels, without ML (headline, connected-component and texture tiers) | 2 | implemented, opt-in, synthetic checks only: [`script`](detectors/script.md) (headline and component tiers; no texture tier; family level, not individual scripts) |
| M5 | `pixel.handwriting` | Handwritten vs machine-printed text, including filled-in blanks on printed forms | 2 | implemented, opt-in, **weak**: [`handwriting`](detectors/handwriting.md) (finds round cursive-like writing in synthetic tests, nothing in block capitals; real handwriting untested) |
| M6 | `pixel.stamp_seal` | Rubber stamps and seals, by colour clustering and by shape | 3 | implemented, opt-in, needs a colour page, synthetic checks only: [`stamp`](detectors/stamp.md) |
| M7 | `pixel.signature` | Wet signatures | 3 | implemented, opt-in, synthetic checks only; overlaps handwriting: [`signature`](detectors/signature.md) |
| M8 | `pixel.thumb_impression` | Thumb and finger impressions, via ridge coherence and frequency | 3 | implemented, opt-in, **weak on faint or smudged prints**, synthetic checks only: [`thumb`](detectors/thumb.md) |
| M9 | `pixel.rulings` | Rulings, tables, checkbox grids, and fold lines vs rules | 3 | implemented, opt-in, synthetic checks only: [`rulings`](detectors/rulings.md) |
| M10 | `pixel.content_type` | Page and region typing (text, form, table, photo, blank, ...) | 0 (blank skeleton), later phases | page typing from the layout and the other detectors' findings, opt-in with `layout`, synthetic only: [`page_type`](detectors/page_type.md); blank page: skeleton, [`blank_page`](detectors/blank_page.md); the picture regions: [`photo`](detectors/photo.md); the other regions are the layout blocks and the other detectors' findings, with no separate region type |
| M11 | `india.*` | Indian-deed detectors: stamp-paper header, e-stamp QR, registrar endorsement, embossed seal | 4 | e-stamp QR (finder patterns, no decoding): [`qr`](detectors/qr.md); embossed seal: [`emboss`](detectors/emboss.md); registrar endorsement across pages: [`endorsement`](detectors/endorsement.md); the `estamp_qr` and `stamp_paper_header` composites: [`fusion`](detectors/fusion.md). All synthetic only, no real deed seen |
| M12 | `provenance.*` | JPEG tables, PDF producer, incremental updates, redaction boxes, near-duplicate pages | 1 and later | near-duplicate pages (perceptual hashes): [`phash`](detectors/phash.md); PDF structure, incremental updates and JPEG tables: [`provenance`](detectors/provenance.md) (cargo feature `pdf`); redaction boxes: pixel part (opt-in, synthetic only) and PDF part (a filled box over text still in the content stream, black or white; one synthetic PDF): [`redaction`](detectors/redaction.md) |
| (extra) | `pixel.coverage` | Extraction coverage: how much of the page's text a text layer or OCR covers | 2 | implemented, opt-in, synthetic checks only: [`coverage`](detectors/coverage.md) |
| M13 | `fusion` | Log-odds evidence sum, calibration, abstention and cross-detector consistency rules | 0 (sum only), 5 | log-odds sum with per-detector group caps; cross-detector rules; per-kind calibration maps and a fitting tool (no map shipped: no labelled data); no learned weights: [`fusion`](detectors/fusion.md) |

## Phases

| Phase | Scope |
|---|---|
| 0 Skeleton | Workspace, schema and JSON Schema export, params, CLI, Python and WASM builds, determinism gate, bench harness |
| 1 Text layer and quality | M1, M2, M3, Howe binarisation, perceptual hash, incremental-update detection |
| 2 Script and handwriting | Layout segmentation ([`layout`](detectors/layout.md)), M4 ([`script`](detectors/script.md)), M5 ([`handwriting`](detectors/handwriting.md)): built, opt-in, synthetic checks only |
| 3 Stamps, signatures, thumbs | M6, M7, M8, M9 |
| 4 Indian deeds | M11, registrar-endorsement repetition across pages; needs a small in-house annotated set |
| 5 Fusion, calibration, release | M13 calibration, npm and PyPI packages, docs site, offline browser demo |
