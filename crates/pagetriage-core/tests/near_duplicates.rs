//! Golden snapshot of the document-level step (module M12): the golden inputs, taken in file-name
//! order as the pages of one document, are hashed through `analyse_page`, and the hashes, the
//! pairwise pHash Hamming distances and `near_duplicates` must match
//! `tests/golden/near_duplicates.json` byte for byte. The per-page hash strings are also inside
//! every report in `tests/golden/expected/`, which the Python and WASM runners compare; the
//! document-level step is integer-only, so the same hashes give the same answer on every build.
//!
//! Regenerate after an intentional change, then review the diff:
//! `UPDATE_GOLDEN=1 cargo test -p pagetriage-core --test near_duplicates`

use std::fs;
use std::path::PathBuf;

use pagetriage_core::pixel::phash::{PageHashes, hamming, near_duplicates};
use pagetriage_core::{PageInput, Params, analyse_page_report};
use serde_json::json;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden")
}

/// Binary PGM (P5, maxval 255): four whitespace-separated header tokens, one whitespace byte, then
/// the pixels.
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
    (w, h, bytes[pos..].to_vec())
}

#[test]
fn document_level_near_duplicates_match_the_golden_file() {
    let mut paths: Vec<PathBuf> = fs::read_dir(golden_dir().join("inputs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "pgm"))
        .collect();
    paths.sort();

    let params = Params::defaults().unwrap().phash;
    let mut names = Vec::new();
    let mut hashes = Vec::new();
    for path in &paths {
        let (w, h, gray) = parse_pgm(&fs::read(path).unwrap());
        let input = PageInput {
            gray: &gray,
            rgb: None,
            width: w,
            height: h,
            dpi: 150,
        };
        let report = analyse_page_report(&input, None, r#"{"detectors":["phash"]}"#).unwrap();
        names.push(path.file_stem().unwrap().to_string_lossy().into_owned());
        hashes.push(PageHashes::from_hex(&report.phash.unwrap(), &report.dhash.unwrap()).unwrap());
    }

    let distances: Vec<Vec<u32>> = hashes
        .iter()
        .map(|a| hashes.iter().map(|b| hamming(a.phash, b.phash)).collect())
        .collect();
    let dhash_distances: Vec<Vec<u32>> = hashes
        .iter()
        .map(|a| hashes.iter().map(|b| hamming(a.dhash, b.dhash)).collect())
        .collect();
    let value = json!({
        "max_hamming": params.max_hamming,
        "dhash_max_hamming": params.dhash_max_hamming,
        "pages": names,
        "phash": hashes.iter().map(PageHashes::phash_hex).collect::<Vec<_>>(),
        "dhash": hashes.iter().map(PageHashes::dhash_hex).collect::<Vec<_>>(),
        "phash_hamming": distances,
        "dhash_hamming": dhash_distances,
        "near_duplicate_of": near_duplicates(&hashes, &params),
    });
    let actual = format!("{}\n", serde_json::to_string_pretty(&value).unwrap());

    let path = golden_dir().join("near_duplicates.json");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::write(&path, &actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing {}; run with UPDATE_GOLDEN=1", path.display()));
    assert_eq!(actual, expected, "near-duplicate golden mismatch");
}
