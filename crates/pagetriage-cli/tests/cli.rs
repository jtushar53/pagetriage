use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pagetriage"))
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("pagetriage-cli-tests");
    fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn write_pgm(name: &str, w: usize, h: usize, pixels: &[u8]) -> PathBuf {
    assert_eq!(pixels.len(), w * h);
    let mut data = format!("P5\n{w} {h}\n255\n").into_bytes();
    data.extend_from_slice(pixels);
    let path = scratch(name);
    fs::write(&path, data).unwrap();
    path
}

fn stdout_json(out: &Output) -> serde_json::Value {
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("stdout is JSON")
}

#[test]
fn analyses_a_blank_gray_page() {
    let path = write_pgm("blank.pgm", 20, 30, &vec![250; 600]);
    let out = bin().args(["analyse"]).arg(&path).output().unwrap();
    let report = stdout_json(&out);

    assert_eq!(report["schema"], "pagetriage/0.1");
    assert_eq!(report["document"]["pages"], 1);
    assert_eq!(report["document"]["source"], "image");
    let expected_sha = pagetriage_core::hash::sha256_hex(&fs::read(&path).unwrap());
    assert_eq!(report["document"]["sha256"], expected_sha.as_str());

    let page = &report["pages"][0];
    assert_eq!(page["analysis_dpi"], 150);
    assert_eq!(page["findings"][0]["kind"], "blank_page");
    assert!(page["debug"].is_null(), "debug is off by default");
}

#[test]
fn output_is_compact_canonical_and_stable_between_runs() {
    let path = write_pgm("stable.pgm", 16, 16, &vec![240; 256]);
    let a = bin()
        .args(["analyse", "--debug"])
        .arg(&path)
        .output()
        .unwrap();
    let b = bin()
        .args(["analyse", "--debug"])
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(a.stdout, b.stdout);
    let text = String::from_utf8(a.stdout).unwrap();
    assert_eq!(
        text.matches('\n').count(),
        1,
        "compact output is a single line"
    );
}

#[test]
fn pretty_output_parses_to_the_same_report() {
    let path = write_pgm("pretty.pgm", 16, 16, &vec![240; 256]);
    let compact = stdout_json(&bin().args(["analyse"]).arg(&path).output().unwrap());
    let pretty = stdout_json(
        &bin()
            .args(["analyse", "--pretty"])
            .arg(&path)
            .output()
            .unwrap(),
    );
    assert_eq!(compact, pretty);
}

#[test]
fn dpi_debug_and_detector_flags_reach_the_core() {
    let path = write_pgm("flags.pgm", 12, 12, &[240; 144]);
    let out = bin()
        .args([
            "analyse",
            "--dpi",
            "300",
            "--debug",
            "--detectors",
            "blank_page",
        ])
        .arg(&path)
        .output()
        .unwrap();
    let page = stdout_json(&out)["pages"][0].clone();
    assert_eq!(page["analysis_dpi"], 300);
    assert_eq!(page["debug"]["blank_page"]["paper_level"], 240.0);

    let none = bin()
        .args(["analyse", "--detectors", "nope"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!none.status.success());
    assert!(String::from_utf8_lossy(&none.stderr).contains("unknown detector"));
}

#[test]
fn params_file_overrides_defaults() {
    let path = write_pgm("override.pgm", 10, 10, &{
        let mut px = vec![250u8; 100];
        px.iter_mut().take(30).for_each(|p| *p = 200); // 50 below paper
        px
    });
    let params = scratch("override.toml");
    fs::write(&params, "[blank_page]\nink_delta = 40\n").unwrap();

    let default = stdout_json(&bin().arg("analyse").arg(&path).output().unwrap());
    assert_eq!(default["pages"][0]["findings"][0]["kind"], "blank_page");
    let tight = stdout_json(
        &bin()
            .arg("analyse")
            .arg(&path)
            .arg("--params")
            .arg(&params)
            .output()
            .unwrap(),
    );
    assert_eq!(tight["pages"][0]["findings"].as_array().unwrap().len(), 0);
}

#[test]
fn colour_images_are_accepted_and_reduced_to_gray() {
    let mut data = b"P6\n8 8\n255\n".to_vec();
    data.extend(std::iter::repeat_n([250u8, 250, 250], 64).flatten());
    let path = scratch("colour.ppm");
    fs::write(&path, data).unwrap();
    let report = stdout_json(&bin().arg("analyse").arg(&path).output().unwrap());
    assert_eq!(report["pages"][0]["findings"][0]["kind"], "blank_page");
}

#[test]
fn pdf_input_to_analyse_points_at_the_structure_subcommand() {
    let path = scratch("fake.pdf");
    fs::write(&path, b"%PDF-1.7\n%%EOF\n").unwrap();
    let out = bin().arg("analyse").arg(&path).output().unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("PDF") && err.contains("pagetriage structure"),
        "stderr: {err}"
    );
}

/// One page with visible text, built by hand: header, five objects, classic xref.
fn tiny_pdf() -> Vec<u8> {
    let content = b"BT /F1 12 Tf 72 700 Td (A short line of visible text) Tj ET";
    let objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_vec(),
        [
            format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(),
            content.to_vec(),
            b"\nendstream".to_vec(),
        ]
        .concat(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n",
            objs.len() + 1
        )
        .as_bytes(),
    );
    out
}

#[test]
fn structure_subcommand_reports_the_text_layer_of_a_pdf() {
    let path = scratch("tiny.pdf");
    fs::write(&path, tiny_pdf()).unwrap();
    let out = bin().arg("structure").arg(&path).output().unwrap();
    let report = stdout_json(&out);
    assert_eq!(report["schema"], "pagetriage-structure/0.1");
    assert_eq!(report["document"]["source"], "pdf");
    assert_eq!(report["document"]["pages"], 1);
    assert_eq!(report["pages"][0]["text_layer"]["state"], "native_text");
    assert_eq!(report["pages"][0]["script"]["dominant"], "latin");

    let again = bin().arg("structure").arg(&path).output().unwrap();
    assert_eq!(out.stdout, again.stdout, "canonical output is stable");
    let pretty = bin()
        .args(["structure", "--pretty"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&pretty.stdout).contains("\n  "));
}

#[test]
fn structure_subcommand_rejects_non_pdf_and_missing_files() {
    let junk = scratch("not-a.pdf");
    fs::write(&junk, b"just some text").unwrap();
    let out = bin().arg("structure").arg(&junk).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a PDF"));
    let out = bin()
        .arg("structure")
        .arg(scratch("nope.pdf"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot read"));
}

#[test]
fn missing_and_undecodable_files_fail_cleanly() {
    let out = bin()
        .arg("analyse")
        .arg(scratch("does-not-exist.png"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot read"));

    let junk = scratch("junk.png");
    fs::write(&junk, b"definitely not an image").unwrap();
    let out = bin().arg("analyse").arg(&junk).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot decode image"));
}

#[test]
fn schema_subcommand_prints_the_core_schema() {
    let out = bin().arg("schema").output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        pagetriage_core::schema_json()
    );
}

#[test]
fn params_subcommand_prints_parseable_toml_matching_the_defaults() {
    let out = bin().arg("params").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("[blank_page]") && text.contains("ink_delta"));
    assert!(pagetriage_core::Params::with_overrides(&text).is_ok());
}

#[test]
fn duplicates_command_links_a_rescan_to_its_original() {
    // Blocky page, a brightness-scaled copy, and an unrelated page.
    let (w, h) = (96usize, 120usize);
    let blocks = |seed: usize| -> Vec<u8> {
        (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let cell = (x / 12 + 8 * (y / 15)) * seed;
                200 - ((cell * 37 + seed * 11) % 150) as u8
            })
            .collect()
    };
    let a = blocks(3);
    let scaled: Vec<u8> = a.iter().map(|&v| (u32::from(v) * 9 / 10) as u8).collect();
    let b = blocks(7);
    let files = [
        write_pgm("dup_a.pgm", w, h, &a),
        write_pgm("dup_b.pgm", w, h, &b),
        write_pgm("dup_a_scaled.pgm", w, h, &scaled),
    ];
    let out = bin().arg("duplicates").args(&files).output().unwrap();
    let pages = stdout_json(&out);
    let pages = pages.as_array().unwrap();
    assert_eq!(pages.len(), 3);
    assert_eq!(pages[0]["near_duplicate_of"], serde_json::Value::Null);
    assert_eq!(pages[2]["near_duplicate_of"], 0, "{pages:?}");
    assert_eq!(pages[0]["phash"].as_str().unwrap().len(), 16);
}

#[test]
fn duplicates_command_needs_at_least_one_file() {
    let out = bin().arg("duplicates").output().unwrap();
    assert!(!out.status.success());
}

/// A page of drawn text-like lines: dark bars 8 px high and 300 px wide every 30 px.
fn bars_page(w: usize, h: usize) -> Vec<u8> {
    let mut g = vec![245u8; w * h];
    let mut y = 40;
    while y + 20 < h {
        for yy in y..y + 8 {
            for xx in 20..w - 20 {
                if (xx / 7) % 4 != 0 {
                    g[yy * w + xx] = 30;
                }
            }
        }
        y += 30;
    }
    g
}

#[test]
fn prepare_writes_images_and_a_manifest() {
    let path = write_pgm("prep_bars.pgm", 400, 300, &bars_page(400, 300));
    let dir = scratch("prep_bars_out");
    let out = bin()
        .args(["prepare", "--dpi", "150", "-o"])
        .arg(&dir)
        .arg(&path)
        .output()
        .unwrap();
    let on_stdout = stdout_json(&out);
    assert_eq!(on_stdout["schema"], "pagetriage-prepare/0.1");
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest, on_stdout, "the file and stdout say the same");
    let page = image::open(dir.join("page.png")).expect("page.png is an image");
    let info = &manifest["outputs"][0];
    assert_eq!(info["file"], "page.png");
    assert_eq!(info["width"], page.width());
    assert_eq!(info["height"], page.height());
}

#[test]
fn prepare_rotate_turns_the_page_first_and_says_so() {
    let path = write_pgm("prep_rot.pgm", 400, 300, &bars_page(400, 300));
    let dir = scratch("prep_rot_out");
    let out = bin()
        .args(["prepare", "--dpi", "150", "--rotate", "90", "-o"])
        .arg(&dir)
        .arg(&path)
        .output()
        .unwrap();
    let m = stdout_json(&out);
    assert_eq!(m["steps"][0]["op"], "rotate");
    assert_eq!(m["steps"][0]["clockwise_deg"], 90.0);
    assert_eq!(m["frame"]["source_width"], 400);
    assert_eq!(
        m["frame"]["width"], 300,
        "turned a quarter, so the sides swap"
    );
}

#[test]
fn prepare_a_blank_page_writes_only_the_manifest_and_clears_an_earlier_run() {
    let dir = scratch("prep_blank_out");
    fs::create_dir_all(dir.join("regions")).unwrap();
    fs::write(dir.join("regions/old.png"), b"x").unwrap();
    fs::write(dir.join("page.png"), b"x").unwrap();
    let path = write_pgm("prep_blank.pgm", 200, 300, &vec![250; 60_000]);
    let out = bin()
        .args(["prepare", "--dpi", "150", "-o"])
        .arg(&dir)
        .arg(&path)
        .output()
        .unwrap();
    let m = stdout_json(&out);
    assert_eq!(m["skip"]["reason"], "blank");
    assert_eq!(m["outputs"].as_array().unwrap().len(), 0);
    assert!(dir.join("manifest.json").is_file());
    assert!(!dir.join("page.png").exists() && !dir.join("regions").exists());
}

#[test]
fn prepare_refuses_a_bad_rotation() {
    let path = write_pgm("prep_bad.pgm", 20, 30, &vec![250; 600]);
    let out = bin()
        .args(["prepare", "--rotate", "45", "-o"])
        .arg(scratch("prep_bad_out"))
        .arg(&path)
        .output()
        .unwrap();
    assert!(!out.status.success());
}
