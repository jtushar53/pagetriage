//! Golden snapshots for the PDF structural layer: each `tests/golden/pdf/NAME.pdf` (synthetic,
//! made by `tests/golden/gen_pdfs.py`) is analysed with default options and must match
//! `tests/golden/pdf_expected/NAME.json` byte for byte. The Python and WASM runners in
//! `tests/determinism/` check the same files, which is what shows the three builds agree.
//!
//! Regenerate after an intentional change, then review the diff:
//! `UPDATE_GOLDEN=1 cargo test -p pagetriage-core --features pdf --test golden_pdf`
#![cfg(feature = "pdf")]

use std::fs;
use std::path::PathBuf;

use pagetriage_core::structural::analyse_pdf_structure;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden")
}

fn fixtures() -> Vec<(String, PathBuf)> {
    let mut found: Vec<(String, PathBuf)> = fs::read_dir(golden_dir().join("pdf"))
        .expect("tests/golden/pdf exists")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "pdf"))
        .map(|p| (p.file_stem().unwrap().to_string_lossy().into_owned(), p))
        .collect();
    found.sort();
    found
}

#[test]
fn golden_structure_reports_match_byte_for_byte() {
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let expected_dir = golden_dir().join("pdf_expected");
    fs::create_dir_all(&expected_dir).unwrap();
    let all = fixtures();
    assert!(
        all.len() >= 6,
        "expected at least 6 golden PDFs, found {}",
        all.len()
    );
    for (name, path) in all {
        let actual = format!(
            "{}\n",
            analyse_pdf_structure(&fs::read(&path).unwrap(), "{}").unwrap()
        );
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
        assert_eq!(actual, expected, "golden mismatch for {name}");
    }
}

#[test]
fn every_expected_structure_file_has_an_input() {
    let inputs: Vec<String> = fixtures().into_iter().map(|(n, _)| n).collect();
    for entry in fs::read_dir(golden_dir().join("pdf_expected"))
        .into_iter()
        .flatten()
    {
        let path = entry.unwrap().path();
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        assert!(
            inputs.contains(&stem),
            "orphaned golden file {}",
            path.display()
        );
    }
}

#[test]
fn the_encrypted_fixture_degrades_instead_of_failing() {
    let bytes = fs::read(golden_dir().join("pdf/encrypted.pdf")).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&analyse_pdf_structure(&bytes, "{}").unwrap()).unwrap();
    assert_eq!(v["degraded"], true);
    assert_eq!(v["structure"]["encrypted"], true);
    assert!(v["pages"][0]["text_layer"].is_null());
}
