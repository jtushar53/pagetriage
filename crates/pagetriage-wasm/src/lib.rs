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
