# `provenance` (module M12), PDF structure part

The signals about a PDF file's history that can be read without rendering it: who produced it,
how it is laid out, whether it was changed after the fact, and what encoder made its JPEG scans.
Perceptual hashes and near-duplicate pages are a different part of M12 (see [`phash`](phash.md));
redaction-box detection is not implemented yet. This part is reported in `structure` (the document
level) and `pages[i].images` of `analyse_pdf_structure` and needs the `pdf` cargo feature.

## What is reported

| Field | Source |
|---|---|
| `document.producer`, `structure.creator`, `creation_date`, `mod_date` | the trailer's `/Info` (PDF text strings: UTF-16BE/LE and UTF-8 with a BOM, otherwise Latin-1 as an approximation of PDFDocEncoding), capped at `max_meta_chars`; `producer` falls back to the XMP `pdf:Producer` |
| `xmp_present`, `xmp_tool` | the catalog's `/Metadata` stream; `xmp:CreatorTool` or `pdf:Producer` by a plain text search (not an XML parser) |
| `producer_family`, `ocr_producer` | substring match of producer, creator and XMP tool against `params/provenance.toml` (`ocrmypdf`, `tesseract`, `abbyy`, `paper capture`, ...). The list is **written from memory and unverified**; `ocr = true` entries feed M1 as weak evidence |
| `xref_style` | `classic`, `stream` or `hybrid` (`/XRefStm`), from the last `startxref` |
| `linearized` | `/Linearized` within the first 2 KiB |
| `encrypted` | `/Encrypt` present (opened with the empty password, or not: the report then has no page verdicts) |
| `pdf_version` | the header |
| `eof_markers`, `revisions`, `signature_count`, `tamper_signal`, `tamper_reasons`, `content_changed_in_update` | next section |
| `jpeg_images`, `jpeg_table_hashes`, `pages[].images[].jpeg` | after that |
| `page_states`, `unreliable_pages` | counts over the pages, for a one-line summary |

`document.incremental_updates` is the number of revisions beyond the first.

## Incremental updates

A PDF can be edited by appending: new and changed objects, a new cross-reference section, a new
trailer with `/Prev`, and another `%%EOF`. The original bytes stay in the file, so the file carries
its own history.

1. **Find revisions.** Every `%%EOF` marker is a candidate end. The prefix up to it is parsed on its
   own with `lopdf`; it counts as a revision only if it loads without cross-reference repair and has
   a catalog. That discards a `%%EOF` inside binary data, and the first part of a linearised file (its
   trailer's `/Prev` points past the prefix), so **a linearised file is one revision, not two**. The
   most recent `max_revisions` (24) prefixes are examined, within `max_revision_parse_bytes` of
   parsing; beyond that `revisions_truncated` is set.
2. **Diff** consecutive revisions object by object (added, changed, freed; streams compared by
   dictionary and bytes). Changed and added objects are classified, most specific first:

   | Kind | Object |
   |---|---|
   | `signature_added` | a `/Type /Sig` dictionary, or one with `/ByteRange`, `/Contents` and `/Filter`, new in this revision |
   | `metadata_changed` | the Info dictionary or the XMP stream |
   | `page_content_changed` (+ page index) | a page's content stream; or a page dictionary whose `/Contents`, `/Resources`, `/MediaBox`, `/CropBox` or `/Rotate` changed |
   | `annotation_added`, `annotation_changed` | an annotation dictionary, or a page whose only changed key is `/Annots` |
   | `form_field_changed` | a widget annotation |
   | `image_added`, `image_changed` | an image XObject (new, or an existing one replaced) |
   | `xobject_changed` | an existing form XObject replaced |
   | `font_changed` | font or font descriptor |
   | `page_tree_changed` | the set of pages differs, or a `/Pages` node changed |
   | `catalog_changed`, `other_changed` | everything else |

3. **Tamper signal.** `tamper_signal` is true when a revision *after* the first revision that
   contains a signature changes what a page shows: `page_content_changed`, `image_changed`,
   `xobject_changed` or `page_tree_changed`. A content change in the *same* revision as the signature
   is how signing works and is not a signal; added annotations and form-field changes after a
   signature are benign by this rule (adding annotations is how a reviewer marks up a signed file).
   `content_changed_in_update` records an edit of page content in any update, signed or not.

Golden fixture `signed_then_edited.pdf`: base with a signature, update 1 adds an annotation
(`annotation_added`, `annotation_changed`), update 2 rewrites the amount in the page content:
`incremental_updates: 2`, `tamper_signal: true`, reason "update 2 changes page content
(page_content_changed) after the signature in revision 0".

## JPEG quantisation tables

From each `/DCTDecode` image stream whose only filter is `DCTDecode` (other chains are skipped), the
header is parsed up to the start of scan without decoding any pixels: `DQT` segments (8- and 16-bit,
any number, zig-zag undone), the `SOF` size, components and coding, the application markers. Reported
per image under `images[].jpeg`:

* `table_hash`: the first 16 bytes of SHA-256 over the tables in id order, so it does not depend on
  the order of the segments;
* `luma_quality` / `chroma_quality`: the IJG (libjpeg) quality whose scaling of the Annex K standard
  table is closest, and `luma_fit_error`, the mean absolute difference per entry. `ijg_scaled` is true
  when that error is at most `ijg_fit_mean_abs_error` (1.0). Encoders that do not scale the standard
  table (Photoshop, many scanner firmwares) give a poor fit, and the quality is then only an index.

Verified against real libjpeg output: Pillow-encoded grayscale JPEGs at qualities 1 to 100 give
`luma_quality` equal to the setting with zero fit error for all 100 (the three-component chroma table
saturates at 255 for qualities 2 and 3, where the estimator reports 1). On the synthetic corpus:
66 / 66 images exact. The idea of using quantisation tables as an encoder fingerprint is H.
Farid's JPEG-ballistics work (Dartmouth technical reports, 2006-2008); that citation is recalled and
unverified. **Not implemented:** a table-hash to scanner-family lookup (we have no verified table set;
the hash is reported so one can be built), and double-compression hints.

## Parameters

`crates/pagetriage-core/params/provenance.toml`. All limits are engineering limits; the producer list
and `ijg_fit_mean_abs_error` are placeholders ("tuned on <none>").

## Measured results

Synthetic corpus (`bench/structure_eval.py`; see `text_layer.md` for what is weak about it and for
the dev / held-out protocol). All 60 documents:

| Check | Result |
|---|---|
| `incremental_updates` equals the label | 60 / 60 (documents with 0 to 3 updates) |
| `document.producer` equals the label | 60 / 60 (the corpus writes "no producer" as `""`; the report has `null`) |
| JPEG quality equals the encoder setting | 66 / 66, all recognised as IJG-scaled |
| Same files rewritten by `qpdf` as object-stream / cross-reference-stream, linearised, and uncompressed | `xref_style`, `linearized` and the page verdicts as expected for all 180; linearised files report 0 updates |

The tamper rule is covered by unit and golden tests, not by corpus labels: the evaluation script does
not score `tamper_signal` or the change kinds, because the corpus has no ground truth for them that
we read.

## Failure gallery

| Case | What happens |
|---|---|
| A tool that rewrites the whole file (Acrobat "Save As", `qpdf`, a print-to-PDF pass) | History is gone: zero updates, no tamper signal. Only incremental saves leave evidence. |
| Signature detection is by dictionary shape (`/Type /Sig` or `/ByteRange` + `/Contents` + `/Filter`); no signature is validated | A file with a fake or broken signature dictionary counts as signed. Certification levels (DocMDP) that forbid later annotations are not read, so annotating a certified document is "benign" here. |
| An update that replaces an object stream, or renumbers objects | Not tested. Objects are compared after `lopdf` expands object streams, so members of a rewritten stream should compare equal when unchanged; renumbering would show as many added and freed objects. |
| Hybrid-reference files, or cross-reference streams spread over revisions | Not tested with updates. An update whose prefix does not load cleanly is skipped, so the count can be lower than the number of `%%EOF` markers. |
| More than 24 revisions | Only the latest are examined; `revisions_truncated`. |
| `%%EOF` inside a stream, in a comment, or in trailing junk | Not a revision (the prefix must load); `eof_markers` still counts the text. |
| Producer strings | Trivially spoofed, and the fingerprint list is from memory: `producer_family` is a hint, not an attribution. A producer disagreeing with the XMP tool is not flagged. |
| Info dates | Reported as written, never compared with the revision history or the file's own timestamps. |
| JPEG in a multi-filter chain (`/Filter [/ASCII85Decode /DCTDecode]`) | Skipped; no tables reported. |
| JPEG 2000, JBIG2, CCITT scans | No tables; the image is listed with its filter only. |
| Progressive or arithmetic-coded JPEG | Tables are read; the coding is reported as `progressive` or `other`. |

## What is not covered

Real signed PDFs (from Indian e-sign services, DSC tokens), real incrementally-edited deeds,
PDFs saved by phone scanner apps, documents carrying several signatures or timestamps, redaction
boxes and under-redaction (not implemented), page-level hashes of rasters (see `phash.md`), and any
check that the file's claimed producer matches how it is actually written.
