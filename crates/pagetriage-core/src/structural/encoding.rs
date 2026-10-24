//! Simple-font (single-byte) encodings and glyph-name decoding for module M2.
//!
//! These tables exist to put a script label on text that has no `ToUnicode` map, and to give the
//! legacy-font statistics something Latin to look at. They are not a full PDF text extractor:
//! `StandardEncoding` is approximated by ASCII plus a handful of letters, and the MacRoman table
//! below was written from memory of the Apple table and has not been checked against a reference.

use super::unicode::Script;

/// What one character code decodes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decoded {
    /// A known Unicode code point.
    Cp(u32),
    /// A letter of a known script whose exact code point is not known (glyph-name suffix).
    ScriptLetter(Script),
    /// Nothing usable (no map, `.notdef`, unrecognised glyph name).
    Unknown,
}

/// Single-byte base encodings named by a font's `/Encoding`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseEncoding {
    WinAnsi,
    MacRoman,
    Standard,
    /// `Symbol` / `ZapfDingbats` built-in encodings: symbols, never script letters.
    Symbolic,
}

impl BaseEncoding {
    pub fn from_name(name: &[u8]) -> Option<BaseEncoding> {
        match name {
            b"WinAnsiEncoding" => Some(BaseEncoding::WinAnsi),
            b"MacRomanEncoding" => Some(BaseEncoding::MacRoman),
            b"StandardEncoding" => Some(BaseEncoding::Standard),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            BaseEncoding::WinAnsi => "WinAnsiEncoding",
            BaseEncoding::MacRoman => "MacRomanEncoding",
            BaseEncoding::Standard => "StandardEncoding",
            BaseEncoding::Symbolic => "Symbolic",
        }
    }

    pub fn decode(self, b: u8) -> Decoded {
        match self {
            BaseEncoding::WinAnsi => Decoded::Cp(winansi(b)),
            BaseEncoding::MacRoman => Decoded::Cp(macroman(b)),
            BaseEncoding::Standard => Decoded::Cp(standard(b)),
            BaseEncoding::Symbolic => Decoded::Cp(0x2022),
        }
    }
}

/// Windows-1252 (PDF `WinAnsiEncoding`). Undefined codes map to the bullet, as the PDF spec says.
pub fn winansi(b: u8) -> u32 {
    const HIGH: [u16; 32] = [
        0x20AC, 0x2022, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160,
        0x2039, 0x0152, 0x2022, 0x017D, 0x2022, 0x2022, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022,
        0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x2022, 0x017E, 0x0178,
    ];
    match b {
        0x80..=0x9F => u32::from(HIGH[usize::from(b - 0x80)]),
        _ => u32::from(b),
    }
}

/// Apple MacRoman, high half (0x80-0xFF). Recalled from memory; unverified.
pub fn macroman(b: u8) -> u32 {
    const HIGH: [u16; 128] = [
        0x00C4, 0x00C5, 0x00C7, 0x00C9, 0x00D1, 0x00D6, 0x00DC, 0x00E1, 0x00E0, 0x00E2, 0x00E4,
        0x00E3, 0x00E5, 0x00E7, 0x00E9, 0x00E8, 0x00EA, 0x00EB, 0x00ED, 0x00EC, 0x00EE, 0x00EF,
        0x00F1, 0x00F3, 0x00F2, 0x00F4, 0x00F6, 0x00F5, 0x00FA, 0x00F9, 0x00FB, 0x00FC, 0x2020,
        0x00B0, 0x00A2, 0x00A3, 0x00A7, 0x2022, 0x00B6, 0x00DF, 0x00AE, 0x00A9, 0x2122, 0x00B4,
        0x00A8, 0x2260, 0x00C6, 0x00D8, 0x221E, 0x00B1, 0x2264, 0x2265, 0x00A5, 0x00B5, 0x2202,
        0x2211, 0x220F, 0x03C0, 0x222B, 0x00AA, 0x00BA, 0x03A9, 0x00E6, 0x00F8, 0x00BF, 0x00A1,
        0x00AC, 0x221A, 0x0192, 0x2248, 0x2206, 0x00AB, 0x00BB, 0x2026, 0x00A0, 0x00C0, 0x00C3,
        0x00D5, 0x0152, 0x0153, 0x2013, 0x2014, 0x201C, 0x201D, 0x2018, 0x2019, 0x00F7, 0x25CA,
        0x00FF, 0x0178, 0x2044, 0x20AC, 0x2039, 0x203A, 0xFB01, 0xFB02, 0x2021, 0x00B7, 0x201A,
        0x201E, 0x2030, 0x00C2, 0x00CA, 0x00C1, 0x00CB, 0x00C8, 0x00CD, 0x00CE, 0x00CF, 0x00CC,
        0x00D3, 0x00D4, 0xF8FF, 0x00D2, 0x00DA, 0x00DB, 0x00D9, 0x0131, 0x02C6, 0x02DC, 0x00AF,
        0x02D8, 0x02D9, 0x02DA, 0x00B8, 0x02DD, 0x02DB, 0x02C7,
    ];
    if b < 0x80 {
        u32::from(b)
    } else {
        u32::from(HIGH[usize::from(b - 0x80)])
    }
}

/// `StandardEncoding`, approximated: ASCII, the few Latin letters of its high half, and Latin-1
/// elsewhere (punctuation and accents there are `Common` for our purposes either way).
pub fn standard(b: u8) -> u32 {
    match b {
        0xE1 => 0x00C6, // AE
        0xE9 => 0x0141, // Lslash
        0xEA => 0x00D8, // Oslash
        0xEB => 0x0152, // OE
        0xF1 => 0x00E6, // ae
        0xF5 => 0x0131, // dotlessi
        0xF8 => 0x0142, // lslash
        0xF9 => 0x00F8, // oslash
        0xFA => 0x0153, // oe
        0xFB => 0x00DF, // germandbls
        0x80..=0xFF => 0x2022,
        _ => u32::from(b),
    }
}

fn hex_digits(s: &[u8]) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut v = 0u32;
    for &c in s {
        let d = match c {
            b'0'..=b'9' => c - b'0',
            b'A'..=b'F' => c - b'A' + 10,
            _ => return None, // the AGL spec requires uppercase hex
        };
        v = v.checked_mul(16)?.checked_add(u32::from(d))?;
    }
    Some(v)
}

/// Decode a glyph name as used in an `/Encoding /Differences` array.
///
/// Handles `uniXXXX`, `uXXXX`-`uXXXXXX`, plain ASCII glyph names (Latin letters, with a short list
/// of punctuation and digit names that are `Common`), and Indic names by suffix (`kadeva`,
/// `ka-deva`, `kabengali`, ...). The suffix rule is recalled from the Adobe Glyph List naming
/// pattern and not checked against the list.
pub fn glyph_name(name: &[u8]) -> Decoded {
    let base = name.split(|&c| c == b'.').next().unwrap_or(name);
    let base = base.split(|&c| c == b'_').next().unwrap_or(base);
    if base.is_empty() || base == b"notdef" {
        return Decoded::Unknown;
    }
    if let Some(rest) = base.strip_prefix(b"uni")
        && rest.len() >= 4
        && rest.len() % 4 == 0
        && let Some(cp) = hex_digits(&rest[..4])
    {
        return Decoded::Cp(cp);
    }
    if let Some(rest) = base.strip_prefix(b"u")
        && (4..=6).contains(&rest.len())
        && let Some(cp) = hex_digits(rest)
    {
        return Decoded::Cp(cp);
    }
    if !base.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-') {
        return Decoded::Unknown;
    }
    let lower: Vec<u8> = base
        .iter()
        .filter(|c| **c != b'-')
        .map(u8::to_ascii_lowercase)
        .collect();
    for (suffix, script) in [
        (&b"deva"[..], Script::Devanagari),
        (b"devanagari", Script::Devanagari),
        (b"bengali", Script::Bengali),
        (b"beng", Script::Bengali),
        (b"gurmukhi", Script::Gurmukhi),
        (b"guru", Script::Gurmukhi),
        (b"gujarati", Script::Gujarati),
        (b"gujr", Script::Gujarati),
        (b"oriya", Script::Oriya),
        (b"orya", Script::Oriya),
        (b"tamil", Script::Tamil),
        (b"taml", Script::Tamil),
        (b"telugu", Script::Telugu),
        (b"telu", Script::Telugu),
        (b"kannada", Script::Kannada),
        (b"knda", Script::Kannada),
        (b"malayalam", Script::Malayalam),
        (b"mlym", Script::Malayalam),
    ] {
        if lower.len() > suffix.len() && lower.ends_with(suffix) {
            return Decoded::ScriptLetter(script);
        }
    }
    const COMMON_NAMES: [&[u8]; 54] = [
        b"space",
        b"exclam",
        b"quotedbl",
        b"numbersign",
        b"dollar",
        b"percent",
        b"ampersand",
        b"quotesingle",
        b"parenleft",
        b"parenright",
        b"asterisk",
        b"plus",
        b"comma",
        b"hyphen",
        b"minus",
        b"period",
        b"slash",
        b"colon",
        b"semicolon",
        b"less",
        b"equal",
        b"greater",
        b"question",
        b"at",
        b"bracketleft",
        b"backslash",
        b"bracketright",
        b"asciicircum",
        b"underscore",
        b"grave",
        b"braceleft",
        b"bar",
        b"braceright",
        b"asciitilde",
        b"zero",
        b"one",
        b"two",
        b"three",
        b"four",
        b"five",
        b"six",
        b"seven",
        b"eight",
        b"nine",
        b"bullet",
        b"endash",
        b"emdash",
        b"quoteleft",
        b"quoteright",
        b"quotedblleft",
        b"quotedblright",
        b"ellipsis",
        b"degree",
        b"section",
    ];
    if COMMON_NAMES.contains(&lower.as_slice()) {
        return Decoded::Cp(0x2022);
    }
    if base.len() == 1 && base[0].is_ascii_alphabetic() {
        return Decoded::Cp(u32::from(base[0]));
    }
    // `g12`, `cid1234`, `glyph5`: tool-generated names carry no script information.
    if lower.iter().any(u8::is_ascii_digit) {
        return Decoded::Unknown;
    }
    if base.len() >= 2 && !lower.starts_with(b"uni") && base.iter().all(u8::is_ascii_alphabetic) {
        // Aacute, ccedilla, fi, germandbls ...: Latin glyph names.
        return Decoded::Cp(u32::from(base[0].to_ascii_uppercase()));
    }
    Decoded::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winansi_high_half() {
        assert_eq!(winansi(0x41), 0x41);
        assert_eq!(winansi(0x80), 0x20AC);
        assert_eq!(winansi(0x93), 0x201C);
        assert_eq!(winansi(0x81), 0x2022);
        assert_eq!(winansi(0xE9), 0xE9);
    }

    #[test]
    fn macroman_spot_checks() {
        assert_eq!(macroman(0x80), 0x00C4);
        assert_eq!(macroman(0x8E), 0x00E9);
        assert_eq!(macroman(0xD0), 0x2013);
        assert_eq!(macroman(0xDE), 0xFB01);
        assert_eq!(macroman(0xFF), 0x02C7);
        assert_eq!(macroman(0x41), 0x41);
    }

    #[test]
    fn glyph_names() {
        assert_eq!(glyph_name(b"uni0915"), Decoded::Cp(0x0915));
        assert_eq!(glyph_name(b"u1F600"), Decoded::Cp(0x1F600));
        assert_eq!(glyph_name(b"uni0915.alt"), Decoded::Cp(0x0915));
        assert_eq!(
            glyph_name(b"kadeva"),
            Decoded::ScriptLetter(Script::Devanagari)
        );
        assert_eq!(
            glyph_name(b"ka-deva"),
            Decoded::ScriptLetter(Script::Devanagari)
        );
        assert_eq!(
            glyph_name(b"kabengali"),
            Decoded::ScriptLetter(Script::Bengali)
        );
        assert_eq!(glyph_name(b"A"), Decoded::Cp(0x41));
        assert_eq!(glyph_name(b"Aacute"), Decoded::Cp(0x41));
        assert_eq!(glyph_name(b"space"), Decoded::Cp(0x2022));
        assert_eq!(glyph_name(b"g1234"), Decoded::Unknown);
        assert_eq!(glyph_name(b".notdef"), Decoded::Unknown);
        assert_eq!(glyph_name(b"uniZZZZ"), Decoded::Unknown);
        assert_eq!(glyph_name(b""), Decoded::Unknown);
        // lowercase hex is not an AGL uniXXXX name
        assert_eq!(glyph_name(b"uni09ab"), Decoded::Unknown);
    }
}
