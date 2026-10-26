//! Fusion layer (module M13): log-odds evidence sum, so every explanation is exact.
//!
//! The sum and the per-evidence clip are in `scorer` and `evidence`. `consistency` holds the
//! cross-detector rules, `calibration` the optional per-kind confidence maps, `page_type` the page
//! typing of module M10 and `document` the pass over a document's pages. Correlated-evidence group
//! caps are not built: each detector's own terms are clipped, and detectors do not share evidence
//! lists.

pub mod calibration;
pub mod consistency;
pub mod document;
pub mod evidence;
pub mod page_type;
pub mod plan;
pub mod scorer;
