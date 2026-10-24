//! The JSON Schema of the PDF structure report is generated from the Rust types and checked in.
//!
//! To regenerate the checked-in file after changing the types in `structural/types.rs`:
//! `UPDATE_SCHEMA=1 cargo test -p pagetriage-core --test structure_schema`

use std::fs;
use std::path::PathBuf;

use pagetriage_core::schema::structure_schema_json;

fn schema_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../schema/pagetriage-structure-0.1.schema.json")
}

#[test]
fn checked_in_structure_schema_matches_the_rust_types() {
    let generated = structure_schema_json();
    if std::env::var_os("UPDATE_SCHEMA").is_some() {
        fs::write(schema_path(), &generated).unwrap();
        return;
    }
    let on_disk = fs::read_to_string(schema_path())
        .expect("schema/pagetriage-structure-0.1.schema.json is missing; run with UPDATE_SCHEMA=1");
    assert_eq!(
        on_disk, generated,
        "structure schema is stale; regenerate with `UPDATE_SCHEMA=1 cargo test -p pagetriage-core --test structure_schema`"
    );
}

#[cfg(feature = "pdf")]
mod support;

#[cfg(feature = "pdf")]
mod with_pdf {
    use super::support;
    use super::*;
    use pagetriage_core::structural::{StructureReport, analyse_pdf_structure};
    use serde_json::Value;

    #[test]
    fn analyser_output_validates_and_round_trips() {
        let schema: Value = serde_json::from_str(&structure_schema_json()).unwrap();
        let validator = jsonschema::validator_for(&schema).expect("schema is valid");
        let pdfs = [
            support::Page::new("BT /F1 12 Tf 72 700 Td (Some visible text on the page) Tj ET")
                .build(Some("Test")),
            support::Page::new("q Q").build(None),
        ];
        for pdf in pdfs {
            let json = analyse_pdf_structure(&pdf, "{}").unwrap();
            let doc: Value = serde_json::from_str(&json).unwrap();
            let errors: Vec<String> = validator.iter_errors(&doc).map(|e| e.to_string()).collect();
            assert!(errors.is_empty(), "{errors:?}\n{json}");
            // Strict types: the output parses back into the typed report.
            let typed: StructureReport = serde_json::from_str(&json).unwrap();
            assert_eq!(typed.schema, "pagetriage-structure/0.1");
            // And a page element is exactly what analyse_page accepts as structural_json.
            let page = serde_json::to_string(&doc["pages"][0]).unwrap();
            let gray = vec![250u8; 16];
            let input = pagetriage_core::PageInput {
                gray: &gray,
                rgb: None,
                width: 4,
                height: 4,
                dpi: 150,
            };
            let report =
                pagetriage_core::analyse_page_report(&input, Some(&page), r#"{"detectors":[]}"#)
                    .unwrap();
            assert!(report.text_layer.is_some());
        }
    }
}
