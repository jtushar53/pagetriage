//! QR decoding (module M11): from three finder centres and the binary page to the code's text.
//! See `docs/detectors/qr.md`.
//!
//! Integer decoding apart from the sampling positions (IEEE `+ - * /` only): the module grid is
//! sampled with an affine map fixed by the three finder centres, the format information is matched
//! against the 32 valid words, the data modules are unmasked and de-interleaved, each Reed-Solomon
//! block is corrected over GF(256), and the segments (numeric, alphanumeric, byte) are read. No
//! perspective or alignment-pattern correction, no kanji or structured append.

use super::qr_tables::{ALIGNMENT, BLOCKS};

/// What was read from one code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub version: usize,
    /// `L`, `M`, `Q` or `H`.
    pub ec_level: char,
    pub mask: u8,
    /// Codeword errors the Reed-Solomon step repaired.
    pub corrected: usize,
    pub bytes: Vec<u8>,
}

/// Where the three finder centres are, in pixels, and the module size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub top_left: (f64, f64),
    pub top_right: (f64, f64),
    pub bottom_left: (f64, f64),
}

// ---- GF(256), primitive polynomial 0x11D ------------------------------------------------------

struct Gf {
    exp: [u8; 512],
    log: [u8; 256],
}

impl Gf {
    fn new() -> Self {
        let (mut exp, mut log) = ([0u8; 512], [0u8; 256]);
        let mut x: u16 = 1;
        for (i, e) in exp.iter_mut().enumerate().take(255) {
            *e = x as u8;
            log[x as usize] = i as u8;
            x <<= 1;
            if x & 0x100 != 0 {
                x ^= 0x11D;
            }
        }
        for i in 255..512 {
            exp[i] = exp[i - 255];
        }
        Gf { exp, log }
    }

    fn mul(&self, a: u8, b: u8) -> u8 {
        if a == 0 || b == 0 {
            0
        } else {
            self.exp[usize::from(self.log[usize::from(a)]) + usize::from(self.log[usize::from(b)])]
        }
    }

    fn div(&self, a: u8, b: u8) -> u8 {
        debug_assert!(b != 0);
        if a == 0 {
            0
        } else {
            self.exp[usize::from(self.log[usize::from(a)]) + 255
                - usize::from(self.log[usize::from(b)])]
        }
    }

    fn pow_alpha(&self, e: usize) -> u8 {
        self.exp[e % 255]
    }
}

/// Correct `block` (data then error-correction codewords, `nsym` of the latter) in place. Returns the
/// number of codewords repaired, or `None` when there are more errors than can be corrected.
fn rs_correct(gf: &Gf, block: &mut [u8], nsym: usize) -> Option<usize> {
    let n = block.len();
    let mut syn = vec![0u8; nsym];
    for (j, s) in syn.iter_mut().enumerate() {
        let mut acc = 0u8;
        for &c in block.iter() {
            acc = gf.mul(acc, gf.pow_alpha(j)) ^ c;
        }
        *s = acc;
    }
    if syn.iter().all(|&s| s == 0) {
        return Some(0);
    }
    // Berlekamp-Massey: the error locator `c` (c[0] = 1), of degree `l`.
    let mut c = vec![1u8];
    let mut b = vec![1u8];
    let (mut l, mut m, mut bb) = (0usize, 1usize, 1u8);
    for i in 0..nsym {
        let mut d = syn[i];
        for k in 1..=l.min(c.len() - 1) {
            d ^= gf.mul(c[k], syn[i - k]);
        }
        if d == 0 {
            m += 1;
            continue;
        }
        let coef = gf.div(d, bb);
        let t = c.clone();
        if c.len() < b.len() + m {
            c.resize(b.len() + m, 0);
        }
        for (k, &bk) in b.iter().enumerate() {
            c[k + m] ^= gf.mul(coef, bk);
        }
        if 2 * l <= i {
            l = i + 1 - l;
            b = t;
            bb = d;
            m = 1;
        } else {
            m += 1;
        }
    }
    while c.len() > 1 && *c.last().expect("non-empty") == 0 {
        c.pop();
    }
    if l == 0 || c.len() - 1 != l || 2 * l > nsym {
        return None;
    }
    // Chien search: position p (power of x) is in error when c(alpha^-p) = 0.
    let mut positions = Vec::new();
    for p in 0..n {
        let inv = gf.pow_alpha(255 - (p % 255));
        let mut acc = 0u8;
        let mut xp = 1u8;
        for &ck in &c {
            acc ^= gf.mul(ck, xp);
            xp = gf.mul(xp, inv);
        }
        if acc == 0 {
            positions.push(p);
        }
    }
    if positions.len() != l {
        return None;
    }
    // Error evaluator omega = (S * c) mod x^nsym.
    let mut omega = vec![0u8; nsym];
    for (i, o) in omega.iter_mut().enumerate() {
        for k in 0..=i.min(c.len() - 1) {
            *o ^= gf.mul(c[k], syn[i - k]);
        }
    }
    for &p in &positions {
        let x = gf.pow_alpha(p);
        let inv = gf.pow_alpha(255 - (p % 255));
        let mut num = 0u8;
        let mut xp = 1u8;
        for &o in &omega {
            num ^= gf.mul(o, xp);
            xp = gf.mul(xp, inv);
        }
        // Formal derivative of the locator at x = inv: odd-degree terms only, c[k] * inv^(k - 1).
        let mut den = 0u8;
        let inv2 = gf.mul(inv, inv);
        let mut xp = 1u8;
        for k in (1..c.len()).step_by(2) {
            den ^= gf.mul(c[k], xp);
            xp = gf.mul(xp, inv2);
        }
        if den == 0 {
            return None;
        }
        let e = gf.mul(x, gf.div(num, den));
        block[n - 1 - p] ^= e;
    }
    // Verify.
    for j in 0..nsym {
        let mut acc = 0u8;
        for &cw in block.iter() {
            acc = gf.mul(acc, gf.pow_alpha(j)) ^ cw;
        }
        if acc != 0 {
            return None;
        }
    }
    Some(positions.len())
}

// ---- format information -----------------------------------------------------------------------

/// The 15-bit format word (BCH(15,5), masked with 0x5412) for 5 data bits.
fn format_word(data: u16) -> u16 {
    let mut v = data << 10;
    for i in (10..15).rev() {
        if v & (1 << i) != 0 {
            v ^= 0x537 << (i - 10);
        }
    }
    ((data << 10) | v) ^ 0x5412
}

/// Closest valid format word within 3 bit errors: (error-correction level, mask).
fn read_format(bits: &[u16]) -> Option<(char, u8)> {
    let mut best: Option<(u32, u16)> = None;
    for data in 0..32u16 {
        let word = format_word(data);
        for &b in bits {
            let d = (word ^ b).count_ones();
            if best.is_none_or(|x| d < x.0) {
                best = Some((d, data));
            }
        }
    }
    let (d, data) = best?;
    if d > 3 {
        return None;
    }
    let level = match data >> 3 {
        0b01 => 'L',
        0b00 => 'M',
        0b11 => 'Q',
        _ => 'H',
    };
    Some((level, (data & 7) as u8))
}

fn level_index(level: char) -> usize {
    match level {
        'L' => 0,
        'M' => 1,
        'Q' => 2,
        _ => 3,
    }
}

// ---- module grid ------------------------------------------------------------------------------

/// `n x n` dark/light modules sampled from a 0/1 mask (1 dark) with the affine map fixed by the three
/// finder centres. Each module is the majority of the pixels within `radius` of its centre.
fn sample(mask: &[u8], w: usize, h: usize, frame: &Frame, n: usize, radius: i64) -> Vec<bool> {
    let steps = (n - 7) as f64;
    let ex = (
        (frame.top_right.0 - frame.top_left.0) / steps,
        (frame.top_right.1 - frame.top_left.1) / steps,
    );
    let ey = (
        (frame.bottom_left.0 - frame.top_left.0) / steps,
        (frame.bottom_left.1 - frame.top_left.1) / steps,
    );
    let mut out = vec![false; n * n];
    for j in 0..n {
        for i in 0..n {
            let (di, dj) = (i as f64 - 3.0, j as f64 - 3.0);
            let cx = frame.top_left.0 + di * ex.0 + dj * ey.0;
            let cy = frame.top_left.1 + di * ex.1 + dj * ey.1;
            let (mut dark, mut total) = (0u32, 0u32);
            for oy in -radius..=radius {
                for ox in -radius..=radius {
                    let (px, py) = (
                        (cx + ox as f64).round() as i64,
                        (cy + oy as f64).round() as i64,
                    );
                    if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                        total += 1;
                        dark += u32::from(mask[py as usize * w + px as usize]);
                    }
                }
            }
            out[j * n + i] = total > 0 && dark * 2 > total;
        }
    }
    out
}

/// Modules that are not data: finders and separators, timing, alignment, format and version areas.
fn function_map(n: usize, version: usize) -> Vec<bool> {
    let mut f = vec![false; n * n];
    let mut set = |x: usize, y: usize| f[y * n + x] = true;
    for y in 0..9 {
        for x in 0..9 {
            set(x, y);
        }
    }
    for y in 0..9 {
        for x in n - 8..n {
            set(x, y);
        }
    }
    for y in n - 8..n {
        for x in 0..9 {
            set(x, y);
        }
    }
    for i in 0..n {
        set(6, i);
        set(i, 6);
    }
    let centres = ALIGNMENT[version - 1];
    for (a, &cy) in centres.iter().enumerate() {
        for (b, &cx) in centres.iter().enumerate() {
            let last = centres.len() - 1;
            // The three positions that would overlap a finder are skipped.
            if (a == 0 && b == 0) || (a == 0 && b == last) || (a == last && b == 0) {
                continue;
            }
            for dy in 0..5usize {
                for dx in 0..5usize {
                    set(usize::from(cx) + dx - 2, usize::from(cy) + dy - 2);
                }
            }
        }
    }
    if version >= 7 {
        for i in 0..6 {
            for j in 0..3 {
                set(n - 11 + j, i);
                set(i, n - 11 + j);
            }
        }
    }
    f
}

fn masked(mask: u8, row: usize, col: usize) -> bool {
    let (i, j) = (row, col);
    match mask {
        0 => (i + j) % 2 == 0,
        1 => i % 2 == 0,
        2 => j % 3 == 0,
        3 => (i + j) % 3 == 0,
        4 => (i / 2 + j / 3) % 2 == 0,
        5 => (i * j) % 2 + (i * j) % 3 == 0,
        6 => ((i * j) % 2 + (i * j) % 3) % 2 == 0,
        _ => ((i + j) % 2 + (i * j) % 3) % 2 == 0,
    }
}

/// Decode a code. `mask` is the page's binary mask (1 dark), `w x h`; `modules` is the side of the
/// code in modules (from the finder spacing, snapped to 17 + 4 * version).
pub fn decode(mask: &[u8], w: usize, h: usize, frame: &Frame, modules: usize) -> Option<Decoded> {
    if !(21..=177).contains(&modules) || !(modules - 17).is_multiple_of(4) || mask.len() != w * h {
        return None;
    }
    let n = modules;
    // Small modules are read at their centre pixel, larger ones as the majority of 3 x 3; the other
    // reading is the fallback.
    let steps = (n - 7) as f64;
    let (lx, ly) = (
        (frame.top_right.0 - frame.top_left.0) / steps,
        (frame.top_right.1 - frame.top_left.1) / steps,
    );
    let module_px = (lx * lx + ly * ly).sqrt();
    let first = i64::from(module_px >= 3.5);
    [first, 1 - first]
        .into_iter()
        .find_map(|r| decode_grid(&sample(mask, w, h, frame, n, r), n))
}

fn decode_grid(grid: &[bool], n: usize) -> Option<Decoded> {
    let version = (n - 17) / 4;
    let at = |x: usize, y: usize| u16::from(grid[y * n + x]);
    // Format words: both copies, read as in the specification.
    let mut copy1 = 0u16;
    for x in 0..6 {
        copy1 = (copy1 << 1) | at(x, 8);
    }
    for (x, y) in [(7, 8), (8, 8), (8, 7)] {
        copy1 = (copy1 << 1) | at(x, y);
    }
    for y in (0..6).rev() {
        copy1 = (copy1 << 1) | at(8, y);
    }
    let mut copy2 = 0u16;
    for y in (n - 7..n).rev() {
        copy2 = (copy2 << 1) | at(8, y);
    }
    for x in n - 8..n {
        copy2 = (copy2 << 1) | at(x, 8);
    }
    let (level, mask_pattern) = read_format(&[copy1, copy2])?;

    // Data bits in the zigzag order, unmasked.
    let func = function_map(n, version);
    let mut bits: Vec<bool> = Vec::with_capacity(n * n);
    let mut x = n as i64 - 1;
    let mut upward = true;
    while x > 0 {
        if x == 6 {
            x -= 1;
        }
        for k in 0..n {
            let y = if upward { n - 1 - k } else { k };
            for dx in 0..2i64 {
                let cx = (x - dx) as usize;
                if !func[y * n + cx] {
                    bits.push(grid[y * n + cx] ^ masked(mask_pattern, y, cx));
                }
            }
        }
        upward = !upward;
        x -= 2;
    }
    let spec = BLOCKS[version - 1][level_index(level)];
    let (g1, t1, d1, g2, t2, d2) = (
        usize::from(spec[0]),
        usize::from(spec[1]),
        usize::from(spec[2]),
        usize::from(spec[3]),
        usize::from(spec[4]),
        usize::from(spec[5]),
    );
    let total = g1 * t1 + g2 * t2;
    if bits.len() < total * 8 {
        return None;
    }
    let codewords: Vec<u8> = bits[..total * 8]
        .as_chunks::<8>()
        .0
        .iter()
        .map(|c| c.iter().fold(0u8, |a, &b| (a << 1) | u8::from(b)))
        .collect();

    // De-interleave: data codewords round-robin over the blocks, then the error-correction ones.
    let blocks: Vec<(usize, usize)> = (0..g1)
        .map(|_| (t1, d1))
        .chain((0..g2).map(|_| (t2, d2)))
        .collect();
    let mut data: Vec<Vec<u8>> = blocks.iter().map(|&(_, d)| Vec::with_capacity(d)).collect();
    let mut ecc: Vec<Vec<u8>> = blocks
        .iter()
        .map(|&(t, d)| Vec::with_capacity(t - d))
        .collect();
    let mut pos = 0;
    let max_data = blocks.iter().map(|b| b.1).max().unwrap_or(0);
    for i in 0..max_data {
        for (k, &(_, d)) in blocks.iter().enumerate() {
            if i < d {
                data[k].push(codewords[pos]);
                pos += 1;
            }
        }
    }
    let max_ec = blocks.iter().map(|b| b.0 - b.1).max().unwrap_or(0);
    for i in 0..max_ec {
        for (k, &(t, d)) in blocks.iter().enumerate() {
            if i < t - d {
                ecc[k].push(codewords[pos]);
                pos += 1;
            }
        }
    }
    let gf = Gf::new();
    let mut corrected = 0;
    let mut payload: Vec<u8> = Vec::new();
    for (k, &(t, d)) in blocks.iter().enumerate() {
        let mut block: Vec<u8> = data[k].iter().chain(ecc[k].iter()).copied().collect();
        corrected += rs_correct(&gf, &mut block, t - d)?;
        payload.extend_from_slice(&block[..d]);
    }
    let bytes = read_segments(&payload, version)?;
    Some(Decoded {
        version,
        ec_level: level,
        mask: mask_pattern,
        corrected,
        bytes,
    })
}

// ---- segments ---------------------------------------------------------------------------------

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn left(&self) -> usize {
        self.data.len() * 8 - self.pos
    }

    fn take(&mut self, n: usize) -> Option<u32> {
        if n > self.left() {
            return None;
        }
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | u32::from((self.data[self.pos / 8] >> (7 - self.pos % 8)) & 1);
            self.pos += 1;
        }
        Some(v)
    }
}

const ALNUM: &[u8; 45] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";

/// Numeric, alphanumeric and byte segments; ECI headers are skipped, kanji and anything else ends
/// the read with `None`. The bytes of byte segments are as stored (UTF-8 or Latin-1: the caller decides).
fn read_segments(data: &[u8], version: usize) -> Option<Vec<u8>> {
    let mut bits = Bits { data, pos: 0 };
    let mut out = Vec::new();
    let size_class = if version <= 9 {
        0
    } else if version <= 26 {
        1
    } else {
        2
    };
    while bits.left() >= 4 {
        let mode = bits.take(4)?;
        match mode {
            0 => break,
            7 => {
                // ECI designator: 1 to 3 bytes, skipped.
                let first = bits.take(8)?;
                let extra = if first & 0x80 == 0 {
                    0
                } else if first & 0xC0 == 0x80 {
                    1
                } else {
                    2
                };
                bits.take(8 * extra)?;
            }
            1 => {
                let count = bits.take([10, 12, 14][size_class])? as usize;
                let mut left = count;
                while left >= 3 {
                    let v = bits.take(10)?;
                    if v > 999 {
                        return None;
                    }
                    out.extend_from_slice(format!("{v:03}").as_bytes());
                    left -= 3;
                }
                if left == 2 {
                    let v = bits.take(7)?;
                    if v > 99 {
                        return None;
                    }
                    out.extend_from_slice(format!("{v:02}").as_bytes());
                } else if left == 1 {
                    let v = bits.take(4)?;
                    if v > 9 {
                        return None;
                    }
                    out.push(b'0' + v as u8);
                }
            }
            2 => {
                let count = bits.take([9, 11, 13][size_class])? as usize;
                let mut left = count;
                while left >= 2 {
                    let v = bits.take(11)? as usize;
                    if v >= 45 * 45 {
                        return None;
                    }
                    out.push(ALNUM[v / 45]);
                    out.push(ALNUM[v % 45]);
                    left -= 2;
                }
                if left == 1 {
                    let v = bits.take(6)? as usize;
                    if v >= 45 {
                        return None;
                    }
                    out.push(ALNUM[v]);
                }
            }
            4 => {
                let count = bits.take([8, 16, 16][size_class])? as usize;
                for _ in 0..count {
                    out.push(bits.take(8)? as u8);
                }
            }
            _ => return None,
        }
    }
    Some(out)
}
