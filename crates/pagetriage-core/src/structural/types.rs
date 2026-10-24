//! Output types of the structural layer: what `analyse_pdf_structure` returns and what
//! `analyse_page`'s `structural_json` argument accepts (one [`PageStructure`] object).
//!
//! The JSON Schema for [`StructureReport`] is generated from these types
//! (`schema::structure_schema_json`) and checked in as `schema/pagetriage-structure-0.1.schema.json`.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::schema::{DocumentInfo, Evidence, TextLayer};

/// Schema id of [`StructureReport`].
pub const STRUCTURE_SCHEMA_ID: &str = "pagetriage-structure/0.1";

/// One font used on a page (module M2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FontInfo {
    /// `/BaseFont` without the subset prefix, truncated.
    pub name: String,
    /// `Type1`, `TrueType`, `Type0`, `Type3`, ... as written in the file.
    pub subtype: String,
    pub embedded: bool,
    pub to_unicode: bool,
    /// Base encoding name, or `Identity-H`, a predefined CMap name, or `None` for absent.
    pub encoding: Option<String>,
    /// Non-space glyphs shown with this font on the page.
    pub glyphs: u32,
    /// Of those, glyphs that could not be mapped to Unicode.
    pub undecodable_glyphs: u32,
    pub legacy_encoded: bool,
    /// The curated list entry the name matched, if any.
    pub legacy_name_match: Option<String>,
    /// Confidence in [0, 1] that the font is legacy-encoded.
    pub legacy_confidence: f64,
}

/// Script shares and legacy-font verdict for one page (module M2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScriptInfo {
    /// Machine id of the dominant script (`latin`, `devanagari`, ...), `none` when the page has no
    /// decodable letters, or `unknown` when the text exists but cannot be decoded.
    pub dominant: String,
    /// Display name; `Devanagari (legacy-encoded)` for the legacy case.
    pub name: String,
    /// Share of decoded letters per script, summing to 1 (empty when there are no letters). For a
    /// legacy-encoded page the legacy glyphs are counted under the script they really write.
    pub shares: BTreeMap<String, f64>,
    /// Decoded letters behind the shares.
    pub letters: u32,
    pub glyphs_undecodable: u32,
    pub legacy_encoded: bool,
    /// Script the legacy fonts write, when `legacy_encoded`.
    pub legacy_script: Option<String>,
    /// Share of the page's glyphs set in legacy-encoded fonts.
    pub legacy_glyph_share: f64,
    pub fonts: Vec<FontInfo>,
    pub evidence: Vec<Evidence>,
}

/// JPEG quantisation tables and encoder fingerprint of one `/DCTDecode` image (module M12).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JpegInfo {
    /// First 16 bytes (hex) of SHA-256 over the tables in id order.
    pub table_hash: String,
    pub tables: u32,
    /// Best-fitting IJG (libjpeg) quality of the luminance table.
    pub luma_quality: Option<u32>,
    /// Mean absolute difference per entry between that table and the fitted IJG table.
    pub luma_fit_error: Option<f64>,
    /// True when the luminance table is an IJG scaling of the Annex K table (fit error within
    /// `ijg_fit_mean_abs_error`); false for custom tables.
    pub ijg_scaled: bool,
    pub chroma_quality: Option<u32>,
    pub width: u32,
    pub height: u32,
    pub components: u32,
    /// `baseline`, `extended`, `progressive` or `other`.
    pub coding: String,
    pub app_markers: Vec<String>,
}

/// One image drawn on a page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImageInfo {
    /// Object number of the image XObject; `None` for inline images.
    pub object: Option<u32>,
    pub width: u32,
    pub height: u32,
    /// Last filter of the stream (`DCTDecode`, `FlateDecode`, `CCITTFaxDecode`, ...) or `none`.
    pub filter: String,
    pub color_space: String,
    pub bits_per_component: u32,
    pub is_mask: bool,
    /// Has a soft mask or colour-key mask (may be transparent).
    pub has_mask: bool,
    /// Fraction of the page box the image's drawn rectangle covers (clipped to the page).
    pub cover_fraction: f64,
    /// Effective resolution along the image's width, from pixels over drawn inches.
    pub dpi: Option<f64>,
    pub jpeg: Option<JpegInfo>,
}

/// Counts behind the text-layer verdict, for tuning and for the page-level fusion later on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageFeatures {
    /// Non-space glyphs shown on the page, forms included.
    pub glyphs: u32,
    /// Glyphs drawn with text render mode 3 (invisible) or 7 (clip only).
    pub glyphs_invisible_mode: u32,
    /// Visible-mode glyphs that a later opaque page-covering image hides.
    pub glyphs_under_image: u32,
    /// Glyphs (any mode) whose text origin lies inside an image's box.
    pub glyphs_in_image: u32,
    pub text_ops: u32,
    /// Union of the drawn images' boxes over the page box.
    pub image_cover: f64,
    pub image_count: u32,
    pub paint_ops: u32,
    pub small_filled_paths: u32,
    pub outlined_text_suspected: bool,
    /// Content was cut short by a limit; the numbers are lower bounds.
    pub truncated: bool,
}

/// The structural result for one page: what `analyse_page`'s `structural_json` accepts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageStructure {
    /// 0-based position in the document.
    pub index: u32,
    pub size_pt: [f64; 2],
    pub rotation: u32,
    /// `None` when the page could not be analysed (see `notes`).
    pub text_layer: Option<TextLayer>,
    pub script: Option<ScriptInfo>,
    pub features: Option<PageFeatures>,
    pub images: Vec<ImageInfo>,
    /// Inconsistencies between the page's own signals, e.g. `raster_only+legacy_devanagari`.
    pub conflicts: Vec<String>,
    pub notes: Vec<String>,
}

/// What a later revision changed relative to the one before it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevisionInfo {
    /// 1 for the first incremental update (revision 0 is the original file).
    pub index: u32,
    /// Byte offset just past this revision's `%%EOF`.
    pub end_offset: u64,
    pub objects_added: u32,
    pub objects_changed: u32,
    pub objects_freed: u32,
    /// Sorted change kinds: `page_content_changed`, `image_changed`, `annotation_added`,
    /// `annotation_changed`, `form_field_changed`, `signature_added`, `page_tree_changed`,
    /// `metadata_changed`, `font_changed`, `other_changed`.
    pub kinds: Vec<String>,
    /// 0-based pages whose content stream or resources changed.
    pub content_changed_pages: Vec<u32>,
}

/// Document-level provenance (module M12, structure part).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentStructure {
    pub pdf_version: String,
    pub creator: Option<String>,
    pub creation_date: Option<String>,
    pub mod_date: Option<String>,
    pub xmp_present: bool,
    /// `xmp:CreatorTool` / `pdf:Producer` from the XMP packet when present.
    pub xmp_tool: Option<String>,
    /// Family from the producer fingerprint list, e.g. `ocrmypdf`.
    pub producer_family: Option<String>,
    /// The producer or creator names a text-layer (OCR) tool.
    pub ocr_producer: bool,
    /// `classic`, `stream`, `hybrid` or `unknown`.
    pub xref_style: String,
    pub linearized: bool,
    pub encrypted: bool,
    /// `%%EOF` markers found.
    pub eof_markers: u32,
    pub revisions: Vec<RevisionInfo>,
    /// Older revisions were not examined (limit).
    pub revisions_truncated: bool,
    pub signature_count: u32,
    /// Page content, images or the page tree changed in a revision after a signature.
    pub tamper_signal: bool,
    pub tamper_reasons: Vec<String>,
    /// Number of JPEG (`/DCTDecode`) images examined.
    pub jpeg_images: u32,
    /// Distinct quantisation-table hashes with the number of images using each.
    pub jpeg_table_hashes: BTreeMap<String, u32>,
    /// Pages per text-layer state.
    pub page_states: BTreeMap<String, u32>,
    /// Pages whose text layer is unreliable (legacy-encoded or undecodable).
    pub unreliable_pages: u32,
}

/// The result of `analyse_pdf_structure`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StructureReport {
    /// Always [`STRUCTURE_SCHEMA_ID`].
    pub schema: String,
    /// `source` is `pdf`; `producer` and `incremental_updates` are filled in.
    pub document: DocumentInfo,
    pub structure: DocumentStructure,
    pub pages: Vec<PageStructure>,
    /// True when part of the analysis was skipped (limits, encryption, damaged pages).
    pub degraded: bool,
    pub warnings: Vec<String>,
}
