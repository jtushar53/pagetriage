//! `pagetriage` command line: analyse a page image, print the schema, print default params.
//!
//! Phase 0 accepts raster images only (PNG, JPEG, TIFF, PNM, BMP). PDF input needs the structural
//! layer, which arrives in Phase 1.

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
    /// Print the JSON Schema of the report
    Schema,
    /// Print the embedded default detector parameters as TOML
    Params,
}

#[derive(Args)]
struct AnalyseArgs {
    /// Page image (PNG, JPEG, TIFF, PNM or BMP; the first frame is used)
    file: PathBuf,
    /// Resolution the image was scanned at
    #[arg(long, default_value_t = 150)]
    dpi: u16,
    /// Include raw detector feature values under `debug`
    #[arg(long)]
    debug: bool,
    /// Indent the output. The default compact form is the canonical, byte-reproducible one.
    #[arg(long)]
    pretty: bool,
    /// TOML file merged over the embedded default parameters
    #[arg(long, value_name = "FILE")]
    params: Option<PathBuf>,
    /// Run only these detectors (comma separated)
    #[arg(long, value_delimiter = ',')]
    detectors: Option<Vec<String>>,
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

fn analyse(args: &AnalyseArgs) -> Result<(), String> {
    let bytes =
        fs::read(&args.file).map_err(|e| format!("cannot read {}: {e}", args.file.display()))?;
    let page = load_page(&bytes)?;

    let params_toml = match &args.params {
        Some(path) => Some(
            fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?,
        ),
        None => None,
    };
    let opts = serde_json::json!({
        "debug": args.debug,
        "detectors": args.detectors,
        "params": params_toml,
    })
    .to_string();

    let input = PageInput {
        gray: &page.gray,
        rgb: page.rgb.as_deref(),
        width: page.width,
        height: page.height,
        dpi: args.dpi,
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
            "PDF input is not supported yet: the structural layer arrives in Phase 1. \
                    Rasterise the page and pass the image."
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
