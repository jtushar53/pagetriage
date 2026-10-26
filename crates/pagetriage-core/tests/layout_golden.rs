//! Layout golden snapshots: the pages in `tests/golden/layout_inputs/` and `quality_inputs/` are
//! analysed with only the opt-in `layout`, `script`, `handwriting`, `rulings`, `signature` and `thumb` detectors (and `{"debug":true}`) at 150 dpi and must match
//! `tests/golden/layout_expected/NAME.json` byte for byte. The Python and WASM runners in
//! `tests/determinism/` check the same files.
//!
//! Regenerate after an intentional change, then review the diff:
//! `UPDATE_GOLDEN=1 cargo test -p pagetriage-core --test layout_golden`

use std::fs;
use std::path::PathBuf;

use pagetriage_core::{PageInput, analyse_page};

const OPTS: &str =
    r#"{"debug":true,"detectors":["layout","script","handwriting","rulings","signature","thumb"]}"#;
const DPI: u16 = 150;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden")
}

/// Minimal binary PGM (P5, maxval 255) reader.
fn parse_pgm(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
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
    assert_eq!((tokens[0].as_str(), tokens[3].as_str()), ("P5", "255"));
    let (w, h): (u32, u32) = (tokens[1].parse().unwrap(), tokens[2].parse().unwrap());
    let pixels = bytes[pos..].to_vec();
    assert_eq!(pixels.len(), (w * h) as usize);
    (w, h, pixels)
}

fn fixtures() -> Vec<(String, PathBuf)> {
    let mut found: Vec<(String, PathBuf)> = ["layout_inputs", "quality_inputs"]
        .iter()
        .flat_map(|sub| fs::read_dir(golden_dir().join(sub)).unwrap())
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "pgm"))
        .map(|p| (p.file_stem().unwrap().to_string_lossy().into_owned(), p))
        .collect();
    found.sort();
    found
}

fn report(path: &PathBuf) -> String {
    let (w, h, gray) = parse_pgm(&fs::read(path).unwrap());
    let input = PageInput {
        gray: &gray,
        rgb: None,
        width: w,
        height: h,
        dpi: DPI,
    };
    format!("{}\n", analyse_page(&input, None, OPTS).unwrap())
}

#[test]
fn layout_reports_match_byte_for_byte() {
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let expected_dir = golden_dir().join("layout_expected");
    fs::create_dir_all(&expected_dir).unwrap();
    let all = fixtures();
    assert!(all.len() >= 15, "found only {} layout inputs", all.len());
    for (name, path) in all {
        let actual = report(&path);
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
        assert_eq!(actual, expected, "layout golden mismatch for {name}");
    }
}

#[test]
fn every_layout_expected_file_has_an_input() {
    let inputs: Vec<String> = fixtures().into_iter().map(|(n, _)| n).collect();
    for entry in fs::read_dir(golden_dir().join("layout_expected"))
        .into_iter()
        .flatten()
    {
        let path = entry.unwrap().path();
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        assert!(inputs.contains(&stem), "orphaned {}", path.display());
    }
}

/// The synthetic layout pages have a known structure; pin it so a golden refresh cannot quietly
/// bless a wrong segmentation.
#[test]
fn layout_pages_have_the_structure_they_were_drawn_with() {
    let feature = |page: &str, key: &str| -> f64 {
        let json = report(&golden_dir().join(format!("layout_inputs/{page}.pgm")));
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v["debug"]["layout"][key].as_f64().unwrap()
    };
    assert_eq!(feature("layout_one_column", "lines"), 13.0);
    assert_eq!(feature("layout_one_column", "blocks"), 2.0);
    assert_eq!(feature("layout_one_column", "columns"), 1.0);
    assert_eq!(feature("layout_two_columns", "lines"), 19.0);
    assert_eq!(feature("layout_two_columns", "columns"), 2.0);
    assert_eq!(feature("layout_two_columns", "blocks"), 3.0);
    // The stamp is the one graphic; the frame and the free rules are cut out as rulings.
    assert_eq!(feature("layout_form", "graphics"), 1.0);
    assert_eq!(feature("layout_form", "lines"), 9.0);
    // Script family of the pages drawn for it (module M4).
    let families = |page: &str| -> Vec<String> {
        let json = report(&golden_dir().join(format!("layout_inputs/{page}.pgm")));
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["kind"] == "script")
            .map(|f| f["attributes"]["family"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(families("script_cursive"), ["cursive"]);
    assert_eq!(families("script_mixed"), ["headline", "discrete"]);
    assert_eq!(families("layout_one_column"), ["discrete"]);
    // Handwriting regions (module M5): the two fill-ins on their rules and the scribbled line, and
    // none on a printed page.
    let regions = |page: &str| -> usize {
        let json = report(&golden_dir().join(format!("layout_inputs/{page}.pgm")));
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["kind"] == "handwriting")
            .count()
    };
    // Rulings and tables (module M9): the framed box of layout_form is one 2 x 1 table, the free
    // rules are ruling lines; rulings_table is a 4 x 3 table, five checkboxes (two ticked), one
    // underline and a faint crease.
    let found = |page: &str, kind: &str| -> Vec<serde_json::Value> {
        let json = report(&golden_dir().join(format!("layout_inputs/{page}.pgm")));
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["kind"] == kind)
            .cloned()
            .collect()
    };
    let table = &found("layout_form", "table")[0];
    assert_eq!(
        (&table["attributes"]["rows"], &table["attributes"]["cols"]),
        (&2.into(), &1.into())
    );
    let table = &found("rulings_table", "table")[0];
    assert_eq!(
        (&table["attributes"]["rows"], &table["attributes"]["cols"]),
        (&4.into(), &3.into())
    );
    let grid = &found("rulings_table", "checkbox_grid")[0];
    assert_eq!(
        (&grid["attributes"]["boxes"], &grid["attributes"]["checked"]),
        (&5.into(), &2.into())
    );
    assert_eq!(found("rulings_table", "ruling_lines").len(), 1);
    assert_eq!(found("rulings_table", "fold_line").len(), 1);
    assert!(found("layout_one_column", "table").is_empty());
    assert!(found("layout_one_column", "checkbox_grid").is_empty());
    // Signatures (module M7): the flourish on signature_page, and not its black ring stamp; none on
    // the printed pages.
    assert_eq!(found("signature_page", "signature").len(), 1);
    assert!(found("layout_one_column", "signature").is_empty());
    assert!(found("rulings_table", "signature").is_empty());
    // Thumb impressions (module M8): the ridged oval on thumb_page, and not the hatching beside it.
    let prints = found("thumb_page", "thumb_impression");
    assert_eq!(prints.len(), 1);
    let b = prints[0]["bbox"].as_array().unwrap();
    assert!(
        b[0].as_u64().unwrap() >= 150 && b[2].as_u64().unwrap() <= 290,
        "{b:?}"
    );
    assert!(found("layout_one_column", "thumb_impression").is_empty());
    assert!(found("signature_page", "thumb_impression").is_empty());
    assert_eq!(regions("layout_hand_form"), 3);
    assert_eq!(regions("layout_one_column"), 0);
    assert_eq!(regions("layout_form"), 0);
}
