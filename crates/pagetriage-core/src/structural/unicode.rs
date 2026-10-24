//! Unicode block classification for module M2 (`structural.font_script`).
//!
//! The ranges are the Unicode block boundaries named in the module definition (Devanagari
//! U+0900-097F, Bengali U+0980-09FF, Gurmukhi U+0A00-0A7F, Gujarati U+0A80-0AFF, Oriya
//! U+0B00-0B7F, Tamil U+0B80-0BFF, Telugu U+0C00-0C7F, Kannada U+0C80-0CFF, Malayalam
//! U+0D00-0D7F, Arabic U+0600-06FF, CJK U+4E00-9FFF). They are the published Unicode block
//! assignments, not tuned values, so they live here and not in `params/`.

use std::fmt;

/// A writing system the histogram can attribute a code point to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Script {
    Latin,
    Devanagari,
    Bengali,
    Gurmukhi,
    Gujarati,
    Oriya,
    Tamil,
    Telugu,
    Kannada,
    Malayalam,
    Arabic,
    Cjk,
    /// Letters of any other script (Cyrillic, Greek, Hebrew, Thai, ...).
    Other,
}

impl Script {
    /// All scripts in the fixed order used for histograms and output keys.
    pub const ALL: [Script; 13] = [
        Script::Latin,
        Script::Devanagari,
        Script::Bengali,
        Script::Gurmukhi,
        Script::Gujarati,
        Script::Oriya,
        Script::Tamil,
        Script::Telugu,
        Script::Kannada,
        Script::Malayalam,
        Script::Arabic,
        Script::Cjk,
        Script::Other,
    ];

    /// Machine id used in JSON output and in `params/font_script.toml`.
    pub fn id(self) -> &'static str {
        match self {
            Script::Latin => "latin",
            Script::Devanagari => "devanagari",
            Script::Bengali => "bengali",
            Script::Gurmukhi => "gurmukhi",
            Script::Gujarati => "gujarati",
            Script::Oriya => "oriya",
            Script::Tamil => "tamil",
            Script::Telugu => "telugu",
            Script::Kannada => "kannada",
            Script::Malayalam => "malayalam",
            Script::Arabic => "arabic",
            Script::Cjk => "cjk",
            Script::Other => "other",
        }
    }

    /// Human-readable name.
    pub fn display_name(self) -> &'static str {
        match self {
            Script::Latin => "Latin",
            Script::Devanagari => "Devanagari",
            Script::Bengali => "Bengali",
            Script::Gurmukhi => "Gurmukhi",
            Script::Gujarati => "Gujarati",
            Script::Oriya => "Oriya",
            Script::Tamil => "Tamil",
            Script::Telugu => "Telugu",
            Script::Kannada => "Kannada",
            Script::Malayalam => "Malayalam",
            Script::Arabic => "Arabic",
            Script::Cjk => "CJK",
            Script::Other => "Other",
        }
    }

    pub fn from_id(id: &str) -> Option<Script> {
        Script::ALL.iter().copied().find(|s| s.id() == id)
    }

    /// Position in [`Script::ALL`].
    pub fn index(self) -> usize {
        Script::ALL
            .iter()
            .position(|s| *s == self)
            .expect("every script is in ALL")
    }
}

impl fmt::Display for Script {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// What one code point counts as in the script histogram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeClass {
    /// A letter (or letter-like sign) of the given script.
    Letter(Script),
    /// Digits, punctuation, symbols and whitespace: carry no script and are left out of shares.
    Common,
    /// Private-use area: a font that maps glyphs to U+E000-F8FF has no standard Unicode meaning.
    PrivateUse,
}

/// Classify a code point. Indic combining signs (matras, viramas) count as letters of their
/// block: they only occur in text of that script.
pub fn classify(cp: u32) -> CodeClass {
    use CodeClass::{Common, Letter, PrivateUse};
    match cp {
        0x0000..=0x0040 | 0x005B..=0x0060 | 0x007B..=0x00BF | 0x00D7 | 0x00F7 => Common,
        0x0041..=0x005A | 0x0061..=0x007A | 0x00C0..=0x024F | 0x1E00..=0x1EFF => {
            Letter(Script::Latin)
        }
        0xFB00..=0xFB06 => Letter(Script::Latin), // fi, fl, ff ... ligatures
        0x0250..=0x02FF => Common,                // IPA, spacing modifiers
        0x0600..=0x06FF | 0x0750..=0x077F | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => {
            if (0x0660..=0x0669).contains(&cp) || (0x06F0..=0x06F9).contains(&cp) {
                Common // Arabic-Indic digits
            } else {
                Letter(Script::Arabic)
            }
        }
        0x0900..=0x097F => indic(cp, 0x0900, Script::Devanagari),
        0x0980..=0x09FF => indic(cp, 0x0980, Script::Bengali),
        0x0A00..=0x0A7F => indic(cp, 0x0A00, Script::Gurmukhi),
        0x0A80..=0x0AFF => indic(cp, 0x0A80, Script::Gujarati),
        0x0B00..=0x0B7F => indic(cp, 0x0B00, Script::Oriya),
        0x0B80..=0x0BFF => indic(cp, 0x0B80, Script::Tamil),
        0x0C00..=0x0C7F => indic(cp, 0x0C00, Script::Telugu),
        0x0C80..=0x0CFF => indic(cp, 0x0C80, Script::Kannada),
        0x0D00..=0x0D7F => indic(cp, 0x0D00, Script::Malayalam),
        0x1CD0..=0x1CFF | 0xA8E0..=0xA8FF => Letter(Script::Devanagari), // Vedic, Devanagari Ext
        0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x3040..=0x30FF | 0xAC00..=0xD7AF => {
            Letter(Script::Cjk)
        }
        0x2000..=0x206F | 0x20A0..=0x20CF | 0x2100..=0x2BFF | 0x3000..=0x303F => Common,
        0xE000..=0xF8FF => PrivateUse,
        0xFFFD => Common,
        0x0370..=0x058F | 0x0590..=0x05FF | 0x0E00..=0x0E7F => Letter(Script::Other),
        _ => Letter(Script::Other),
    }
}

/// Within an Indic block, the digit range (offset 0x66-0x6F) and the danda/double danda
/// punctuation (Devanagari 0964-0965, shared by the other Indic blocks) are not script letters.
fn indic(cp: u32, block_start: u32, script: Script) -> CodeClass {
    let off = cp - block_start;
    if (0x66..=0x6F).contains(&off) || off == 0x64 || off == 0x65 {
        CodeClass::Common
    } else {
        CodeClass::Letter(script)
    }
}

/// Per-script letter counts, plus the code points that were not letters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Histogram {
    pub letters: [u64; 13],
    pub common: u64,
    pub private_use: u64,
}

impl Histogram {
    pub fn add(&mut self, cp: u32) {
        match classify(cp) {
            CodeClass::Letter(s) => self.letters[s.index()] += 1,
            CodeClass::Common => self.common += 1,
            CodeClass::PrivateUse => self.private_use += 1,
        }
    }

    pub fn merge(&mut self, other: &Histogram) {
        for (a, b) in self.letters.iter_mut().zip(other.letters.iter()) {
            *a += *b;
        }
        self.common += other.common;
        self.private_use += other.private_use;
    }

    pub fn total_letters(&self) -> u64 {
        self.letters.iter().sum()
    }

    pub fn count(&self, s: Script) -> u64 {
        self.letters[s.index()]
    }

    /// The script with the most letters; ties go to the earlier entry of [`Script::ALL`].
    pub fn dominant(&self) -> Option<Script> {
        let mut best: Option<(Script, u64)> = None;
        for s in Script::ALL {
            let c = self.count(s);
            if c > 0 && best.is_none_or(|(_, bc)| c > bc) {
                best = Some((s, c));
            }
        }
        best.map(|(s, _)| s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_match_the_published_boundaries() {
        let cases = [
            (0x0915, Script::Devanagari), // KA
            (0x0995, Script::Bengali),
            (0x0A15, Script::Gurmukhi),
            (0x0A95, Script::Gujarati),
            (0x0B15, Script::Oriya),
            (0x0B95, Script::Tamil),
            (0x0C15, Script::Telugu),
            (0x0C95, Script::Kannada),
            (0x0D15, Script::Malayalam),
            (0x0627, Script::Arabic),
            (0x4E2D, Script::Cjk),
            (0x0041, Script::Latin),
            (0x00E9, Script::Latin),
        ];
        for (cp, want) in cases {
            assert_eq!(classify(cp), CodeClass::Letter(want), "U+{cp:04X}");
        }
    }

    #[test]
    fn digits_punctuation_and_dandas_are_common() {
        for cp in [
            0x20, 0x30, 0x2C, 0x2014, 0x0966, 0x0964, 0x0965, 0x09E6, 0x0660,
        ] {
            assert_eq!(classify(cp), CodeClass::Common, "U+{cp:04X}");
        }
        assert_eq!(classify(0xE001), CodeClass::PrivateUse);
    }

    #[test]
    fn histogram_dominant_and_ties() {
        let mut h = Histogram::default();
        for cp in "abc".chars() {
            h.add(cp as u32);
        }
        for cp in "\u{0915}\u{0916}\u{0917}".chars() {
            h.add(cp as u32);
        }
        // 3 Latin, 3 Devanagari: the earlier entry of ALL (Latin) wins.
        assert_eq!(h.dominant(), Some(Script::Latin));
        h.add(0x0918);
        assert_eq!(h.dominant(), Some(Script::Devanagari));
        assert_eq!(h.total_letters(), 7);
        assert_eq!(Histogram::default().dominant(), None);
    }

    #[test]
    fn script_ids_round_trip() {
        for s in Script::ALL {
            assert_eq!(Script::from_id(s.id()), Some(s));
        }
    }
}
