//! The QR decoder on real codes: `tests/golden/qr_vectors.json` holds module matrices made with the
//! `qrcode` Python package (versions 1 to 27, all four error-correction levels, all eight masks,
//! numeric, alphanumeric and byte segments). Each is drawn on a page and read back through the whole
//! `qr` detector.

use std::fs;
use std::path::PathBuf;

use pagetriage_core::{PageInput, analyse_page};
use serde_json::Value;

struct Vector {
    version: u64,
    level: String,
    text: String,
    rows: Vec<Vec<bool>>,
}

fn vectors() -> Vec<Vector> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/qr_vectors.json");
    let v: Vec<Value> = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    v.iter()
        .map(|c| Vector {
            version: c["version"].as_u64().unwrap(),
            level: c["level"].as_str().unwrap().to_owned(),
            text: c["text"].as_str().unwrap().to_owned(),
            rows: c["rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r.as_str().unwrap().chars().map(|ch| ch == '1').collect())
                .collect(),
        })
        .collect()
}

/// Draw a module matrix on a white page, `scale` pixels per module, a four-module quiet zone, the
/// code at (60, 50).
fn draw(rows: &[Vec<bool>], scale: usize) -> (Vec<u8>, usize, usize) {
    let n = rows.len();
    let (w, h) = (n * scale + 160, n * scale + 140);
    let mut g = vec![255u8; w * h];
    for (y, row) in rows.iter().enumerate() {
        for (x, &dark) in row.iter().enumerate() {
            if dark {
                for dy in 0..scale {
                    for dx in 0..scale {
                        g[(50 + y * scale + dy) * w + 60 + x * scale + dx] = 0;
                    }
                }
            }
        }
    }
    (g, w, h)
}

fn read(gray: &[u8], w: usize, h: usize) -> Vec<Value> {
    let input = PageInput {
        gray,
        rgb: None,
        width: w as u32,
        height: h as u32,
        dpi: 150,
    };
    let out = analyse_page(&input, None, r#"{"detectors":["qr"]}"#).unwrap();
    let v: Value = serde_json::from_str(&out).unwrap();
    v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["kind"] == "qr_code")
        .cloned()
        .collect()
}

fn rotate(rows: &[Vec<bool>]) -> Vec<Vec<bool>> {
    let n = rows.len();
    (0..n)
        .map(|y| (0..n).map(|x| rows[n - 1 - x][y]).collect())
        .collect()
}

#[test]
fn every_vector_decodes_to_its_text() {
    let all = vectors();
    assert!(all.len() >= 20);
    for v in &all {
        let (g, w, h) = draw(&v.rows, 4);
        let found = read(&g, w, h);
        assert_eq!(found.len(), 1, "v{} {}: {found:?}", v.version, v.level);
        let a = &found[0]["attributes"];
        assert_eq!(
            a["decoded"], true,
            "v{} {} did not decode",
            v.version, v.level
        );
        assert_eq!(a["text"], v.text.as_str(), "v{} {}", v.version, v.level);
        assert_eq!(a["version"], v.version);
        assert_eq!(a["ec_level"], v.level.as_str());
        assert_eq!(a["corrected_codewords"], 0);
    }
}

#[test]
fn rotated_codes_decode_too() {
    for v in vectors()
        .iter()
        .filter(|v| matches!(v.version, 1 | 4 | 7 | 14))
    {
        let mut rows = v.rows.clone();
        for turn in 1..4 {
            rows = rotate(&rows);
            let (g, w, h) = draw(&rows, 4);
            let found = read(&g, w, h);
            assert_eq!(found.len(), 1, "v{} turn {turn}", v.version);
            assert_eq!(
                found[0]["attributes"]["text"],
                v.text.as_str(),
                "v{} turn {turn}",
                v.version
            );
        }
    }
}

#[test]
fn damaged_codes_are_repaired_up_to_the_level_and_not_beyond() {
    let all = vectors();
    let v = all
        .iter()
        .find(|v| v.version == 7 && v.level == "H")
        .unwrap();
    let n = v.rows.len();
    // Flip data modules in the lower-right quarter, away from every function pattern.
    let damage = |count: usize| -> Vec<Vec<bool>> {
        // The lower-right quarter has side * side modules; visit them in a scrambled order with a
        // stride coprime to that count, so each is flipped once.
        let side = n / 2 - 3;
        let total = side * side;
        assert!(count <= total);
        let mut stride = 37;
        while (2..=total).any(|d| stride % d == 0 && total % d == 0) {
            stride += 2;
        }
        let mut rows = v.rows.clone();
        for k in 0..count {
            let idx = (k * stride) % total;
            let (x, y) = (n / 2 + 3 + idx % side, n / 2 + 3 + idx / side);
            rows[y][x] = !rows[y][x];
        }
        rows
    };
    // Twelve flipped modules cost a few codewords; even 250 (a third of the quarter) are repaired at
    // level H, which can lose about 30% of its codewords.
    let (g, w, h) = draw(&damage(12), 4);
    let found = read(&g, w, h);
    assert_eq!(found[0]["attributes"]["decoded"], true);
    assert_eq!(found[0]["attributes"]["text"], v.text.as_str());
    assert!(
        found[0]["attributes"]["corrected_codewords"]
            .as_u64()
            .unwrap()
            > 0
    );
    // Every module away from the finder patterns flipped: far past the capacity of level H.
    let mut rows = v.rows.clone();
    for (y, row) in rows.iter_mut().enumerate() {
        for (x, m) in row.iter_mut().enumerate() {
            if x >= 9 && y >= 9 {
                *m = !*m;
            }
        }
    }
    let (g, w, h) = draw(&rows, 4);
    let found = read(&g, w, h);
    assert_eq!(found.len(), 1, "the finder patterns are still there");
    assert_eq!(found[0]["attributes"]["decoded"], false);
    assert!(found[0]["attributes"].get("text").is_none());
}

#[test]
fn a_mirrored_code_is_found_but_not_read() {
    let all = vectors();
    let v = all.iter().find(|v| v.version == 4).unwrap();
    let mirrored: Vec<Vec<bool>> = v
        .rows
        .iter()
        .map(|r| r.iter().rev().copied().collect())
        .collect();
    let (g, w, h) = draw(&mirrored, 4);
    let found = read(&g, w, h);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["attributes"]["decoded"], false);
}

#[test]
fn smaller_modules_still_decode() {
    let all = vectors();
    let v = all
        .iter()
        .find(|v| v.version == 2 && v.level == "M")
        .unwrap();
    for scale in [3usize, 5, 6] {
        let (g, w, h) = draw(&v.rows, scale);
        let found = read(&g, w, h);
        assert_eq!(found.len(), 1, "scale {scale}");
        assert_eq!(
            found[0]["attributes"]["text"],
            v.text.as_str(),
            "scale {scale}"
        );
    }
}
