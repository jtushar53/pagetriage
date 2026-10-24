//! Binariser golden hashes: every golden input (`inputs/` and `binarize_inputs/`) is binarised with every case in
//! `tests/golden/binarize_cases.json`, and the SHA-256 of the 0/1 ink bytes must match
//! `tests/golden/binarize_expected.json`. The Python and WASM runners in `tests/determinism/` check
//! the same file, which is what proves the three builds binarise identically (Howe included: its
//! graph cut is integer arithmetic and its float steps use only IEEE operations and the libm crate).
//!
//! Regenerate after an intentional change, then review the diff:
//! `UPDATE_GOLDEN=1 cargo test -p pagetriage-core --test binarize_golden`

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use pagetriage_core::Params;
use pagetriage_core::hash::sha256_hex;
use pagetriage_core::pixel::binarize::{binarize, parse_method};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    method: String,
    params: String,
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden")
}

/// Minimal binary PGM (P5, maxval 255) reader.
fn parse_pgm(bytes: &[u8]) -> (usize, usize, Vec<u8>) {
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
    let (w, h): (usize, usize) = (tokens[1].parse().unwrap(), tokens[2].parse().unwrap());
    let pixels = bytes[pos..].to_vec();
    assert_eq!(pixels.len(), w * h);
    (w, h, pixels)
}

#[test]
fn binariser_outputs_match_the_golden_hashes() {
    let dir = golden_dir();
    let cases: Vec<Case> =
        serde_json::from_str(&fs::read_to_string(dir.join("binarize_cases.json")).unwrap())
            .unwrap();
    let mut inputs: Vec<PathBuf> = ["inputs", "binarize_inputs"]
        .iter()
        .flat_map(|sub| fs::read_dir(dir.join(sub)).unwrap())
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "pgm"))
        .collect();
    inputs.sort();
    assert!(inputs.len() >= 6, "found {} inputs", inputs.len());

    let mut actual: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for path in &inputs {
        let (w, h, gray) = parse_pgm(&fs::read(path).unwrap());
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        for case in &cases {
            let params = if case.params.is_empty() {
                Params::defaults().unwrap()
            } else {
                Params::with_overrides(&case.params).unwrap()
            };
            let ink = binarize(&gray, w, h, parse_method(&case.method).unwrap(), &params).unwrap();
            assert!(ink.iter().all(|&v| v <= 1));
            actual
                .entry(stem.clone())
                .or_default()
                .insert(case.name.clone(), sha256_hex(&ink));
        }
    }

    let rendered = serde_json::to_string_pretty(&actual).unwrap() + "\n";
    let expected_path = dir.join("binarize_expected.json");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::write(&expected_path, &rendered).unwrap();
        return;
    }
    let expected = fs::read_to_string(&expected_path)
        .expect("tests/golden/binarize_expected.json missing; run with UPDATE_GOLDEN=1");
    assert_eq!(
        rendered, expected,
        "binariser output changed; review it, then regenerate with UPDATE_GOLDEN=1"
    );
}
