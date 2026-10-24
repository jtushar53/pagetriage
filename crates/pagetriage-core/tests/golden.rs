//! Golden snapshots: each `tests/golden/inputs/NAME.pgm` is analysed with `{"debug":true}` at
//! 150 dpi and must match `tests/golden/expected/NAME.json` byte for byte. The Python and WASM
//! runners in `tests/determinism/` check the same files, which is what proves the three builds
//! agree.
//!
//! Regenerate after an intentional change, then review the diff:
//! `UPDATE_GOLDEN=1 cargo test -p pagetriage-core --test golden`

use std::fs;
use std::path::PathBuf;

use pagetriage_core::{PageInput, analyse_page};

const OPTS: &str = r#"{"debug":true}"#;
const DPI: u16 = 150;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden")
}

/// Minimal binary PGM (P5, maxval 255) reader. The header is whitespace-separated tokens followed
/// by exactly one whitespace byte, then the pixels.
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
    assert_eq!(tokens[0], "P5");
    assert_eq!(tokens[3], "255");
    let (w, h): (u32, u32) = (tokens[1].parse().unwrap(), tokens[2].parse().unwrap());
    let pixels = bytes[pos..].to_vec();
    assert_eq!(pixels.len(), (w * h) as usize);
    (w, h, pixels)
}

fn fixtures() -> Vec<(String, PathBuf)> {
    let mut found: Vec<(String, PathBuf)> = fs::read_dir(golden_dir().join("inputs"))
        .expect("tests/golden/inputs exists")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "pgm"))
        .map(|p| (p.file_stem().unwrap().to_string_lossy().into_owned(), p))
        .collect();
    found.sort();
    found
}

#[test]
fn golden_reports_match_byte_for_byte() {
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let expected_dir = golden_dir().join("expected");
    fs::create_dir_all(&expected_dir).unwrap();

    let all = fixtures();
    assert!(
        all.len() >= 4,
        "expected at least 4 golden inputs, found {}",
        all.len()
    );
    for (name, path) in all {
        let (w, h, gray) = parse_pgm(&fs::read(&path).unwrap());
        let input = PageInput {
            gray: &gray,
            rgb: None,
            width: w,
            height: h,
            dpi: DPI,
        };
        let actual = format!("{}\n", analyse_page(&input, None, OPTS).unwrap());

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
fn every_expected_file_has_an_input() {
    let inputs: Vec<String> = fixtures().into_iter().map(|(n, _)| n).collect();
    for entry in fs::read_dir(golden_dir().join("expected"))
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
