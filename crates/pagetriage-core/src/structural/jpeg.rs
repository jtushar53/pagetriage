//! JPEG quantisation tables from `/DCTDecode` image streams (module M12), parsed from the marker
//! segments without decoding any pixels.
//!
//! The table hash identifies the encoder family's table set; the quality estimate fits the
//! luminance table to the IJG (libjpeg) scaling of the Annex K standard table. The basis for using
//! quantisation tables as an encoder fingerprint is H. Farid's JPEG-ballistics work (Dartmouth
//! technical reports, 2006-2008); that citation is recalled from memory and unverified.

use sha2::{Digest, Sha256};

/// Zig-zag order: `ZIGZAG[k]` is the natural (row-major) index of the k-th coefficient in a DQT
/// segment.
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// Annex K (ITU-T T.81) example luminance table, natural order.
const STD_LUMA: [u16; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56,
    14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113,
    92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];

/// Annex K example chrominance table, natural order.
const STD_CHROMA: [u16; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99,
    47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];

/// One quantisation table, natural order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuantTable {
    pub id: u8,
    pub precision_bits: u8,
    pub values: [u16; 64],
}

/// What was read from a JPEG header.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JpegHeader {
    pub tables: Vec<QuantTable>,
    pub width: u32,
    pub height: u32,
    pub components: u8,
    /// SOF marker type (0 = baseline, 1 = extended, 2 = progressive, ...), if a SOF was seen.
    pub sof_type: Option<u8>,
    /// First application marker names seen (`JFIF`, `Adobe`, `Exif`, ...), in order.
    pub app_markers: Vec<String>,
}

/// Parse marker segments up to the start of scan (or `max_bytes`). Tolerant: stops at the first
/// structural inconsistency and returns what was read; returns `None` if the data does not start
/// with the SOI marker.
pub fn parse_header(data: &[u8], max_bytes: usize) -> Option<JpegHeader> {
    let data = &data[..data.len().min(max_bytes)];
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut h = JpegHeader::default();
    let mut i = 2usize;
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        if marker == 0xFF {
            i += 1; // fill byte
            continue;
        }
        if marker == 0x00 || marker == 0x01 || (0xD0..=0xD8).contains(&marker) {
            i += 2; // stuffed byte, TEM, RSTn, SOI: no length
            continue;
        }
        if marker == 0xD9 || marker == 0xDA {
            break; // EOI, SOS: scan data follows
        }
        let len = usize::from(u16::from_be_bytes([data[i + 2], data[i + 3]]));
        if len < 2 {
            break;
        }
        let seg_end = (i + 2 + len).min(data.len());
        let seg = &data[i + 4..seg_end];
        match marker {
            0xDB => parse_dqt(seg, &mut h.tables),
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                if seg.len() >= 6 && h.sof_type.is_none() {
                    h.sof_type = Some(marker - 0xC0);
                    h.height = u32::from(u16::from_be_bytes([seg[1], seg[2]]));
                    h.width = u32::from(u16::from_be_bytes([seg[3], seg[4]]));
                    h.components = seg[5];
                }
            }
            0xE0..=0xEF if h.app_markers.len() < 8 => {
                let name: String = seg
                    .iter()
                    .take_while(|&&b| b != 0)
                    .take(12)
                    .filter(|b| b.is_ascii_graphic())
                    .map(|&b| b as char)
                    .collect();
                h.app_markers.push(name);
            }
            _ => {}
        }
        i = seg_end;
    }
    Some(h)
}

fn parse_dqt(mut seg: &[u8], out: &mut Vec<QuantTable>) {
    while !seg.is_empty() && out.len() < 8 {
        let pq_tq = seg[0];
        let precision = pq_tq >> 4;
        let id = pq_tq & 0x0F;
        let bytes_per = if precision == 0 { 1 } else { 2 };
        if precision > 1 || seg.len() < 1 + 64 * bytes_per {
            return;
        }
        let mut values = [0u16; 64];
        for (k, slot) in ZIGZAG.iter().enumerate() {
            values[*slot] = if bytes_per == 1 {
                u16::from(seg[1 + k])
            } else {
                u16::from_be_bytes([seg[1 + 2 * k], seg[2 + 2 * k]])
            };
        }
        out.push(QuantTable {
            id,
            precision_bits: if precision == 0 { 8 } else { 16 },
            values,
        });
        seg = &seg[1 + 64 * bytes_per..];
    }
}

/// libjpeg `jpeg_set_quality` scaling of a standard table, with `force_baseline` (8-bit values).
pub fn ijg_scaled(standard: &[u16; 64], quality: u32) -> [u16; 64] {
    let q = quality.clamp(1, 100);
    let scale = if q < 50 { 5000 / q } else { 200 - q * 2 };
    let mut out = [0u16; 64];
    for (o, s) in out.iter_mut().zip(standard.iter()) {
        let v = (u32::from(*s) * scale + 50) / 100;
        *o = v.clamp(1, 255) as u16;
    }
    out
}

/// Best IJG quality for a table and how well it fits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QualityFit {
    pub quality: u32,
    /// Mean absolute difference per table entry between the observed and the fitted table.
    pub mean_abs_error: f64,
}

/// Fit `table` (natural order) to the IJG scaling of the Annex K luminance (`chroma = false`) or
/// chrominance table. Ties go to the lower quality.
pub fn fit_ijg_quality(table: &[u16; 64], chroma: bool) -> QualityFit {
    let std = if chroma { &STD_CHROMA } else { &STD_LUMA };
    let mut best = QualityFit {
        quality: 1,
        mean_abs_error: f64::INFINITY,
    };
    let mut best_sum = u32::MAX;
    for q in 1..=100u32 {
        let fitted = ijg_scaled(std, q);
        let sum: u32 = fitted
            .iter()
            .zip(table.iter())
            .map(|(a, b)| u32::from(a.abs_diff(*b)))
            .sum();
        if sum < best_sum {
            best_sum = sum;
            best = QualityFit {
                quality: q,
                mean_abs_error: f64::from(sum) / 64.0,
            };
        }
    }
    best
}

/// SHA-256 over the tables in id order (each as id, precision, then 64 big-endian values in
/// natural order), as lower-case hex of the first 16 bytes. Independent of the order the DQT
/// segments appear in.
pub fn table_hash(tables: &[QuantTable]) -> String {
    let mut sorted: Vec<&QuantTable> = tables.iter().collect();
    sorted.sort_by_key(|t| t.id);
    let mut h = Sha256::new();
    for t in sorted {
        h.update([t.id, t.precision_bits]);
        for v in t.values {
            h.update(v.to_be_bytes());
        }
    }
    let digest = h.finalize();
    digest.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A JPEG header with the given tables (zig-zag already applied by `dqt`), a baseline SOF and
    /// no scan.
    fn jpeg_with(tables: &[(u8, [u16; 64])], w: u16, h: u16) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        out.extend_from_slice(b"JFIF\0\x01\x01\x00\x00\x01\x00\x01\x00\x00");
        for (id, natural) in tables {
            out.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x43, *id]);
            for slot in ZIGZAG {
                out.push(natural[slot] as u8);
            }
        }
        out.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 0x08]);
        out.extend_from_slice(&h.to_be_bytes());
        out.extend_from_slice(&w.to_be_bytes());
        out.extend_from_slice(&[0x01, 0x01, 0x11, 0x00]);
        out.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00]);
        out
    }

    #[test]
    fn ijg_scaling_reproduces_known_libjpeg_values() {
        // libjpeg quality 50 leaves the standard table unchanged; quality 100 is all ones.
        assert_eq!(ijg_scaled(&STD_LUMA, 50), STD_LUMA);
        assert!(ijg_scaled(&STD_LUMA, 100).iter().all(|&v| v == 1));
        // Quality 75 (scale 50): first entries 16, 11, 10 -> 8, 6, 5.
        let q75 = ijg_scaled(&STD_LUMA, 75);
        assert_eq!(&q75[..3], &[8, 6, 5]);
        // Quality 10 (scale 500) clamps at 255 for the large entries.
        let q10 = ijg_scaled(&STD_LUMA, 10);
        assert_eq!(q10[0], 80);
        assert_eq!(q10[63], 255);
    }

    #[test]
    fn every_ijg_quality_is_recovered_exactly() {
        for q in 1..=100u32 {
            let fit = fit_ijg_quality(&ijg_scaled(&STD_LUMA, q), false);
            // Several adjacent qualities can give identical tables near 100; the fit must be exact
            // and not further than that tie from the truth.
            assert_eq!(fit.mean_abs_error, 0.0, "q={q}");
            let again = ijg_scaled(&STD_LUMA, fit.quality);
            assert_eq!(again, ijg_scaled(&STD_LUMA, q), "q={q}");
            let cfit = fit_ijg_quality(&ijg_scaled(&STD_CHROMA, q), true);
            assert_eq!(cfit.mean_abs_error, 0.0, "chroma q={q}");
        }
    }

    #[test]
    fn a_custom_table_fits_poorly() {
        let mut custom = [0u16; 64];
        for (i, v) in custom.iter_mut().enumerate() {
            *v = 3 + (i as u16 * 7) % 31;
        }
        assert!(fit_ijg_quality(&custom, false).mean_abs_error > 3.0);
    }

    #[test]
    fn parses_tables_dimensions_and_markers() {
        let bytes = jpeg_with(
            &[
                (0, ijg_scaled(&STD_LUMA, 90)),
                (1, ijg_scaled(&STD_CHROMA, 90)),
            ],
            640,
            480,
        );
        let h = parse_header(&bytes, 65536).unwrap();
        assert_eq!(h.tables.len(), 2);
        assert_eq!(h.tables[0].values, ijg_scaled(&STD_LUMA, 90));
        assert_eq!((h.width, h.height, h.components), (640, 480, 1));
        assert_eq!(h.sof_type, Some(0));
        assert_eq!(h.app_markers, vec!["JFIF".to_string()]);
        assert_eq!(fit_ijg_quality(&h.tables[0].values, false).quality, 90);
    }

    #[test]
    fn table_hash_ignores_segment_order_and_distinguishes_tables() {
        let a = (0u8, ijg_scaled(&STD_LUMA, 75));
        let b = (1u8, ijg_scaled(&STD_CHROMA, 75));
        let h1 = parse_header(&jpeg_with(&[a, b], 8, 8), 4096).unwrap();
        let h2 = parse_header(&jpeg_with(&[b, a], 8, 8), 4096).unwrap();
        assert_eq!(table_hash(&h1.tables), table_hash(&h2.tables));
        let c = (0u8, ijg_scaled(&STD_LUMA, 80));
        let h3 = parse_header(&jpeg_with(&[c, b], 8, 8), 4096).unwrap();
        assert_ne!(table_hash(&h1.tables), table_hash(&h3.tables));
        assert_eq!(table_hash(&h1.tables).len(), 32);
    }

    #[test]
    fn rejects_non_jpeg_and_survives_truncation() {
        assert!(parse_header(b"", 100).is_none());
        assert!(parse_header(b"\x89PNG....", 100).is_none());
        let full = jpeg_with(&[(0, ijg_scaled(&STD_LUMA, 75))], 8, 8);
        for cut in 0..full.len() {
            let _ = parse_header(&full[..cut], 65536);
        }
        // A DQT cut mid-table yields no table rather than garbage.
        let cut = parse_header(&full[..30], 65536);
        assert!(cut.is_none_or(|h| h.tables.is_empty()));
    }

    #[test]
    fn sixteen_bit_tables_are_read() {
        let mut data = vec![0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x83, 0x10];
        for k in 0..64u16 {
            data.extend_from_slice(&(300 + k).to_be_bytes());
        }
        data.extend_from_slice(&[0xFF, 0xD9]);
        let h = parse_header(&data, 4096).unwrap();
        assert_eq!(h.tables.len(), 1);
        assert_eq!(h.tables[0].precision_bits, 16);
        assert_eq!(h.tables[0].values[0], 300);
        assert_eq!(h.tables[0].values[1], 301);
    }
}
