//! The PDF structural layer: modules M1 `structural.text_layer`, M2 `structural.font_script` and
//! the structure part of M12 `provenance`.
//!
//! Everything that only needs bytes and numbers (the Unicode block histogram, `ToUnicode` CMaps,
//! the legacy-font statistics, the JPEG quantisation-table parser, the content-stream tokenizer
//! and the text-layer rules) is always compiled. Reading a PDF file itself needs `lopdf` and is
//! behind the off-by-default `pdf` cargo feature, so the pixel-only WASM build stays small.

pub mod cmap;
pub mod content;
pub mod encoding;
pub mod font_script;
pub mod jpeg;
pub mod legacy;
pub mod params;
pub mod text_layer;
pub mod types;
pub mod unicode;

#[cfg(feature = "pdf")]
mod pdf;

pub use params::StructuralParams;
pub use types::{
    DocumentStructure, FontInfo, ImageInfo, JpegInfo, PageFeatures, PageStructure, RevisionInfo,
    STRUCTURE_SCHEMA_ID, ScriptInfo, StructureReport,
};

/// Analyse the structure of a PDF: per-page text-layer state (M1) and script (M2), and the
/// document's provenance signals (M12). Returns the canonical JSON of a [`StructureReport`]; the
/// same bytes on every platform.
///
/// `opts_json` is `{}` or `{"params": "<TOML merged over the embedded defaults>"}`.
#[cfg(feature = "pdf")]
pub fn analyse_pdf_structure(bytes: &[u8], opts_json: &str) -> crate::error::Result<String> {
    crate::schema::to_canonical_json(&analyse_pdf_structure_report(bytes, opts_json)?)
}

/// Same as [`analyse_pdf_structure`] but returns the typed report.
#[cfg(feature = "pdf")]
pub fn analyse_pdf_structure_report(
    bytes: &[u8],
    opts_json: &str,
) -> crate::error::Result<StructureReport> {
    let options = StructureOptions::from_json(opts_json)?;
    let params = match &options.params {
        Some(toml) => StructuralParams::with_overrides(toml)?,
        None => StructuralParams::defaults()?,
    };
    pdf::analyse(bytes, &params)
}

/// Parsed from `opts_json`. Unknown keys are rejected so a typo cannot silently do nothing.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct StructureOptions {
    /// TOML merged over the embedded structural defaults (`[text_layer]`, `[font_script]`,
    /// `[provenance]`, `[limits]` sections).
    pub params: Option<String>,
}

impl StructureOptions {
    pub fn from_json(opts_json: &str) -> crate::error::Result<Self> {
        if opts_json.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str(opts_json)
            .map_err(|e| crate::error::Error::InvalidOptions(e.to_string()))
    }
}
