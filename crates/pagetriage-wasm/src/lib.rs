//! WASM bindings. A thin wrapper over `pagetriage-core`: it only moves bytes and strings across the
//! boundary. Build with `scripts/build-wasm.sh`.
//!
//! The pixel build deliberately has no PDF rasteriser and no PDF parser; callers render pages
//! (pdf.js to canvas to `ImageData`) and pass pixel buffers in.

use pagetriage_core::PageInput;
use wasm_bindgen::prelude::*;

/// Analyse one page and return the canonical JSON of its page report; identical bytes to the native
/// and Python builds. Throws an `Error` for inconsistent buffers, dimensions or options.
#[wasm_bindgen]
pub fn analyse_page(
    gray: &[u8],
    rgb: Option<Box<[u8]>>,
    width: u32,
    height: u32,
    dpi: u16,
    structural_json: Option<String>,
    opts_json: &str,
) -> Result<String, JsError> {
    let input = PageInput {
        gray,
        rgb: rgb.as_deref(),
        width,
        height,
        dpi,
    };
    pagetriage_core::analyse_page(&input, structural_json.as_deref(), opts_json)
        .map_err(|e| JsError::new(&e.to_string()))
}

/// Document-level pass over a document's page reports (a JSON array of what `analyse_page` returned,
/// in page order): near-duplicate pages and registrar-endorsement repetition. Returns the canonical
/// JSON array; identical bytes to the native and Python builds.
#[wasm_bindgen]
pub fn finish_document(pages_json: &str, opts_json: &str) -> Result<String, JsError> {
    pagetriage_core::finish_document(pages_json, opts_json)
        .map_err(|e| JsError::new(&e.to_string()))
}

/// The resolution an image file states in its header (PNG, JPEG with JFIF or Exif, TIFF, BMP), as the
/// JSON `{"x":..,"y":..,"source":".."}`, or `null` if it states none.
#[wasm_bindgen]
pub fn image_dpi(bytes: &[u8]) -> String {
    match pagetriage_core::scale::image_dpi(bytes) {
        Some(d) => serde_json::json!({"x": d.x, "y": d.y, "source": d.source}).to_string(),
        None => "null".to_owned(),
    }
}

/// The standard paper sizes and resolutions that a page of `width x height` pixels fits within
/// `tolerance` (a fraction, for example 0.02), best first, as the JSON array of
/// `{"paper","dpi","landscape","error"}`. A hint only.
#[wasm_bindgen]
pub fn paper_matches(width: u32, height: u32, tolerance: f64) -> String {
    let list: Vec<_> = pagetriage_core::scale::paper_matches(width, height, tolerance)
        .into_iter()
        .map(|m| serde_json::json!({"paper": m.paper, "dpi": m.dpi, "landscape": m.landscape, "error": m.error}))
        .collect();
    serde_json::Value::Array(list).to_string()
}

/// Binarise a gray page: `width * height` bytes, `1` for ink and `0` for paper. `method` is
/// `otsu`, `sauvola`, `wolf` or `howe`; `params_toml` overrides the embedded defaults. Identical
/// bytes to the native and Python builds.
#[wasm_bindgen]
pub fn binarize(
    gray: &[u8],
    width: u32,
    height: u32,
    method: &str,
    params_toml: Option<String>,
) -> Result<Vec<u8>, JsError> {
    let params = match params_toml.as_deref() {
        Some(t) if !t.is_empty() => pagetriage_core::Params::with_overrides(t),
        _ => pagetriage_core::Params::defaults(),
    }
    .map_err(|e| JsError::new(&e.to_string()))?;
    let method = pagetriage_core::pixel::binarize::parse_method(method)
        .map_err(|e| JsError::new(&e.to_string()))?;
    pagetriage_core::pixel::binarize::binarize(
        gray,
        width as usize,
        height as usize,
        method,
        &params,
    )
    .map_err(|e| JsError::new(&e.to_string()))
}

/// Gray from interleaved RGB bytes using the core's fixed-point luma, so a page that decodes an
/// image on a canvas gets the same gray as the native and Python builds. Throws if the length is
/// not a multiple of 3.
#[wasm_bindgen]
pub fn rgb_to_gray(rgb: &[u8]) -> Result<Vec<u8>, JsError> {
    pagetriage_core::raster::rgb_to_gray(rgb).map_err(|e| JsError::new(&e.to_string()))
}

/// Structure of a PDF (modules M1, M2, M12): the canonical JSON of a `StructureReport`, identical
/// bytes to the native and Python builds. Only present when built with the `pdf` feature. Throws an
/// `Error` for input that is not a PDF, is over the size limits, or cannot be parsed.
#[cfg(feature = "pdf")]
#[wasm_bindgen]
pub fn analyse_pdf_structure(bytes: &[u8], opts_json: Option<String>) -> Result<String, JsError> {
    pagetriage_core::structural::analyse_pdf_structure(bytes, opts_json.as_deref().unwrap_or("{}"))
        .map_err(|e| JsError::new(&e.to_string()))
}

/// JSON Schema of the structure report, for validation on the JS side.
#[cfg(feature = "pdf")]
#[wasm_bindgen]
pub fn structure_schema_json() -> String {
    pagetriage_core::schema::structure_schema_json()
}

/// JSON Schema of the report, for validation on the JS side.
#[wasm_bindgen]
pub fn schema_json() -> String {
    pagetriage_core::schema_json()
}

/// Crate version, so a page can show which build it loaded.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}
