//! Perceptual hashes and near-duplicate pages (module M12).
//!
//! Two 64-bit hashes per page, both taken from a small area-averaged copy of the page:
//!
//! * **pHash**: resize to 32x32, orthonormal 2-D DCT-II, keep the top-left 8x8 block of
//!   low-frequency coefficients, and set bit `i` when coefficient `i` is above the median of the 63
//!   non-DC coefficients (the DC term is excluded from the median, but it still gets a bit).
//! * **dHash**: resize to 9 wide by 8 high, bit = "the cell to the right is brighter".
//!
//! Both are written as 16 lowercase hex characters, first coefficient (or cell) in the most
//! significant bit, so the string reads in scan order.
//!
//! Determinism: the resize accumulates in exact `u64` integers (overlap weights are integers, see
//! [`axis_weights`]) and divides once per cell; the DCT uses a cosine table built with
//! [`math::cos`] and sums every product list with [`math::pairwise_sum`] in fixed order. Nothing
//! depends on the platform libm, thread count or SIMD width.
//!
//! The input is any 8-bit buffer. Phase 1 hashes the gray page; the plan hashes the deskewed,
//! binarised page, which is the same call with a different buffer (see [`hash_ink_mask`]). Nothing
//! below assumes the buffer is a raw scan.
//!
//! Known behaviour, measured in `docs/detectors/phash.md`: hashes survive JPEG, noise, small
//! brightness changes and shifts that are small against the page, but a shift or crop of a few
//! percent of the page changes many bits, and distinct pages that share a layout (typed forms,
//! consecutive deed pages) hash close together.

use std::collections::BTreeMap;

use super::Detection;
use crate::error::{Error, Result};
use crate::math;
use crate::params::PhashParams;
use crate::schema::PageReport;

pub const PHASH: &str = "phash";

/// Side of the square the page is reduced to before the DCT. Part of the hash definition.
const GRID: usize = 32;
/// Side of the low-frequency block kept from the DCT: 8 x 8 = 64 bits.
const BLOCK: usize = 8;
/// dHash grid: 9 columns give 8 horizontal comparisons per row, 8 rows give 64 bits.
const DH_COLS: usize = 9;
const DH_ROWS: usize = 8;

/// The two hashes of one page. A page with no structure to hash (see
/// [`PhashParams::flat_rms`]) has both hashes zero; [`PageHashes::is_flat`] tests for that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageHashes {
    pub phash: u64,
    pub dhash: u64,
}

impl PageHashes {
    pub fn phash_hex(&self) -> String {
        to_hex(self.phash)
    }

    pub fn dhash_hex(&self) -> String {
        to_hex(self.dhash)
    }

    pub fn from_hex(phash: &str, dhash: &str) -> Result<Self> {
        Ok(Self {
            phash: parse_hex(phash)?,
            dhash: parse_hex(dhash)?,
        })
    }

    /// True for the all-zero hash that stands for "no structure". A page with structure always has
    /// about half its pHash bits set (31 or 32 of 64), so this cannot be a real hash.
    pub fn is_flat(&self) -> bool {
        self.phash == 0
    }
}

/// 16 lowercase hex characters, most significant nibble first.
pub fn to_hex(hash: u64) -> String {
    format!("{hash:016x}")
}

/// Inverse of [`to_hex`]. Exactly 16 hex digits, either case; anything else is an error.
pub fn parse_hex(s: &str) -> Result<u64> {
    if s.len() != 16 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::InvalidInput(format!(
            "hash {s:?} is not 16 hex characters"
        )));
    }
    u64::from_str_radix(s, 16).map_err(|e| Error::InvalidInput(format!("hash {s:?}: {e}")))
}

/// Number of differing bits.
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Integer overlap weights of an area-average resize along one axis, as `(source, target, weight)`.
///
/// Scale both axes by a common factor so a source pixel spans `out_len` units and a target cell
/// spans `src_len` units. Every overlap is then a whole number of units, each target cell has total
/// weight exactly `src_len`, and no fractional arithmetic is needed to resize an arbitrary size.
fn axis_weights(src_len: usize, out_len: usize) -> Vec<(usize, usize, u64)> {
    let mut weights = Vec::with_capacity(src_len + out_len);
    for target in 0..out_len {
        let lo = target * src_len;
        let hi = lo + src_len;
        for source in lo / out_len..=(hi - 1) / out_len {
            let overlap = hi.min((source + 1) * out_len) - lo.max(source * out_len);
            weights.push((source, target, overlap as u64));
        }
    }
    weights
}

/// Area-average resize of a `width * height` page to `out_w * out_h` cells, returned as exact
/// integer sums: cell value = sum / (width * height). Row-major, `out_w` cells per row. Comparing
/// two sums compares the two cell means exactly, because every cell shares that denominator.
fn area_sums(gray: &[u8], width: usize, height: usize, out_w: usize, out_h: usize) -> Vec<u64> {
    let x_weights = axis_weights(width, out_w);
    let y_weights = axis_weights(height, out_h);
    // Horizontal pass: each source row reduced to `out_w` sums.
    let mut rows = vec![0u64; height * out_w];
    for (row, acc) in gray.chunks_exact(width).zip(rows.chunks_exact_mut(out_w)) {
        for &(source, target, weight) in &x_weights {
            acc[target] += weight * u64::from(row[source]);
        }
    }
    // Vertical pass.
    let mut out = vec![0u64; out_h * out_w];
    for &(source, target, weight) in &y_weights {
        let from = &rows[source * out_w..(source + 1) * out_w];
        for (acc, &v) in out[target * out_w..(target + 1) * out_w]
            .iter_mut()
            .zip(from)
        {
            *acc += weight * v;
        }
    }
    out
}

/// `table[k][n] = c_k * cos(pi * (2n + 1) * k / (2 * GRID))`, the first `BLOCK` rows of the
/// orthonormal DCT-II matrix: `c_0 = sqrt(1/GRID)`, `c_k = sqrt(2/GRID)`.
fn cosine_table() -> [[f64; GRID]; BLOCK] {
    let mut table = [[0.0; GRID]; BLOCK];
    for (k, row) in table.iter_mut().enumerate() {
        let scale = if k == 0 {
            (1.0 / GRID as f64).sqrt()
        } else {
            (2.0 / GRID as f64).sqrt()
        };
        for (n, cell) in row.iter_mut().enumerate() {
            let angle = std::f64::consts::PI * ((2 * n + 1) * k) as f64 / (2 * GRID) as f64;
            *cell = scale * math::cos(angle);
        }
    }
    table
}

/// Top-left `BLOCK x BLOCK` coefficients of the orthonormal 2-D DCT-II of a `GRID x GRID` image,
/// row-major (`[u * BLOCK + v]`, `u` vertical frequency). Only the coefficients that are kept are
/// computed. Orthonormal, so the sum of squares of all coefficients equals that of the pixels.
fn dct_block(image: &[f64]) -> [f64; BLOCK * BLOCK] {
    debug_assert_eq!(image.len(), GRID * GRID);
    let table = cosine_table();
    // Rows first: `horizontal[r][v]` = frequency `v` along row `r`.
    let mut horizontal = [[0.0f64; BLOCK]; GRID];
    let mut products = [0.0f64; GRID];
    for (r, out) in horizontal.iter_mut().enumerate() {
        let row = &image[r * GRID..(r + 1) * GRID];
        for (v, coefficient) in out.iter_mut().enumerate() {
            for n in 0..GRID {
                products[n] = row[n] * table[v][n];
            }
            *coefficient = math::pairwise_sum(&products);
        }
    }
    let mut block = [0.0f64; BLOCK * BLOCK];
    for v in 0..BLOCK {
        for u in 0..BLOCK {
            for r in 0..GRID {
                products[r] = horizontal[r][v] * table[u][r];
            }
            block[u * BLOCK + v] = math::pairwise_sum(&products);
        }
    }
    block
}

fn pack_bits(bits: impl Iterator<Item = bool>) -> u64 {
    bits.enumerate()
        .fold(0u64, |acc, (i, bit)| acc | (u64::from(bit) << (63 - i)))
}

/// Both hashes of a `width * height` 8-bit page, plus the raw values behind them (surfaced in the
/// report under `debug.phash`).
pub fn hash_page(
    gray: &[u8],
    width: usize,
    height: usize,
    params: &PhashParams,
) -> Result<(PageHashes, BTreeMap<String, f64>)> {
    if width == 0 || height == 0 || width.checked_mul(height) != Some(gray.len()) {
        return Err(Error::InvalidInput(format!(
            "gray buffer of {} bytes does not match {width}x{height}",
            gray.len()
        )));
    }
    let area = (width as u64 * height as u64) as f64;

    let sums = area_sums(gray, width, height, GRID, GRID);
    let means: Vec<f64> = sums.iter().map(|&s| s as f64 / area).collect();
    let block = dct_block(&means);

    let dc = block[0];
    let squares: Vec<f64> = block[1..].iter().map(|c| c * c).collect();
    let lowfreq_rms = (math::pairwise_sum(&squares) / (GRID * GRID) as f64).sqrt();
    let mut sorted = block[1..].to_vec();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];

    let flat = lowfreq_rms < params.flat_rms;
    let hashes = if flat {
        PageHashes { phash: 0, dhash: 0 }
    } else {
        let phash = pack_bits(block.iter().map(|&c| c > median));
        let cells = area_sums(gray, width, height, DH_COLS, DH_ROWS);
        let dhash = pack_bits((0..DH_ROWS).flat_map(|r| {
            let row = &cells[r * DH_COLS..(r + 1) * DH_COLS];
            (0..DH_COLS - 1).map(move |c| row[c + 1] > row[c])
        }));
        PageHashes { phash, dhash }
    };

    let mut features = BTreeMap::new();
    features.insert(
        "mean_gray".to_owned(),
        math::finite(dc / GRID as f64, "mean_gray")?,
    );
    features.insert(
        "lowfreq_rms".to_owned(),
        math::finite(lowfreq_rms, "lowfreq_rms")?,
    );
    features.insert("median_ac".to_owned(), math::finite(median, "median_ac")?);
    features.insert("flat".to_owned(), f64::from(u8::from(flat)));
    Ok((hashes, features))
}

/// [`hash_page`] for an ink mask (`1` = ink, `0` = paper, as the binarisers return it): ink is
/// drawn black on white, so the hash describes the ink distribution and ignores paper colour,
/// stains and lighting. This is the form the plan hashes (deskewed, binarised page).
pub fn hash_ink_mask(
    mask: &[u8],
    width: usize,
    height: usize,
    params: &PhashParams,
) -> Result<(PageHashes, BTreeMap<String, f64>)> {
    let gray: Vec<u8> = mask.iter().map(|&m| if m == 0 { 255 } else { 0 }).collect();
    hash_page(&gray, width, height, params)
}

/// The detector entry used by `analyse_page`: no findings (a hash is not a claim about the page),
/// the raw values as features, and the hashes themselves.
pub fn detect(
    gray: &[u8],
    width: usize,
    height: usize,
    params: &PhashParams,
) -> Result<(Detection, PageHashes)> {
    let (hashes, features) = hash_page(gray, width, height, params)?;
    Ok((
        Detection {
            findings: Vec::new(),
            features,
        },
        hashes,
    ))
}

/// Document-level step: for each page, the index of the **earliest earlier** page whose pHash is
/// within `max_hamming` bits (and whose dHash is within `dhash_max_hamming`; 64 disables that
/// test). Pages with no structure (all-zero hashes) are never reported as duplicates and are never
/// the target of one. "Earliest" is not "nearest", and the relation is not transitive: with A~B and
/// B~C but A not~C, page C points to B.
pub fn near_duplicates(hashes: &[PageHashes], params: &PhashParams) -> Vec<Option<usize>> {
    hashes
        .iter()
        .enumerate()
        .map(|(i, page)| {
            if page.is_flat() {
                return None;
            }
            hashes[..i].iter().position(|earlier| {
                !earlier.is_flat()
                    && hamming(page.phash, earlier.phash) <= params.max_hamming
                    && hamming(page.dhash, earlier.dhash) <= params.dhash_max_hamming
            })
        })
        .collect()
}

/// [`near_duplicates`] from the hex strings in the reports. `dhashes` may be `None` only when the
/// dHash test is off (`dhash_max_hamming >= 64`).
pub fn near_duplicates_hex(
    phashes: &[&str],
    dhashes: Option<&[&str]>,
    params: &PhashParams,
) -> Result<Vec<Option<usize>>> {
    let dhashes: Vec<u64> = match dhashes {
        Some(d) if d.len() != phashes.len() => {
            return Err(Error::InvalidInput(format!(
                "{} phashes but {} dhashes",
                phashes.len(),
                d.len()
            )));
        }
        Some(d) => d.iter().map(|h| parse_hex(h)).collect::<Result<_>>()?,
        None if params.dhash_max_hamming < 64 => {
            return Err(Error::InvalidInput(
                "phash.dhash_max_hamming is below 64, so dhashes are required".into(),
            ));
        }
        None => vec![0; phashes.len()],
    };
    let hashes: Vec<PageHashes> = phashes
        .iter()
        .zip(dhashes)
        .map(|(p, dhash)| {
            Ok(PageHashes {
                phash: parse_hex(p)?,
                dhash,
            })
        })
        .collect::<Result<_>>()?;
    Ok(near_duplicates(&hashes, params))
}

/// Fill `near_duplicate_of` on a document's page reports, in order. A report without a `phash` is
/// skipped and left `None`; the value written is the earlier report's `index`.
pub fn mark_near_duplicates(reports: &mut [PageReport], params: &PhashParams) -> Result<()> {
    let mut positions = Vec::new();
    let mut hashes = Vec::new();
    for (position, report) in reports.iter().enumerate() {
        let Some(phash) = &report.phash else { continue };
        let dhash = report.dhash.as_deref().ok_or_else(|| {
            Error::InvalidInput(format!("page {} has a phash but no dhash", report.index))
        })?;
        positions.push(position);
        hashes.push(PageHashes::from_hex(phash, dhash)?);
    }
    let found = near_duplicates(&hashes, params);
    let targets: Vec<Option<u32>> = found
        .iter()
        .map(|f| f.map(|j| reports[positions[j]].index))
        .collect();
    for report in reports.iter_mut() {
        report.near_duplicate_of = None;
    }
    for (&position, target) in positions.iter().zip(targets) {
        reports[position].near_duplicate_of = target;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> PhashParams {
        crate::Params::defaults().unwrap().phash
    }

    struct Lcg(u32);
    impl Lcg {
        fn next(&mut self, bound: u32) -> u32 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (self.0 >> 16) % bound
        }
    }

    /// Blocky page with structure at several scales: rectangles of random gray on paper.
    fn blocky_page(seed: u32, width: usize, height: usize) -> Vec<u8> {
        let mut rng = Lcg(seed);
        let mut px = vec![235u8; width * height];
        for _ in 0..40 {
            let (w, h) = (
                1 + rng.next(width as u32 / 3) as usize,
                1 + rng.next(height as u32 / 3) as usize,
            );
            let (x0, y0) = (
                rng.next((width - w) as u32 + 1) as usize,
                rng.next((height - h) as u32 + 1) as usize,
            );
            let v = rng.next(200) as u8;
            for y in y0..y0 + h {
                for x in x0..x0 + w {
                    px[y * width + x] = v;
                }
            }
        }
        px
    }

    fn hash(gray: &[u8], w: usize, h: usize) -> PageHashes {
        hash_page(gray, w, h, &params()).unwrap().0
    }

    #[test]
    fn axis_weights_are_exact_and_cover_every_cell() {
        for (src, out) in [(1, 32), (7, 32), (32, 32), (33, 32), (100, 9), (1000, 8)] {
            let weights = axis_weights(src, out);
            for target in 0..out {
                let total: u64 = weights.iter().filter(|w| w.1 == target).map(|w| w.2).sum();
                assert_eq!(total, src as u64, "src {src} out {out} cell {target}");
            }
            for source in 0..src {
                let total: u64 = weights.iter().filter(|w| w.0 == source).map(|w| w.2).sum();
                assert_eq!(total, out as u64, "src {src} out {out} pixel {source}");
            }
        }
    }

    #[test]
    fn resize_of_a_constant_page_is_that_constant() {
        for (w, h) in [(5, 3), (32, 32), (47, 61)] {
            let gray = vec![173u8; w * h];
            let sums = area_sums(&gray, w, h, GRID, GRID);
            assert!(sums.iter().all(|&s| s == 173 * (w * h) as u64), "{w}x{h}");
        }
    }

    #[test]
    fn resize_matches_a_hand_computed_case() {
        // 3x1 -> 2x1: cell 0 = (2*a + 1*b) / 3, cell 1 = (1*b + 2*c) / 3.
        let sums = area_sums(&[30, 60, 90], 3, 1, 2, 1);
        assert_eq!(sums, [2 * 30 + 60, 60 + 2 * 90]);
    }

    #[test]
    fn dct_matches_a_direct_evaluation() {
        let mut rng = Lcg(9);
        let image: Vec<f64> = (0..GRID * GRID).map(|_| f64::from(rng.next(256))).collect();
        let block = dct_block(&image);
        let c = |k: usize| {
            if k == 0 {
                (1.0 / GRID as f64).sqrt()
            } else {
                (2.0 / GRID as f64).sqrt()
            }
        };
        for u in 0..BLOCK {
            for v in 0..BLOCK {
                let mut direct = 0.0;
                for y in 0..GRID {
                    for x in 0..GRID {
                        let ay = std::f64::consts::PI * ((2 * y + 1) * u) as f64 / 64.0;
                        let ax = std::f64::consts::PI * ((2 * x + 1) * v) as f64 / 64.0;
                        direct += image[y * GRID + x] * math::cos(ay) * math::cos(ax);
                    }
                }
                direct *= c(u) * c(v);
                assert!(
                    (block[u * BLOCK + v] - direct).abs() < 1e-9,
                    "coefficient ({u},{v}): {} vs {direct}",
                    block[u * BLOCK + v]
                );
            }
        }
    }

    #[test]
    fn dc_term_is_the_mean_times_grid() {
        let image = vec![100.0; GRID * GRID];
        assert!((dct_block(&image)[0] - 100.0 * GRID as f64).abs() < 1e-9);
    }

    #[test]
    fn hex_round_trip_and_rejection() {
        assert_eq!(to_hex(0), "0000000000000000");
        assert_eq!(to_hex(0xdead_beef), "00000000deadbeef");
        assert_eq!(parse_hex("00000000deadbeef").unwrap(), 0xdead_beef);
        assert_eq!(parse_hex("00000000DEADBEEF").unwrap(), 0xdead_beef);
        for bad in [
            "",
            "abc",
            "00000000deadbeeg",
            "00000000deadbeef0",
            "+000000000000001",
        ] {
            assert!(parse_hex(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn hamming_counts_bits() {
        assert_eq!(hamming(0, 0), 0);
        assert_eq!(hamming(u64::MAX, 0), 64);
        assert_eq!(hamming(0b1011, 0b0110), 3);
    }

    #[test]
    fn hash_is_deterministic_and_has_a_balanced_phash() {
        let page = blocky_page(1, 120, 150);
        let a = hash(&page, 120, 150);
        assert_eq!(a, hash(&page, 120, 150));
        // 31 AC bits are above the median of 63 distinct values; the DC bit is the 32nd.
        assert!(
            (31..=32).contains(&a.phash.count_ones()),
            "{:016x}",
            a.phash
        );
        assert!(!a.is_flat());
    }

    #[test]
    fn brightness_scaling_and_resolution_barely_move_the_hash() {
        let page = blocky_page(2, 128, 160);
        let base = hash(&page, 128, 160);
        let darker: Vec<u8> = page
            .iter()
            .map(|&v| (u32::from(v) * 9 / 10) as u8)
            .collect();
        assert!(hamming(base.phash, hash(&darker, 128, 160).phash) <= 3);
        // The same page at 2x (pixel replication).
        let mut big = Vec::new();
        for row in page.as_chunks::<128>().0 {
            let wide: Vec<u8> = row.iter().flat_map(|&v| [v, v]).collect();
            big.extend_from_slice(&wide);
            big.extend_from_slice(&wide);
        }
        let doubled = hash(&big, 256, 320);
        assert_eq!(
            base, doubled,
            "pixel replication must not change the hashes"
        );
    }

    #[test]
    fn different_pages_are_far_apart() {
        let mut close = 0;
        for seed in 1..=20 {
            let a = hash(&blocky_page(seed, 100, 130), 100, 130);
            let b = hash(&blocky_page(seed + 100, 100, 130), 100, 130);
            if hamming(a.phash, b.phash) <= 10 {
                close += 1;
            }
        }
        assert!(close <= 1, "{close} of 20 unrelated pairs within 10 bits");
    }

    #[test]
    fn flat_pages_hash_to_zero_even_with_noise() {
        let p = params();
        let flat = vec![240u8; 64 * 80];
        assert_eq!(hash(&flat, 64, 80), PageHashes { phash: 0, dhash: 0 });
        let mut rng = Lcg(5);
        let noisy: Vec<u8> = (0..64 * 80).map(|_| 240 + rng.next(5) as u8 - 2).collect();
        let (h, features) = hash_page(&noisy, 64, 80, &p).unwrap();
        assert!(h.is_flat());
        assert_eq!(features["flat"], 1.0);
        assert!(features["lowfreq_rms"] < p.flat_rms);
        // A page with real structure is not flat.
        assert!(!hash(&blocky_page(3, 64, 80), 64, 80).is_flat());
    }

    #[test]
    fn flat_threshold_is_a_parameter() {
        let mut p = params();
        let page = blocky_page(4, 64, 80);
        p.flat_rms = 1.0e6;
        assert!(hash_page(&page, 64, 80, &p).unwrap().0.is_flat());
        p.flat_rms = 0.0;
        assert!(!hash_page(&page, 64, 80, &p).unwrap().0.is_flat());
    }

    #[test]
    fn dhash_reads_left_to_right_gradients() {
        // Gray ramp: every cell is brighter than the one on its left, so all 64 bits are set.
        let (w, h) = (90, 80);
        let ramp: Vec<u8> = (0..w * h).map(|i| (i % w * 2) as u8).collect();
        assert_eq!(hash(&ramp, w, h).dhash, u64::MAX);
        let reversed: Vec<u8> = ramp.iter().map(|&v| 255 - v).collect();
        assert_eq!(hash(&reversed, w, h).dhash, 0);
    }

    #[test]
    fn bad_buffers_are_rejected() {
        let p = params();
        assert!(hash_page(&[0; 5], 2, 2, &p).is_err());
        assert!(hash_page(&[], 0, 0, &p).is_err());
        assert!(hash_page(&[0; 4], 0, 4, &p).is_err());
    }

    #[test]
    fn a_tiny_page_still_hashes() {
        let (h, _) = hash_page(&[0, 255, 255, 0], 2, 2, &params()).unwrap();
        assert!(!h.is_flat());
        let (one, _) = hash_page(&[7], 1, 1, &params()).unwrap();
        assert!(one.is_flat());
    }

    #[test]
    fn ink_mask_hash_ignores_paper_colour() {
        let (w, h) = (100, 120);
        let mask: Vec<u8> = blocky_page(6, w, h)
            .iter()
            .map(|&v| u8::from(v < 200))
            .collect();
        let p = params();
        let (a, _) = hash_ink_mask(&mask, w, h, &p).unwrap();
        let gray: Vec<u8> = mask.iter().map(|&m| if m == 1 { 0 } else { 255 }).collect();
        assert_eq!(a, hash_page(&gray, w, h, &p).unwrap().0);
    }

    fn pages(phash: &[u64]) -> Vec<PageHashes> {
        phash
            .iter()
            .map(|&p| PageHashes { phash: p, dhash: 0 })
            .collect()
    }

    #[test]
    fn near_duplicates_point_to_the_earliest_earlier_page() {
        let p = params();
        let a = 0xf0f0_f0f0_f0f0_f0f0u64;
        let near_a = a ^ 0b1111; // 4 bits away
        let far = !a;
        let near_a_too = a ^ 0b11_1111_1111; // 10 bits away
        let hashes = pages(&[far, a, near_a, near_a_too, far]);
        assert_eq!(
            near_duplicates(&hashes, &p),
            [None, None, Some(1), Some(1), Some(0)]
        );
        // Eleven bits is outside the default threshold of ten.
        let eleven = pages(&[a, a ^ 0b111_1111_1111]);
        assert_eq!(near_duplicates(&eleven, &p), [None, None]);
    }

    #[test]
    fn flat_pages_never_match() {
        let p = params();
        let hashes = vec![
            PageHashes { phash: 0, dhash: 0 },
            PageHashes { phash: 0, dhash: 0 },
            PageHashes {
                phash: 0xff,
                dhash: 1,
            },
        ];
        assert_eq!(near_duplicates(&hashes, &p), [None, None, None]);
    }

    #[test]
    fn dhash_gate_is_applied_only_when_set() {
        let mut p = params();
        let same_p = [
            PageHashes {
                phash: 0xf0f0,
                dhash: 0,
            },
            PageHashes {
                phash: 0xf0f0,
                dhash: u64::MAX,
            },
        ];
        assert_eq!(near_duplicates(&same_p, &p), [None, Some(0)]);
        p.dhash_max_hamming = 20;
        assert_eq!(near_duplicates(&same_p, &p), [None, None]);
    }

    #[test]
    fn hex_entry_point_checks_its_input() {
        let p = params();
        let a = to_hex(0xf0f0_f0f0_f0f0_f0f0);
        assert_eq!(
            near_duplicates_hex(&[&a, &a], None, &p).unwrap(),
            [None, Some(0)]
        );
        assert!(near_duplicates_hex(&[&a, "zz"], None, &p).is_err());
        assert!(near_duplicates_hex(&[&a, &a], Some(&[&a]), &p).is_err());
        let mut gated = p.clone();
        gated.dhash_max_hamming = 20;
        assert!(near_duplicates_hex(&[&a, &a], None, &gated).is_err());
        assert_eq!(
            near_duplicates_hex(&[&a, &a], Some(&[&a, &a]), &gated).unwrap(),
            [None, Some(0)]
        );
        assert!(near_duplicates_hex(&[], None, &p).unwrap().is_empty());
    }

    fn report(index: u32, phash: Option<&str>) -> PageReport {
        let g = vec![250u8; 4];
        let mut r = crate::analyse_page_report(
            &crate::PageInput {
                gray: &g,
                rgb: None,
                width: 2,
                height: 2,
                dpi: 150,
            },
            None,
            "{}",
        )
        .unwrap();
        r.index = index;
        r.phash = phash.map(str::to_owned);
        r.dhash = phash.map(|_| to_hex(0));
        r.near_duplicate_of = Some(99);
        r
    }

    #[test]
    fn mark_near_duplicates_writes_the_earlier_pages_index() {
        let a = to_hex(0xf0f0_f0f0_f0f0_f0f0);
        let b = to_hex(!0xf0f0_f0f0_f0f0_f0f0u64);
        let mut reports = vec![
            report(10, Some(&a)),
            report(11, None),
            report(12, Some(&b)),
            report(13, Some(&a)),
        ];
        mark_near_duplicates(&mut reports, &params()).unwrap();
        let marks: Vec<Option<u32>> = reports.iter().map(|r| r.near_duplicate_of).collect();
        assert_eq!(marks, [None, None, None, Some(10)]);
    }

    #[test]
    fn mark_near_duplicates_needs_a_dhash_beside_every_phash() {
        let a = to_hex(1);
        let mut r = report(0, Some(&a));
        r.dhash = None;
        assert!(mark_near_duplicates(&mut [r], &params()).is_err());
    }
}
