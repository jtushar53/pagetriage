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
pub mod jpeg;
pub mod legacy;
pub mod params;
pub mod text_layer;
pub mod types;
pub mod unicode;

pub use params::StructuralParams;
pub use types::{
    DocumentStructure, FontInfo, ImageInfo, JpegInfo, PageFeatures, PageStructure, RevisionInfo,
    STRUCTURE_SCHEMA_ID, ScriptInfo, StructureReport,
};
