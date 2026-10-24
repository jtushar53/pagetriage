//! `pdf-inspector-compare FILE.pdf...` prints one JSON object per file:
//! `{"file", "pdf_type", "confidence", "page_count", "pages_with_text", "pages_needing_ocr",
//! "reasons": {"<page>": [..]}}` using `ScanStrategy::Full` (every page examined), or `{"file",
//! "error"}`.
use pdf_inspector::{DetectionConfig, ScanStrategy, detect_pdf_type_mem_with_config};
use serde_json::json;

fn main() {
    for path in std::env::args().skip(1) {
        let name = std::path::Path::new(&path)
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let line = match std::fs::read(&path) {
            Err(e) => json!({"file": name, "error": e.to_string()}),
            Ok(bytes) => {
                let cfg = DetectionConfig {
                    strategy: ScanStrategy::Full,
                    ..DetectionConfig::default()
                };
                match detect_pdf_type_mem_with_config(&bytes, cfg) {
                    Err(e) => json!({"file": name, "error": e.to_string()}),
                    Ok(r) => json!({
                        "file": name,
                        "pdf_type": format!("{:?}", r.pdf_type),
                        "confidence": r.confidence,
                        "page_count": r.page_count,
                        "pages_with_text": r.pages_with_text,
                        "pages_needing_ocr": r.pages_needing_ocr,
                        "reasons": r.ocr_reasons_by_page,
                    }),
                }
            }
        };
        println!("{line}");
    }
}
