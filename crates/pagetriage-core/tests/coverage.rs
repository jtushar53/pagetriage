//! The `coverage` detector on a synthetic text page (`layout_one_column`, 320 x 260 px, 13 lines):
//! findings follow the text boxes it is given, and a PDF's own boxes (`text_boxes_pt` in the
//! structure) are used when the caller gives none.

use std::fs;
use std::path::PathBuf;

use pagetriage_core::{PageInput, analyse_page};
use serde_json::Value;

fn page() -> (u32, u32, Vec<u8>) {
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

fn run(opts: &str, structural: Option<&str>) -> Value {
    let (w, h, gray) = page();
    let input = PageInput {
        gray: &gray,
        rgb: None,
        width: w,
        height: h,
        dpi: 150,
    };
    serde_json::from_str(&analyse_page(&input, structural, opts).unwrap()).unwrap()
}

fn gap(v: &Value) -> Option<&Value> {
    v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["kind"] == "extraction_gap")
}

fn feature(v: &Value, key: &str) -> f64 {
    v["debug"]["coverage"][key].as_f64().unwrap()
}

#[test]
fn boxes_over_all_the_text_leave_no_gap() {
    let v = run(
        r#"{"debug":true,"detectors":["coverage"],"text_boxes":[[0,0,319,259]]}"#,
        None,
    );
    assert!(feature(&v, "ink_covered_share") > 0.99);
    assert_eq!(feature(&v, "lines"), 13.0);
    assert_eq!(feature(&v, "lines_covered"), 13.0);
    assert!(gap(&v).is_none());
}

#[test]
fn boxes_over_the_top_third_leave_a_gap_below_them() {
    let v = run(
        r#"{"debug":true,"detectors":["coverage"],"text_boxes":[[0,0,319,86]]}"#,
        None,
    );
    let share = feature(&v, "ink_covered_share");
    assert!(share > 0.1 && share < 0.6, "{share}");
    let f = gap(&v).expect("a gap");
    assert!(f["confidence"].as_f64().unwrap() > 0.5);
    assert!(f["bbox"][1].as_u64().unwrap() >= 60, "{}", f["bbox"]);
    assert!(f["attributes"]["lines_uncovered"].as_u64().unwrap() >= 5);
}

#[test]
fn an_empty_list_means_no_text_and_no_list_means_unknown() {
    let v = run(
        r#"{"debug":true,"detectors":["coverage"],"text_boxes":[]}"#,
        None,
    );
    assert_eq!(feature(&v, "ink_covered_share"), 0.0);
    assert!(gap(&v).is_some());
    let v = run(r#"{"debug":true,"detectors":["coverage"]}"#, None);
    assert_eq!(feature(&v, "have_boxes"), 0.0);
    assert!(gap(&v).is_none());
}

#[test]
fn boxes_over_blank_paper_are_counted() {
    let v = run(
        r#"{"debug":true,"detectors":["coverage"],"text_boxes":[[0,0,319,259],[0,0,5,5]]}"#,
        None,
    );
    // The page's corner has no text; the full-page box has some.
    assert_eq!(feature(&v, "boxes"), 2.0);
    assert_eq!(feature(&v, "boxes_without_ink"), 1.0);
}

#[test]
fn a_pdfs_own_boxes_are_used_when_none_are_given() {
    // 150 dpi: 72 points are 150 pixels; the page is 153.6 x 124.8 pt. Boxes over the top third.
    let structural = r#"{"index":0,"size_pt":[153.6,124.8],"rotation":0,"text_layer":null,"script":null,"features":null,"images":[],"conflicts":[],"notes":[],"covered_text":[],"text_boxes_pt":[[0.0,0.0,153.6,41.0]]}"#;
    let v = run(
        r#"{"debug":true,"detectors":["coverage"]}"#,
        Some(structural),
    );
    assert_eq!(feature(&v, "have_boxes"), 1.0);
    assert!(feature(&v, "ink_covered_share") < 0.6);
    assert!(gap(&v).is_some());
    // The caller's boxes win over the PDF's.
    let v = run(
        r#"{"debug":true,"detectors":["coverage"],"text_boxes":[[0,0,319,259]]}"#,
        Some(structural),
    );
    assert!(gap(&v).is_none());
}

/// The report for the top-third case is pinned byte for byte (`tests/golden/coverage_expected/`);
/// the Python and WASM runners check the same file. Regenerate with
/// `UPDATE_GOLDEN=1 cargo test -p pagetriage-core --test coverage`.
#[test]
fn coverage_report_matches_byte_for_byte() {
    let (w, h, gray) = page();
    let input = PageInput {
        gray: &gray,
        rgb: None,
        width: w,
        height: h,
        dpi: 150,
    };
    let opts = r#"{"debug":true,"detectors":["coverage"],"text_boxes":[[0,0,319,86]]}"#;
    let actual = format!("{}\n", analyse_page(&input, None, opts).unwrap());
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/coverage_expected/layout_one_column.json");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&path).expect("run with UPDATE_GOLDEN=1");
    assert_eq!(actual, expected);
}
