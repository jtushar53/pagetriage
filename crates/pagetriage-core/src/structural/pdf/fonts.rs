//! Font dictionaries to a decoder model: enough to map the bytes of a text string to Unicode (or
//! to say they cannot be mapped) and to read the facts M2 reports (name, subtype, embedding).

use std::collections::BTreeMap;
use std::rc::Rc;

use lopdf::{Dictionary, Document, Object};

use crate::structural::cmap::CMap;
use crate::structural::encoding::{BaseEncoding, Decoded, glyph_name};
use crate::structural::params::{FontScriptParams, LimitsParams};

/// How a font's string bytes turn into characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeKind {
    /// A `ToUnicode` CMap maps the codes.
    ToUnicode,
    /// Single-byte base encoding plus `/Differences`.
    Simple,
    /// Composite font (Type0) without a `ToUnicode` map: codes are CIDs, not characters.
    CidNoMap,
    /// Type3 font without a `ToUnicode` map or named glyphs.
    Type3NoMap,
    /// `Symbol` / `ZapfDingbats`: symbols, not script letters.
    Symbolic,
}

#[derive(Debug, Clone)]
pub struct FontModel {
    /// `/BaseFont` as written (subset prefix included), lossy UTF-8, truncated.
    pub base_font: String,
    /// Other names that identify the family (`/FontName` of the descriptor, descendant font).
    pub alt_names: Vec<String>,
    pub subtype: String,
    pub embedded: bool,
    pub encoding_label: Option<String>,
    pub kind: DecodeKind,
    cmap: Option<CMap>,
    code_width: u8,
    base: BaseEncoding,
    diffs: BTreeMap<u8, Decoded>,
    pub has_to_unicode: bool,
}

/// One decoded character code: usually one value, several for a ligature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyph<'a> {
    pub values: &'a [Decoded],
    /// The code is a word space (or other whitespace) and does not count as a glyph.
    pub is_space: bool,
}

fn lossy(bytes: &[u8], max: usize) -> String {
    String::from_utf8_lossy(bytes).chars().take(max).collect()
}

fn name_of(dict: &Dictionary, key: &[u8]) -> Option<Vec<u8>> {
    dict.get(key).ok()?.as_name().ok().map(<[u8]>::to_vec)
}

fn deref<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Object> {
    doc.dereference(o).ok().map(|(_, o)| o)
}

fn dict_of<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Dictionary> {
    match deref(doc, o)? {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

fn has_font_file(doc: &Document, descriptor: &Dictionary) -> bool {
    [&b"FontFile"[..], b"FontFile2", b"FontFile3"]
        .iter()
        .any(|k| descriptor.get(k).is_ok_and(|o| deref(doc, o).is_some()))
}

impl FontModel {
    /// Build a model from a font dictionary.
    pub fn from_dict(
        doc: &Document,
        font: &Dictionary,
        fs: &FontScriptParams,
        limits: &LimitsParams,
    ) -> FontModel {
        let subtype =
            name_of(font, b"Subtype").map_or_else(|| "Unknown".to_owned(), |n| lossy(&n, 24));
        let base_font = name_of(font, b"BaseFont").map_or_else(String::new, |n| lossy(&n, 96));
        let mut alt_names = Vec::new();
        let mut embedded = false;

        let descriptor_of = |d: &Dictionary| -> Option<Dictionary> {
            d.get(b"FontDescriptor")
                .ok()
                .and_then(|o| dict_of(doc, o))
                .cloned()
        };
        let note_descriptor = |desc: &Dictionary, alt: &mut Vec<String>| {
            if let Some(n) = name_of(desc, b"FontName") {
                alt.push(lossy(&n, 96));
            }
            if let Ok(Object::String(s, _)) = desc.get(b"FontFamily") {
                alt.push(lossy(s, 96));
            }
        };

        if subtype == "Type0" {
            if let Some(desc_font) = font
                .get(b"DescendantFonts")
                .ok()
                .and_then(|o| deref(doc, o))
                .and_then(|o| o.as_array().ok())
                .and_then(|a| a.first())
                .and_then(|o| dict_of(doc, o))
            {
                if let Some(n) = name_of(desc_font, b"BaseFont") {
                    alt_names.push(lossy(&n, 96));
                }
                if let Some(desc) = descriptor_of(desc_font) {
                    embedded = has_font_file(doc, &desc);
                    note_descriptor(&desc, &mut alt_names);
                }
            }
        } else if let Some(desc) = descriptor_of(font) {
            embedded = has_font_file(doc, &desc);
            note_descriptor(&desc, &mut alt_names);
        }

        // Encoding: a name, or a dictionary with /BaseEncoding and /Differences.
        let mut encoding_label = None;
        let mut base = None;
        let mut diffs: BTreeMap<u8, Decoded> = BTreeMap::new();
        if let Some(enc) = font.get(b"Encoding").ok().and_then(|o| deref(doc, o)) {
            match enc {
                Object::Name(n) => {
                    encoding_label = Some(lossy(n, 40));
                    base = BaseEncoding::from_name(n);
                }
                Object::Dictionary(d) => {
                    if let Some(n) = name_of(d, b"BaseEncoding") {
                        encoding_label = Some(lossy(&n, 40));
                        base = BaseEncoding::from_name(&n);
                    } else {
                        encoding_label = Some("Differences".to_owned());
                    }
                    if let Some(Object::Array(items)) =
                        d.get(b"Differences").ok().and_then(|o| deref(doc, o))
                    {
                        let mut code: i64 = 0;
                        for item in items.iter().take(4096) {
                            match deref(doc, item) {
                                Some(Object::Integer(n)) => code = *n,
                                Some(Object::Name(n)) => {
                                    if (0..=255).contains(&code) {
                                        diffs.insert(code as u8, glyph_name(n));
                                    }
                                    code += 1;
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // ToUnicode.
        let mut cmap = None;
        if let Some(Object::Stream(s)) = font.get(b"ToUnicode").ok().and_then(|o| deref(doc, o))
            && let Ok(data) = s.decompressed_content_with_limit(limits.max_decompressed_bytes)
        {
            let parsed = CMap::parse(&data, fs.max_cmap_entries);
            if !parsed.is_empty() {
                cmap = Some(parsed);
            }
        }
        let has_to_unicode = cmap.is_some();

        let symbolic = {
            let n = base_font.to_ascii_lowercase();
            subtype != "Type0"
                && (n.contains("symbol") || n.contains("zapfdingbats") || n.contains("wingdings"))
        };
        let code_width = if subtype == "Type0" {
            cmap.as_ref().and_then(CMap::uniform_width).unwrap_or(2)
        } else {
            1
        };
        let kind = if has_to_unicode {
            DecodeKind::ToUnicode
        } else if subtype == "Type0" {
            DecodeKind::CidNoMap
        } else if symbolic {
            DecodeKind::Symbolic
        } else if subtype == "Type3" && diffs.is_empty() && base.is_none() {
            DecodeKind::Type3NoMap
        } else {
            DecodeKind::Simple
        };
        FontModel {
            base_font,
            alt_names,
            subtype,
            embedded,
            encoding_label,
            kind,
            cmap,
            code_width,
            base: base.unwrap_or(BaseEncoding::WinAnsi),
            diffs,
            has_to_unicode,
        }
    }

    /// A stand-in for text shown before any font was selected.
    pub fn fallback() -> FontModel {
        FontModel {
            base_font: String::new(),
            alt_names: Vec::new(),
            subtype: "Unknown".into(),
            embedded: false,
            encoding_label: None,
            kind: DecodeKind::Simple,
            cmap: None,
            code_width: 1,
            base: BaseEncoding::WinAnsi,
            diffs: BTreeMap::new(),
            has_to_unicode: false,
        }
    }

    /// All names to compare with the curated legacy list.
    pub fn candidate_names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.base_font.as_str()).chain(self.alt_names.iter().map(String::as_str))
    }

    fn simple_value(&self, b: u8) -> Decoded {
        if let Some(d) = self.diffs.get(&b) {
            return *d;
        }
        match self.kind {
            DecodeKind::Symbolic => Decoded::Cp(0x2022),
            DecodeKind::Type3NoMap => Decoded::Unknown,
            _ => {
                if b < 0x80 {
                    Decoded::Cp(u32::from(b))
                } else {
                    self.base.decode(b)
                }
            }
        }
    }

    /// Decode `bytes` and call `f` once per character code.
    pub fn decode(&self, bytes: &[u8], mut f: impl FnMut(Glyph<'_>)) {
        let mut cps: Vec<u32> = Vec::new();
        let mut vals: Vec<Decoded> = Vec::new();
        match (&self.cmap, self.kind) {
            (Some(cmap), _) => {
                cmap.for_each_code(bytes, self.code_width, |code, width| {
                    cps.clear();
                    vals.clear();
                    if cmap.lookup(code, &mut cps) && !cps.is_empty() {
                        let is_space = cps
                            .iter()
                            .all(|&c| char::from_u32(c).is_some_and(char::is_whitespace));
                        vals.extend(cps.iter().map(|&c| Decoded::Cp(c)));
                        f(Glyph {
                            values: &vals,
                            is_space,
                        });
                    } else if width == 1 && self.subtype != "Type0" {
                        vals.push(self.simple_value(code as u8));
                        f(Glyph {
                            values: &vals,
                            is_space: code == 0x20,
                        });
                    } else {
                        vals.push(Decoded::Unknown);
                        f(Glyph {
                            values: &vals,
                            is_space: false,
                        });
                    }
                });
            }
            (None, DecodeKind::CidNoMap) => {
                let w = usize::from(self.code_width.max(1));
                for _ in bytes.chunks(w) {
                    f(Glyph {
                        values: &[Decoded::Unknown],
                        is_space: false,
                    });
                }
            }
            (None, _) => {
                for &b in bytes {
                    f(Glyph {
                        values: &[self.simple_value(b)],
                        is_space: b == 0x20,
                    });
                }
            }
        }
    }
}

/// Fonts built so far in this document, by object id.
#[derive(Default)]
pub struct FontCache {
    by_id: BTreeMap<(u32, u16), Rc<FontModel>>,
}

impl FontCache {
    pub fn get_or_build(
        &mut self,
        doc: &Document,
        id: (u32, u16),
        dict: &Dictionary,
        fs: &FontScriptParams,
        limits: &LimitsParams,
    ) -> Rc<FontModel> {
        if let Some(m) = self.by_id.get(&id) {
            return Rc::clone(m);
        }
        let m = Rc::new(FontModel::from_dict(doc, dict, fs, limits));
        self.by_id.insert(id, Rc::clone(&m));
        m
    }
}
