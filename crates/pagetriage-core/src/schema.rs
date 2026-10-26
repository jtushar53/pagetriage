//! Output schema `pagetriage/0.1`. These types are the contract: the JSON Schema in
//! `schema/pagetriage-0.1.schema.json` is generated from them, and a test fails if the checked-in
//! file drifts.
//!
//! Fields that later phases fill in (`text_layer`, `quality`, `page_type`, `phash`, ...) exist now
//! so the schema does not break when they arrive; Phase 0 emits them as `null`.

use std::collections::BTreeMap;

use schemars::generate::SchemaSettings;
use schemars::transform::{Transform, transform_subschemas};
use schemars::{JsonSchema, Schema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{Error, Result};
use crate::math;

pub const SCHEMA_ID: &str = "pagetriage/0.1";

/// One named measurement and its signed contribution to a finding's log-odds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    /// Dotted detector id, e.g. `stamp.color_cluster`.
    pub detector: String,
    /// Feature name, e.g. `hue_cluster_compactness`.
    pub feature: String,
    pub value: f64,
    pub threshold: Option<f64>,
    /// Signed log-likelihood-ratio contribution to the finding.
    pub llr: f64,
    pub note: String,
    /// Source the feature or threshold comes from, or "tuned on <set>".
    pub citation: String,
}

impl Evidence {
    /// Builds an evidence record, refusing non-finite numbers so they can never reach the JSON
    /// (`serde_json` would silently turn them into `null`).
    pub fn new(
        detector: &str,
        feature: &str,
        value: f64,
        threshold: Option<f64>,
        llr: f64,
        note: String,
        citation: &str,
    ) -> Result<Self> {
        let what = |part: &str| format!("{detector}.{feature} {part}");
        Ok(Self {
            detector: detector.to_owned(),
            feature: feature.to_owned(),
            value: math::finite(value, &what("value"))?,
            threshold: match threshold {
                Some(t) => Some(math::finite(t, &what("threshold"))?),
                None => None,
            },
            llr: math::finite(llr, &what("llr"))?,
            note,
            citation: citation.to_owned(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    TextLayer,
    Script,
    LanguageHint,
    PrintedText,
    Handwriting,
    Signature,
    Stamp,
    Seal,
    EmbossedSeal,
    ThumbImpression,
    #[serde(rename = "barcode_1d")]
    Barcode1d,
    QrCode,
    EstampQr,
    StampPaperHeader,
    RegistrarEndorsement,
    PageNumberMargin,
    Table,
    RulingLines,
    CheckboxGrid,
    FoldLine,
    Photo,
    VectorGraphic,
    RedactionBox,
    BlankPage,
    /// Text on the page that the supplied text layer or OCR boxes do not cover.
    ExtractionGap,
    /// The page's text does not run left to right with its tops up; `rotate_clockwise_deg` says how to turn it.
    Misoriented,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub kind: Kind,
    /// Calibrated confidence in [0, 1].
    pub confidence: f64,
    /// `[x0, y0, x1, y1]` in page pixel coordinates at the analysis DPI.
    pub bbox: Option<[u32; 4]>,
    pub polygon: Option<Vec<[u32; 2]>>,
    pub attributes: Map<String, Value>,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentInfo {
    pub sha256: String,
    pub pages: u32,
    /// `pdf`, `image` or `raw`.
    pub source: String,
    pub producer: Option<String>,
    pub incremental_updates: Option<u32>,
}

/// Wall-clock timings are measured by the host (CLI, bindings), never by the core: they differ run
/// to run, and `std::time::Instant` is unavailable on `wasm32-unknown-unknown`. The core always
/// leaves this `None`, which keeps the report byte-reproducible.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Timings {
    pub structural: Option<f64>,
    pub pixel: Option<f64>,
    pub fusion: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextLayerState {
    NativeText,
    RasterOnly,
    InvisibleOcrUnderImage,
    Mixed,
    VectorOnly,
    Empty,
}

/// Structural text-layer verdict (module M1, Phase 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextLayer {
    pub state: TextLayerState,
    pub confidence: f64,
    /// False when the layer is legacy-encoded or garbled (module M2).
    pub reliable: Option<bool>,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Binarizer {
    Otsu,
    Sauvola,
    Wolf,
    Howe,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GenerationEstimate {
    pub value: u32,
    /// Capped at 0.6: the method is experimental (module M3).
    pub confidence: f64,
}

/// Degradation triage (module M3, Phase 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Quality {
    pub skew_deg: f64,
    pub blur_laplacian_var: f64,
    pub contrast_rms: f64,
    pub speckle_density: f64,
    pub bleed_through: f64,
    pub fold_lines: u32,
    pub photocopy_generation_est: Option<GenerationEstimate>,
    pub recommended_binarizer: Binarizer,
    /// Letter grade; the scale is defined in Phase 1.
    pub grade: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PageLabel {
    Text,
    TextWithAnnotations,
    Form,
    Table,
    Photo,
    Mixed,
    Blank,
    CoverStampPaper,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageType {
    pub label: PageLabel,
    pub confidence: f64,
    pub alternatives: BTreeMap<String, f64>,
}

/// How much to enlarge or shrink the page for a downstream reader (OCR engines read best at a
/// character size of a few dozen pixels).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanScale {
    /// Measured height of a lower-case letter, pixels, as the page is.
    pub xheight_px: f64,
    pub target_xheight_px: f64,
    /// Multiply the page's size by this (1.0 leaves it).
    pub factor: f64,
    /// The resolution that factor corresponds to, given the analysis dpi.
    pub suggested_dpi: u32,
}

/// A piece of the page and where to send it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanRegion {
    /// `[x0, y0, x1, y1]` in the page as given (before any rotation or deskew), pixels.
    pub bbox: [u32; 4],
    /// `text_block`, `table`, `handwriting`, `signature`, `stamp`, `seal`, `embossed_seal`,
    /// `thumb_impression`, `photo`, `qr_code`, `checkbox_grid` or `redaction_box`.
    pub kind: String,
    /// `ocr`, `table`, `handwriting`, `mark`, `vision`, `decoded`, `decode`, `fields` or `note`.
    pub route: String,
    /// 0 reads first (text, tables, handwriting), 1 next (marks and pictures), 2 last (codes, notes).
    pub priority: u32,
}

/// A tile of a page too big to send whole, cut along empty rows and columns, overlapping its neighbours.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanTile {
    pub bbox: [u32; 4],
    pub row: u32,
    pub column: u32,
}

/// Why a page need not be sent anywhere.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanSkip {
    /// `blank` or `near_duplicate`.
    pub reason: String,
    /// For a near-duplicate: the `index` of the earlier page it repeats.
    pub of: Option<u32>,
}

/// Deterministic instructions for whatever reads this page next (an OCR engine, a handwriting model, a
/// vision-language model): how to turn and scale the page, which pieces to send where, which to skip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PagePlan {
    /// Turn the page this many degrees clockwise first (0, 90, 180 or 270).
    pub rotate_clockwise_deg: u32,
    /// Then turn it this many degrees clockwise to level its lines (the measured skew, so a page whose
    /// text rises to the right is turned clockwise). `None` when the page needs a quarter turn first:
    /// the skew and the regions below were measured on it sideways.
    pub deskew_clockwise_deg: Option<f64>,
    /// The page was read sideways or upside down: analyse the rotated image again for regions.
    pub needs_rerun: bool,
    pub scale: PlanScale,
    /// Box of everything worth reading, with a margin, in the page as given; the whole page if nothing
    /// was found.
    pub content_bbox: [u32; 4],
    /// The binarisation method to use before OCR (module M3).
    pub binarizer: String,
    /// 0 (easy) to 1 (hard): a rough budget guide, not a probability.
    pub difficulty: f64,
    pub skip: Option<PlanSkip>,
    /// Scripts the page is probably written in, from the script detector, most likely first.
    pub scripts: Vec<String>,
    pub regions: Vec<PlanRegion>,
    pub tiles: Vec<PlanTile>,
    /// SHA-256 of the pixels, size and resolution: the same page always has the same key.
    pub input_sha256: String,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageReport {
    pub index: u32,
    pub size_pt: [f64; 2],
    pub analysis_dpi: u16,
    pub timings_ms: Option<Timings>,
    pub text_layer: Option<TextLayer>,
    pub quality: Option<Quality>,
    pub findings: Vec<Finding>,
    pub page_type: Option<PageType>,
    /// What to do with the page next; present when the `plan` detector is selected.
    pub plan: Option<PagePlan>,
    /// 64-bit perceptual hash as 16 lowercase hex characters (module M12); all zeros for a page
    /// with no structure to hash.
    pub phash: Option<String>,
    /// 64-bit difference hash, same format as `phash`.
    pub dhash: Option<String>,
    /// `index` of the earliest earlier page whose hashes are within the near-duplicate distance.
    /// One page cannot know its twins, so this is filled by the document-level step
    /// (`pixel::phash::mark_near_duplicates`), not by `analyse_page`.
    pub near_duplicate_of: Option<u32>,
    /// Raw detector feature values, present only when `debug` is requested. Keyed
    /// `detector -> feature -> value`; this is what `bench/` uses to tune `params/*.toml`.
    pub debug: Option<BTreeMap<String, BTreeMap<String, f64>>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Report {
    /// Always [`SCHEMA_ID`].
    pub schema: String,
    pub document: DocumentInfo,
    pub pages: Vec<PageReport>,
}

impl Report {
    pub fn new(document: DocumentInfo, pages: Vec<PageReport>) -> Self {
        Self {
            schema: SCHEMA_ID.to_owned(),
            document,
            pages,
        }
    }
}

/// Our types never skip a field when serialising: an unset `Option` is an explicit `null`, so every
/// declared property is always present. `schemars` would mark `Option` fields as not required;
/// this transform tightens that so consumers can rely on the key being there. (The obvious
/// `#[schemars(required)]` attribute is not an option: it also drops `null` from the field's type.)
#[derive(Clone)]
struct RequireAllProperties;

impl Transform for RequireAllProperties {
    fn transform(&mut self, schema: &mut Schema) {
        transform_subschemas(self, schema);
        let Some(object) = schema.as_object_mut() else {
            return;
        };
        let names: Option<Vec<Value>> = match object.get("properties") {
            Some(Value::Object(properties)) => Some(
                properties
                    .keys()
                    .map(|k| Value::String(k.clone()))
                    .collect(),
            ),
            _ => None,
        };
        if let Some(names) = names {
            object.insert("required".into(), Value::Array(names));
        }
    }
}

/// JSON Schema (draft 2020-12) for [`Report`], pretty-printed with a trailing newline.
pub fn schema_json() -> String {
    let schema = SchemaSettings::draft2020_12()
        .with_transform(RequireAllProperties)
        .into_generator()
        .into_root_schema_for::<Report>();
    let mut s = serde_json::to_string_pretty(&schema).expect("a schema always serialises");
    s.push('\n');
    s
}

/// JSON Schema (draft 2020-12) for the PDF structure report (`analyse_pdf_structure`), pretty-printed
/// with a trailing newline. Checked in as `schema/pagetriage-structure-0.1.schema.json`.
pub fn structure_schema_json() -> String {
    let schema = SchemaSettings::draft2020_12()
        .with_transform(RequireAllProperties)
        .into_generator()
        .into_root_schema_for::<crate::structural::types::StructureReport>();
    let mut s = serde_json::to_string_pretty(&schema).expect("a schema always serialises");
    s.push('\n');
    s
}

/// The canonical byte form of any report value: object keys sorted, every float rounded to
/// [`math::OUTPUT_DECIMALS`] decimals, `-0.0` normalised, compact. This is what the determinism
/// test compares across native, Python and WASM.
pub fn to_canonical_json<T: Serialize>(value: &T) -> Result<String> {
    let mut v = serde_json::to_value(value)
        .map_err(|e| Error::InvalidInput(format!("report is not serialisable: {e}")))?;
    canonicalise(&mut v)?;
    // `serde_json::Map` is a BTreeMap here (no `preserve_order`), so keys come out sorted;
    // `canonical_form_has_sorted_keys` guards against another crate turning that feature on.
    serde_json::to_string(&v)
        .map_err(|e| Error::InvalidInput(format!("report is not serialisable: {e}")))
}

fn canonicalise(v: &mut Value) -> Result<()> {
    match v {
        Value::Number(n) if n.is_f64() => {
            let x = n.as_f64().expect("is_f64 checked");
            let r = math::round_output(math::finite(x, "report number")?);
            *v = Value::Number(
                serde_json::Number::from_f64(r)
                    .ok_or_else(|| Error::NonFinite(format!("report number {r}")))?,
            );
        }
        Value::Array(items) => {
            for item in items {
                canonicalise(item)?;
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                canonicalise(item)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_form_has_sorted_keys() {
        let mut m = Map::new();
        m.insert("zeta".into(), Value::from(1));
        m.insert("alpha".into(), Value::from(2));
        let s = to_canonical_json(&Value::Object(m)).unwrap();
        assert_eq!(s, r#"{"alpha":2,"zeta":1}"#);
    }

    #[test]
    fn canonical_form_rounds_floats_but_not_integers() {
        let v = serde_json::json!({"f": 0.123_456_789, "i": 12_345_678_901_u64, "neg0": -0.0});
        let s = to_canonical_json(&v).unwrap();
        assert_eq!(s, r#"{"f":0.123457,"i":12345678901,"neg0":0.0}"#);
    }

    #[test]
    fn evidence_new_rejects_non_finite() {
        let bad = Evidence::new("d", "f", f64::NAN, None, 0.0, String::new(), "c");
        assert!(matches!(bad, Err(Error::NonFinite(_))));
        let bad = Evidence::new("d", "f", 1.0, Some(f64::INFINITY), 0.0, String::new(), "c");
        assert!(matches!(bad, Err(Error::NonFinite(_))));
    }

    #[test]
    fn kind_names_match_the_plan() {
        let names: Vec<String> = [
            Kind::Barcode1d,
            Kind::EstampQr,
            Kind::BlankPage,
            Kind::RulingLines,
        ]
        .iter()
        .map(|k| {
            serde_json::to_value(k)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
        assert_eq!(
            names,
            ["barcode_1d", "estamp_qr", "blank_page", "ruling_lines"]
        );
    }
}
