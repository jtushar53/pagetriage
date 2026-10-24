//! End-to-end tests of the PDF structural layer on small synthetic PDFs built in memory.
#![cfg(feature = "pdf")]

mod support;

use pagetriage_core::structural::analyse_pdf_structure;
use serde_json::Value;
use support::{Builder, Page, gray_image};

fn run(bytes: &[u8]) -> Value {
    run_with(bytes, "{}")
}

fn run_with(bytes: &[u8], opts: &str) -> Value {
    let json = analyse_pdf_structure(bytes, opts).expect("analysis succeeds");
    serde_json::from_str(&json).unwrap()
}

fn state(v: &Value, page: usize) -> String {
    v["pages"][page]["text_layer"]["state"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn page_with_image(content: &str) -> Vec<u8> {
    let (dict, data) = gray_image(40, 50);
    Page::new(content)
        .resources("/Font << /F1 5 0 R >> /XObject << /Im1 6 0 R >>")
        .stream(6, &dict, &data)
        .build(None)
}

const FULL_PAGE: &str = "q 612 0 0 792 0 0 cm /Im1 Do Q";

#[test]
fn native_text_page() {
    let pdf = Page::new("BT /F1 12 Tf 72 700 Td (The vendor shall convey the property) Tj ET")
        .build(Some("Test Writer 1.0"));
    let v = run(&pdf);
    assert_eq!(state(&v, 0), "native_text");
    let p = &v["pages"][0];
    assert_eq!(p["text_layer"]["reliable"], true);
    assert_eq!(p["script"]["dominant"], "latin");
    assert_eq!(p["script"]["name"], "Latin");
    assert_eq!(p["features"]["glyphs"], 31);
    assert_eq!(p["size_pt"], serde_json::json!([612.0, 792.0]));
    assert_eq!(v["document"]["producer"], "Test Writer 1.0");
    assert_eq!(v["document"]["incremental_updates"], 0);
    assert_eq!(v["document"]["source"], "pdf");
    assert_eq!(v["schema"], "pagetriage-structure/0.1");
    assert_eq!(v["structure"]["xref_style"], "classic");
    assert_eq!(v["degraded"], false);
}

#[test]
fn raster_only_page() {
    let v = run(&page_with_image(FULL_PAGE));
    assert_eq!(state(&v, 0), "raster_only");
    let p = &v["pages"][0];
    assert!(p["features"]["image_cover"].as_f64().unwrap() > 0.99);
    assert_eq!(p["images"][0]["width"], 40);
    assert_eq!(p["images"][0]["filter"], "none");
    assert_eq!(p["script"]["dominant"], "none");
    assert!(p["text_layer"]["reliable"].is_null());
}

#[test]
fn invisible_text_over_a_page_image_is_an_ocr_layer() {
    let content =
        format!("{FULL_PAGE} BT 3 Tr /F1 10 Tf 72 700 Td (OCR text over the scanned page) Tj ET");
    let v = run(&page_with_image(&content));
    assert_eq!(state(&v, 0), "invisible_ocr_under_image");
    let f = &v["pages"][0]["features"];
    assert_eq!(f["glyphs_invisible_mode"], f["glyphs"]);
    assert_eq!(f["glyphs_in_image"], f["glyphs"]);
    assert_eq!(v["pages"][0]["text_layer"]["reliable"], true);
}

#[test]
fn visible_text_drawn_before_an_opaque_page_image_is_hidden() {
    let content = format!("BT /F1 10 Tf 72 700 Td (Text hidden under the scan) Tj ET {FULL_PAGE}");
    let v = run(&page_with_image(&content));
    assert_eq!(state(&v, 0), "invisible_ocr_under_image");
    let f = &v["pages"][0]["features"];
    assert_eq!(f["glyphs_invisible_mode"], 0);
    assert_eq!(f["glyphs_under_image"], f["glyphs"]);
}

#[test]
fn visible_text_drawn_after_the_page_image_is_not_hidden() {
    let content =
        format!("{FULL_PAGE} BT /F1 10 Tf 72 700 Td (Visible text on top of the scan) Tj ET");
    let v = run(&page_with_image(&content));
    assert_eq!(state(&v, 0), "mixed");
}

#[test]
fn a_masked_image_does_not_hide_earlier_text() {
    let (dict, data) = gray_image(40, 50);
    let content =
        format!("BT /F1 10 Tf 72 700 Td (Text under a soft-masked image) Tj ET {FULL_PAGE}");
    let pdf = Page::new(&content)
        .resources("/Font << /F1 5 0 R >> /XObject << /Im1 6 0 R >>")
        .stream(6, &format!("{dict} /SMask 7 0 R"), &data)
        .stream(7, &dict, &data)
        .build(None);
    let v = run(&pdf);
    assert_eq!(v["pages"][0]["features"]["glyphs_under_image"], 0);
    assert_eq!(state(&v, 0), "mixed");
}

#[test]
fn an_image_tiled_in_strips_still_covers_the_page() {
    let mut content = String::new();
    for i in 0..4 {
        content.push_str(&format!("q 612 0 0 198 0 {} cm /Im1 Do Q\n", i * 198));
    }
    let v = run(&page_with_image(&content));
    assert_eq!(state(&v, 0), "raster_only");
    assert!(v["pages"][0]["features"]["image_cover"].as_f64().unwrap() > 0.97);
    assert_eq!(v["pages"][0]["features"]["image_count"], 4);
}

#[test]
fn vector_only_empty_and_outlined_text() {
    let rules = Page::new(
        "0.5 w 50 50 m 500 50 l S 50 100 m 500 100 l S 50 150 m 500 150 l S 50 200 500 100 re S",
    )
    .build(None);
    assert_eq!(state(&run(&rules), 0), "vector_only");

    let empty = Page::new("q Q").build(None);
    assert_eq!(state(&run(&empty), 0), "empty");
    let one_rule = Page::new("50 50 m 500 50 l S").build(None);
    assert_eq!(state(&run(&one_rule), 0), "empty");
    let blank_text_object = Page::new("BT /F1 12 Tf ET").build(None);
    assert_eq!(state(&run(&blank_text_object), 0), "empty");

    // Text outlined into paths: many small filled curves, no text operators.
    let mut c = String::new();
    for i in 0..300 {
        let x = 50 + (i % 40) * 10;
        let y = 700 - (i / 40) * 14;
        c.push_str(&format!(
            "{x} {y} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} l h f\n",
            x + 2,
            y + 5,
            x + 4,
            y + 5,
            x + 6,
            y,
            x + 6,
            y - 2,
            x + 4,
            y - 4,
            x + 3,
            y - 4,
            x + 1,
            y - 2
        ));
    }
    let v = run(&Page::new(&c).build(None));
    assert_eq!(state(&v, 0), "vector_only");
    assert!(
        v["pages"][0]["features"]["small_filled_paths"]
            .as_u64()
            .unwrap()
            >= 150
    );
    assert_eq!(v["pages"][0]["features"]["glyphs"], 0);
}

#[test]
fn no_contents_entry_is_an_empty_page() {
    let mut b = Builder::new();
    b.obj(1, "<< /Type /Catalog /Pages 2 0 R >>");
    b.obj(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    b.obj(3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] >>");
    b.finish(1, None, 4);
    let v = run(&b.out);
    assert_eq!(state(&v, 0), "empty");
    assert_eq!(v["pages"][0]["size_pt"], serde_json::json!([595.0, 842.0]));
}

#[test]
fn legacy_devanagari_font_is_flagged_and_the_layer_is_unreliable() {
    let pdf = Page::new(
        "BT /F1 12 Tf 72 700 Td (Hkkjr ljdkj ds ukeku ls ;g foys[k fu\"iknu fd;k tkrk gS) Tj ET",
    )
    .resources("/Font << /F1 5 0 R >>")
    .object(5, "<< /Type /Font /Subtype /TrueType /BaseFont /ABCDEF+KrutiDev010 /Encoding /WinAnsiEncoding >>")
    .build(None);
    let v = run(&pdf);
    let p = &v["pages"][0];
    assert_eq!(state(&v, 0), "native_text");
    assert_eq!(p["script"]["legacy_encoded"], true);
    assert_eq!(p["script"]["name"], "Devanagari (legacy-encoded)");
    assert_eq!(p["script"]["dominant"], "devanagari");
    assert_eq!(p["text_layer"]["reliable"], false);
    assert_eq!(p["script"]["fonts"][0]["legacy_name_match"], "krutidev");
    assert!(p["conflicts"].as_array().unwrap().is_empty());
}

#[test]
fn legacy_text_in_an_unnamed_font_is_caught_by_the_statistics() {
    let line =
        "lEifRr ds fy;s ;g foys[k Hkkjr ljdkj dk gS {ks=Qy fu\"iknu vk/kkj ¼[kjhnh½ fcØh ys[k ";
    let text = line.repeat(4);
    let pdf = Page::new(&format!(
        "BT /F1 12 Tf 72 700 Td ({}) Tj ET",
        text.replace('¼', "\\274")
            .replace('½', "\\275")
            .replace('Ø', "\\330")
    ))
    .object(
        5,
        "<< /Type /Font /Subtype /TrueType /BaseFont /Fnt1 /Encoding /WinAnsiEncoding >>",
    )
    .build(None);
    let v = run(&pdf);
    assert_eq!(
        v["pages"][0]["script"]["legacy_encoded"], true,
        "{}",
        v["pages"][0]["script"]
    );
    assert!(v["pages"][0]["script"]["fonts"][0]["legacy_name_match"].is_null());
}

#[test]
fn english_prose_in_an_ordinary_font_is_not_legacy() {
    let text = "This Deed of Sale is made between the Vendor and the Purchaser whereas the Vendor is the absolute owner of the property described in the Schedule hereunder ".repeat(3);
    let pdf = Page::new(&format!("BT /F1 12 Tf 72 700 Td ({text}) Tj ET")).build(None);
    let v = run(&pdf);
    assert_eq!(v["pages"][0]["script"]["legacy_encoded"], false);
    assert_eq!(v["pages"][0]["text_layer"]["reliable"], true);
}

#[test]
fn identity_h_font_without_tounicode_is_undecodable() {
    let pdf = Page::new("BT /F1 12 Tf 72 700 Td <00240025002600270028002900300031> Tj ET")
        .object(5, "<< /Type /Font /Subtype /Type0 /BaseFont /ABCDEF+Mangal /Encoding /Identity-H /DescendantFonts [6 0 R] >>")
        .object(6, "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /ABCDEF+Mangal /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> >>")
        .build(None);
    let v = run(&pdf);
    let p = &v["pages"][0];
    assert_eq!(p["script"]["dominant"], "unknown");
    assert_eq!(p["text_layer"]["reliable"], false);
    assert_eq!(p["script"]["glyphs_undecodable"], 8);
}

#[test]
fn type0_font_with_tounicode_gives_devanagari_shares() {
    let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
        1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
        3 beginbfchar <0003> <0020> <0024> <0915> <0025> <0916> endbfchar\n\
        1 beginbfrange <0026> <0030> <0917> endbfrange\n endcmap";
    let pdf = Page::new("BT /F1 12 Tf 72 700 Td <0024002500260027002800290003002A> Tj ET")
        .object(5, "<< /Type /Font /Subtype /Type0 /BaseFont /ABCDEF+Mangal /Encoding /Identity-H /DescendantFonts [6 0 R] /ToUnicode 7 0 R >>")
        .object(6, "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /ABCDEF+Mangal >>")
        .stream(7, "", cmap.as_bytes())
        .build(None);
    let v = run(&pdf);
    let p = &v["pages"][0];
    assert_eq!(p["script"]["dominant"], "devanagari");
    assert_eq!(p["script"]["name"], "Devanagari");
    assert_eq!(p["script"]["legacy_encoded"], false);
    assert_eq!(p["script"]["letters"], 7);
    assert_eq!(p["text_layer"]["reliable"], true);
    assert_eq!(p["script"]["fonts"][0]["to_unicode"], true);
}

#[test]
fn text_inside_a_form_xobject_counts_and_a_self_referencing_form_terminates() {
    let form = "BT /F1 12 Tf 10 10 Td (Text inside a form XObject) Tj ET /Fm1 Do";
    let pdf = Page::new("q 1 0 0 1 100 600 cm /Fm1 Do Q")
        .resources("/Font << /F1 5 0 R >> /XObject << /Fm1 6 0 R >>")
        .stream(6, "/Type /XObject /Subtype /Form /BBox [0 0 400 100] /Resources << /Font << /F1 5 0 R >> /XObject << /Fm1 6 0 R >> >>", form.as_bytes())
        .build(None);
    let v = run(&pdf);
    assert_eq!(state(&v, 0), "native_text");
    assert_eq!(v["pages"][0]["features"]["glyphs"], 22);
}

#[test]
fn tj_adjustments_become_word_spaces_in_the_sample() {
    // Words separated only by TJ kerning still count as separate words for the statistics; here
    // we simply check the glyph count ignores the numbers.
    let pdf =
        Page::new("BT /F1 12 Tf 72 700 Td [(The) -300 (vendor) -300 (shall)] TJ ET").build(None);
    assert_eq!(run(&pdf)["pages"][0]["features"]["glyphs"], 14);
}

#[test]
fn rotated_page_swaps_the_display_size() {
    let pdf = Page::new("BT /F1 12 Tf 72 700 Td (Rotated page text) Tj ET").build(None);
    let text = String::from_utf8_lossy(&pdf).replace("/Parent 2 0 R", "/Parent 2 0 R /Rotate 90");
    let v = run(text.as_bytes());
    assert_eq!(v["pages"][0]["rotation"], 90);
    assert_eq!(v["pages"][0]["size_pt"], serde_json::json!([792.0, 612.0]));
}

#[test]
fn ocr_producer_fingerprint_is_read_and_noted() {
    let content =
        format!("{FULL_PAGE} BT 3 Tr /F1 10 Tf 72 700 Td (OCR text over the scanned page) Tj ET");
    let (dict, data) = gray_image(40, 50);
    let pdf = Page::new(&content)
        .resources("/Font << /F1 5 0 R >> /XObject << /Im1 6 0 R >>")
        .stream(6, &dict, &data)
        .build(Some("ocrmypdf 16.0 / Tesseract OCR-PDF 5.3"));
    let v = run(&pdf);
    assert_eq!(v["structure"]["ocr_producer"], true);
    assert_eq!(v["structure"]["producer_family"], "ocrmypdf");
    let ev = v["pages"][0]["text_layer"]["evidence"].as_array().unwrap();
    assert!(
        ev.iter()
            .any(|e| e["feature"] == "producer_ocr" && e["llr"].as_f64().unwrap() > 0.0)
    );

    // The same producer on a page that has no text layer is noted, with no effect on the verdict.
    let raster = {
        let (dict, data) = gray_image(40, 50);
        Page::new(FULL_PAGE)
            .resources("/XObject << /Im1 6 0 R >>")
            .stream(6, &dict, &data)
            .build(Some("OCRmyPDF 16"))
    };
    let v = run(&raster);
    assert_eq!(state(&v, 0), "raster_only");
    let ev = v["pages"][0]["text_layer"]["evidence"].as_array().unwrap();
    assert!(
        ev.iter()
            .any(|e| e["feature"] == "producer_ocr" && e["llr"] == 0.0)
    );
}

fn dct_header(quality_tables: &[(u8, [u8; 64])]) -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8];
    for (id, zigzag) in quality_tables {
        out.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x43, *id]);
        out.extend_from_slice(zigzag);
    }
    out.extend_from_slice(&[
        0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x32, 0x00, 0x28, 0x01, 0x01, 0x11, 0x00,
    ]);
    out.extend_from_slice(&[
        0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, 0xFF, 0xD9,
    ]);
    out
}

#[test]
fn jpeg_quantisation_tables_are_read_from_dct_images() {
    // The standard luminance table in zig-zag order is the libjpeg quality-50 table.
    let q50: [u8; 64] = [
        16, 11, 12, 14, 12, 10, 16, 14, 13, 14, 18, 17, 16, 19, 24, 40, 26, 24, 22, 22, 24, 49, 35,
        37, 29, 40, 58, 51, 61, 60, 57, 51, 56, 55, 64, 72, 92, 78, 64, 68, 87, 69, 55, 56, 80,
        109, 81, 87, 95, 98, 103, 104, 103, 62, 77, 113, 121, 112, 100, 120, 92, 101, 103, 99,
    ];
    let jpeg = dct_header(&[(0, q50)]);
    let pdf = Page::new(FULL_PAGE)
        .resources("/XObject << /Im1 6 0 R >>")
        .stream(6, "/Type /XObject /Subtype /Image /Width 40 /Height 50 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /DCTDecode", &jpeg)
        .build(None);
    let v = run(&pdf);
    let j = &v["pages"][0]["images"][0]["jpeg"];
    assert_eq!(j["luma_quality"], 50);
    assert_eq!(j["ijg_scaled"], true);
    assert_eq!(j["width"], 40);
    assert_eq!(j["coding"], "baseline");
    assert_eq!(v["structure"]["jpeg_images"], 1);
    assert_eq!(
        v["structure"]["jpeg_table_hashes"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
}

fn base_for_updates(with_signature: bool) -> Builder {
    let mut b = Builder::new();
    b.obj(1, "<< /Type /Catalog /Pages 2 0 R >>");
    b.obj(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    b.obj(3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>");
    b.stream(
        4,
        "",
        b"BT /F1 12 Tf 72 700 Td (Amount payable is Rs. 100000) Tj ET",
    );
    b.obj(
        5,
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    );
    b.obj(6, "<< /Producer (Test Writer) >>");
    if with_signature {
        b.obj(7, "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [0 10 20 30] /Contents <00112233> >>");
    }
    let size = if with_signature { 8 } else { 7 };
    b.finish(1, Some(6), size);
    b
}

#[test]
fn an_added_annotation_is_a_benign_incremental_update() {
    let mut b = base_for_updates(true);
    b.obj(
        8,
        "<< /Type /Annot /Subtype /Text /Rect [10 10 30 30] /Contents (Reviewed) >>",
    );
    b.obj(3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> /Annots [8 0 R] >>");
    b.finish(1, Some(6), 9);
    let v = run(&b.out);
    assert_eq!(v["document"]["incremental_updates"], 1);
    assert_eq!(v["structure"]["eof_markers"], 2);
    assert_eq!(v["structure"]["signature_count"], 1);
    let kinds = &v["structure"]["revisions"][0]["kinds"];
    assert!(
        kinds
            .as_array()
            .unwrap()
            .iter()
            .any(|k| k == "annotation_added"),
        "{kinds}"
    );
    assert!(
        !kinds
            .as_array()
            .unwrap()
            .iter()
            .any(|k| k == "page_content_changed")
    );
    assert_eq!(v["structure"]["tamper_signal"], false);
    assert_eq!(v["structure"]["content_changed_in_update"], false);
}

#[test]
fn changing_page_content_after_a_signature_is_a_tamper_signal() {
    let mut b = base_for_updates(true);
    b.stream(
        4,
        "",
        b"BT /F1 12 Tf 72 700 Td (Amount payable is Rs. 900000) Tj ET",
    );
    b.finish(1, Some(6), 8);
    let v = run(&b.out);
    assert_eq!(v["document"]["incremental_updates"], 1);
    assert_eq!(v["structure"]["tamper_signal"], true);
    assert_eq!(
        v["structure"]["revisions"][0]["content_changed_pages"],
        serde_json::json!([0])
    );
    assert!(
        v["structure"]["tamper_reasons"][0]
            .as_str()
            .unwrap()
            .contains("after the signature")
    );
}

#[test]
fn changing_page_content_without_a_signature_is_an_edit_not_a_tamper_signal() {
    let mut b = base_for_updates(false);
    b.stream(
        4,
        "",
        b"BT /F1 12 Tf 72 700 Td (Amount payable is Rs. 900000) Tj ET",
    );
    b.finish(1, Some(6), 7);
    let v = run(&b.out);
    assert_eq!(v["structure"]["tamper_signal"], false);
    assert_eq!(v["structure"]["content_changed_in_update"], true);
}

#[test]
fn signing_in_the_same_update_that_changes_content_is_not_tamper() {
    let mut b = base_for_updates(false);
    b.stream(
        4,
        "",
        b"BT /F1 12 Tf 72 700 Td (Amount payable is Rs. 900000) Tj ET",
    );
    b.obj(
        7,
        "<< /Type /Sig /ByteRange [0 10 20 30] /Contents <00112233> /Filter /Adobe.PPKLite >>",
    );
    b.finish(1, Some(6), 8);
    let v = run(&b.out);
    assert_eq!(v["structure"]["signature_count"], 1);
    assert_eq!(v["structure"]["tamper_signal"], false);
}

#[test]
fn an_eof_marker_inside_a_stream_is_not_a_revision() {
    let pdf =
        Page::new("BT /F1 12 Tf 72 700 Td (Some visible words here) Tj ET\n% %%EOF in a comment\n")
            .build(None);
    let v = run(&pdf);
    assert_eq!(v["structure"]["eof_markers"], 2);
    assert_eq!(v["document"]["incremental_updates"], 0);
}

#[test]
fn not_a_pdf_and_empty_input_are_errors() {
    for bad in [
        &b""[..],
        b"hello",
        b"%PDF-",
        b"\x89PNG\r\n\x1a\n",
        b"%PDF-1.4\n",
    ] {
        // Either an error or a result with zero pages, never a panic.
        match analyse_pdf_structure(bad, "{}") {
            Ok(json) => {
                let v: Value = serde_json::from_str(&json).unwrap();
                assert_eq!(v["pages"].as_array().unwrap().len(), 0);
            }
            Err(e) => assert!(!e.to_string().is_empty()),
        }
    }
    assert!(analyse_pdf_structure(b"hello", "{}").is_err());
}

#[test]
fn bad_options_are_rejected() {
    let pdf = Page::new("q Q").build(None);
    assert!(analyse_pdf_structure(&pdf, "not json").is_err());
    assert!(analyse_pdf_structure(&pdf, r#"{"nope":1}"#).is_err());
    assert!(analyse_pdf_structure(&pdf, r#"{"params":"[text_layer]\nmin_glyph = 3"}"#).is_err());
    let v = run_with(&pdf, r#"{"params":"[text_layer]\nmin_glyphs = 5"}"#);
    assert_eq!(state(&v, 0), "empty");
}

#[test]
fn size_limits_refuse_oversized_input() {
    let pdf = Page::new("q Q").build(None);
    let r = analyse_pdf_structure(&pdf, r#"{"params":"[limits]\nmax_input_bytes = 100"}"#);
    assert!(r.is_err());
}

#[test]
fn truncated_and_corrupted_files_never_panic() {
    let content = format!("{FULL_PAGE} BT 3 Tr /F1 10 Tf 72 700 Td (OCR text) Tj ET");
    let mut b = base_for_updates(true);
    b.stream(4, "", b"BT /F1 12 Tf 72 700 Td (Edited) Tj ET");
    b.finish(1, Some(6), 8);
    for original in [page_with_image(&content), b.out.clone()] {
        // Every truncation point.
        for cut in 0..original.len() {
            let _ = analyse_pdf_structure(&original[..cut], "{}");
        }
        // Byte flips at a spread of positions with a few different values.
        let mut seed = 0x1234_5678_u32;
        for _ in 0..400 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let pos = (seed >> 8) as usize % original.len();
            let mut copy = original.clone();
            copy[pos] = (seed >> 24) as u8;
            let _ = analyse_pdf_structure(&copy, "{}");
        }
    }
}

#[test]
fn hostile_structures_terminate() {
    // A page tree whose kid is its own parent, and a page whose /Contents array references itself.
    let mut b = Builder::new();
    b.obj(1, "<< /Type /Catalog /Pages 2 0 R >>");
    b.obj(2, "<< /Type /Pages /Kids [2 0 R 3 0 R] /Count 2 >>");
    b.obj(
        3,
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>",
    );
    b.obj(4, "[4 0 R 4 0 R]");
    b.finish(1, None, 5);
    let v = run(&b.out);
    assert_eq!(v["pages"].as_array().unwrap().len(), 1);

    // Absurd /Count and a deep chain of nested Pages nodes.
    let mut b = Builder::new();
    b.obj(1, "<< /Type /Catalog /Pages 2 0 R >>");
    let depth = 300u32;
    for i in 0..depth {
        let id = 2 + i;
        b.obj(
            id,
            &format!(
                "<< /Type /Pages /Kids [{} 0 R] /Count 4000000000 >>",
                id + 1
            ),
        );
    }
    b.obj(2 + depth, "<< /Type /Page /MediaBox [0 0 100 100] >>");
    b.finish(1, None, 3 + depth);
    let v = run(&b.out);
    assert_eq!(v["degraded"], true);
}

#[test]
fn a_decompression_bomb_in_page_content_is_refused_by_the_limit() {
    use std::io::Write;
    // 64 KiB of zeros compresses to a few dozen bytes; with a 1 KiB limit it must be refused.
    let zeros = vec![0u8; 65_536];
    let mut enc = flate2_free::deflate_zlib(&zeros);
    enc.shrink_to_fit();
    let mut b = Builder::new();
    b.obj(1, "<< /Type /Catalog /Pages 2 0 R >>");
    b.obj(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    b.obj(
        3,
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>",
    );
    b.stream(4, "/Filter /FlateDecode", &enc);
    b.finish(1, None, 5);
    let _ = std::io::stdout().flush();
    let v = run_with(
        &b.out,
        r#"{"params":"[limits]\nmax_decompressed_bytes = 1024"}"#,
    );
    assert_eq!(v["degraded"], true);
    assert_eq!(v["pages"][0]["features"]["truncated"], true);
}

/// A stored-block zlib stream is enough to check the limit path; no compression library needed.
mod flate2_free {
    pub fn deflate_zlib(data: &[u8]) -> Vec<u8> {
        // zlib header, then stored (uncompressed) blocks, then the Adler-32 checksum. This does not
        // shrink the data, so the test uses a limit below the raw size instead of a real bomb.
        let mut out = vec![0x78, 0x01];
        let mut chunks = data.chunks(65_535).peekable();
        while let Some(chunk) = chunks.next() {
            let last = chunks.peek().is_none();
            out.push(u8::from(last));
            out.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
            out.extend_from_slice(&(!(chunk.len() as u16)).to_le_bytes());
            out.extend_from_slice(chunk);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &x in data {
            a = (a + u32::from(x)) % 65_521;
            b = (b + a) % 65_521;
        }
        out.extend_from_slice(&((b << 16) | a).to_be_bytes());
        out
    }
}

#[test]
fn output_is_identical_between_runs() {
    let pdf = page_with_image(&format!(
        "{FULL_PAGE} BT 3 Tr /F1 10 Tf 72 700 Td (OCR text) Tj ET"
    ));
    let a = analyse_pdf_structure(&pdf, "{}").unwrap();
    let b = analyse_pdf_structure(&pdf, "{}").unwrap();
    assert_eq!(a, b);
}
