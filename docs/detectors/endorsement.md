# Registrar endorsement across pages (module M11, document level)

A registered deed carries the registrar's endorsement (a stamp, a seal or a signature block) on every
page, usually in a margin. One page cannot know that a mark repeats, so this runs over a document's page
reports, **after** the page analyses, like the near-duplicate pass of [phash](phash.md). It adds
`kind: "registrar_endorsement"` findings to the pages that carry the repeated mark, with `attributes`:
`pages` (how many pages have it), `document_pages`, `mark` (`stamp`, `seal` or `signature`).

It does not look at pixels, and it does not know what an endorsement says or whether it is genuine. It
finds **a mark that sits in the same place on many pages**. **Read this first**: it has been tried only
on synthetic page reports; no real deed has been looked at.

## How to run it

- Rust: `pagetriage_core::finish_document(pages_json, opts_json)`; Python and WASM: `finish_document`
  with the same arguments; CLI: `pagetriage document page1.png page2.png ... --detectors stamp,...`.
  `pages_json` is a JSON array of the page reports `analyse_page` returned, in page order; the result
  is the canonical JSON array. The pass also fills `near_duplicate_of` when the reports carry hashes
  (the `phash` detector), and running it twice changes nothing.
- It needs the stamp, seal or signature findings, so those detectors must have run on the pages (stamp
  needs a colour page).

## Algorithm

1. Marks are the page's `stamp`, `seal` and `signature` findings with a bbox, converted to fractions of
   the page (centre, width, height; the page size comes from `size_pt` and `analysis_dpi`).
2. A mark joins the group of an earlier page's mark when the kind is the same, the centre is within
   `position_tolerance` (6% of the page) in x and y, and the width and height are within `size_ratio`
   (1.5). Each group takes at most one mark per page, the first that fits.
3. A group with marks on at least `min_pages` (3) pages and `min_share` (50%) of the document's pages is
   an endorsement.
4. Confidence `sigmoid(prior + share term + margin term + spread term)`: how much of the document
   carries it, whether it sits within 20% of a page edge, and how tightly the centres agree. The three
   terms are in the finding's `evidence`. Parameters: `params/document.toml`, all hand set.

## Evaluation

Unit tests and one golden: a five-page synthetic deed (`tests/golden/document_inputs/deed_pages.json`)
with a stamp in the same corner of four pages (found, confidence 0.85), a seal on one page and a
signature on two (not found), and two pages with equal hashes (page 4 marked a near-duplicate of
page 0). The native, Python and WASM runners compare the result with the expected file byte for byte.
There is **no benchmark beyond that**: the inputs were written to have these answers.

## Failure gallery

| Case | What happens | Where |
|---|---|---|
| **Real deeds** | Never seen. Endorsements are often on only some pages, rotated, or in the text area. | Open |
| **Drift** | A stamp applied by hand moves more than 6% of the page between pages: not grouped. | By construction |
| **Letterhead logos, page numbers, footers** | A logo detected as a stamp on every page is reported as an endorsement. Nothing separates a repeated stamp from a repeated printed mark. | By construction |
| **Pages missing a mark** | A mark on fewer than half of the pages is not an endorsement; the pages that lack it get none. | By construction |
| **Mixed document sizes** | Positions are fractions of each page, so pages scanned at different sizes group; a landscape insert does not. | Not tested |
| **Greedy grouping** | Groups are built from the first unused mark; two marks close together on one page could be assigned in a different way than a global match would. | By construction |
