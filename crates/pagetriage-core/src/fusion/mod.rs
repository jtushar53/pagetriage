//! Fusion layer (module M13): log-odds evidence sum, so every explanation is exact.
//!
//! Phase 0 has the sum and the per-evidence clip. Correlated-evidence group caps, isotonic
//! calibration and the page-level consistency rules arrive with M13 in Phase 5.

pub mod evidence;
pub mod scorer;
