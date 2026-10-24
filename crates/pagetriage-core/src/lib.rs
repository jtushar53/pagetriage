//! `pagetriage-core`: deterministic, non-ML page-content analysis. Pure Rust, no I/O, no threads.
//!
//! The same input gives the same output, byte for byte, on native, Python and WASM builds. See
//! `CLAUDE.md` for the rules that keep it that way.

pub mod error;
pub mod math;
pub mod params;
pub mod schema;

pub use error::{Error, Result};
pub use params::Params;
pub use schema::{Evidence, Finding, Kind, PageReport, Report, SCHEMA_ID};
