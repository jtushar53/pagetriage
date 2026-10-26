//! `pagetriage-core`: deterministic, non-ML page-content analysis. Pure Rust, no I/O, no threads.
//!
//! The same input gives the same output, byte for byte, on native, Python and WASM builds. See
//! `CLAUDE.md` for the rules that keep it that way.

pub mod analyse;
pub mod error;
pub mod fusion;
pub mod hash;
pub mod math;
pub mod params;
pub mod pixel;
pub mod raster;
pub mod scale;
pub mod schema;
pub mod structural;

pub use analyse::{
    DETECTORS, OPT_IN, Options, PageInput, analyse_page, analyse_page_report, finish_document,
};
pub use error::{Error, Result};
pub use params::Params;
pub use schema::{
    Evidence, Finding, Kind, PageReport, Report, SCHEMA_ID, schema_json, to_canonical_json,
};
