//! Legacy (non-Unicode) Indian font detection for module M2.
//!
//! Fonts such as Kruti Dev, DV-TTYogesh, Shree-Lipi or Chanakya put Devanagari glyphs on Latin
//! code points, so the extracted text of such a PDF is Latin-looking gibberish while the page
//! shows Hindi. Two kinds of evidence are combined as a log-odds sum, per font actually used on
//! the page:
//!
//! 1. the font name against a curated list (`params/font_script.toml`, domain knowledge, not
//!    checked against any published list), and
//! 2. statistics of the text the font's codes decode to: how many adjacent letter pairs are common
//!    English bigrams, how many words have a capital after a lower-case letter, and how many
//!    words contain a character that prose does not put inside a word.

use super::params::{FontScriptParams, NameMatch, Strength};
use super::unicode::Script;
use crate::error::Result;
use crate::math;
use crate::schema::Evidence;

const DETECTOR_ID: &str = "structural.font_script";
const CITATION: &str = "module M2: legacy-encoded Indian fonts (curated font-name list plus non-dictionary Latin text statistics); domain knowledge, not a published method";

/// Drop a subset prefix (`ABCDEF+`), lower-case, and keep only ASCII letters and digits.
pub fn normalise_name(raw: &[u8]) -> String {
    let body = match raw.iter().position(|&c| c == b'+') {
        Some(6) if raw[..6].iter().all(u8::is_ascii_uppercase) => &raw[7..],
        _ => raw,
    };
    body.iter()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase() as char)
        .collect()
}

/// A hit in the curated list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameHit {
    pub entry: String,
    pub script: Script,
    pub strength: Strength,
}

/// Strongest curated-list match for a font name, if any (strong beats weak, then list order).
pub fn match_name(font_name: &[u8], p: &FontScriptParams) -> Option<NameHit> {
    let norm = normalise_name(font_name);
    if norm.is_empty() {
        return None;
    }
    let mut best: Option<NameHit> = None;
    for e in &p.legacy_font {
        let hit = match e.matching {
            NameMatch::Prefix => norm.starts_with(&e.name),
            NameMatch::Contains => norm.contains(&e.name),
        };
        if !hit {
            continue;
        }
        let Some(script) = Script::from_id(&e.script) else {
            continue;
        };
        let candidate = NameHit {
            entry: e.name.clone(),
            script,
            strength: e.strength,
        };
        let better = match &best {
            None => true,
            Some(b) => b.strength == Strength::Weak && candidate.strength == Strength::Strong,
        };
        if better {
            best = Some(candidate);
        }
    }
    best
}

/// Rates measured on a text sample.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextStats {
    /// ASCII letters in the sample.
    pub letters: usize,
    /// Share of adjacent in-word letter pairs found in the top English bigram list.
    pub bigram_coverage: f64,
    /// Share of 3+ letter words with a lower-case letter followed by a capital.
    pub mid_caps_rate: f64,
    /// Share of 2+ character words that contain a mid-word marker character.
    pub marker_rate: f64,
    pub words: usize,
    pub bigrams: usize,
}

fn is_strong_marker(c: char) -> bool {
    matches!(c, '[' | ']' | '{' | '}' | '\\' | '^' | '~' | '|' | '`')
}

fn is_weak_marker(c: char) -> bool {
    let cp = c as u32;
    matches!(
        c,
        ';' | '<' | '>' | '=' | '@' | '#' | '$' | '%' | '&' | '*' | '+' | '_'
    ) || (matches!(
        cp,
        0xA1..=0xBF | 0xD7 | 0xF7 | 0x2C6 | 0x2DC | 0x192 | 0x2020 | 0x2021 | 0x2030
    ) && !matches!(cp, 0xAB | 0xBB | 0xA0 | 0xAD | 0xB7))
}

/// Whether `chars[i]` is a character that prose does not put inside a word.
fn marker_at(chars: &[char], i: usize) -> bool {
    let c = chars[i];
    if is_strong_marker(c) {
        return true;
    }
    let letter_left = i > 0 && chars[i - 1].is_ascii_alphabetic();
    let letter_right = chars.get(i + 1).is_some_and(char::is_ascii_alphabetic);
    let inside = i > 0 && i + 1 < chars.len();
    // A semicolon-like mark at a word start before a letter is also a marker in legacy text
    // (";g"), but a trailing one ("word;") is ordinary punctuation.
    if is_weak_marker(c) && ((inside && (letter_left || letter_right)) || (i == 0 && letter_right))
    {
        return true;
    }
    // An accented Latin-1 letter between ASCII letters.
    (0xC0..=0xFF).contains(&(c as u32)) && letter_left && letter_right
}

impl TextStats {
    pub fn measure(sample: &str, top_bigrams: &[String]) -> TextStats {
        let mut letters = 0usize;
        let (mut words, mut long_words) = (0usize, 0usize);
        let (mut mid_caps_words, mut marker_words) = (0usize, 0usize);
        let (mut bigrams, mut bigrams_hit) = (0usize, 0usize);

        for token in sample.split_whitespace() {
            let chars: Vec<char> = token.chars().collect();
            letters += chars.iter().filter(|c| c.is_ascii_alphabetic()).count();
            if chars.len() >= 2 {
                words += 1;
                let marker = (0..chars.len()).any(|i| marker_at(&chars, i));
                if marker {
                    marker_words += 1;
                }
            }
            let alpha: Vec<char> = chars
                .iter()
                .copied()
                .filter(char::is_ascii_alphabetic)
                .collect();
            if alpha.len() >= 3 {
                long_words += 1;
                if alpha
                    .windows(2)
                    .any(|w| w[0].is_ascii_lowercase() && w[1].is_ascii_uppercase())
                {
                    mid_caps_words += 1;
                }
            }
            // Bigrams inside runs of letters.
            let mut prev: Option<char> = None;
            for &c in &chars {
                if c.is_ascii_alphabetic() {
                    if let Some(p) = prev {
                        bigrams += 1;
                        let pair: String = [p.to_ascii_lowercase(), c.to_ascii_lowercase()]
                            .iter()
                            .collect();
                        if top_bigrams.contains(&pair) {
                            bigrams_hit += 1;
                        }
                    }
                    prev = Some(c);
                } else {
                    prev = None;
                }
            }
        }
        let ratio = |a: usize, b: usize| if b == 0 { 0.0 } else { a as f64 / b as f64 };
        TextStats {
            letters,
            bigram_coverage: ratio(bigrams_hit, bigrams),
            mid_caps_rate: ratio(mid_caps_words, long_words),
            marker_rate: ratio(marker_words, words),
            words,
            bigrams,
        }
    }
}

/// The decision for one font.
#[derive(Debug, Clone, PartialEq)]
pub struct FontVerdict {
    pub name_hit: Option<NameHit>,
    pub stats: Option<TextStats>,
    pub logit: f64,
    pub confidence: f64,
    pub legacy: bool,
    /// Script the legacy font writes (from the list entry, or the configured default).
    pub script: Option<Script>,
    pub evidence: Vec<Evidence>,
}

/// Decide whether one used font is legacy-encoded. `sample` is the text its codes decode to (empty
/// when nothing decoded). `font_label` goes into evidence notes.
pub fn judge_font(
    font_name: &[u8],
    font_label: &str,
    sample: &str,
    p: &FontScriptParams,
) -> Result<FontVerdict> {
    let name_hit = match_name(font_name, p);
    let mut evidence = Vec::new();
    let mut logit = p.prior_logit;

    if let Some(hit) = &name_hit {
        let llr = match hit.strength {
            Strength::Strong => p.strong_name_llr,
            Strength::Weak => p.weak_name_llr,
        };
        logit += llr;
        evidence.push(Evidence::new(
            DETECTOR_ID,
            "legacy_font_name",
            1.0,
            Some(1.0),
            llr,
            format!(
                "font {font_label} matches curated legacy {} entry {:?} ({:?})",
                hit.script.id(),
                hit.entry,
                hit.strength
            ),
            CITATION,
        )?);
    }

    let mut stats = None;
    let measured = TextStats::measure(sample, &p.english.top_bigrams);
    if measured.letters >= p.min_sample_letters {
        let rows = [
            (
                "english_bigram_coverage",
                measured.bigram_coverage,
                &p.bigram,
            ),
            ("mid_word_capital_rate", measured.mid_caps_rate, &p.mid_caps),
            ("mid_word_marker_rate", measured.marker_rate, &p.marker),
        ];
        for (feature, value, pw) in rows {
            let llr = pw.apply(value);
            logit += llr;
            evidence.push(Evidence::new(
                DETECTOR_ID,
                feature,
                value,
                Some(pw.lo),
                llr,
                format!(
                    "font {font_label}: {feature} = {value:.3} over {} letters",
                    measured.letters
                ),
                CITATION,
            )?);
        }
        stats = Some(measured);
    }

    let confidence = math::sigmoid(logit);
    let legacy = confidence >= p.emit_min_confidence;
    let script = if legacy {
        Some(match &name_hit {
            Some(h) => h.script,
            None => Script::from_id(&p.default_legacy_script).unwrap_or(Script::Devanagari),
        })
    } else {
        None
    };
    Ok(FontVerdict {
        name_hit,
        stats,
        logit,
        confidence,
        legacy,
        script,
        evidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structural::params::StructuralParams;

    fn params() -> FontScriptParams {
        StructuralParams::defaults().unwrap().font_script
    }

    const ENGLISH: &str = "This Deed of Sale is made at Mumbai on this day between the Vendor and the \
        Purchaser, whereas the Vendor is the absolute owner of the property described in the Schedule \
        hereunder written and has agreed to sell the same to the Purchaser for the consideration \
        mentioned herein. The Purchaser shall pay the stamp duty and registration charges.";

    // Hindi typed in a Kruti-Dev-like layout: lower-case letters, mid-word capitals, brackets and
    // semicolons used as consonants. Synthetic text of this style, not real document text.
    const KRUTI_LIKE: &str = "Hkkjr ljdkj ds ukeku ls ;g foys[k fu";

    #[test]
    fn normalise_strips_subset_prefix_and_punctuation() {
        assert_eq!(normalise_name(b"ABCDEF+Kruti Dev 010"), "krutidev010");
        assert_eq!(normalise_name(b"Kruti_Dev-010Wide"), "krutidev010wide");
        assert_eq!(normalise_name(b"AB+Foo"), "abfoo");
        assert_eq!(normalise_name(b""), "");
    }

    #[test]
    fn curated_names_match() {
        let p = params();
        for name in [
            &b"KrutiDev010"[..],
            b"ABCDEF+Kruti_Dev_010",
            b"DevLys 010",
            b"DV-TTYogesh",
            b"Walkman-Chanakya-902",
            b"ShreeLipi-0714",
            b"APS-Dev-Priyanka",
        ] {
            let hit = match_name(name, &p);
            assert!(hit.is_some(), "{}", String::from_utf8_lossy(name));
            assert_eq!(hit.unwrap().script, Script::Devanagari);
        }
        for name in [
            &b"Mangal"[..],
            b"Arial",
            b"Times-Roman",
            b"NotoSansDevanagari",
            b"Nirmala UI",
        ] {
            assert!(
                match_name(name, &p).is_none(),
                "{}",
                String::from_utf8_lossy(name)
            );
        }
        // A weak entry is reported as weak.
        assert_eq!(match_name(b"Agra", &p).unwrap().strength, Strength::Weak);
    }

    #[test]
    fn english_prose_statistics_look_english() {
        let p = params();
        let s = TextStats::measure(ENGLISH, &p.english.top_bigrams);
        assert!(s.bigram_coverage > 0.45, "{s:?}");
        assert!(s.mid_caps_rate < 0.05, "{s:?}");
        assert!(s.marker_rate < 0.05, "{s:?}");
    }

    #[test]
    fn legacy_style_text_has_low_coverage_and_markers() {
        let p = params();
        let text =
            "lEifRr ds fy;s ;g foys[k Hkkjr ljdkj dk gS {ks=Qy fu\"iknu vk/kkj ¼[kjhnh½ fcØh ys[k";
        let s = TextStats::measure(text, &p.english.top_bigrams);
        assert!(s.mid_caps_rate > 0.05, "{s:?}");
        assert!(s.marker_rate > 0.15, "{s:?}");
    }

    #[test]
    fn name_alone_decides_a_strong_match_but_not_a_weak_one() {
        let p = params();
        let v = judge_font(b"KrutiDev010", "F1", "", &p).unwrap();
        assert!(v.legacy && v.script == Some(Script::Devanagari));
        let w = judge_font(b"Agra", "F1", "", &p).unwrap();
        assert!(!w.legacy);
        let none = judge_font(b"Helvetica", "F1", "", &p).unwrap();
        assert!(!none.legacy && none.evidence.is_empty());
    }

    #[test]
    fn english_text_in_a_strongly_named_font_is_still_flagged_by_name_but_with_lower_confidence() {
        let p = params();
        let by_name = judge_font(b"KrutiDev010", "F1", "", &p).unwrap();
        let english = judge_font(b"KrutiDev010", "F1", ENGLISH, &p).unwrap();
        assert!(english.confidence < by_name.confidence);
    }

    #[test]
    fn statistics_alone_flag_legacy_text_in_an_unnamed_font() {
        let p = params();
        let text = KRUTI_LIKE.repeat(6)
            + &"lEifRr ds fy;s ;g foys[k Hkkjr ljdkj dk gS {ks=Qy fu\"iknu vk/kkj ¼[kjhnh½ "
                .repeat(4);
        let v = judge_font(b"CustomFont1", "F1", &text, &p).unwrap();
        assert!(v.legacy, "{v:?}");
        let eng = judge_font(b"CustomFont1", "F1", &ENGLISH.repeat(3), &p).unwrap();
        assert!(!eng.legacy, "{eng:?}");
    }

    #[test]
    fn short_samples_carry_no_statistical_weight() {
        let p = params();
        let v = judge_font(b"Unknown", "F1", "Hkkjr ljdkj", &p).unwrap();
        assert!(v.stats.is_none());
        assert!(!v.legacy);
    }
}
