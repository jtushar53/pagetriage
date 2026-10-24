//! Pixel layer. Detectors take the gray page buffer and return a [`Detection`]: the findings they
//! established plus their raw feature values (surfaced in the report under `debug`).

use std::collections::BTreeMap;

use crate::schema::Finding;

pub mod binarize;
pub mod components;
pub mod content_type;
pub mod quality;

#[derive(Debug, Default)]
pub struct Detection {
    pub findings: Vec<Finding>,
    /// `feature -> value`, recorded whether or not a finding was emitted.
    pub features: BTreeMap<String, f64>,
}
