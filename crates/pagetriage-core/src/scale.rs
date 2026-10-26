//! Scale of a scan: the resolution written in an image's header, and the resolutions that the pixel
//! size of a page is consistent with for the standard paper sizes. See `docs/detectors/scale.md`.
//!
//! The analyser needs the resolution (`dpi`) of the page it is given and does not guess it. This module
//! helps the caller get it: [`image_dpi`] reads PNG, JPEG (JFIF and Exif), TIFF and BMP headers, and
//! [`paper_matches`] lists the (paper size, standard dpi) pairs a pixel size fits. Both are pure and
//! integer or plain `f64` arithmetic, so they give the same answer on every target.

/// A resolution read from a file header.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageDpi {
    pub x: f64,
    pub y: f64,
    /// Where it came from: `png_phys`, `jfif`, `exif`, `tiff` or `bmp`.
    pub source: &'static str,
}

const MIN_DPI: f64 = 10.0;
const MAX_DPI: f64 = 10_000.0;

fn sane(x: f64, y: f64, source: &'static str) -> Option<ImageDpi> {
    let ok = |v: f64| v.is_finite() && (MIN_DPI..=MAX_DPI).contains(&v);
    (ok(x) && ok(y)).then_some(ImageDpi { x, y, source })
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

/// The resolution an image file states, if it states one. Only the first few headers are read; a
/// file that is not one of the supported formats, or states none, gives `None`.
pub fn image_dpi(bytes: &[u8]) -> Option<ImageDpi> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return png(bytes);
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        return jpeg(bytes);
    }
    if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        return tiff(bytes, "tiff");
    }
    if bytes.starts_with(b"BM") {
        return bmp(bytes);
    }
    None
}

fn png(b: &[u8]) -> Option<ImageDpi> {
    let mut pos = 8;
    while pos + 8 <= b.len() {
        let len = be32(b, pos)? as usize;
        let kind = b.get(pos + 4..pos + 8)?;
        if kind == b"pHYs" {
            let (px, py, unit) = (be32(b, pos + 8)?, be32(b, pos + 12)?, *b.get(pos + 16)?);
            // Unit 1 is the metre; 0 means only the aspect ratio is given.
            return (unit == 1)
                .then(|| sane(f64::from(px) * 0.0254, f64::from(py) * 0.0254, "png_phys"))?;
        }
        if kind == b"IDAT" || kind == b"IEND" {
            return None;
        }
        pos = pos.checked_add(12)?.checked_add(len)?;
    }
    None
}

fn jpeg(b: &[u8]) -> Option<ImageDpi> {
    let mut pos = 2;
    let mut exif: Option<ImageDpi> = None;
    while pos + 4 <= b.len() && b[pos] == 0xFF {
        let marker = b[pos + 1];
        if marker == 0xFF {
            pos += 1;
            continue;
        }
        // Start of scan or end of image: no more headers.
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        let len = be16(b, pos + 2)? as usize;
        let seg = b.get(pos + 4..pos + 2 + len)?;
        if marker == 0xE0 && seg.starts_with(b"JFIF\0") && seg.len() >= 12 {
            let (units, xd, yd) = (seg[7], be16(seg, 8)?, be16(seg, 10)?);
            let scale = match units {
                1 => 1.0,
                2 => 2.54,
                _ => 0.0,
            };
            if scale > 0.0
                && let Some(d) = sane(f64::from(xd) * scale, f64::from(yd) * scale, "jfif")
            {
                return Some(d);
            }
        } else if marker == 0xE1 && seg.starts_with(b"Exif\0\0") && exif.is_none() {
            exif = tiff(&seg[6..], "exif");
        }
        pos += 2 + len;
    }
    exif
}

fn tiff(b: &[u8], source: &'static str) -> Option<ImageDpi> {
    let little = match b.get(..4)? {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let s: [u8; 2] = b.get(at..at + 2)?.try_into().ok()?;
        Some(if little {
            u16::from_le_bytes(s)
        } else {
            u16::from_be_bytes(s)
        })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let s: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
        Some(if little {
            u32::from_le_bytes(s)
        } else {
            u32::from_be_bytes(s)
        })
    };
    let ifd = u32_at(4)? as usize;
    let count = u16_at(ifd)? as usize;
    let (mut xres, mut yres, mut unit) = (None, None, 2u16);
    for i in 0..count.min(256) {
        let e = ifd + 2 + 12 * i;
        let (tag, kind) = (u16_at(e)?, u16_at(e + 2)?);
        match tag {
            // XResolution / YResolution: a rational stored at an offset.
            282 | 283 if kind == 5 => {
                let off = u32_at(e + 8)? as usize;
                let (num, den) = (u32_at(off)?, u32_at(off + 4)?);
                if den != 0 {
                    let v = f64::from(num) / f64::from(den);
                    if tag == 282 {
                        xres = Some(v);
                    } else {
                        yres = Some(v);
                    }
                }
            }
            // ResolutionUnit: 1 none, 2 inch, 3 centimetre.
            296 if kind == 3 => unit = u16_at(e + 8)?,
            _ => {}
        }
    }
    let scale = match unit {
        2 => 1.0,
        3 => 2.54,
        _ => return None,
    };
    sane(xres? * scale, yres.or(xres)? * scale, source)
}

fn bmp(b: &[u8]) -> Option<ImageDpi> {
    let le = |at: usize| -> Option<i32> {
        Some(i32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
    };
    let (px, py) = (le(38)?, le(42)?);
    (px > 0 && py > 0).then(|| sane(f64::from(px) * 0.0254, f64::from(py) * 0.0254, "bmp"))?
}

/// A paper size and standard resolution that a pixel size fits.
#[derive(Debug, Clone, PartialEq)]
pub struct PaperMatch {
    pub paper: &'static str,
    pub dpi: u16,
    pub landscape: bool,
    /// Largest relative difference between the pixel size and the paper at that resolution.
    pub error: f64,
}

/// Paper sizes in inches, short side first.
const PAPERS: &[(&str, f64, f64)] = &[
    // The common office sizes first: a pixel size that fits two papers at two resolutions (Letter at
    // 300 dpi and ANSI C at 150 are the same pixels) is listed with the commoner paper first.
    ("A4", 8.27, 11.69),
    ("Letter", 8.5, 11.0),
    ("Legal", 8.5, 14.0),
    ("A3", 11.69, 16.54),
    ("A5", 5.83, 8.27),
    ("Tabloid", 11.0, 17.0),
    ("A2", 16.54, 23.39),
    ("A1", 23.39, 33.11),
    ("A0", 33.11, 46.81),
    ("A6", 4.13, 5.83),
    ("B4", 9.84, 13.90),
    ("B5", 6.93, 9.84),
    ("Executive", 7.25, 10.5),
    ("ANSI C", 17.0, 22.0),
    ("ANSI D", 22.0, 34.0),
    ("ANSI E", 34.0, 44.0),
    ("Arch A", 9.0, 12.0),
    ("Arch B", 12.0, 18.0),
    ("Arch C", 18.0, 24.0),
    ("Arch D", 24.0, 36.0),
    ("Arch E", 36.0, 48.0),
];

/// Resolutions scanners and print drivers use.
const STANDARD_DPI: &[u16] = &[72, 96, 100, 120, 150, 200, 240, 300, 400, 600, 1200];

/// The (paper, standard dpi) pairs a page of `width x height` pixels fits within `tolerance` (a
/// fraction, for example 0.02), best first. Scans are usually cropped or have margins, so an empty
/// answer means nothing about the scan; a single answer is a hint, not a fact.
pub fn paper_matches(width: u32, height: u32, tolerance: f64) -> Vec<PaperMatch> {
    let (short, long) = (f64::from(width.min(height)), f64::from(width.max(height)));
    let landscape = width > height;
    let mut out: Vec<(usize, PaperMatch)> = Vec::new();
    for (index, &(paper, ps, pl)) in PAPERS.iter().enumerate() {
        for &dpi in STANDARD_DPI {
            let d = f64::from(dpi);
            let (es, el) = (
                (short - ps * d).abs() / (ps * d),
                (long - pl * d).abs() / (pl * d),
            );
            let error = es.max(el);
            if error <= tolerance {
                out.push((
                    index,
                    PaperMatch {
                        paper,
                        dpi,
                        landscape,
                        error,
                    },
                ));
            }
        }
    }
    // Best fit first (to a tenth of a percent), then the commoner paper, then the lower resolution.
    out.sort_by_key(|(index, m)| ((m.error * 1000.0).round() as u32, *index, m.dpi));
    out.into_iter().map(|(_, m)| m).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_with(phys: Option<(u32, u32, u8)>) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&[0; 13]);
        v.extend_from_slice(&[0; 4]);
        if let Some((x, y, u)) = phys {
            v.extend_from_slice(&9u32.to_be_bytes());
            v.extend_from_slice(b"pHYs");
            v.extend_from_slice(&x.to_be_bytes());
            v.extend_from_slice(&y.to_be_bytes());
            v.push(u);
            v.extend_from_slice(&[0; 4]);
        }
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(b"IDAT");
        v
    }

    #[test]
    fn png_phys_in_metres() {
        // 11811 pixels per metre is 300.0 dpi.
        let d = image_dpi(&png_with(Some((11811, 11811, 1)))).unwrap();
        assert!(
            (d.x - 299.9994).abs() < 0.01 && d.source == "png_phys",
            "{d:?}"
        );
        assert!(
            image_dpi(&png_with(Some((1, 1, 0)))).is_none(),
            "unit 0 is an aspect ratio"
        );
        assert!(image_dpi(&png_with(None)).is_none());
    }

    fn jfif(units: u8, x: u16, y: u16) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16];
        v.extend_from_slice(b"JFIF\0");
        v.extend_from_slice(&[1, 1, units]);
        v.extend_from_slice(&x.to_be_bytes());
        v.extend_from_slice(&y.to_be_bytes());
        v.extend_from_slice(&[0, 0]);
        v.extend_from_slice(&[0xFF, 0xDA, 0, 2]);
        v
    }

    #[test]
    fn jfif_inches_and_centimetres() {
        assert_eq!(image_dpi(&jfif(1, 200, 200)).unwrap().x, 200.0);
        let d = image_dpi(&jfif(2, 118, 118)).unwrap();
        assert!((d.x - 299.72).abs() < 0.01);
        assert!(
            image_dpi(&jfif(0, 1, 1)).is_none(),
            "units 0: aspect ratio only"
        );
        assert!(
            image_dpi(&jfif(1, 1, 1)).is_none(),
            "1 dpi is not a scan resolution"
        );
    }

    fn tiff_le(xres: u32, unit: u16) -> Vec<u8> {
        let mut v = b"II*\0".to_vec();
        v.extend_from_slice(&8u32.to_le_bytes());
        v.extend_from_slice(&3u16.to_le_bytes());
        // 282 XResolution rational at offset 8 + 2 + 36 + 4 = 50; 283 the same value; 296 unit.
        for (tag, off) in [(282u16, 50u32), (283, 58)] {
            v.extend_from_slice(&tag.to_le_bytes());
            v.extend_from_slice(&5u16.to_le_bytes());
            v.extend_from_slice(&1u32.to_le_bytes());
            v.extend_from_slice(&off.to_le_bytes());
        }
        v.extend_from_slice(&296u16.to_le_bytes());
        v.extend_from_slice(&3u16.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&u32::from(unit).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        for _ in 0..2 {
            v.extend_from_slice(&xres.to_le_bytes());
            v.extend_from_slice(&1u32.to_le_bytes());
        }
        v
    }

    #[test]
    fn tiff_rational_with_unit() {
        let d = image_dpi(&tiff_le(300, 2)).unwrap();
        assert_eq!((d.x, d.y, d.source), (300.0, 300.0, "tiff"));
        let d = image_dpi(&tiff_le(118, 3)).unwrap();
        assert!((d.x - 299.72).abs() < 0.01);
        assert!(image_dpi(&tiff_le(300, 1)).is_none(), "unit none");
    }

    #[test]
    fn exif_in_a_jpeg() {
        let tiff = tiff_le(240, 2);
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE1];
        v.extend_from_slice(&((tiff.len() + 8) as u16).to_be_bytes());
        v.extend_from_slice(b"Exif\0\0");
        v.extend_from_slice(&tiff);
        v.extend_from_slice(&[0xFF, 0xDA, 0, 2]);
        let d = image_dpi(&v).unwrap();
        assert_eq!((d.x, d.source), (240.0, "exif"));
    }

    #[test]
    fn bmp_pixels_per_metre() {
        let mut v = b"BM".to_vec();
        v.resize(38, 0);
        v.extend_from_slice(&11811i32.to_le_bytes());
        v.extend_from_slice(&11811i32.to_le_bytes());
        assert!((image_dpi(&v).unwrap().x - 300.0).abs() < 0.01);
        v[38..46].fill(0);
        assert!(image_dpi(&v).is_none());
    }

    #[test]
    fn junk_and_truncated_files_give_none() {
        for b in [
            &b""[..],
            b"hello",
            b"\x89PNG\r\n\x1a\n",
            b"\xFF\xD8\xFF",
            b"II*\0\x08",
            b"BM",
        ] {
            assert!(image_dpi(b).is_none());
        }
    }

    #[test]
    fn paper_sizes_and_resolutions() {
        let m = paper_matches(1240, 1754, 0.02);
        assert_eq!((m[0].paper, m[0].dpi, m[0].landscape), ("A4", 150, false));
        let m = paper_matches(2550, 3300, 0.02);
        assert_eq!((m[0].paper, m[0].dpi), ("Letter", 300));
        let m = paper_matches(3507, 2481, 0.02);
        assert_eq!((m[0].paper, m[0].dpi, m[0].landscape), ("A4", 300, true));
        assert!(paper_matches(777, 1111, 0.005).is_empty());
        // A4 at 100 dpi and A5 at 150 dpi differ by a factor; A3 at 100 and A4 at 150 are listed together.
        let both = paper_matches(1169, 1654, 0.02);
        assert!(both.iter().any(|m| m.paper == "A3" && m.dpi == 100));
    }
}
