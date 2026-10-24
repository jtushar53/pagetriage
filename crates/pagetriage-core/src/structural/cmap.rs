//! `ToUnicode` CMap parser (module M2). Bounded and tolerant: a malformed CMap yields whatever
//! entries parsed before the fault, never a panic.
//!
//! Only the parts a `ToUnicode` CMap uses are read: `begincodespacerange`, `beginbfchar` and
//! `beginbfrange`. Everything else (CMap names, `usecmap`, CID ranges) is skipped.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodeSpace {
    width: u8,
    lo: u32,
    hi: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RangeDst {
    /// Destination of `lo`; code `c` maps to this with the last code point increased by `c - lo`.
    Start(Vec<u32>),
    /// One destination per code in the range.
    List(Vec<Vec<u32>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Range {
    lo: u32,
    hi: u32,
    dst: RangeDst,
}

/// A parsed `ToUnicode` CMap.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CMap {
    codespaces: Vec<CodeSpace>,
    chars: BTreeMap<u32, Vec<u32>>,
    ranges: Vec<Range>,
    /// True if parsing stopped at the entry limit.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok<'a> {
    Hex(Vec<u8>),
    Word(&'a [u8]),
    ArrayOpen,
    ArrayClose,
}

struct Lexer<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Lexer<'a> {
    fn next(&mut self) -> Option<Tok<'a>> {
        loop {
            let c = *self.b.get(self.i)?;
            match c {
                b'%' => {
                    while self.i < self.b.len() && !matches!(self.b[self.i], b'\n' | b'\r') {
                        self.i += 1;
                    }
                }
                c if c.is_ascii_whitespace() || c == 0 => self.i += 1,
                b'[' => {
                    self.i += 1;
                    return Some(Tok::ArrayOpen);
                }
                b']' => {
                    self.i += 1;
                    return Some(Tok::ArrayClose);
                }
                b'<' if self.b.get(self.i + 1) == Some(&b'<') => {
                    self.i += 2;
                    return Some(Tok::Word(b"<<"));
                }
                b'>' if self.b.get(self.i + 1) == Some(&b'>') => {
                    self.i += 2;
                    return Some(Tok::Word(b">>"));
                }
                b'<' => {
                    self.i += 1;
                    let mut digits: Vec<u8> = Vec::new();
                    while let Some(&d) = self.b.get(self.i) {
                        self.i += 1;
                        if d == b'>' {
                            break;
                        }
                        if let Some(v) = hex_val(d) {
                            digits.push(v);
                        }
                    }
                    if digits.len() % 2 == 1 {
                        digits.push(0);
                    }
                    return Some(Tok::Hex(
                        digits.chunks(2).map(|p| p[0] * 16 + p[1]).collect(),
                    ));
                }
                b'(' => {
                    // A literal string: skip it, honouring nesting and escapes.
                    let mut depth = 0usize;
                    while let Some(&d) = self.b.get(self.i) {
                        self.i += 1;
                        match d {
                            b'\\' => self.i += 1,
                            b'(' => depth += 1,
                            b')' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    return Some(Tok::Word(b"()"));
                }
                _ => {
                    let start = self.i;
                    while let Some(&d) = self.b.get(self.i) {
                        if d.is_ascii_whitespace()
                            || matches!(d, b'<' | b'>' | b'[' | b']' | b'(' | b'%')
                        {
                            break;
                        }
                        self.i += 1;
                    }
                    if self.i == start {
                        self.i += 1; // a stray delimiter such as `)`
                        continue;
                    }
                    return Some(Tok::Word(&self.b[start..self.i]));
                }
            }
        }
    }
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn be_value(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .take(4)
        .fold(0u32, |acc, b| (acc << 8) | u32::from(*b))
}

/// UTF-16BE bytes to code points. Unpaired surrogates become U+FFFD.
fn utf16be(bytes: &[u8]) -> Vec<u32> {
    let units: Vec<u16> = bytes
        .chunks(2)
        .map(|p| {
            if p.len() == 2 {
                u16::from_be_bytes([p[0], p[1]])
            } else {
                u16::from(p[0])
            }
        })
        .collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < units.len() {
        let u = u32::from(units[i]);
        if (0xD800..0xDC00).contains(&u) {
            if let Some(&lo) = units.get(i + 1) {
                let lo = u32::from(lo);
                if (0xDC00..0xE000).contains(&lo) {
                    out.push(0x10000 + ((u - 0xD800) << 10) + (lo - 0xDC00));
                    i += 2;
                    continue;
                }
            }
            out.push(0xFFFD);
        } else if (0xDC00..0xE000).contains(&u) {
            out.push(0xFFFD);
        } else {
            out.push(u);
        }
        i += 1;
    }
    out
}

impl CMap {
    /// Parse a CMap, keeping at most `max_entries` char and range entries together.
    pub fn parse(data: &[u8], max_entries: usize) -> CMap {
        let mut cmap = CMap::default();
        let mut lx = Lexer { b: data, i: 0 };
        let mut entries = 0usize;
        while let Some(tok) = lx.next() {
            let Tok::Word(w) = tok else { continue };
            match w {
                b"begincodespacerange" => {
                    while let Some(t1) = lx.next() {
                        let Tok::Hex(lo) = t1 else { break };
                        let Some(Tok::Hex(hi)) = lx.next() else { break };
                        let width = lo.len().min(hi.len()).min(4) as u8;
                        if width > 0 && cmap.codespaces.len() < 64 {
                            cmap.codespaces.push(CodeSpace {
                                width,
                                lo: be_value(&lo),
                                hi: be_value(&hi),
                            });
                        }
                    }
                }
                b"beginbfchar" => {
                    while let Some(t1) = lx.next() {
                        let Tok::Hex(src) = t1 else { break };
                        let Some(Tok::Hex(dst)) = lx.next() else {
                            break;
                        };
                        if entries >= max_entries {
                            cmap.truncated = true;
                            break;
                        }
                        entries += 1;
                        cmap.chars.insert(be_value(&src), utf16be(&dst));
                    }
                }
                b"beginbfrange" => {
                    while let Some(t1) = lx.next() {
                        let Tok::Hex(lo) = t1 else { break };
                        let Some(Tok::Hex(hi)) = lx.next() else { break };
                        let (lo, hi) = (be_value(&lo), be_value(&hi));
                        let dst = match lx.next() {
                            Some(Tok::Hex(d)) => RangeDst::Start(utf16be(&d)),
                            Some(Tok::ArrayOpen) => {
                                let mut list = Vec::new();
                                while let Some(t) = lx.next() {
                                    match t {
                                        Tok::Hex(d) => {
                                            if list.len() < 65_536 {
                                                list.push(utf16be(&d));
                                            }
                                        }
                                        Tok::ArrayClose => break,
                                        _ => {}
                                    }
                                }
                                RangeDst::List(list)
                            }
                            _ => break,
                        };
                        if entries >= max_entries {
                            cmap.truncated = true;
                            break;
                        }
                        entries += 1;
                        if hi >= lo {
                            cmap.ranges.push(Range { lo, hi, dst });
                        }
                    }
                }
                _ => {}
            }
            if cmap.truncated {
                break;
            }
        }
        cmap.ranges.sort_by_key(|r| (r.lo, r.hi));
        cmap
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty() && self.ranges.is_empty()
    }

    pub fn entry_count(&self) -> usize {
        self.chars.len() + self.ranges.len()
    }

    /// Code width in bytes when every codespace range has the same width.
    pub fn uniform_width(&self) -> Option<u8> {
        let first = self.codespaces.first()?.width;
        self.codespaces
            .iter()
            .all(|c| c.width == first)
            .then_some(first)
    }

    /// Look up `code`; on a hit the Unicode code points are appended to `out`.
    pub fn lookup(&self, code: u32, out: &mut Vec<u32>) -> bool {
        if let Some(v) = self.chars.get(&code) {
            out.extend_from_slice(v);
            return true;
        }
        // Ranges are sorted by `lo`; earlier-starting ranges may still contain `code`, so scan
        // back from the partition point (bounded: overlapping ranges are not expected).
        let upper = self.ranges.partition_point(|r| r.lo <= code);
        for r in self.ranges[..upper].iter().rev().take(16) {
            if code <= r.hi {
                match &r.dst {
                    RangeDst::Start(v) => {
                        let mut v = v.clone();
                        if let Some(last) = v.last_mut() {
                            *last = last.wrapping_add(code - r.lo);
                        }
                        out.extend_from_slice(&v);
                        return true;
                    }
                    RangeDst::List(list) => {
                        if let Some(v) = list.get((code - r.lo) as usize) {
                            out.extend_from_slice(v);
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Split `bytes` into character codes using the codespace ranges (or `default_width` bytes
    /// each when the CMap declares none) and call `f(code, width)` for each.
    pub fn for_each_code(&self, bytes: &[u8], default_width: u8, mut f: impl FnMut(u32, u8)) {
        let default_width = default_width.clamp(1, 4) as usize;
        let mut i = 0;
        while i < bytes.len() {
            let mut taken = 0usize;
            if self.codespaces.is_empty() {
                taken = default_width.min(bytes.len() - i);
            } else {
                for w in 1..=4usize {
                    if i + w > bytes.len() {
                        break;
                    }
                    let code = be_value(&bytes[i..i + w]);
                    if self
                        .codespaces
                        .iter()
                        .any(|c| usize::from(c.width) == w && in_range(code, c, w))
                    {
                        taken = w;
                        break;
                    }
                }
                if taken == 0 {
                    taken = 1;
                }
            }
            f(be_value(&bytes[i..i + taken]), taken as u8);
            i += taken;
        }
    }
}

fn in_range(code: u32, c: &CodeSpace, w: usize) -> bool {
    (0..w).all(|k| {
        let shift = 8 * (w - 1 - k) as u32;
        let b = (code >> shift) & 0xFF;
        let lo = (c.lo >> shift) & 0xFF;
        let hi = (c.hi >> shift) & 0xFF;
        (lo..=hi).contains(&b)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
        /CMapName /Adobe-Identity-UCS def /CMapType 2 def\n\
        1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
        2 beginbfchar <0003> <0020> <0024> <0915> endbfchar\n\
        2 beginbfrange <0030> <0033> <0930> <0040> <0041> [<0958> <D83DDE00>] endbfrange\n\
        endcmap";

    fn decode(c: &CMap, code: u32) -> Option<Vec<u32>> {
        let mut out = Vec::new();
        c.lookup(code, &mut out).then_some(out)
    }

    #[test]
    fn bfchar_bfrange_and_array_destinations() {
        let c = CMap::parse(SAMPLE, 1000);
        assert_eq!(decode(&c, 0x24), Some(vec![0x0915]));
        assert_eq!(decode(&c, 0x03), Some(vec![0x20]));
        assert_eq!(decode(&c, 0x30), Some(vec![0x0930]));
        assert_eq!(decode(&c, 0x32), Some(vec![0x0932]));
        assert_eq!(decode(&c, 0x34), None);
        assert_eq!(decode(&c, 0x40), Some(vec![0x0958]));
        assert_eq!(decode(&c, 0x41), Some(vec![0x1F600])); // surrogate pair
        assert_eq!(c.uniform_width(), Some(2));
        assert!(!c.truncated);
    }

    #[test]
    fn codes_are_split_by_codespace_width() {
        let c = CMap::parse(SAMPLE, 1000);
        let mut seen = Vec::new();
        c.for_each_code(&[0x00, 0x24, 0x00, 0x03, 0x00], 2, |code, w| {
            seen.push((code, w))
        });
        // Two 2-byte codes, then a dangling byte consumed as a 1-byte code.
        assert_eq!(seen, vec![(0x24, 2), (0x03, 2), (0x00, 1)]);
    }

    #[test]
    fn mixed_width_codespaces() {
        let c = CMap::parse(
            b"2 begincodespacerange <00> <7F> <8140> <9FFC> endcodespacerange",
            10,
        );
        let mut seen = Vec::new();
        c.for_each_code(&[0x41, 0x81, 0x40, 0x42], 1, |code, w| seen.push((code, w)));
        assert_eq!(seen, vec![(0x41, 1), (0x8140, 2), (0x42, 1)]);
    }

    #[test]
    fn entry_limit_truncates() {
        let mut src =
            String::from("1 begincodespacerange <00> <FF> endcodespacerange\nbeginbfchar\n");
        for i in 0..100 {
            src.push_str(&format!("<{i:02X}> <{:04X}>\n", 0x41 + i));
        }
        src.push_str("endbfchar");
        let c = CMap::parse(src.as_bytes(), 10);
        assert!(c.truncated);
        assert_eq!(c.entry_count(), 10);
    }

    #[test]
    fn garbage_does_not_panic() {
        for input in [
            &b""[..],
            b"beginbfchar <",
            b"beginbfrange <00> <10> [",
            b"beginbfrange <10> <00> <0041> endbfrange",
            b"(((((",
            b"\xff\xfe\x00\x01 begincodespacerange <00",
            b"beginbfchar <0041> <D800> endbfchar",
        ] {
            let c = CMap::parse(input, 100);
            let mut out = Vec::new();
            let _ = c.lookup(0x41, &mut out);
            c.for_each_code(b"abc", 1, |_, _| {});
        }
    }
}
