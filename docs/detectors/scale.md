# `scale` (resolution from image headers, paper-size hints)

Nothing in the core is tied to A4 or to 150 dpi: the page size comes from the pixels and the resolution from
the `dpi` you pass. This module helps the host get that number right. It is **not a detector** (it adds no
finding); it is two functions in `pagetriage_core::scale`, also exposed as `image_dpi` and `paper_matches` in
the Python and WASM bindings, and used by the CLI.

## `image_dpi(bytes)`

Reads the resolution a file *claims* from its header, without decoding pixels: PNG `pHYs`, JPEG JFIF and Exif
(resolution unit inches or centimetres), TIFF `XResolution`/`YResolution`/`ResolutionUnit`, BMP pixels per
metre. Returns `x`, `y` and the `source`, or nothing when the file says nothing or says something absurd.

## `paper_matches(width_px, height_px, tolerance)`

For a pixel size, lists the standard papers (A0 to A6, Letter, Legal, Tabloid, ...) and the dpi at which each
would give that size, within `tolerance` (a share of the side), landscape or portrait, best first. It is a
**hint**: 1240 x 1754 fits A4 at 150 dpi, but also a different paper at another dpi.

## CLI behaviour

`--dpi` wins; otherwise the header; otherwise 150, with a note on stderr that names the paper-size hint. The
hint is never used as the dpi. `document` uses the first image's resolved dpi for every page. `--max-pixels`
(also `Options.max_pixels`) refuses an image above the cap with `exceeds max_pixels`; the core has no tiling
and a poster-sized sheet takes seconds and over a gigabyte.

## Failure gallery

| Case | What happens | Where |
|---|---|---|
| **Scanner writes 72 or 96 dpi** | Many scanners and phone apps leave a placeholder. The header is read as stated, so the page is sized as a poster. The paper hint on stderr shows it; the fix is `--dpi`. | By construction |
| **Header and pixels disagree** (a file resized without updating it) | Nothing checks them against each other. | By construction |
| **Non-standard paper** (deeds on foolscap, 13 x 8 in, receipts) | No paper matches; the hint list is empty. | By construction |
| **Several papers fit** | A4 at 150 dpi and A5 at 212 dpi are the same pixels; both are listed, none chosen. | By construction |
| **PDF** | Its pages have a size in points; the rasteriser, not this module, picks the dpi. | n/a |
| **HEIC, WebP, JPEG 2000, multi-page TIFF subimages** | Not read; no dpi. | Open |

Unit tests in `scale.rs` (7) and `tests/page_sizes.rs`.
