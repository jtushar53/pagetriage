//! The `orientation` detector on a synthetic page of Latin-like text lines (drawn in this file), turned by
//! quarter turns: the finding names the turn that restores it, and an upright page has none.

use pagetriage_core::{PageInput, analyse_page};
use serde_json::Value;

/// A page of Latin-like text lines drawn with whole numbers only: each letter is a block 6 px wide
/// and 9 px high (the core), a quarter of the letters carry a 7 px ascender above it and 7% a 7 px
/// descender below it, words are separated by a gap, lines are 36 px apart.
fn page() -> (usize, usize, Vec<u8>) {
    let (w, h) = (480usize, 380usize);
    let mut g = vec![245u8; w * h];
    let mut state: u32 = 12345;
    let mut next = |n: u32| {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 16) % n
    };
    let mut y = 40;
    while y + 40 < h {
        let mut x = 20;
        while x + 8 < w - 20 {
            if next(6) == 0 {
                x += 9; // word gap
                continue;
            }
            let (top, bottom) = (y, y + 9);
            for yy in top..bottom {
                for xx in x..x + 5 {
                    g[yy * w + xx] = 30;
                }
            }
            let r = next(100);
            if r < 25 {
                for yy in top - 7..top {
                    g[yy * w + x + 1] = 30;
                    g[yy * w + x + 2] = 30;
                }
            } else if r < 32 {
                for yy in bottom..bottom + 7 {
                    g[yy * w + x + 1] = 30;
                    g[yy * w + x + 2] = 30;
                }
            }
            x += 7;
        }
        y += 36;
    }
    (w, h, g)
}

/// Turn clockwise by `deg` (0, 90, 180 or 270).
fn turn(w: usize, h: usize, g: &[u8], deg: u32) -> (usize, usize, Vec<u8>) {
    match deg {
        0 => (w, h, g.to_vec()),
        90 => {
            let mut out = vec![0u8; w * h];
            for y in 0..h {
                for x in 0..w {
                    out[x * h + (h - 1 - y)] = g[y * w + x];
                }
            }
            (h, w, out)
        }
        180 => (w, h, g.iter().rev().copied().collect()),
        _ => {
            let (a, b, c) = turn(w, h, g, 90);
            let (a, b, c) = turn(a, b, &c, 90);
            turn(a, b, &c, 90)
        }
    }
}

fn run(w: usize, h: usize, g: &[u8]) -> Value {
    let input = PageInput {
        gray: g,
        rgb: None,
        width: w as u32,
        height: h as u32,
        dpi: 150,
    };
    let opts = r#"{"debug":true,"detectors":["orientation"]}"#;
    serde_json::from_str(&analyse_page(&input, None, opts).unwrap()).unwrap()
}

fn flagged(v: &Value) -> Option<u64> {
    v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["kind"] == "misoriented")
        .map(|f| f["attributes"]["rotate_clockwise_deg"].as_u64().unwrap())
}

#[test]
fn quarter_turns_are_named_and_upright_pages_are_left_alone() {
    let (w, h, g) = page();
    assert_eq!(
        flagged(&run(w, h, &g)),
        None,
        "an upright page has no finding"
    );
    for deg in [90u32, 180, 270] {
        let (tw, th, tg) = turn(w, h, &g, deg);
        let v = run(tw, th, &tg);
        // The page was turned `deg` clockwise; restoring it takes the rest of the circle.
        assert_eq!(
            flagged(&v),
            Some(u64::from(360 - deg)),
            "turned {deg}: {:?}",
            v["debug"]["orientation"]
        );
        assert!(v["debug"]["orientation"]["confidence"].as_f64().unwrap() >= 0.5);
    }
}

#[test]
fn blank_and_empty_pages_abstain() {
    let g = vec![255u8; 200 * 300];
    let v = run(200, 300, &g);
    assert_eq!(flagged(&v), None);
    assert_eq!(v["debug"]["orientation"]["bands"].as_f64().unwrap(), 0.0);
    // One solid black rectangle is not text.
    let mut g = vec![255u8; 200 * 300];
    for y in 50..250 {
        for x in 20..180 {
            g[y * 200 + x] = 10;
        }
    }
    assert_eq!(flagged(&run(200, 300, &g)), None);
}
