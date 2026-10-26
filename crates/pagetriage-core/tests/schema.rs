//! The JSON Schema is generated from the Rust types and checked in. These tests keep the three
//! in step: types -> schema file -> actual analyser output.
//!
//! To regenerate the checked-in file after changing the types:
//! `UPDATE_SCHEMA=1 cargo test -p pagetriage-core --test schema`

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use pagetriage_core::schema::{
    Binarizer, DocumentInfo, Evidence, GenerationEstimate, PageLabel, PagePlan, PageType,
    PlanRegion, PlanScale, PlanSkip, PlanTile, Quality, TextLayer, TextLayerState, Timings,
};
use pagetriage_core::{
    Finding, Kind, PageInput, PageReport, Report, analyse_page, schema_json, to_canonical_json,
};
use serde_json::{Value, json};

fn schema_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/pagetriage-0.1.schema.json")
}

fn validator() -> jsonschema::Validator {
    let schema: Value = serde_json::from_str(&schema_json()).unwrap();
    jsonschema::validator_for(&schema).expect("generated schema is itself valid")
}

fn document() -> DocumentInfo {
    DocumentInfo {
        sha256: "0".repeat(64),
        pages: 1,
        source: "image".into(),
        producer: None,
        incremental_updates: None,
    }
}

fn analyse_to_value(gray: &[u8], w: u32, h: u32, opts: &str) -> Value {
    let input = PageInput {
        gray,
        rgb: None,
        width: w,
        height: h,
        dpi: 150,
    };
    serde_json::from_str(&analyse_page(&input, None, opts).unwrap()).unwrap()
}

fn wrap(page: Value) -> Value {
    json!({
        "schema": "pagetriage/0.1",
        "document": serde_json::to_value(document()).unwrap(),
        "pages": [page],
    })
}

#[test]
fn checked_in_schema_matches_the_rust_types() {
    let generated = schema_json();
    if std::env::var_os("UPDATE_SCHEMA").is_some() {
        fs::write(schema_path(), &generated).unwrap();
        return;
    }
    let on_disk = fs::read_to_string(schema_path())
        .expect("schema/pagetriage-0.1.schema.json is missing; run with UPDATE_SCHEMA=1");
    assert_eq!(
        on_disk, generated,
        "schema file is stale; regenerate with `UPDATE_SCHEMA=1 cargo test -p pagetriage-core --test schema`"
    );
}

#[test]
fn analyser_output_validates_against_the_schema() {
    let v = validator();
    let blank = vec![250u8; 30 * 40];
    let mut inked = blank.clone();
    inked.iter_mut().take(300).for_each(|p| *p = 10);

    for gray in [&blank, &inked] {
        for opts in ["{}", r#"{"debug":true}"#, r#"{"detectors":[]}"#] {
            let doc = wrap(analyse_to_value(gray, 30, 40, opts));
            let errors: Vec<String> = v.iter_errors(&doc).map(|e| e.to_string()).collect();
            assert!(errors.is_empty(), "opts {opts}: {errors:?}\n{doc}");
        }
    }
}

#[test]
fn every_option_field_is_present_as_null_not_omitted() {
    // No detectors, so hash fields stay unset (`phash` and `dhash` are filled by the phash detector).
    let page = analyse_to_value(&[250u8; 16], 4, 4, r#"{"detectors":[]}"#);
    for key in [
        "timings_ms",
        "text_layer",
        "page_type",
        "phash",
        "dhash",
        "near_duplicate_of",
        "debug",
    ] {
        assert!(
            page.get(key).is_some_and(Value::is_null),
            "{key} should be an explicit null"
        );
    }
    // The quality block (module M3) is always filled in, whatever detectors were selected.
    assert!(page.get("quality").is_some_and(Value::is_object));
}

#[test]
fn a_fully_populated_report_validates_and_round_trips() {
    let evidence = Evidence::new(
        "stamp.color_cluster",
        "hue_cluster_compactness",
        0.41,
        Some(0.3),
        1.7,
        "blue-violet cluster, 4.1% of ink pixels".into(),
        "Micenková & van Beusekom 2011",
    )
    .unwrap();
    let mut attributes = serde_json::Map::new();
    attributes.insert("color".into(), json!("violet"));
    let finding = Finding {
        kind: Kind::Stamp,
        confidence: 0.88,
        bbox: Some([10, 20, 110, 120]),
        polygon: Some(vec![[10, 20], [110, 20], [110, 120], [10, 120]]),
        attributes,
        evidence: vec![evidence.clone()],
    };
    let page = PageReport {
        index: 3,
        size_pt: [595.0, 842.0],
        analysis_dpi: 150,
        timings_ms: Some(Timings {
            structural: Some(3.0),
            pixel: Some(210.0),
            fusion: None,
        }),
        text_layer: Some(TextLayer {
            state: TextLayerState::InvisibleOcrUnderImage,
            confidence: 0.97,
            reliable: Some(true),
            evidence: vec![evidence],
        }),
        quality: Some(Quality {
            skew_deg: 1.8,
            blur_laplacian_var: 212.4,
            contrast_rms: 0.21,
            speckle_density: 0.004,
            bleed_through: 0.31,
            fold_lines: 1,
            photocopy_generation_est: Some(GenerationEstimate {
                value: 3,
                confidence: 0.42,
            }),
            recommended_binarizer: Binarizer::Howe,
            grade: "C".into(),
        }),
        findings: vec![finding],
        page_type: Some(PageType {
            label: PageLabel::TextWithAnnotations,
            confidence: 0.77,
            alternatives: BTreeMap::from([("form".into(), 0.15), ("mixed".into(), 0.08)]),
        }),
        plan: Some(PagePlan {
            rotate_clockwise_deg: 90,
            deskew_clockwise_deg: None,
            needs_rerun: true,
            scale: PlanScale {
                xheight_px: 11.0,
                target_xheight_px: 24.0,
                factor: 2.18,
                suggested_dpi: 327,
            },
            content_bbox: [0, 0, 100, 200],
            binarizer: "sauvola".into(),
            difficulty: 0.3,
            skip: Some(PlanSkip {
                reason: "near_duplicate".into(),
                of: Some(1),
            }),
            scripts: vec!["latin".into()],
            regions: vec![PlanRegion {
                bbox: [1, 2, 3, 4],
                kind: "text_block".into(),
                route: "ocr".into(),
                priority: 0,
            }],
            tiles: vec![PlanTile {
                bbox: [0, 0, 10, 10],
                row: 0,
                column: 0,
            }],
            input_sha256: "ab".into(),
            notes: vec![],
        }),
        phash: Some("c3a1".into()),
        dhash: Some("9f2e".into()),
        near_duplicate_of: Some(1),
        debug: Some(BTreeMap::from([(
            "blank_page".into(),
            BTreeMap::from([("ink_coverage".into(), 0.07)]),
        )])),
    };
    let report = Report::new(document(), vec![page]);

    let canonical = to_canonical_json(&report).unwrap();
    let value: Value = serde_json::from_str(&canonical).unwrap();
    let errors: Vec<String> = validator()
        .iter_errors(&value)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");

    let back: Report = serde_json::from_str(&canonical).unwrap();
    assert_eq!(back, report);
}

#[test]
fn schema_rejects_malformed_reports() {
    let v = validator();
    let good = wrap(analyse_to_value(&[250u8; 16], 4, 4, "{}"));
    assert!(v.is_valid(&good));

    let mut unknown_key = good.clone();
    unknown_key["pages"][0]["surprise"] = json!(1);
    assert!(!v.is_valid(&unknown_key), "unknown keys must be rejected");

    let mut missing_key = good.clone();
    missing_key["pages"][0]
        .as_object_mut()
        .unwrap()
        .remove("phash");
    assert!(!v.is_valid(&missing_key), "Option fields must be present");

    let mut bad_kind = good.clone();
    bad_kind["pages"][0]["findings"][0]["kind"] = json!("hologram");
    assert!(!v.is_valid(&bad_kind), "kind is a closed enum");

    let mut out_of_range = good.clone();
    out_of_range["pages"][0]["analysis_dpi"] = json!(70_000);
    assert!(!v.is_valid(&out_of_range), "dpi is a u16");

    let mut wrong_type = good;
    wrong_type["pages"][0]["findings"][0]["confidence"] = json!("high");
    assert!(!v.is_valid(&wrong_type));
}
