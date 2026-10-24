# `phash` (module M12): perceptual hashes and near-duplicate pages

Gives every page two 64-bit hashes, `phash` and `dhash`, as 16 hex characters in the page report.
A second, document-level step (`near_duplicates`) compares the hashes of a document's pages and
reports, for each page, the earliest earlier page it is a near-duplicate of. The target case is the
same deed page scanned twice. It produces no `finding`: a hash is a fingerprint, not a claim about
the page.

**Read this first.** The hash is good at "the same scan, re-encoded, noisy, dimmer or nudged by
less than about 1% of the page", and bad at three things that matter for deeds: it misses a copy
moved by 3% or more or rotated by a couple of degrees, it misses a page scanned upside down, and it
cannot tell apart pages that share a layout. On made-up typed pages with an identical layout, 12%
to 16% of *distinct* page pairs fall within the plan's 10-bit threshold (3.6% to 8.6% when the
layouts vary); on 136 very different DIBCO pages the rate is zero. The DIBCO number says nothing
about consecutive pages of one typed deed. See [Evaluation](#evaluation) and the
[failure gallery](#failure-gallery).

## Algorithm

All steps are integer or fixed-order IEEE arithmetic, so the strings are identical on native,
Python and WASM (checked by `scripts/determinism.sh`, see [Determinism](#determinism)).

1. **Area-average resize** of the `width x height` gray page to 32 x 32, in exact integers. Scale
   both axes so a source pixel spans `out` units and a target cell spans `src` units; every overlap
   is then a whole number, each target cell has total weight `src`, and a cell's mean is
   `sum / (width * height)`. One float division per cell. Any page size works, including pages
   smaller than 32 px. Pixel replication (a page at 2x) gives bit-identical hashes.
2. **2-D DCT-II**, orthonormal, built from a cosine table of `math::cos` (the pure-Rust `libm`).
   Only the top-left 8 x 8 block is computed. Every sum of products goes through
   `math::pairwise_sum`.
3. **pHash**: bit `i` (row-major over the 8 x 8 block, `(0,0)` first, most significant bit first) is
   set when coefficient `i` is greater than the **median of the 63 non-DC coefficients**. The DC
   coefficient is left out of the median but still gets a bit (set unless the page is nearly black).
   With distinct coefficients exactly 31 AC bits are set, so a real pHash has 31 or 32 bits set.
4. **dHash**: resize to 9 columns by 8 rows (same exact method), bit = "the cell to the right is
   brighter than this one", row-major, 64 bits.
5. **Flat pages.** If the standard deviation of the 63 low-frequency AC coefficients (Parseval:
   the gray-level spread of the low-passed page) is below `flat_rms`, both hashes are all zeros.
   A real pHash always has about 32 bits set, so the all-zero value is unambiguous and
   `PageHashes::is_flat()` tests for it. `near_duplicates` never matches a flat page and never
   points at one. Without this, the bits of a blank scan would be decided by sensor noise.

The hash takes **any 8-bit buffer**. Phase 1 hashes the gray page. The plan calls for the
deskewed, binarised page; that is the same call with a different buffer, and `hash_ink_mask` does
the conversion from a binariser's `1 = ink` mask (ink drawn black on white, so the hash then
ignores paper colour, stains and lighting). **Hashing a binarised or deskewed page has not been
evaluated here**; the numbers below are for the gray page.

## Parameters

`crates/pagetriage-core/params/phash.toml`:

| Key | Value | Source |
|---|---|---|
| `max_hamming` | 10 | The project plan (Hamming <= 10 is a near-duplicate). **Not tuned by us.** |
| `dhash_max_hamming` | 22 | Tuned on the synthetic typed-page "tune" sets; 64 would switch the dHash test off. Rule and caveats below. |
| `flat_rms` | 0.25 | A stability guard, not tuned: below one 8-bit gray level. Smallest value seen on the 136 DIBCO pages: 2.41. |

The grid sizes (32 x 32, 8 x 8 block, 9 x 8 dHash) are part of the hash definition and are
constants in `pixel/phash.rs`: changing one changes every hash.

`dhash_max_hamming`: the smallest bound on the grid {10, 14, 18, 22, 26, 64} that keeps recall of
every perturbed-copy type within 2 points of having no dHash test, on both tune sets. The rule
was written before any set but the tune sets was used to choose; the "test" sets were printed beside
the tune sets in the same run (so they are not blind), and a third "check" set with a fresh seed was
added afterwards. See the synthetic results below for what it buys.

## Use

- **Per page.** `analyse_page` runs `phash` by default (`"detectors": ["phash"]` to run only it).
  The report gets `phash` and `dhash`; with `debug` it also gets `debug.phash.{mean_gray,
  lowfreq_rms, median_ac, flat}`. `near_duplicate_of` stays `null`: one page cannot know its twins.
- **Per document, Rust.** `pixel::phash::near_duplicates(&[PageHashes], &PhashParams)` returns,
  for each page, `Option<usize>`: the position of the **earliest earlier** page with pHash within
  `max_hamming` bits and dHash within `dhash_max_hamming`. `mark_near_duplicates(&mut [PageReport],
  &PhashParams)` does the same on the reports and writes the earlier report's `index` into
  `near_duplicate_of`. "Earliest" is not "nearest" and the relation is not transitive: if A~B and
  B~C but A is not ~C, C points at B (the golden fixtures show this: `text_like_shift1` points at
  `text_like_cropped`, not at the page it was made from).
- **Python.** `pagetriage_py.near_duplicates(phashes, dhashes, params_toml=None)` takes the hex
  strings from the reports and returns a list of `int | None`. `params_toml` overrides in the
  analyser's layout, for example `"[phash]\nmax_hamming = 6"`.
- **CLI.** `pagetriage duplicates page1.png page2.png ...` hashes the files in the order given and
  prints one JSON object per page with its hashes and `near_duplicate_of`.

## Determinism

The hashes appear in every golden report (`tests/golden/expected/*.json`), which the Python and
WASM runners compare byte for byte; `scripts/determinism.sh` passed on native, Python and WASM
(macOS arm64) with the fixtures below. The document-level step is integer-only, and
`tests/golden/near_duplicates.json` pins its output, with the pairwise Hamming matrices, for the
golden fixtures taken as one document in file-name order (`cargo test -p pagetriage-core --test
near_duplicates`; `UPDATE_GOLDEN=1` regenerates it).

## Evaluation

Everything below comes from `bench/phash_eval.py` (run it with
`python -I bench/phash_eval.py --data bench/data --synthetic 100 --out bench/results/phash_eval.json`;
about a minute; two runs give identical numbers apart from timing). It needs the Python binding,
numpy and Pillow (13.0.0 and numpy 2.5.4 here). Run 2026-10-24.

**Data.** The 136 DIBCO page images (11 editions, 2009 to 2019, ground-truth images excluded). They
are **cropped text regions, not full pages**: median 1419 x 647 px, 245 x 191 to 4161 x 3465, many
of them wide strips with no margins. Manuscripts, printed pages and papyri, all very different from
one another. No threshold is fitted to them: `max_hamming` is the plan's, `flat_rms` is a guard.
The pages are split by whole edition into a tuning half (2009, 2011, 2013, 2017, 2019 track A:
72 pages) and a held-out half (64 pages), chosen before looking at any result, so a later retune
has an untouched half; both are reported.

**Perturbations.** Each page, converted to gray with the core's luma, is perturbed 16 ways, all
seeded: shifts of 1, 2 and 3 px (diagonal, border pixels repeated), JPEG at quality 60 and 85,
Gaussian noise sigma 10, 0.5 degree rotation, brightness x0.9 and x1.1, and a 90%-of-area central
crop. Rows marked *extra* go beyond that set, because 1 to 3 px is tiny against a 1400 px page:
shifts of 0.5%, 1% and 3% of the page size (in both axes), a 97% crop, 50% downscale, 2 degree
rotation, a 180 degree turn, and a combined "rescan" copy (0.5% shift, 0.3 degrees, x0.95, noise
sigma 6, JPEG 85). Perturbations act on the gray page, so there is no chroma subsampling effect.

**Metrics.** *Bits changed*: Hamming distance between a page's pHash and its copy's. *Recall*: the
copy is within `max_hamming` = 10 bits of its own original. *False positive*: a perturbed copy of
page i is within 10 bits of the original of a different page j (18,360 pairs per row; 0 hits is
"under 0.016% at 95% confidence" by the rule of three). dHash gating only removes matches, so its
recall column can only be lower.

### Distinct DIBCO pages

9180 pairs of different pages: pHash distance min 14, p1 22, p5 24, median 30, p99 40, max 44.
**No pair is within 10 bits**; five are within 16 and 86 within 20. The closest pair
(`dibco2013/PR01` and `dibco2017/17`, 14 bits) are different documents (a Dutch letter and an
English play). The held-out half alone: min 16, median 32, none within 10. An unrelated pair
averages 32 bits apart, which is what independent 64-bit hashes give.

### Perturbed copies (136 DIBCO pages)

| perturbation | bits changed, median | p95 | max | recall @10, all 136 | recall @10, held-out 64 | recall with dHash<=22, all | dHash bits, median | false positives @10 |
|---|---|---|---|---|---|---|---|---|
| shift_1px | 0 | 2 | 6 | 136/136 (100.0%) | 64/64 (100.0%) | 136/136 | 1 | 0/18360 |
| shift_2px | 2 | 4 | 8 | 136/136 (100.0%) | 64/64 (100.0%) | 136/136 | 1 | 0/18360 |
| shift_3px | 2 | 6 | 8 | 136/136 (100.0%) | 64/64 (100.0%) | 136/136 | 2 | 0/18360 |
| jpeg_q60 | 0 | 2 | 2 | 136/136 (100.0%) | 64/64 (100.0%) | 136/136 | 0 | 0/18360 |
| jpeg_q85 | 0 | 0 | 2 | 136/136 (100.0%) | 64/64 (100.0%) | 136/136 | 0 | 0/18360 |
| noise_sigma10 | 0 | 2 | 4 | 136/136 (100.0%) | 64/64 (100.0%) | 136/136 | 0 | 0/18360 |
| rotate_0.5deg | 2 | 6 | 8 | 136/136 (100.0%) | 64/64 (100.0%) | 136/136 | 2 | 0/18360 |
| brightness_-10% | 0 | 2 | 2 | 136/136 (100.0%) | 64/64 (100.0%) | 136/136 | 0 | 0/18360 |
| brightness_+10% | 0 | 4 | 10 | 136/136 (100.0%) | 64/64 (100.0%) | 136/136 | 0 | 0/18360 |
| **crop_90%_area** | 8 | 12 | 16 | **123/136 (90.4%)** | 59/64 (92.2%) | 123/136 | 7 | 0/18360 |
| shift_0.5% (extra) | 2 | 6 | 8 | 136/136 (100.0%) | 64/64 (100.0%) | 136/136 | 3 | 0/18360 |
| shift_1% (extra) | 4 | 10 | 16 | 131/136 (96.3%) | 62/64 (96.9%) | 131/136 | 6 | 0/18360 |
| **shift_3% (extra)** | 12 | 18 | 22 | **53/136 (39.0%)** | 29/64 (45.3%) | 51/136 | 14 | 0/18360 |
| crop_97%_area (extra) | 2 | 6 | 10 | 136/136 (100.0%) | 64/64 (100.0%) | 136/136 | 2 | 0/18360 |
| downscale_50% (extra) | 0 | 0 | 2 | 136/136 (100.0%) | 64/64 (100.0%) | 136/136 | 0 | 0/18360 |
| **rotate_2deg (extra)** | 10 | 24 | 32 | **77/136 (56.6%)** | 35/64 (54.7%) | 77/136 | 7 | 0/18360 |
| **rotate_180deg (extra)** | 32 | 34 | 38 | **0/136 (0.0%)** | 0/64 (0.0%) | 0/136 | 30 | 0/18360 |
| rescan_combo (extra) | 2 | 8 | 12 | 135/136 (99.3%) | 64/64 (100.0%) | 135/136 | 4 | 1/18360 |

Recall by threshold, all 136 pages (the 10-bit column is the table above):

| perturbation | <=4 | <=6 | <=8 | <=10 | <=12 | <=16 |
|---|---|---|---|---|---|---|
| shift_1px | 99% | 100% | 100% | 100% | 100% | 100% |
| shift_3px | 93% | 96% | 100% | 100% | 100% | 100% |
| rotate_0.5deg | 88% | 97% | 100% | 100% | 100% | 100% |
| crop_90%_area | 15% | 47% | 70% | 90% | 96% | 100% |
| shift_1% (extra) | 59% | 80% | 94% | 96% | 99% | 100% |
| shift_3% (extra) | 0% | 1% | 13% | 39% | 62% | 90% |
| rotate_2deg (extra) | 15% | 28% | 49% | 57% | 70% | 82% |
| rescan_combo (extra) | 82% | 94% | 99% | 99% | 100% | 100% |

(Brightness, JPEG, noise and downscale are 100% at every threshold from 6 up.)

**Document-level check** (`pagetriage_py.near_duplicates` on a document of the 136 originals
followed by one copy of each, default parameters including the dHash test): a copy is right when it points at its own
original. JPEG, noise, brightness, shifts up to 1%, 0.5 degree rotation, 97% crop, downscale: 136
right, 0 missed, 0 wrong. Crop 90%: 123 right, 13 missed. Rescan combo: 135 right, 1 missed.
Shift 3%: 51 right, 85 missed. Rotate 2 degrees: 77 right, 58 missed, **1 wrong target**. Rotate
180 degrees: 0 right. No original was flagged as a duplicate of another original. The Rust function
agrees with an independent Python implementation of the rule on the originals.

### Synthetic typed pages with a shared layout

DIBCO cannot answer the question that matters for deeds, which is whether *different pages of the
same kind of document* collide. So the harness renders made-up A4 text pages at 100 dpi (Pillow's
bundled font, a vocabulary of 73 distinct legal-sounding words, Gaussian sensor noise): **dense**, where every
page has the same margins and fills every line, ragged right, and **varied**, with a random
heading, paragraph breaks, line count and signature block. 100 pages each, three seeds (tune, test,
check), 4950 distinct pairs per set, and four perturbed copies per page. **These are not scans and
not real deeds.** Real pages have stamps, seals, mixed fonts and handwriting that may separate
them better, or tables and forms that may separate them worse; nobody has measured it.

Distinct pairs, **check set** (the one that was unseen when `dhash_max_hamming` was chosen):

| layout | min | p1 | p5 | median | p99 | pairs within 10 bits |
|---|---|---|---|---|---|---|
| dense | 2 | 6 | 8 | 16 | 28 | **15.58%** |
| varied | 4 | 8 | 10 | 16 | 26 | **8.61%** |

The tune and test sets agree (dense 18.3% and 16.0%, varied 9.0% and 9.5%). Independent hashes
would be 32 bits apart; pages that share a layout also share most of their low-frequency
structure, so half of the bits are decided by the layout and the median distance is 16.

False positives against recall as `max_hamming` varies, check set, with the shipped dHash rule in
the last row:

| dense layout, `max_hamming` | false-positive rate | recall: rescan combo | shift 1% | shift 3 px | crop 90% |
|---|---|---|---|---|---|
| 4 | 0.38% | 22% | 5% | 71% | 70% |
| 6 | 2.59% | 39% | 15% | 89% | 94% |
| 8 | 7.09% | 63% | 35% | 95% | 100% |
| 10 | 15.58% | 84% | 58% | 100% | 100% |
| 12 | 26.83% | 93% | 81% | 100% | 100% |
| 10 + dHash<=22 | **12.30%** | 84% | 58% | 100% | 100% |

| varied layout, `max_hamming` | false-positive rate | recall: rescan combo | shift 1% | shift 3 px | crop 90% |
|---|---|---|---|---|---|
| 4 | 0.08% | 96% | 61% | 98% | 58% |
| 6 | 0.71% | 100% | 89% | 100% | 87% |
| 8 | 2.77% | 100% | 98% | 100% | 97% |
| 10 | 8.61% | 100% | 100% | 100% | 100% |
| 12 | 18.57% | 100% | 100% | 100% | 100% |
| 10 + dHash<=22 | **3.64%** | 100% | 100% | 100% | 100% |

So the plan's threshold of 10 is **too loose for typed pages that share a layout**: at 10 bits,
about one distinct dense page pair in seven and one varied pair in twelve is called a duplicate,
and the dHash test removes only some of those (to 12.3% and 3.6%). A threshold of 6 gives 2.6% and
0.7% false positives and still catches a same-registration rescan of a varied page; it loses
shifted dense pages. The default stays at the plan's 10 because there are no real deed scans to
choose from. Whoever sets it for production should do so on an in-house set of real consecutive
pages and real rescans, which this repository does not have yet.

### Benchmark line

Hashing (both hashes, one call through the Python binding, release build, Apple silicon laptop):
median **3.0 ms per page** and **3.4 ms per megapixel** over the 136 DIBCO pages, slowest page
30 ms (an 8.9 MP scan, the largest in the set). Measured by the host; the core never reads the clock. WASM was not
timed. The WASM binary stays within the size budget (427 KB raw, 173 KB gzipped).

## What breaks, and what it means for rescanned deeds

- **Shifts and crops.** A 1 to 3 px shift of a 1400 px page (0.07% to 0.2%) changes at most 8 bits.
  A shift of 1% of the page (2 to 3 mm on A4) still passes 96% of the time at 10 bits, but a
  shift of 3% (6 to 9 mm on A4) passes only 39%, and a 90%-area crop (5% off every side) passes 90%.
  For the dense synthetic layout a 1% shift passes only 58%. A rescan of a deed on a flatbed, or an
  auto-crop that trims the margins differently, can easily move content by a few millimetres, so
  **expect misses on rescans that were not scanned with the page registered to the same corner.**
  The millimetre figures are my arithmetic from A4 dimensions; I have no rescanned deed pairs to
  measure real misregistration.
- **Rotation.** 0.5 degrees is harmless; 2 degrees passes 57%; a page scanned upside down or sideways
  is never found (180 degrees: 0 of 136). Deskewing before hashing (planned) fixes the first
  case, but not an upside-down page.
- **Same-layout pages collide.** See above. This is the main risk for the intended use.
- **Robust to** JPEG down to quality 60, Gaussian noise of sigma 10, brightness changes of 10%,
  halving the resolution, and shifts under 0.5%: 100% recall with no false positives in all of them.

## Failure gallery

Known wrong or fragile answers, pinned where possible by synthetic fixtures in `tests/golden/`
(64 x 80 px, generated by `tests/golden/gen_inputs.py`; distances are pHash bits against
`text_like`, from `tests/golden/near_duplicates.json`).

| Case | What happens | Evidence |
|---|---|---|
| **Same page, rescanned at the same registration** (5% dimmer, noise) | Found: 0 bits apart. | `text_like_rescan.pgm` |
| **Content moved by about 3% or more** | Not found. 36 bits apart for a (10 px, 8 px) shift; 12 bits (a miss at 10) for a 1 px shift on this 64 px page, which is 1.6% of its width. At real page sizes 1 px is under 0.1% and harmless. | `text_like_shifted.pgm`, `text_like_shift1.pgm`; DIBCO shift rows above |
| **Crop of about 90% of the area** | Borderline: 10 bits, exactly at the threshold, matched here; 90% of DIBCO pages match, 13 of 136 do not (`P05`, `PR8`, `H02`, `H14`, `HW01`, `HW03`, `PR01`, `2016/8`, `2017/17`, `2018/1`, `2018/3`, `2019A/2`, `2019A/4`). | `text_like_cropped.pgm`; `crop_90%_area` row |
| **Different page, same layout** (typed forms, consecutive deed pages) | Called a duplicate in 12% to 16% of dense and 3.6% to 8.6% of varied synthetic pairs at the default setting. The 64 x 80 fixture is too small to show it (22 bits apart, no collision); the evidence is the synthetic arm of `bench/phash_eval.py`, at 827 x 1169 px. | `text_like_other.pgm` (does not reproduce at this size); synthetic tables |
| **Chain of near-duplicates** | A~B and B~C but A not~C: C points at B, the earliest earlier page *within the threshold*, which may not be the page C was copied from. | `text_like_shift1` points at `text_like_cropped` (6 bits), not at `text_like` (12 bits) |
| **Blank page with sensor noise only** | Both hashes all zeros; never matched, never a target. | `flat_blank.pgm` |
| **Blank page with a few dust specks** | Not flat (5 specks give a low-frequency spread of 0.76 against the 0.25 guard), so its bits are decided by where the specks fell: about 32 bits from everything, so it neither matches nor is matched. A blank page with a consistent defect (scanner streak, binding shadow, vignetting) would probably hash alike on every page; **not measured**. | `blank_speckle.pgm` |
| **Page scanned upside down or rotated 90 degrees** | Never found (180 degrees: 0 of 136; the median distance is 32, the value for unrelated pages). | `rotate_180deg` row |
| **Large rotation** | 2 degrees: 57% found, and one copy pointed at a different page. | `rotate_2deg` row |
| **Hash of a binarised or deskewed page** | Not evaluated. The function takes any buffer; whether it removes the shift or rotation weakness is open (shifts it will not fix). | none |
| **Same-layout false-positive rate on real deeds** | Unknown. No real consecutive deed pages are available to this project. | none |

## Not done

- Hashing the deskewed, binarised page (needs M3's deskew; `hash_ink_mask` is ready for it).
- A document-level step that removes the template shared by all pages before comparing, which is the
  obvious way to cut the same-layout false positives. It would need the DCT coefficients, not just
  the 64-bit strings, so it is a different interface; not tried.
- Shift-tolerant comparison (compare against shifted copies of the page, or hash several crops).
- A line-length or ragged-edge signature, which would separate typed pages that share margins.
- An in-house set of real deed pages and rescans to set `max_hamming` and `dhash_max_hamming`.

## References

pHash follows C. Zauner's description of the DCT hash and dHash follows N. Krawetz's difference
hash; the orthonormal DCT-II is Ahmed, Natarajan and Rao. All three citations are recalled from
memory and not re-checked; see `docs/REFERENCES.md`. The 10-bit threshold is the project plan's,
not a number from a paper.
