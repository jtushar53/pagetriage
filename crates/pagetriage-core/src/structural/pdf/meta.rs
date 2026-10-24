//! Document-level metadata for module M12: producer, creator, XMP, xref style, linearisation.

use lopdf::{Dictionary, Document, Object};

use crate::structural::params::ProvenanceParams;

#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub version: String,
    pub producer: Option<String>,
    pub creator: Option<String>,
    pub creation_date: Option<String>,
    pub mod_date: Option<String>,
    pub xmp_present: bool,
    pub xmp_tool: Option<String>,
    pub producer_family: Option<String>,
    pub ocr_producer: bool,
    pub xref_style: &'static str,
    pub linearized: bool,
    pub encrypted: bool,
}

/// PDF text string to `String`: UTF-16BE/LE or UTF-8 with a BOM, otherwise Latin-1 as an
/// approximation of PDFDocEncoding. Control characters become spaces; length is capped.
pub fn text_string(raw: &[u8], max_chars: usize) -> String {
    let decoded: String = if raw.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = raw[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| u16::from_be_bytes(*p))
            .collect();
        String::from_utf16_lossy(&units)
    } else if raw.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = raw[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| u16::from_le_bytes(*p))
            .collect();
        String::from_utf16_lossy(&units)
    } else if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(&raw[3..]).into_owned()
    } else {
        raw.iter().map(|&b| b as char).collect()
    };
    decoded
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn info_string(doc: &Document, info: &Dictionary, key: &[u8], max: usize) -> Option<String> {
    let (_, o) = doc.dereference(info.get(key).ok()?).ok()?;
    let s = match o {
        Object::String(s, _) => text_string(s, max),
        Object::Name(n) => text_string(n, max),
        _ => return None,
    };
    (!s.is_empty()).then_some(s)
}

/// Text of the first `<tag ...>text</tag>` in an XMP packet, or of `tag="text"`, tags stripped.
fn xmp_value(xmp: &str, tag: &str, max: usize) -> Option<String> {
    let open = format!("<{tag}");
    if let Some(start) = xmp.find(&open) {
        let after = &xmp[start + open.len()..];
        if let Some(gt) = after.find('>') {
            let body = &after[gt + 1..];
            let end = body.find(&format!("</{tag}>"))?;
            let mut out = String::new();
            let mut in_tag = false;
            for c in body[..end].chars() {
                match c {
                    '<' => in_tag = true,
                    '>' => in_tag = false,
                    _ if !in_tag => out.push(c),
                    _ => {}
                }
            }
            let out: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
            let out: String = out.chars().take(max).collect();
            if !out.is_empty() {
                return Some(out);
            }
        }
    }
    let attr = format!("{tag}=\"");
    let start = xmp.find(&attr)? + attr.len();
    let end = xmp[start..].find('"')?;
    let v: String = xmp[start..start + end].chars().take(max).collect();
    (!v.is_empty()).then_some(v)
}

fn xref_style(bytes: &[u8]) -> &'static str {
    let tail_start = bytes.len().saturating_sub(2048);
    let Some(rel) = rfind(&bytes[tail_start..], b"startxref") else {
        return "unknown";
    };
    let mut i = tail_start + rel + b"startxref".len();
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let mut off: usize = 0;
    let mut digits = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() && digits < 12 {
        off = off * 10 + usize::from(bytes[i] - b'0');
        i += 1;
        digits += 1;
    }
    if digits == 0 || off >= bytes.len() {
        return "unknown";
    }
    let at = &bytes[off..];
    if at.starts_with(b"xref") {
        let window = &at[..at.len().min(1 << 20)];
        let trailer_end = find(window, b"startxref").unwrap_or(window.len());
        if find(&window[..trailer_end], b"/XRefStm").is_some() {
            "hybrid"
        } else {
            "classic"
        }
    } else if at.first().is_some_and(u8::is_ascii_digit) {
        "stream"
    } else {
        "unknown"
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

pub fn read_meta(doc: &Document, bytes: &[u8], p: &ProvenanceParams, max_stream: usize) -> Meta {
    let mut m = Meta {
        version: doc.version.clone(),
        xref_style: xref_style(bytes),
        linearized: {
            let head = &bytes[..bytes.len().min(2048)];
            find(head, b"/Linearized").is_some()
        },
        encrypted: doc.trailer.has(b"Encrypt") || doc.was_encrypted(),
        ..Meta::default()
    };
    if let Some(info) = doc
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_dict().ok())
    {
        let max = p.max_meta_chars;
        m.producer = info_string(doc, info, b"Producer", max);
        m.creator = info_string(doc, info, b"Creator", max);
        m.creation_date = info_string(doc, info, b"CreationDate", 64);
        m.mod_date = info_string(doc, info, b"ModDate", 64);
    }
    // XMP packet from the catalog's /Metadata stream.
    if let Some(Object::Stream(s)) = doc
        .trailer
        .get(b"Root")
        .ok()
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_dict().ok())
        .and_then(|cat| cat.get(b"Metadata").ok())
        .and_then(|o| doc.dereference(o).ok())
        .map(|(_, o)| o)
    {
        m.xmp_present = true;
        if let Ok(data) = s.decompressed_content_with_limit(p.max_xmp_bytes.min(max_stream)) {
            let xmp = String::from_utf8_lossy(&data);
            let tool = xmp_value(&xmp, "xmp:CreatorTool", p.max_meta_chars)
                .or_else(|| xmp_value(&xmp, "pdf:Producer", p.max_meta_chars));
            m.xmp_tool = tool;
            if m.producer.is_none() {
                m.producer = xmp_value(&xmp, "pdf:Producer", p.max_meta_chars);
            }
        }
    }
    // Fingerprints: first match in list order decides the family; any OCR match sets the flag.
    let haystack = [&m.producer, &m.creator, &m.xmp_tool]
        .iter()
        .filter_map(|s| s.as_deref())
        .collect::<Vec<_>>()
        .join(" | ")
        .to_lowercase();
    for fp in &p.producer {
        if haystack.contains(&fp.pattern) {
            if m.producer_family.is_none() {
                m.producer_family = Some(fp.family.clone());
            }
            m.ocr_producer |= fp.ocr;
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_strings_decode_bom_forms() {
        assert_eq!(
            text_string(b"Plain Producer 1.0", 100),
            "Plain Producer 1.0"
        );
        assert_eq!(
            text_string(&[0xFE, 0xFF, 0x00, 0x41, 0x00, 0x42], 100),
            "AB"
        );
        assert_eq!(text_string(&[0xFF, 0xFE, 0x41, 0x00], 100), "A");
        assert_eq!(text_string(&[0xEF, 0xBB, 0xBF, b'o', b'k'], 100), "ok");
        assert_eq!(text_string(b"a\x00b\nc", 100), "a b c");
        assert_eq!(text_string(b"abcdef", 3), "abc");
    }

    #[test]
    fn xmp_values_from_elements_and_attributes() {
        let xmp = "<x:xmpmeta><rdf:Description xmp:CreatorTool=\"Scanner X1\">\
                   <pdf:Producer>OCRmyPDF <b>16</b>\n</pdf:Producer></rdf:Description></x:xmpmeta>";
        assert_eq!(
            xmp_value(xmp, "xmp:CreatorTool", 100).as_deref(),
            Some("Scanner X1")
        );
        assert_eq!(
            xmp_value(xmp, "pdf:Producer", 100).as_deref(),
            Some("OCRmyPDF 16")
        );
        assert_eq!(xmp_value(xmp, "dc:title", 100), None);
    }

    #[test]
    fn xref_styles() {
        let classic = b"%PDF-1.4\nxref\n0 1\n0000000000 65535 f \ntrailer\n<< /Size 1 >>\nstartxref\n9\n%%EOF";
        assert_eq!(xref_style(classic), "classic");
        let stream =
            b"%PDF-1.5\n1 0 obj\n<< /Type /XRef >>\nstream\nendstream\nendobj\nstartxref\n9\n%%EOF";
        assert_eq!(xref_style(stream), "stream");
        let hybrid = b"%PDF-1.5\nxref\n0 1\n0000000000 65535 f \ntrailer\n<< /XRefStm 99 >>\nstartxref\n9\n%%EOF";
        assert_eq!(xref_style(hybrid), "hybrid");
        assert_eq!(xref_style(b"%PDF-1.4 nothing"), "unknown");
        assert_eq!(xref_style(b"startxref\n99999\n%%EOF"), "unknown");
    }
}
