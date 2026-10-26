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

/// Binarise a gray page: `width * height` bytes, `1` for ink and `0` for paper.
///
/// `method` is `otsu`, `sauvola`, `wolf` or `howe`; `params_toml` overrides the embedded defaults
/// (for example `[howe]\nmode = "static"`). Releases the GIL, since Howe takes seconds per page.
#[pyfunction]
#[pyo3(signature = (gray, width, height, method = "sauvola", params_toml = None))]
fn binarize<'py>(
    py: Python<'py>,
    gray: &[u8],
    width: usize,
    height: usize,
    method: &str,
    params_toml: Option<&str>,
) -> PyResult<Bound<'py, PyBytes>> {
    let params = match params_toml {
        Some(t) => Params::with_overrides(t),
        None => Params::defaults(),
    }
    .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let method = pagetriage_core::pixel::binarize::parse_method(method)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let ink = py
        .detach(|| pagetriage_core::pixel::binarize::binarize(gray, width, height, method, &params))
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(PyBytes::new(py, &ink))
}

/// Howe binarisation with the parameters it settled on: `(ink bytes, c, t_hi)`.
#[pyfunction]
#[pyo3(signature = (gray, width, height, params_toml = None))]
fn binarize_howe<'py>(
    py: Python<'py>,
    gray: &[u8],
    width: usize,
    height: usize,
    params_toml: Option<&str>,
) -> PyResult<(Bound<'py, PyBytes>, f64, f64)> {
    let params = match params_toml {
        Some(t) => Params::with_overrides(t),
        None => Params::defaults(),
    }
    .map_err(|e| PyValueError::new_err(e.to_string()))?;
    if width == 0 || height == 0 || gray.len() != width * height {
        return Err(PyValueError::new_err(format!(
            "gray buffer of {} bytes does not match {width}x{height}",
            gray.len()
        )));
    }
    let result = py.detach(|| {
        pagetriage_core::pixel::binarize::howe::binarize(gray, width, height, &params.howe)
    });
    Ok((PyBytes::new(py, &result.ink), result.c, result.t_hi))
}

/// Analyse one page and return the canonical JSON of its page report.
///
/// `gray` is `width * height` bytes, row-major. `rgb`, if given, is `3 * width * height`
/// interleaved bytes. `opts_json` takes `debug`, `detectors`, `params` and `page_index`.
/// Raises `ValueError` for inconsistent buffers, dimensions or options. Releases the GIL, so a
/// thread pool can analyse pages in parallel (each call is independent and deterministic).
#[pyfunction]
#[pyo3(signature = (gray, width, height, dpi = 150, rgb = None, structural_json = None, opts_json = "{}"))]
// `py` is PyO3's GIL token, not a Python argument: callers pass seven.
#[allow(clippy::too_many_arguments)]
fn analyse_page(
    py: Python<'_>,
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
    py.detach(|| pagetriage_core::analyse_page(&input, structural_json, opts_json))
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Document-level near-duplicate pages (module M12): for each page, the index of the earliest
/// earlier page whose pHash is within `phash.max_hamming` bits, else `None`.
///
/// `phashes` and `dhashes` (one per page) are the 16-character hex strings from the page reports;
/// `dhashes` may be omitted only if `params_toml` sets `phash.dhash_max_hamming = 64`, which turns
/// the dHash test off (the default bound is below 64). `params_toml` overrides the defaults in
/// the same layout as the analyser's `params` option, for example `"[phash]\nmax_hamming = 6"`.
/// Pages whose hash is all zeros (no structure) are never matched. Raises `ValueError` for
/// malformed hashes or lengths.
#[pyfunction]
#[pyo3(signature = (phashes, dhashes = None, params_toml = None))]
fn near_duplicates(
    phashes: Vec<String>,
    dhashes: Option<Vec<String>>,
    params_toml: Option<&str>,
) -> PyResult<Vec<Option<usize>>> {
    let params = match params_toml {
        Some(toml) => Params::with_overrides(toml),
        None => Params::defaults(),
    }
    .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let p: Vec<&str> = phashes.iter().map(String::as_str).collect();
    let d: Option<Vec<&str>> = dhashes
        .as_ref()
        .map(|d| d.iter().map(String::as_str).collect());
    pagetriage_core::pixel::phash::near_duplicates_hex(&p, d.as_deref(), &params.phash)
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Document-level pass over the page reports of one document, in page order: near-duplicate pages
/// (module M12) and registrar-endorsement repetition (module M11). `pages_json` is a JSON array of
/// the strings `analyse_page` returned; the result is the canonical JSON array with
/// `near_duplicate_of` and the `registrar_endorsement` findings filled. `opts_json` takes `params`.
/// Raises `ValueError` for malformed input.
#[pyfunction]
#[pyo3(signature = (pages_json, opts_json = "{}"))]
fn finish_document(pages_json: &str, opts_json: &str) -> PyResult<String> {
    pagetriage_core::finish_document(pages_json, opts_json)
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Structure of a PDF (modules M1, M2 and the structure part of M12): per-page text-layer state,
/// script shares and legacy-font verdict, and document provenance. Returns the canonical JSON of a
/// `StructureReport`. `opts_json` takes `params` (a TOML string merged over the defaults).
/// Raises `ValueError` for input that is not a PDF, is over the size limits, or cannot be parsed.
/// Releases the GIL.
#[pyfunction]
#[pyo3(signature = (data, opts_json = "{}"))]
fn analyse_pdf_structure(py: Python<'_>, data: &[u8], opts_json: &str) -> PyResult<String> {
    py.detach(|| pagetriage_core::structural::analyse_pdf_structure(data, opts_json))
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

/// JSON Schema of the structure report, as a string.
#[pyfunction]
fn structure_schema_json() -> String {
    pagetriage_core::schema::structure_schema_json()
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
    m.add_function(wrap_pyfunction!(near_duplicates, m)?)?;
    m.add_function(wrap_pyfunction!(finish_document, m)?)?;
    m.add_function(wrap_pyfunction!(schema_json, m)?)?;
    m.add_function(wrap_pyfunction!(analyse_pdf_structure, m)?)?;
    m.add_function(wrap_pyfunction!(structure_schema_json, m)?)?;
    m.add_function(wrap_pyfunction!(params_toml, m)?)?;
    m.add_function(wrap_pyfunction!(rgb_to_gray, m)?)?;
    m.add_function(wrap_pyfunction!(binarize, m)?)?;
    m.add_function(wrap_pyfunction!(binarize_howe, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
