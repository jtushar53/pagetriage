//! The quality gate: on a page graded D the text-line detectors are not run and a `degraded` finding
//! says so; the plan sends the whole page to a vision model. A clean page is untouched.

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

/// Box blur with the given radius, twice over, in integers.
fn blur(w: usize, h: usize, g: &[u8], r: usize) -> Vec<u8> {
    let pass = |src: &[u8], horizontal: bool| -> Vec<u8> {
        let mut out = src.to_vec();
        for y in 0..h {
            for x in 0..w {
                let (mut sum, mut n) = (0u32, 0u32);
                for d in 0..=2 * r {
                    let (xx, yy) = if horizontal {
                        ((x + d).checked_sub(r), Some(y))
                    } else {
                        (Some(x), (y + d).checked_sub(r))
                    };
                    if let (Some(xx), Some(yy)) = (xx, yy)
                        && xx < w
                        && yy < h
                    {
                        sum += u32::from(src[yy * w + xx]);
                        n += 1;
                    }
                }
                out[y * w + x] = (sum / n) as u8;
            }
        }
        out
    };
    let a = pass(g, true);
    pass(&a, false)
}

fn run(w: usize, h: usize, g: &[u8], opts: &str) -> Value {
    let input = PageInput {
        gray: g,
        rgb: None,
        width: w as u32,
        height: h as u32,
        dpi: 150,
    };
    serde_json::from_str(&analyse_page(&input, None, opts).unwrap()).unwrap()
}

fn kinds(v: &Value) -> Vec<String> {
    v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["kind"].as_str().unwrap().to_owned())
        .collect()
}

const OPTS: &str =
    r#"{"detectors":["script","handwriting","signature","layout","plan"],"debug":true}"#;

#[test]
fn a_page_graded_d_skips_the_line_detectors_and_says_so() {
    let (w, h, g) = page();
    let v = run(w, h, &blur(w, h, &g, 12), OPTS);
    assert_eq!(v["quality"]["grade"], "D");
    assert_eq!(kinds(&v), ["degraded"]);
    let f = &v["findings"][0];
    assert!(f["confidence"].as_f64().unwrap() >= 0.5);
    assert_eq!(f["attributes"]["grade"], "D");
    assert_eq!(v["debug"]["script"]["gated"], 1.0);
    // The plan sends the whole page to a vision model.
    let regions = v["plan"]["regions"].as_array().unwrap();
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0]["route"], "vision");
    assert_eq!(regions[0]["bbox"], serde_json::json!([0, 0, w - 1, h - 1]));
}

#[test]
fn no_gate_runs_the_detectors_anyway() {
    let (w, h, g) = page();
    let b = blur(w, h, &g, 12);
    let opts = r#"{"detectors":["script"],"no_gate":true,"debug":true}"#;
    let v = run(w, h, &b, opts);
    assert!(!kinds(&v).contains(&"degraded".to_owned()));
    assert!(v["debug"]["script"].get("gated").is_none());
}

#[test]
fn clean_and_moderately_blurred_pages_are_not_gated() {
    let (w, h, g) = page();
    for r in [0usize, 4, 9] {
        let b = if r == 0 { g.clone() } else { blur(w, h, &g, r) };
        let v = run(w, h, &b, OPTS);
        assert_ne!(v["quality"]["grade"], "D", "blur {r}");
        assert!(!kinds(&v).contains(&"degraded".to_owned()), "blur {r}");
    }
}

#[test]
fn nothing_to_skip_means_no_finding() {
    // Only detectors that do not read text lines: nothing is skipped, so there is nothing to report.
    let (w, h, g) = page();
    let v = run(w, h, &blur(w, h, &g, 12), r#"{"detectors":["qr"]}"#);
    assert!(!kinds(&v).contains(&"degraded".to_owned()));
}
