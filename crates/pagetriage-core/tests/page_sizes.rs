//! The analyser takes the page size and resolution from its input; nothing is tied to A4 or to 150 dpi.
//! The synthetic text page `layout_one_column` (320 x 260 px at 150 dpi, 13 lines) is enlarged by whole
//! factors (nearest neighbour, so the structure is exact and the dpi is the factor times 150) and
//! padded with paper to other shapes (a narrow strip, a wide landscape sheet, a tall legal sheet):
//! the layout must find the same lines and blocks and the same skew every time.

use std::fs;
use std::path::PathBuf;

use pagetriage_core::{PageInput, analyse_page};
use serde_json::Value;

fn page() -> (usize, usize, Vec<u8>) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/layout_inputs/layout_one_column.pgm");
    let bytes = fs::read(path).unwrap();
    let (mut pos, mut tokens) = (0, Vec::new());
    while tokens.len() < 4 {
        while bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let start = pos;
        while !bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        tokens.push(String::from_utf8_lossy(&bytes[start..pos]).into_owned());
    }
    pos += 1;
    (
        tokens[1].parse().unwrap(),
        tokens[2].parse().unwrap(),
        bytes[pos..].to_vec(),
    )
}

/// Enlarge by `k` and pad with the paper colour to `w2 x h2` (at least the enlarged size).
fn shaped(k: usize, w2: usize, h2: usize) -> (usize, usize, Vec<u8>) {
    let (w, h, g) = page();
    let paper = *g.iter().max().unwrap();
    assert!(w2 >= w * k && h2 >= h * k);
    let mut out = vec![paper; w2 * h2];
    for y in 0..h * k {
        for x in 0..w * k {
            out[y * w2 + x] = g[(y / k) * w + x / k];
        }
    }
    (w2, h2, out)
}

fn read(w: usize, h: usize, g: &[u8], dpi: u16) -> Value {
    let input = PageInput {
        gray: g,
        rgb: None,
        width: w as u32,
        height: h as u32,
        dpi,
    };
    let opts = r#"{"debug":true,"detectors":["layout","script"]}"#;
    serde_json::from_str(&analyse_page(&input, None, opts).unwrap()).unwrap()
}

#[test]
fn layout_does_not_depend_on_page_size_or_resolution() {
    let (w, h, _) = page();
    // (factor, canvas width, canvas height): the same page at 150, 300 and 450 dpi, on canvases of
    // other shapes (a 1:3 strip, a landscape sheet, a legal-length sheet, a large square).
    let cases = [
        (1, w, h),
        (1, w + 200, h),
        (1, w, h + 600),
        (1, w * 3, h + 40),
        (2, w * 2, h * 2),
        (2, w * 2 + 300, h * 2 + 900),
        (3, w * 3 + 17, h * 3 + 5),
        (3, w * 4, h * 4),
    ];
    for (k, cw, ch) in cases {
        let (pw, ph, g) = shaped(k, cw, ch);
        let r = read(pw, ph, &g, (150 * k) as u16);
        let layout = &r["debug"]["layout"];
        assert_eq!(layout["lines"], 13.0, "{k}x on {pw}x{ph}");
        assert_eq!(layout["blocks"], 2.0, "{k}x on {pw}x{ph}");
        assert_eq!(layout["columns"], 1.0, "{k}x on {pw}x{ph}");
        assert_eq!(r["quality"]["skew_deg"], 0.0, "{k}x on {pw}x{ph}");
        let size_pt = r["size_pt"].as_array().unwrap();
        let want = [
            pw as f64 * 72.0 / (150.0 * k as f64),
            ph as f64 * 72.0 / (150.0 * k as f64),
        ];
        assert!(
            (size_pt[0].as_f64().unwrap() - want[0]).abs() < 1e-3,
            "{size_pt:?}"
        );
    }
}

#[test]
fn a_wrong_dpi_changes_the_physical_size_and_not_the_layout() {
    // Told 300 dpi for a 150 dpi page, the layout (measured in x-heights) is the same and the page is
    // reported half the size. The caller owns the dpi: inch-based parameters follow what it says.
    let (w, h, g) = page();
    let a = read(w, h, &g, 150);
    let b = read(w, h, &g, 300);
    assert_eq!(a["debug"]["layout"]["lines"], b["debug"]["layout"]["lines"]);
    assert_eq!(
        a["size_pt"][0].as_f64().unwrap(),
        2.0 * b["size_pt"][0].as_f64().unwrap()
    );
}
