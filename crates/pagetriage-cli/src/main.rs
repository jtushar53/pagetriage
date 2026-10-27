//! `pagetriage` command line: analyse a page image, print the schema, print default params.
//!
//! `analyse` takes raster images (PNG, JPEG, TIFF, PNM, BMP). `structure` takes a PDF and reports
//! the per-page text-layer state, script and provenance without rendering anything.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use image::DynamicImage;
use pagetriage_core::hash::sha256_hex;
use pagetriage_core::raster::rgb_to_gray;
use pagetriage_core::schema::DocumentInfo;
use pagetriage_core::{
    PageInput, Params, Report, analyse_page_report, schema_json, to_canonical_json,
};

#[derive(Parser)]
#[command(
    name = "pagetriage",
    version,
    about = "Deterministic page-content analyser for scanned documents"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Analyse a page image and print the JSON report on stdout
    Analyse(AnalyseArgs),
    /// Read a PDF and print its per-page text-layer state, script and provenance as JSON
    Structure(StructureArgs),
    /// Hash page images and report near-duplicates among them (pages in the order given)
    Duplicates(DuplicatesArgs),
    /// Analyse the page images of one document and print one report with the document-level pass
    /// (near-duplicate pages, registrar endorsements) applied
    Document(DocumentArgs),
    /// Apply the page plan: turn, level, crop and scale a page image and cut out its regions, into a
    /// directory of images with a manifest (`manifest.json`, also printed on stdout)
    Prepare(PrepareArgs),
    /// Print the JSON Schema of the report
    Schema,
    /// Print the embedded default detector parameters as TOML
    Params,
}

#[derive(Args)]
struct AnalyseArgs {
    /// Page image (PNG, JPEG, TIFF, PNM or BMP; the first frame is used)
    file: PathBuf,
    /// Resolution the image was scanned at. Without it the image's own header is read (PNG, JPEG,
    /// TIFF, BMP); failing that, 150 is used, with a paper-size hint. Which applied is printed on stderr.
    #[arg(long)]
    dpi: Option<u16>,
    /// Refuse images with more pixels than this (default: no limit)
    #[arg(long, value_name = "PIXELS")]
    max_pixels: Option<u64>,
    /// Include raw detector feature values under `debug`
    #[arg(long)]
    debug: bool,
    /// Run the text-line detectors even on a badly blurred page (by default they are skipped there)
    #[arg(long)]
    no_gate: bool,
    /// Indent the output. The default compact form is the canonical, byte-reproducible one.
    #[arg(long)]
    pretty: bool,
    /// TOML file merged over the embedded default parameters
    #[arg(long, value_name = "FILE")]
    params: Option<PathBuf>,
    /// Run only these detectors (comma separated)
    #[arg(long, value_delimiter = ',')]
    detectors: Option<Vec<String>>,
    /// JSON file with the page's text boxes, `[[x0, y0, x1, y1], ...]` in pixels, for the `coverage`
    /// detector
    #[arg(long, value_name = "FILE")]
    text_boxes: Option<PathBuf>,
}

#[derive(Args)]
struct PrepareArgs {
    /// Page image (PNG, JPEG, TIFF, PNM or BMP; the first frame is used)
    file: PathBuf,
    /// Directory for the output images and `manifest.json` (created if missing). Files of an earlier
    /// run (`page.png`, `tiles/`, `regions/`, `manifest.json`) are replaced.
    #[arg(long, short, value_name = "DIR")]
    out: PathBuf,
    /// Resolution the image was scanned at (see `analyse --dpi`)
    #[arg(long)]
    dpi: Option<u16>,
    /// Refuse images with more pixels than this (default: no limit)
    #[arg(long, value_name = "PIXELS")]
    max_pixels: Option<u64>,
    /// Run the text-line detectors even on a badly blurred page
    #[arg(long)]
    no_gate: bool,
    /// Turn the page clockwise by this many degrees (0, 90, 180 or 270) before anything else, when
    /// you know it was scanned sideways or upside down and the detector is not sure
    #[arg(long, value_name = "DEG", value_parser = clap::value_parser!(u32).range(0..=270))]
    rotate: Option<u32>,
    /// TOML file merged over the embedded default parameters
    #[arg(long, value_name = "FILE")]
    params: Option<PathBuf>,
    /// Run only these detectors (comma separated; `plan` is always added). Default: all that feed the plan
    #[arg(long, value_delimiter = ',')]
    detectors: Option<Vec<String>>,
}

#[derive(Args)]
struct StructureArgs {
    /// PDF file
    file: PathBuf,
    /// Indent the output. The default compact form is the canonical, byte-reproducible one.
    #[arg(long)]
    pretty: bool,
    /// TOML file merged over the embedded structural parameters (`[text_layer]`, `[font_script]`,
    /// `[provenance]`, `[limits]`)
    #[arg(long, value_name = "FILE")]
    params: Option<PathBuf>,
}

#[derive(Args)]
struct DocumentArgs {
    /// Page images, in page order
    #[arg(required = true)]
    files: Vec<PathBuf>,
    /// Resolution the images were scanned at (default: read from the first image, as for `analyse`)
    #[arg(long)]
    dpi: Option<u16>,
    /// Include raw detector feature values under `debug`
    #[arg(long)]
    debug: bool,
    /// Indent the output. The default compact form is the canonical, byte-reproducible one.
    #[arg(long)]
    pretty: bool,
    /// TOML file merged over the embedded default parameters
    #[arg(long, value_name = "FILE")]
    params: Option<PathBuf>,
    /// Run only these detectors (comma separated); `phash` is added so near-duplicates are found
    #[arg(long, value_delimiter = ',')]
    detectors: Option<Vec<String>>,
}

#[derive(Args)]
struct DuplicatesArgs {
    /// Page images, in page order; a page is matched against the pages before it
    #[arg(required = true)]
    files: Vec<PathBuf>,
    /// TOML file merged over the embedded default parameters (see `[phash]` in `params`)
    #[arg(long, value_name = "FILE")]
    params: Option<PathBuf>,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Analyse(args) => analyse(&args),
        Command::Structure(args) => structure(&args),
        Command::Duplicates(args) => duplicates(&args),
        Command::Document(args) => document(&args),
        Command::Prepare(args) => prepare(&args),
        Command::Schema => {
            print!("{}", schema_json());
            Ok(())
        }
        Command::Params => {
            // Fail loudly if the embedded defaults ever stop parsing.
            Params::defaults().map_err(|e| e.to_string())?;
            print!("{}", Params::defaults_toml());
            Ok(())
        }
    }
}

fn structure(args: &StructureArgs) -> Result<(), String> {
    let bytes =
        fs::read(&args.file).map_err(|e| format!("cannot read {}: {e}", args.file.display()))?;
    let params_toml = match &args.params {
        Some(path) => Some(
            fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?,
        ),
        None => None,
    };
    let opts = serde_json::json!({ "params": params_toml }).to_string();
    let canonical = pagetriage_core::structural::analyse_pdf_structure(&bytes, &opts)
        .map_err(|e| e.to_string())?;
    if args.pretty {
        let value: serde_json::Value =
            serde_json::from_str(&canonical).map_err(|e| e.to_string())?;
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
        );
    } else {
        println!("{canonical}");
    }
    Ok(())
}

fn analyse(args: &AnalyseArgs) -> Result<(), String> {
    let bytes =
        fs::read(&args.file).map_err(|e| format!("cannot read {}: {e}", args.file.display()))?;
    let page = load_page(&bytes)?;
    let dpi = resolve_dpi(args.dpi, &bytes, &page);

    let params_toml = match &args.params {
        Some(path) => Some(
            fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?,
        ),
        None => None,
    };
    let text_boxes: Option<serde_json::Value> = match &args.text_boxes {
        Some(path) => Some(
            serde_json::from_str(
                &fs::read_to_string(path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))?,
            )
            .map_err(|e| format!("{}: {e}", path.display()))?,
        ),
        None => None,
    };
    let opts = serde_json::json!({
        "debug": args.debug,
        "detectors": args.detectors,
        "params": params_toml,
        "text_boxes": text_boxes,
        "max_pixels": args.max_pixels,
        "no_gate": args.no_gate,
    })
    .to_string();

    let input = PageInput {
        gray: &page.gray,
        rgb: page.rgb.as_deref(),
        width: page.width,
        height: page.height,
        dpi,
    };
    let page_report = analyse_page_report(&input, None, &opts).map_err(|e| e.to_string())?;

    let report = Report::new(
        DocumentInfo {
            sha256: sha256_hex(&bytes),
            pages: 1,
            source: "image".into(),
            producer: None,
            incremental_updates: None,
        },
        vec![page_report],
    );
    let canonical = to_canonical_json(&report).map_err(|e| e.to_string())?;
    if args.pretty {
        let value: serde_json::Value =
            serde_json::from_str(&canonical).map_err(|e| e.to_string())?;
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
        );
    } else {
        println!("{canonical}");
    }
    Ok(())
}

fn prepare(args: &PrepareArgs) -> Result<(), String> {
    let bytes =
        fs::read(&args.file).map_err(|e| format!("cannot read {}: {e}", args.file.display()))?;
    let page = load_page(&bytes)?;
    let dpi = resolve_dpi(args.dpi, &bytes, &page);
    let params_toml = match &args.params {
        Some(path) => Some(
            fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?,
        ),
        None => None,
    };
    let opts = serde_json::json!({
        "detectors": args.detectors,
        "params": params_toml,
        "max_pixels": args.max_pixels,
        "no_gate": args.no_gate,
        "pre_rotate_deg": args.rotate,
    })
    .to_string();
    let input = PageInput {
        gray: &page.gray,
        rgb: page.rgb.as_deref(),
        width: page.width,
        height: page.height,
        dpi,
    };
    let prepared = pagetriage_core::prepare::prepare(&input, &opts).map_err(|e| e.to_string())?;

    // Remove what an earlier run left, so the directory never mixes two runs.
    for stale in ["tiles", "regions"] {
        let path = args.out.join(stale);
        if path.is_dir() {
            fs::remove_dir_all(&path)
                .map_err(|e| format!("cannot clear {}: {e}", path.display()))?;
        }
    }
    for stale in ["page.png", "manifest.json"] {
        let path = args.out.join(stale);
        if path.is_file() {
            fs::remove_file(&path).map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
        }
    }
    fs::create_dir_all(&args.out)
        .map_err(|e| format!("cannot create {}: {e}", args.out.display()))?;
    for out in &prepared.outputs {
        let path = args.out.join(&out.name);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        let r = &out.raster;
        let color = if r.channels == 3 {
            image::ColorType::Rgb8
        } else {
            image::ColorType::L8
        };
        image::save_buffer(&path, &r.data, r.width as u32, r.height as u32, color)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    let canonical = to_canonical_json(&prepared.manifest).map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_str(&canonical).map_err(|e| e.to_string())?;
    let pretty = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    let manifest_path = args.out.join("manifest.json");
    fs::write(&manifest_path, format!("{pretty}\n"))
        .map_err(|e| format!("cannot write {}: {e}", manifest_path.display()))?;

    let m = &prepared.manifest;
    match &m.skip {
        Some(skip) => eprintln!("skipped: the page is {}; no images written", skip.reason),
        None => eprintln!(
            "wrote {} image{} to {} (analysed {}x; steps: {})",
            prepared.outputs.len(),
            if prepared.outputs.len() == 1 { "" } else { "s" },
            args.out.display(),
            m.analyses,
            if m.steps.is_empty() {
                "none".to_owned()
            } else {
                m.steps
                    .iter()
                    .map(|s| format!("{} {:.1}", s.op, s.clockwise_deg))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        ),
    }
    println!("{canonical}");
    Ok(())
}

fn document(args: &DocumentArgs) -> Result<(), String> {
    let params_toml = match &args.params {
        Some(path) => Some(
            fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?,
        ),
        None => None,
    };
    let detectors = args.detectors.clone().map(|mut d| {
        if !d.iter().any(|n| n == "phash") {
            d.push("phash".into());
        }
        d
    });
    let mut all_bytes = Vec::new();
    let mut reports = Vec::new();
    // One resolution for the whole document: the flag, else what the first image says.
    let mut doc_dpi = args.dpi;
    for (index, file) in args.files.iter().enumerate() {
        let bytes = fs::read(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
        let page = load_page(&bytes).map_err(|e| format!("{}: {e}", file.display()))?;
        let dpi = *doc_dpi.get_or_insert_with(|| resolve_dpi(None, &bytes, &page));
        let opts = serde_json::json!({
            "debug": args.debug,
            "detectors": detectors,
            "params": params_toml,
            "page_index": index,
        })
        .to_string();
        let input = PageInput {
            gray: &page.gray,
            rgb: page.rgb.as_deref(),
            width: page.width,
            height: page.height,
            dpi,
        };
        reports.push(analyse_page_report(&input, None, &opts).map_err(|e| e.to_string())?);
        all_bytes.extend_from_slice(&bytes);
    }
    let pages_json = to_canonical_json(&reports).map_err(|e| e.to_string())?;
    let opts = serde_json::json!({ "params": params_toml }).to_string();
    let finished =
        pagetriage_core::finish_document(&pages_json, &opts).map_err(|e| e.to_string())?;
    let pages = serde_json::from_str(&finished).map_err(|e| e.to_string())?;
    let report = Report::new(
        DocumentInfo {
            sha256: sha256_hex(&all_bytes),
            pages: u32::try_from(args.files.len()).map_err(|e| e.to_string())?,
            source: "image".into(),
            producer: None,
            incremental_updates: None,
        },
        pages,
    );
    let canonical = to_canonical_json(&report).map_err(|e| e.to_string())?;
    if args.pretty {
        let value: serde_json::Value =
            serde_json::from_str(&canonical).map_err(|e| e.to_string())?;
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
        );
    } else {
        println!("{canonical}");
    }
    Ok(())
}

/// Hash every file with the `phash` detector and print one JSON object per page: its hashes and,
/// when an earlier page is within the near-duplicate distance, that page's index.
fn duplicates(args: &DuplicatesArgs) -> Result<(), String> {
    let params_toml = match &args.params {
        Some(path) => Some(
            fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?,
        ),
        None => None,
    };
    let params = match &params_toml {
        Some(toml) => Params::with_overrides(toml),
        None => Params::defaults(),
    }
    .map_err(|e| e.to_string())?;

    let mut reports = Vec::new();
    for (index, file) in args.files.iter().enumerate() {
        let bytes = fs::read(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
        let page = load_page(&bytes).map_err(|e| format!("{}: {e}", file.display()))?;
        let opts = serde_json::json!({
            "detectors": ["phash"],
            "page_index": index,
            "params": params_toml,
        })
        .to_string();
        let input = PageInput {
            gray: &page.gray,
            rgb: None,
            width: page.width,
            height: page.height,
            dpi: 150,
        };
        reports.push(analyse_page_report(&input, None, &opts).map_err(|e| e.to_string())?);
    }
    pagetriage_core::pixel::phash::mark_near_duplicates(&mut reports, &params.phash)
        .map_err(|e| e.to_string())?;

    let pages: Vec<serde_json::Value> = args
        .files
        .iter()
        .zip(&reports)
        .map(|(file, r)| {
            serde_json::json!({
                "file": file.display().to_string(),
                "index": r.index,
                "phash": r.phash,
                "dhash": r.dhash,
                "near_duplicate_of": r.near_duplicate_of,
            })
        })
        .collect();
    println!("{}", serde_json::Value::Array(pages));
    Ok(())
}

/// The resolution to analyse an image at: the flag; else the image's own header; else
/// 150. What was used is said on stderr, with a paper-size hint when the pixel size fits one.
fn resolve_dpi(flag: Option<u16>, bytes: &[u8], page: &LoadedPage) -> u16 {
    if let Some(d) = flag {
        return d;
    }
    if let Some(h) = pagetriage_core::scale::image_dpi(bytes) {
        let d = h.x.round().clamp(1.0, f64::from(u16::MAX)) as u16;
        eprintln!(
            "note: resolution {d} dpi from the image header ({})",
            h.source
        );
        return d;
    }
    // A pixel size that fits a standard paper at a standard resolution is only a hint: any crop of a
    // scan fits some paper somewhere. It is printed, never used.
    let hint = pagetriage_core::scale::paper_matches(page.width, page.height, 0.01)
        .first()
        .map(|m| {
            format!(
                " (its {}x{} pixels would fit {} at {} dpi)",
                page.width, page.height, m.paper, m.dpi
            )
        })
        .unwrap_or_default();
    eprintln!("note: no resolution in the image; assuming 150 dpi{hint}; pass --dpi to set it");
    150
}

struct LoadedPage {
    gray: Vec<u8>,
    rgb: Option<Vec<u8>>,
    width: u32,
    height: u32,
}

/// Decode an image into the gray (and, if the source has colour, RGB) buffers the core expects.
/// 8-bit gray is used as is. Anything else becomes RGB (alpha dropped, not composited), and gray is
/// derived with the core's integer luma, not the `image` crate's float conversion, so the result
/// does not depend on a third-party rounding choice.
fn load_page(bytes: &[u8]) -> Result<LoadedPage, String> {
    if bytes.starts_with(b"%PDF-") {
        return Err(
            "`analyse` reads page images, not PDFs. Use `pagetriage structure FILE.pdf` for the \
             PDF's text-layer, script and provenance report, or rasterise a page and pass the image."
                .into(),
        );
    }
    let img = image::load_from_memory(bytes).map_err(|e| format!("cannot decode image: {e}"))?;
    let (width, height) = (img.width(), img.height());
    match img {
        DynamicImage::ImageLuma8(buf) => Ok(LoadedPage {
            gray: buf.into_raw(),
            rgb: None,
            width,
            height,
        }),
        other => {
            let rgb = other.to_rgb8().into_raw();
            let gray = rgb_to_gray(&rgb).map_err(|e| e.to_string())?;
            Ok(LoadedPage {
                gray,
                rgb: Some(rgb),
                width,
                height,
            })
        }
    }
}
