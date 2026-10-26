//! Stamp golden snapshots: the colour pages in `tests/golden/stamp_inputs/` (PPM, P6) are analysed
//! with only the opt-in `stamp` detector (and `{"debug":true}`) at 150 dpi, gray taken from the
//! colour with the library's own conversion, and must match `tests/golden/stamp_expected/NAME.json`
//! byte for byte. The Python and WASM runners in `tests/determinism/` check the same files.
//!
//! Regenerate after an intentional change, then review the diff:
//! `UPDATE_GOLDEN=1 cargo test -p pagetriage-core --test stamp_golden`

use std::fs;
use std::path::PathBuf;

use pagetriage_core::raster::rgb_to_gray;
use pagetriage_core::{PageInput, analyse_page};

const OPTS: &str = r#"{"debug":true,"detectors":["stamp"]}"#;
const DPI: u16 = 150;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden")
}

/// Minimal binary PPM (P6, maxval 255) reader.
fn parse_ppm(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut pos = 0;
    let mut tokens = Vec::new();
    while tokens.len() < 4 {
        while bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let start = pos;
        while !bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        tokens.push(std::str::from_utf8(&bytes[start..pos]).unwrap().to_owned());
    }
    pos += 1;
    assert_eq!((tokens[0].as_str(), tokens[3].as_str()), ("P6", "255"));
    let (w, h): (u32, u32) = (tokens[1].parse().unwrap(), tokens[2].parse().unwrap());
    let pixels = bytes[pos..].to_vec();
    assert_eq!(pixels.len(), (w * h * 3) as usize);
    (w, h, pixels)
}

fn fixtures() -> Vec<(String, PathBuf)> {
    let mut found: Vec<(String, PathBuf)> = fs::read_dir(golden_dir().join("stamp_inputs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "ppm"))
        .map(|p| (p.file_stem().unwrap().to_string_lossy().into_owned(), p))
        .collect();
    found.sort();
    found
}

fn report(path: &PathBuf) -> serde_json::Value {
    serde_json::from_str(&report_text(path)).unwrap()
}

fn report_text(path: &PathBuf) -> String {
    let (w, h, rgb) = parse_ppm(&fs::read(path).unwrap());
    let gray = rgb_to_gray(&rgb).unwrap();
    let input = PageInput {
        gray: &gray,
        rgb: Some(&rgb),
        width: w,
        height: h,
        dpi: DPI,
    };
    format!("{}\n", analyse_page(&input, None, OPTS).unwrap())
}

#[test]
fn stamp_reports_match_byte_for_byte() {
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let expected_dir = golden_dir().join("stamp_expected");
    fs::create_dir_all(&expected_dir).unwrap();
    let all = fixtures();
    assert!(all.len() >= 3, "found only {} stamp inputs", all.len());
    for (name, path) in all {
        let actual = report_text(&path);
        let expected_path = expected_dir.join(format!("{name}.json"));
        if update {
            fs::write(&expected_path, &actual).unwrap();
            continue;
        }
        let expected = fs::read_to_string(&expected_path).unwrap_or_else(|_| {
            panic!(
                "missing {}; run with UPDATE_GOLDEN=1",
                expected_path.display()
            )
        });
        assert_eq!(actual, expected, "stamp golden mismatch for {name}");
    }
}

#[test]
fn every_stamp_expected_file_has_an_input() {
    let inputs: Vec<String> = fixtures().into_iter().map(|(n, _)| n).collect();
    for entry in fs::read_dir(golden_dir().join("stamp_expected"))
        .into_iter()
        .flatten()
    {
        let path = entry.unwrap().path();
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        assert!(inputs.contains(&stem), "orphaned {}", path.display());
    }
}

/// What the fixtures were drawn with, pinned so a golden refresh cannot bless a wrong answer.
#[test]
fn stamp_pages_have_what_they_were_drawn_with() {
    let stamps = |page: &str| -> Vec<(String, String, String)> {
        let v = report(&golden_dir().join(format!("stamp_inputs/{page}.ppm")));
        v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                (
                    f["kind"].as_str().unwrap().to_owned(),
                    f["attributes"]["shape"].as_str().unwrap().to_owned(),
                    f["attributes"]["colour"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };
    let t = |a: &str, b: &str, c: &str| (a.to_owned(), b.to_owned(), c.to_owned());
    assert_eq!(
        stamps("stamp_circle_over_text"),
        [t("stamp", "circle", "blue")]
    );
    assert_eq!(
        stamps("stamp_violet_seal_and_red_rectangle"),
        [
            t("seal", "circle", "violet"),
            t("stamp", "rectangle", "red")
        ]
    );
    assert!(stamps("stamp_confusers").is_empty());
}
