//! Module M3 (`pixel.quality`) on the synthetic pages in `tests/golden/quality_inputs/`, each of
//! which has one degradation of a known size (see `tests/golden/gen_inputs.py`), plus degenerate
//! inputs that must give finite, sensible values. The byte-exact snapshots of the same pages are
//! checked by `tests/golden.rs`; these tests say what the numbers must mean.

use std::fs;
use std::path::PathBuf;

use pagetriage_core::schema::{Binarizer, PageReport};
use pagetriage_core::{PageInput, analyse_page_report};

fn pgm(name: &str) -> (u32, u32, Vec<u8>) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/quality_inputs")
        .join(format!("{name}.pgm"));
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let (mut pos, mut tokens) = (0, Vec::new());
    while tokens.len() < 4 {
        while bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let start = pos;
        while !bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        tokens.push(String::from_utf8_lossy(&bytes[start..pos]).into_owned());
    }
    pos += 1;
    (
        tokens[1].parse().unwrap(),
        tokens[2].parse().unwrap(),
        bytes[pos..].to_vec(),
    )
}

fn analyse(gray: &[u8], w: u32, h: u32) -> PageReport {
    let input = PageInput {
        gray,
        rgb: None,
        width: w,
        height: h,
        dpi: 150,
    };
    analyse_page_report(&input, None, r#"{"debug":true,"detectors":[]}"#).unwrap()
}

fn fixture(name: &str) -> PageReport {
    let (w, h, gray) = pgm(name);
    analyse(&gray, w, h)
}

fn feature(r: &PageReport, name: &str) -> f64 {
    r.debug.as_ref().unwrap()["quality"][name]
}

#[test]
fn clean_page_is_upright_sharp_and_grade_a() {
    let r = fixture("quality_clean");
    let q = r.quality.unwrap();
    assert!(q.skew_deg.abs() <= 0.1, "skew {}", q.skew_deg);
    assert_eq!((q.fold_lines, q.speckle_density), (0, 0.0));
    assert!(q.bleed_through < 0.1, "bleed {}", q.bleed_through);
    assert_eq!(q.recommended_binarizer, Binarizer::Sauvola);
    assert_eq!(q.grade, "A");
    assert!(q.contrast_rms > 0.2, "contrast {}", q.contrast_rms);
}

#[test]
fn skew_has_the_right_sign_and_size() {
    // atan(1/19) = 3.01 degrees counter-clockwise: positive.
    let up = fixture("quality_skew_p3").quality.unwrap().skew_deg;
    assert!((up - 3.01).abs() <= 0.3, "got {up}");
    // Headline words, atan(1/29) = 1.98 degrees clockwise: negative.
    let down = fixture("quality_deva_skew_m2").quality.unwrap().skew_deg;
    assert!((down + 1.98).abs() <= 0.3, "got {down}");
}

#[test]
fn blur_lowers_edge_sharpness_and_laplacian_variance() {
    let (clean, soft) = (fixture("quality_clean"), fixture("quality_blurred"));
    let (qc, qs) = (
        clean.quality.as_ref().unwrap(),
        soft.quality.as_ref().unwrap(),
    );
    assert!(qs.blur_laplacian_var < 0.05 * qc.blur_laplacian_var);
    assert!(
        feature(&soft, "blur_sigma_px") > 2.0 * feature(&clean, "blur_sigma_px"),
        "sigma {} vs {}",
        feature(&soft, "blur_sigma_px"),
        feature(&clean, "blur_sigma_px")
    );
}

#[test]
fn speckle_density_tracks_the_specks_added() {
    // 100 specks over 200 x 256 pixels = 0.00195; some land on text or merge, so allow a wide
    // band, but it must be far above the clean page's zero.
    let q = fixture("quality_speckled").quality.unwrap();
    assert!(
        (0.0008..=0.0025).contains(&q.speckle_density),
        "density {}",
        q.speckle_density
    );
}

#[test]
fn show_through_is_flagged_and_switches_the_binariser_to_howe() {
    let r = fixture("quality_bleed");
    let q = r.quality.unwrap();
    assert!(q.bleed_through >= 0.5, "bleed {}", q.bleed_through);
    assert_eq!(q.recommended_binarizer, Binarizer::Howe);
    assert_ne!(q.grade, "A");
}

#[test]
fn one_crease_is_counted_once_and_a_clean_page_has_none() {
    let q = fixture("quality_fold").quality.unwrap();
    assert_eq!(q.fold_lines, 1);
    assert_eq!(fixture("quality_clean").quality.unwrap().fold_lines, 0);
}

#[test]
fn every_fixture_gives_finite_values_and_a_generation_confidence_of_at_most_0_6() {
    for name in [
        "quality_clean",
        "quality_skew_p3",
        "quality_deva_skew_m2",
        "quality_blurred",
        "quality_speckled",
        "quality_bleed",
        "quality_fold",
    ] {
        let r = fixture(name);
        let q = r.quality.as_ref().unwrap();
        for v in [
            q.skew_deg,
            q.blur_laplacian_var,
            q.contrast_rms,
            q.speckle_density,
            q.bleed_through,
        ] {
            assert!(v.is_finite(), "{name}");
        }
        if let Some(g) = &q.photocopy_generation_est {
            assert!(g.confidence <= 0.6 + 1e-12, "{name}: {}", g.confidence);
        }
        for (k, v) in &r.debug.as_ref().unwrap()["quality"] {
            assert!(v.is_finite(), "{name}: feature {k} = {v}");
        }
    }
}

#[test]
fn degenerate_pages_give_finite_sensible_values() {
    let mut noise = vec![0u8; 64 * 48];
    let mut s = 5u32;
    for v in &mut noise {
        s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *v = (s >> 24) as u8;
    }
    let cases: Vec<(&str, u32, u32, Vec<u8>)> = vec![
        ("1x1", 1, 1, vec![128]),
        ("2x2", 2, 2, vec![0, 255, 255, 0]),
        ("3x3", 3, 3, vec![9; 9]),
        ("1x500 strip", 1, 500, vec![200; 500]),
        ("500x1 strip", 500, 1, vec![200; 500]),
        ("white", 120, 90, vec![255; 120 * 90]),
        ("black", 120, 90, vec![0; 120 * 90]),
        ("mid gray", 120, 90, vec![128; 120 * 90]),
        ("noise", 64, 48, noise),
    ];
    for (name, w, h, gray) in cases {
        let r = analyse(&gray, w, h);
        let q = r
            .quality
            .as_ref()
            .unwrap_or_else(|| panic!("{name}: no quality"));
        for v in [
            q.skew_deg,
            q.blur_laplacian_var,
            q.contrast_rms,
            q.speckle_density,
            q.bleed_through,
        ] {
            assert!(v.is_finite(), "{name}");
        }
        assert!(q.skew_deg.abs() <= 15.0, "{name}: skew {}", q.skew_deg);
        assert!(["A", "B", "C", "D"].contains(&q.grade.as_str()), "{name}");
        for (k, v) in &r.debug.as_ref().unwrap()["quality"] {
            assert!(v.is_finite(), "{name}: feature {k} = {v}");
        }
    }
    // A blank white page has nothing to measure: no skew, no folds, no generation estimate, and
    // there is no ink for Howe to binarise, so Sauvola stays recommended.
    let (w, h) = (120u32, 90u32);
    let blank = analyse(&vec![255; (w * h) as usize], w, h);
    let q = blank.quality.unwrap();
    assert_eq!((q.skew_deg, q.fold_lines, q.contrast_rms), (0.0, 0, 0.0));
    assert!(q.photocopy_generation_est.is_none());
    assert_eq!(q.recommended_binarizer, Binarizer::Sauvola);
}
