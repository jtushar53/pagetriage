//! The page plan: routes, scale, turns, skipping, tiles and the cache key, on the synthetic golden
//! pages and on pages drawn here.

use std::fs;
use std::path::PathBuf;

use pagetriage_core::{PageInput, analyse_page};
use serde_json::Value;

fn pgm(name: &str) -> (usize, usize, Vec<u8>) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../tests/golden/layout_inputs/{name}.pgm"));
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

const OPTS: &str =
    r#"{"detectors":["plan","qr","photo","rulings","signature","handwriting"],"debug":true}"#;

fn run(w: usize, h: usize, g: &[u8], dpi: u16, opts: &str) -> Value {
    let input = PageInput {
        gray: g,
        rgb: None,
        width: w as u32,
        height: h as u32,
        dpi,
    };
    serde_json::from_str(&analyse_page(&input, None, opts).unwrap()).unwrap()
}

fn routes(v: &Value) -> Vec<(String, String)> {
    v["plan"]["regions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["kind"].as_str().unwrap().to_owned(),
                r["route"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn a_text_and_picture_page_sends_text_to_ocr_and_the_picture_to_vision() {
    let (w, h, g) = pgm("photo_page");
    let v = run(w, h, &g, 150, OPTS);
    let r = routes(&v);
    assert!(r.contains(&("text_block".into(), "ocr".into())), "{r:?}");
    assert!(r.contains(&("photo".into(), "vision".into())), "{r:?}");
    let plan = &v["plan"];
    assert_eq!(plan["rotate_clockwise_deg"], 0);
    assert_eq!(plan["needs_rerun"], false);
    assert!(plan["skip"].is_null());
    // Small type on a small page: enlarge it to the target size.
    assert!(
        plan["scale"]["factor"].as_f64().unwrap() > 1.5,
        "{}",
        plan["scale"]
    );
    // Text and tables read first, the picture after.
    let pr: Vec<u64> = plan["regions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["priority"].as_u64().unwrap())
        .collect();
    assert!(pr.windows(2).all(|w| w[0] <= w[1]), "{pr:?}");
    // The box holds every region.
    let cb: Vec<u64> = plan["content_bbox"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_u64().unwrap())
        .collect();
    for reg in plan["regions"].as_array().unwrap() {
        let b = reg["bbox"].as_array().unwrap();
        assert!(b[0].as_u64().unwrap() >= cb[0] && b[3].as_u64().unwrap() <= cb[3]);
    }
}

#[test]
fn a_decoded_qr_code_is_marked_decoded() {
    let (w, h, g) = pgm("qr_page");
    let v = run(w, h, &g, 150, OPTS);
    assert!(
        routes(&v).contains(&("qr_code".into(), "decoded".into())),
        "{:?}",
        routes(&v)
    );
}

#[test]
fn a_blank_page_is_skipped() {
    let g = vec![250u8; 300 * 400];
    let v = run(300, 400, &g, 150, OPTS);
    assert_eq!(v["plan"]["skip"]["reason"], "blank");
    assert!(v["plan"]["regions"].as_array().unwrap().is_empty());
}

#[test]
fn nothing_is_planned_unless_asked() {
    let (w, h, g) = pgm("photo_page");
    let v = run(w, h, &g, 150, r#"{"detectors":["photo"]}"#);
    assert!(v["plan"].is_null());
}

#[test]
fn the_cache_key_follows_the_pixels_the_size_and_the_resolution() {
    let (w, h, g) = pgm("layout_one_column");
    let key = |g: &[u8], dpi: u16| {
        run(w, h, g, dpi, OPTS)["plan"]["input_sha256"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let a = key(&g, 150);
    assert_eq!(a.len(), 64);
    assert_eq!(a, key(&g, 150), "same page, same key");
    assert_ne!(a, key(&g, 300), "another resolution");
    let mut g2 = g.clone();
    g2[0] ^= 1;
    assert_ne!(a, key(&g2, 150), "one pixel differs");
}

/// A page of Latin-like text lines, as in tests/orientation.rs (drawn with whole numbers).
fn latin_page() -> (usize, usize, Vec<u8>) {
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
                x += 9;
                continue;
            }
            for yy in y..y + 9 {
                for xx in x..x + 5 {
                    g[yy * w + xx] = 30;
                }
            }
            let r = next(100);
            if r < 25 {
                for yy in y - 7..y {
                    g[yy * w + x + 1] = 30;
                    g[yy * w + x + 2] = 30;
                }
            } else if r < 32 {
                for yy in y + 9..y + 16 {
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

#[test]
fn a_page_on_its_side_is_to_be_turned_and_analysed_again() {
    let (w, h, g) = latin_page();
    // Turned a quarter clockwise: new row x is old column x read bottom to top.
    let mut t = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            t[x * h + (h - 1 - y)] = g[y * w + x];
        }
    }
    let v = run(h, w, &t, 150, OPTS);
    let plan = &v["plan"];
    assert_eq!(plan["rotate_clockwise_deg"], 270);
    assert_eq!(plan["needs_rerun"], true);
    assert!(plan["deskew_clockwise_deg"].is_null());
    assert!(
        plan["regions"].as_array().unwrap().is_empty(),
        "the layout of a sideways page is not trusted"
    );
    assert!(
        plan["skip"].is_null(),
        "a page that needs turning is not blank"
    );
    // Upright: no turn, regions found.
    let v = run(w, h, &g, 150, OPTS);
    assert_eq!(v["plan"]["rotate_clockwise_deg"], 0);
    assert!(!v["plan"]["regions"].as_array().unwrap().is_empty());
}

#[test]
fn a_page_too_large_to_send_whole_is_cut_along_empty_rows_and_columns() {
    // 3000 x 4400 paper with a dark bar at rows 2150..2200 and columns 1400..1450 (the even cuts fall
    // at 2200 and 1500), and short text-like strokes elsewhere.
    let (w, h) = (3000usize, 4400usize);
    let mut g = vec![245u8; w * h];
    for y in 100..4300 {
        if !(2150..2200).contains(&y) && y % 40 < 10 {
            for x in 200..1300 {
                g[y * w + x] = 30;
            }
        }
    }
    for y in 2150..2200 {
        for x in 0..w {
            g[y * w + x] = 30;
        }
    }
    let v = run(w, h, &g, 300, r#"{"detectors":["plan"]}"#);
    let tiles = v["plan"]["tiles"].as_array().unwrap();
    // 3000 / 2048 -> 2 columns, 4400 / 2048 -> 3 rows.
    assert_eq!(tiles.len(), 6, "{tiles:?}");
    for t in tiles {
        let b: Vec<u64> = t["bbox"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap())
            .collect();
        assert!(
            b[2] - b[0] < 2048 + 2 * 200 && b[3] - b[1] < 2048 + 2 * 200,
            "{b:?}"
        );
        assert!(b[2] < w as u64 && b[3] < h as u64);
    }
    // Neighbouring tiles overlap.
    let b = |i: usize| -> Vec<u64> {
        tiles[i]["bbox"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap())
            .collect()
    };
    assert!(b(0)[2] >= b(1)[0], "columns overlap");
    assert!(b(0)[3] >= b(2)[1], "rows overlap");
    // The cut between the first and second tile rows avoids the dark bar. The first tile reaches 8% of its
    // own height past the cut; its height is the cut (no overlap above the first row).
    let cut = {
        let y1 = b(0)[3] + 1;
        // y1 = cut + round(0.08 * (cut - 0)) -> cut = y1 / 1.08
        (y1 as f64 / 1.08).round() as u64
    };
    assert!(
        !(2145..2205).contains(&cut),
        "cut at {cut} runs through the bar"
    );
    // A normal page has none.
    let (w, h, g) = pgm("layout_one_column");
    assert!(
        run(w, h, &g, 150, OPTS)["plan"]["tiles"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
