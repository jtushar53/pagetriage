//! Pixel layer. Detectors take the gray page buffer and return a [`Detection`]: the findings they
//! established plus their raw feature values (surfaced in the report under `debug`).

use std::collections::BTreeMap;

use crate::schema::Finding;

pub mod binarize;
pub mod components;
pub mod content_type;
pub mod handwriting;
pub mod layout;
pub mod phash;
pub mod qr;
pub mod quality;
pub mod redaction;
pub mod rulings;
pub mod script;
pub mod signature;
pub mod stamp;
pub mod thumb;

#[derive(Debug, Default)]
pub struct Detection {
    pub findings: Vec<Finding>,
    /// `feature -> value`, recorded whether or not a finding was emitted.
    pub features: BTreeMap<String, f64>,
}
