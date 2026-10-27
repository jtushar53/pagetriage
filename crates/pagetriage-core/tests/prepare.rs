//! `prepare` on a synthetic page of Latin-like text lines (drawn in this file): turned pages come back
//! upright, tilted pages level, blank pages are skipped, and every output is described by the manifest.

use pagetriage_core::prepare::{Raster, prepare};
use pagetriage_core::{PageInput, analyse_page, to_canonical_json};
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

/// Turn the page clockwise by `deg` degrees about its middle onto a bigger white canvas, nearest
/// neighbour, drawn independently of the library's own rotation.
fn tilt(w: usize, h: usize, g: &[u8], deg: f64) -> (usize, usize, Vec<u8>) {
    let t = deg.to_radians();
    let (s, c) = (pagetriage_core::math::sin(t), pagetriage_core::math::cos(t));
    let nw = (w as f64 * c.abs() + h as f64 * s.abs()).ceil() as usize;
    let nh = (w as f64 * s.abs() + h as f64 * c.abs()).ceil() as usize;
    let mut out = vec![245u8; nw * nh];
    for y in 0..nh {
        for x in 0..nw {
            let (dx, dy) = (x as f64 - nw as f64 / 2.0, y as f64 - nh as f64 / 2.0);
            let sx = (c * dx + s * dy + w as f64 / 2.0).floor();
            let sy = (-s * dx + c * dy + h as f64 / 2.0).floor();
            if sx >= 0.0 && sy >= 0.0 && (sx as usize) < w && (sy as usize) < h {
                out[y * nw + x] = g[sy as usize * w + sx as usize];
            }
        }
    }
    (nw, nh, out)
}

fn run(w: usize, h: usize, g: &[u8], opts: &str) -> pagetriage_core::prepare::Prepared {
    let input = PageInput {
        gray: g,
        rgb: None,
        width: w as u32,
        height: h as u32,
        dpi: 150,
    };
    prepare(&input, opts).unwrap()
}

const OPTS: &str = r#"{"detectors":["layout","script","orientation","plan"]}"#;

fn skew_of(r: &Raster) -> f64 {
    let input = PageInput {
        gray: &r.data,
        rgb: None,
        width: r.width as u32,
        height: r.height as u32,
        dpi: 150,
    };
    let v: Value =
        serde_json::from_str(&analyse_page(&input, None, r#"{"detectors":[]}"#).unwrap()).unwrap();
    v["quality"]["skew_deg"].as_f64().unwrap()
}

#[test]
fn an_upright_page_is_cropped_and_scaled_not_turned() {
    let (w, h, g) = page();
    let r = run(w, h, &g, OPTS);
    assert_eq!(r.manifest.analyses, 1);
    assert!(r.manifest.steps.is_empty());
    assert_eq!(r.manifest.frame.width as usize, w);
    let page_out = &r.outputs[0];
    assert_eq!(page_out.name, "page.png");
    let info = &r.manifest.outputs[0];
    assert_eq!(info.kind, "page");
    assert!(
        info.scale > 1.5,
        "an x-height of 9 px is scaled up toward 24: {}",
        info.scale
    );
    assert_eq!(info.width as usize, page_out.raster.width);
    assert_eq!(info.sha256, page_out.raster.sha256());
}

#[test]
fn turned_pages_are_turned_back() {
    let (w, h, g) = page();
    for deg in [90, 180, 270] {
        let (tw, th, tg) = turn(w, h, &g, deg);
        let r = run(tw, th, &tg, OPTS);
        let step = r
            .manifest
            .steps
            .first()
            .unwrap_or_else(|| panic!("{deg}: no step"));
        assert_eq!(step.op, "rotate", "{deg}");
        assert_eq!(step.clockwise_deg, f64::from(360 - deg), "{deg}");
        assert_eq!(
            r.manifest.plan.rotate_clockwise_deg, 0,
            "{deg}: still turned"
        );
        assert_eq!(
            (
                r.manifest.frame.width as usize,
                r.manifest.frame.height as usize
            ),
            (w, h),
            "{deg}"
        );
        // The prepared frame is the original page, pixel for pixel.
        assert_eq!(
            r.manifest.frame_sha256,
            run(w, h, &g, OPTS).manifest.frame_sha256,
            "{deg}"
        );
    }
}

#[test]
fn a_tilted_page_is_levelled() {
    let (w, h, g) = page();
    let (tw, th, tg) = tilt(w, h, &g, 4.0);
    let before = skew_of(&Raster::new(tw, th, 1, tg.clone()).unwrap());
    let r = run(tw, th, &tg, OPTS);
    let step = r
        .manifest
        .steps
        .iter()
        .find(|s| s.op == "deskew")
        .expect("a deskew step");
    assert!(
        step.clockwise_deg * 4.0 < 0.0,
        "levelling turns the other way: {}",
        step.clockwise_deg
    );
    assert!(
        (step.clockwise_deg + 4.0).abs() < 1.0,
        "step {}",
        step.clockwise_deg
    );
    let after = skew_of(&r.outputs[0].raster);
    assert!(
        after.abs() < 0.5 && after.abs() < before.abs() / 3.0,
        "skew {before} -> {after}"
    );
}

#[test]
fn a_blank_page_is_skipped_and_makes_no_images() {
    let r = run(200, 300, &vec![250u8; 60_000], OPTS);
    assert!(r.outputs.is_empty());
    assert_eq!(r.manifest.skip.as_ref().unwrap().reason, "blank");
}

#[test]
fn every_output_is_described_and_the_result_is_reproducible() {
    let (w, h, g) = page();
    let (tw, th, tg) = tilt(w, h, &g, -3.0);
    let a = run(tw, th, &tg, OPTS);
    let b = run(tw, th, &tg, OPTS);
    assert_eq!(
        to_canonical_json(&a.manifest).unwrap(),
        to_canonical_json(&b.manifest).unwrap()
    );
    assert_eq!(a.outputs, b.outputs);
    assert_eq!(a.manifest.outputs.len(), a.outputs.len());
    for (info, out) in a.manifest.outputs.iter().zip(&a.outputs) {
        assert_eq!(info.file, out.name);
        assert_eq!(info.sha256, out.raster.sha256());
        assert!(info.bbox[2] < a.manifest.frame.width && info.bbox[3] < a.manifest.frame.height);
    }
}

#[test]
fn bad_options_are_refused() {
    let input = PageInput {
        gray: &[0; 4],
        rgb: None,
        width: 2,
        height: 2,
        dpi: 150,
    };
    assert!(prepare(&input, "[1]").is_err());
    assert!(prepare(&input, r#"{"detectors":["nope"]}"#).is_err());
}
