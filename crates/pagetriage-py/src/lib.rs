//! Python bindings. A thin wrapper: all analysis lives in `pagetriage-core`, and this module only
//! moves bytes and strings across the boundary. The `bench/` tuning harness imports it as
//! `pagetriage_py`.

use pagetriage_core::{PageInput, Params};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

/// Gray from interleaved RGB bytes using the core's fixed-point luma, so the harness does not
/// bring its own (differently rounded) conversion.
#[pyfunction]
fn rgb_to_gray<'py>(py: Python<'py>, rgb: &[u8]) -> PyResult<Bound<'py, PyBytes>> {
    let gray = pagetriage_core::raster::rgb_to_gray(rgb)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(PyBytes::new(py, &gray))
}

/// Analyse one page and return the canonical JSON of its page report.
///
/// `gray` is `width * height` bytes, row-major. `rgb`, if given, is `3 * width * height`
/// interleaved bytes. `opts_json` takes `debug`, `detectors`, `params` and `page_index`.
/// Raises `ValueError` for inconsistent buffers, dimensions or options.
#[pyfunction]
#[pyo3(signature = (gray, width, height, dpi = 150, rgb = None, structural_json = None, opts_json = "{}"))]
fn analyse_page(
    gray: &[u8],
    width: u32,
    height: u32,
    dpi: u16,
    rgb: Option<&[u8]>,
    structural_json: Option<&str>,
    opts_json: &str,
) -> PyResult<String> {
    let input = PageInput {
        gray,
        rgb,
        width,
        height,
        dpi,
    };
    pagetriage_core::analyse_page(&input, structural_json, opts_json)
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

/// JSON Schema of the report, as a string.
#[pyfunction]
fn schema_json() -> String {
    pagetriage_core::schema_json()
}

/// The embedded default parameters as TOML, for the tuning harness to diff and override.
#[pyfunction]
fn params_toml() -> PyResult<String> {
    Params::defaults().map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(Params::defaults_toml())
}

#[pymodule]
fn pagetriage_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(analyse_page, m)?)?;
    m.add_function(wrap_pyfunction!(schema_json, m)?)?;
    m.add_function(wrap_pyfunction!(params_toml, m)?)?;
    m.add_function(wrap_pyfunction!(rgb_to_gray, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
