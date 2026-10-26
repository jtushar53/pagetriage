//! Golden test of the document-level pass (`finish_document`): the page reports of a synthetic
//! five-page deed in `tests/golden/document_inputs/` must come back as the file in
//! `tests/golden/document_expected/`, byte for byte. The Python and WASM runners check the same
//! files. After an intended change:
//! `UPDATE_GOLDEN=1 cargo test -p pagetriage-core --test document_golden`, then review the diff.

use std::fs;
use std::path::PathBuf;

use pagetriage_core::finish_document;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden")
}

fn run() -> String {
    let input = fs::read_to_string(golden_dir().join("document_inputs/deed_pages.json")).unwrap();
    format!("{}\n", finish_document(&input, "{}").unwrap())
}

#[test]
fn document_pass_matches_byte_for_byte() {
    let actual = run();
    let path = golden_dir().join("document_expected/deed_pages.json");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing {}; run with UPDATE_GOLDEN=1", path.display()));
    assert_eq!(actual, expected, "document golden mismatch");
}

#[test]
fn the_deed_has_its_endorsements_and_duplicate() {
    let pages: Vec<serde_json::Value> = serde_json::from_str(&run()).unwrap();
    let endorsed: Vec<usize> = pages
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            p["findings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["kind"] == "registrar_endorsement")
        })
        .map(|(i, _)| i)
        .collect();
    assert_eq!(endorsed, vec![0, 1, 2, 4]);
    assert_eq!(pages[4]["near_duplicate_of"], 0);
    assert!(pages[0]["near_duplicate_of"].is_null());
    // The seal and the signatures are not repeated enough to count.
    for p in &pages {
        for f in p["findings"].as_array().unwrap() {
            if f["kind"] == "registrar_endorsement" {
                assert_eq!(f["attributes"]["mark"], "stamp");
            }
        }
    }
}

#[test]
fn finishing_twice_changes_nothing() {
    let once = run();
    let twice = finish_document(once.trim_end(), "{}").unwrap();
    assert_eq!(format!("{twice}\n"), once);
}

#[test]
fn bad_input_is_an_error() {
    assert!(finish_document("not json", "{}").is_err());
    assert!(finish_document("{}", "{}").is_err());
    assert!(finish_document("[]", r#"{"nonsense":1}"#).is_err());
    assert_eq!(finish_document("[]", "{}").unwrap(), "[]");
}
