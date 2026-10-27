//! Applying the page plan (module M14, `prepare`): analyse a page, turn it upright, level it, crop it
//! to its content, scale it to the size a reader wants, and cut out the regions the plan routes to
//! different readers. The result is a set of images and a manifest saying what each is and where it
//! came from, so a downstream OCR or vision model gets small, upright, level pictures instead of the
//! scan as it came off the scanner.
//!
//! Pixels are integers throughout and the one rotation uses libm's sin and cos once per call, so the
//! output is the same on every target. The core still does no I/O: the host writes the files.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::analyse::{PageInput, analyse_page_report};
use crate::error::{Error, Result};
use crate::hash::sha256_hex;
use crate::math;
use crate::params::Params;
use crate::raster::rgb_to_gray;
use crate::schema::{PagePlan, PageReport, PlanSkip};

/// Schema id of the manifest.
pub const MANIFEST_ID: &str = "pagetriage-prepare/0.1";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareParams {
    pub detectors: Vec<String>,
    pub min_deskew_deg: f64,
    pub max_deskew_deg: f64,
    pub max_analyses: u32,
    pub min_scale_change: f64,
    pub max_output_side_px: u32,
    pub region_pad_in: f64,
    pub skip_routes: Vec<String>,
}

impl PrepareParams {
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidParams(format!("prepare: {m}")));
        let all = [
            self.min_deskew_deg,
            self.max_deskew_deg,
            self.min_scale_change,
            self.region_pad_in,
        ];
        if all.iter().any(|v| !v.is_finite() || v.abs() > 1.0e6) {
            return bad("every value must be finite and within 1e6");
        }
        if self.min_deskew_deg < 0.0 || self.max_deskew_deg < self.min_deskew_deg {
            return bad("0 <= min_deskew_deg <= max_deskew_deg");
        }
        if self.max_analyses == 0 || self.max_analyses > 8 {
            return bad("max_analyses must be in 1..=8");
        }
        if self.min_scale_change < 0.0 || self.region_pad_in < 0.0 {
            return bad("min_scale_change and region_pad_in must be >= 0");
        }
        if self.max_output_side_px < 64 {
            return bad("max_output_side_px must be >= 64");
        }
        Ok(())
    }
}

// ------------------------------------------------------------------------------------ pixels

/// A row-major 8-bit image with 1 (gray) or 3 (RGB) channels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raster {
    pub width: usize,
    pub height: usize,
    pub channels: usize,
    pub data: Vec<u8>,
}

impl Raster {
    pub fn new(width: usize, height: usize, channels: usize, data: Vec<u8>) -> Result<Self> {
        if !(channels == 1 || channels == 3) || data.len() != width * height * channels {
            return Err(Error::InvalidInput(format!(
                "a {width}x{height} raster with {channels} channels needs {} bytes, got {}",
                width * height * channels,
                data.len()
            )));
        }
        Ok(Self {
            width,
            height,
            channels,
            data,
        })
    }

    /// The gray the analyser sees: the data itself, or the core's integer luma of the RGB.
    pub fn gray(&self) -> Result<Vec<u8>> {
        if self.channels == 1 {
            Ok(self.data.clone())
        } else {
            rgb_to_gray(&self.data)
        }
    }

    /// SHA-256 of the size line and the pixels.
    pub fn sha256(&self) -> String {
        let mut bytes = format!("{}x{}x{}\n", self.width, self.height, self.channels).into_bytes();
        bytes.extend_from_slice(&self.data);
        sha256_hex(&bytes)
    }

    fn px(&self, x: usize, y: usize) -> &[u8] {
        let i = (y * self.width + x) * self.channels;
        &self.data[i..i + self.channels]
    }
}

/// Turn clockwise by 0, 90, 180 or 270 degrees.
pub fn rotate_quarter(img: &Raster, cw_deg: u32) -> Result<Raster> {
    let (w, h, c) = (img.width, img.height, img.channels);
    let (nw, nh) = match cw_deg {
        0 | 180 => (w, h),
        90 | 270 => (h, w),
        other => {
            return Err(Error::InvalidInput(format!(
                "a quarter turn is 0, 90, 180 or 270 degrees, not {other}"
            )));
        }
    };
    let mut out = vec![0u8; img.data.len()];
    for y in 0..h {
        for x in 0..w {
            let (nx, ny) = match cw_deg {
                0 => (x, y),
                90 => (h - 1 - y, x),
                180 => (w - 1 - x, h - 1 - y),
                _ => (y, w - 1 - x),
            };
            let (src, dst) = ((y * w + x) * c, (ny * nw + nx) * c);
            out[dst..dst + c].copy_from_slice(&img.data[src..src + c]);
        }
    }
    Raster::new(nw, nh, c, out)
}

/// The colour of the paper: the most common gray, and for RGB the mean colour of the pixels with that gray.
pub fn paper_fill(img: &Raster) -> Result<Vec<u8>> {
    let gray = img.gray()?;
    let mut hist = [0u64; 256];
    for g in &gray {
        hist[usize::from(*g)] += 1;
    }
    let mode = (0..256)
        .max_by_key(|&v| (hist[v], std::cmp::Reverse(v)))
        .unwrap_or(255);
    if img.channels == 1 {
        return Ok(vec![mode as u8]);
    }
    let (mut sum, mut n) = ([0u64; 3], 0u64);
    for (i, g) in gray.iter().enumerate() {
        if usize::from(*g) == mode {
            for (k, s) in sum.iter_mut().enumerate() {
                *s += u64::from(img.data[i * 3 + k]);
            }
            n += 1;
        }
    }
    let n = n.max(1);
    Ok(sum.iter().map(|s| ((*s + n / 2) / n) as u8).collect())
}

/// Turn clockwise by `cw_deg` (a page whose text rises to the right needs a positive angle) about the
/// middle, onto a canvas just big enough to hold the whole page, with `fill` around it. Bilinear, with
/// 8-bit weights; samples outside the page are `fill`.
pub fn deskew(img: &Raster, cw_deg: f64, fill: &[u8]) -> Result<Raster> {
    let theta = cw_deg * core::f64::consts::PI / 180.0;
    let (s, c) = (math::sin(theta), math::cos(theta));
    let (w, h, ch) = (img.width, img.height, img.channels);
    if fill.len() != ch {
        return Err(Error::InvalidInput(
            "fill has the wrong number of channels".into(),
        ));
    }
    let nw = ((w as f64) * c.abs() + (h as f64) * s.abs() - 1.0e-9)
        .ceil()
        .max(1.0) as usize;
    let nh = ((w as f64) * s.abs() + (h as f64) * c.abs() - 1.0e-9)
        .ceil()
        .max(1.0) as usize;
    let (csx, csy) = ((w as f64 - 1.0) / 2.0, (h as f64 - 1.0) / 2.0);
    let (cdx, cdy) = ((nw as f64 - 1.0) / 2.0, (nh as f64 - 1.0) / 2.0);
    let mut out = vec![0u8; nw * nh * ch];
    let sample = |ix: i64, iy: i64, k: usize| -> u32 {
        if ix < 0 || iy < 0 || ix >= w as i64 || iy >= h as i64 {
            u32::from(fill[k])
        } else {
            u32::from(img.data[(iy as usize * w + ix as usize) * ch + k])
        }
    };
    for y in 0..nh {
        let dy = y as f64 - cdy;
        for x in 0..nw {
            let dx = x as f64 - cdx;
            let sx = c * dx + s * dy + csx;
            let sy = -s * dx + c * dy + csy;
            let (fx, fy) = (sx.floor(), sy.floor());
            let (ix, iy) = (fx as i64, fy as i64);
            let wx = ((sx - fx) * 256.0) as u32;
            let wy = ((sy - fy) * 256.0) as u32;
            for k in 0..ch {
                let top = sample(ix, iy, k) * (256 - wx) + sample(ix + 1, iy, k) * wx;
                let bottom = sample(ix, iy + 1, k) * (256 - wx) + sample(ix + 1, iy + 1, k) * wx;
                let v = (top * (256 - wy) + bottom * wy + 32768) >> 16;
                out[(y * nw + x) * ch + k] = v as u8;
            }
        }
    }
    Raster::new(nw, nh, ch, out)
}

/// The part of the image inside `bbox` (`[x0, y0, x1, y1]`, inclusive), clamped to the image.
pub fn crop(img: &Raster, bbox: [u32; 4]) -> Result<Raster> {
    let x1 = (bbox[2] as usize).min(img.width.saturating_sub(1));
    let y1 = (bbox[3] as usize).min(img.height.saturating_sub(1));
    let (x0, y0) = ((bbox[0] as usize).min(x1), (bbox[1] as usize).min(y1));
    let (w, h, c) = (x1 - x0 + 1, y1 - y0 + 1, img.channels);
    let mut out = Vec::with_capacity(w * h * c);
    for y in y0..=y1 {
        let i = (y * img.width + x0) * c;
        out.extend_from_slice(&img.data[i..i + w * c]);
    }
    Raster::new(w, h, c, out)
}

/// Resize to `nw` x `nh`: the average over the covered source area when the image gets smaller (exact
/// integer weights), bilinear when it gets larger.
pub fn resize(img: &Raster, nw: usize, nh: usize) -> Result<Raster> {
    let (sw, sh, c) = (img.width, img.height, img.channels);
    if nw == 0 || nh == 0 {
        return Err(Error::InvalidInput(
            "a resized image needs at least one pixel".into(),
        ));
    }
    if (nw, nh) == (sw, sh) {
        return Ok(img.clone());
    }
    let mut out = vec![0u8; nw * nh * c];
    if nw <= sw && nh <= sh {
        // Source pixel i covers [i*n, (i+1)*n) in units of 1/n source pixels; output pixel x covers
        // [x*s, (x+1)*s). The overlap lengths are integers and add up to s.
        let weights = |s: usize, n: usize| -> Vec<Vec<(usize, u64)>> {
            (0..n)
                .map(|x| {
                    let (lo, hi) = (x * s, (x + 1) * s);
                    (lo / n..=(hi - 1) / n)
                        .filter_map(|i| {
                            let o = hi.min((i + 1) * n).saturating_sub(lo.max(i * n));
                            (o > 0).then_some((i, o as u64))
                        })
                        .collect()
                })
                .collect()
        };
        let (wx, wy) = (weights(sw, nw), weights(sh, nh));
        let total = (sw * sh) as u64;
        // Horizontal pass: for every source row and output column, the weighted sum.
        let mut rows = vec![0u64; sh * nw * c];
        for y in 0..sh {
            for (x, list) in wx.iter().enumerate() {
                for k in 0..c {
                    rows[(y * nw + x) * c + k] = list
                        .iter()
                        .map(|&(i, w)| u64::from(img.data[(y * sw + i) * c + k]) * w)
                        .sum();
                }
            }
        }
        for (y, ylist) in wy.iter().enumerate() {
            for x in 0..nw {
                for k in 0..c {
                    let sum: u64 = ylist
                        .iter()
                        .map(|&(j, w)| rows[(j * nw + x) * c + k] * w)
                        .sum();
                    out[(y * nw + x) * c + k] = ((sum + total / 2) / total) as u8;
                }
            }
        }
    } else {
        // Pixel centres: source coordinate (x + 0.5) * s / n - 0.5, in 1/65536 units.
        let coord = |x: usize, s: usize, n: usize| -> (usize, usize, u32) {
            let v = ((2 * x as i64 + 1) * s as i64 * 32768) / n as i64 - 32768;
            let v = v.clamp(0, (s as i64 - 1) * 65536);
            let i = (v >> 16) as usize;
            (i, (i + 1).min(s - 1), ((v & 0xFFFF) >> 8) as u32)
        };
        for y in 0..nh {
            let (y0, y1, wy) = coord(y, sh, nh);
            for x in 0..nw {
                let (x0, x1, wx) = coord(x, sw, nw);
                for k in 0..c {
                    let p = |xx: usize, yy: usize| u32::from(img.px(xx, yy)[k]);
                    let top = p(x0, y0) * (256 - wx) + p(x1, y0) * wx;
                    let bottom = p(x0, y1) * (256 - wx) + p(x1, y1) * wx;
                    out[(y * nw + x) * c + k] =
                        ((top * (256 - wy) + bottom * wy + 32768) >> 16) as u8;
                }
            }
        }
    }
    Raster::new(nw, nh, c, out)
}

// ------------------------------------------------------------------------------------ frame

/// How the prepared page (the "frame" every box in the manifest refers to) relates to the page as
/// given: first turned by `rotate_cw_deg`, then levelled by `deskew_cw_deg` about its middle onto a
/// larger canvas. Coordinates are pixel centres (the first pixel is at 0).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub source_width: u32,
    pub source_height: u32,
    pub rotate_cw_deg: u32,
    pub deskew_cw_deg: f64,
    pub width: u32,
    pub height: u32,
}

impl Frame {
    fn turned_size(&self) -> (f64, f64) {
        let (w, h) = (f64::from(self.source_width), f64::from(self.source_height));
        if self.rotate_cw_deg % 180 == 90 {
            (h, w)
        } else {
            (w, h)
        }
    }

    /// A point of the page as given, in the prepared frame.
    pub fn from_source(&self, x: f64, y: f64) -> (f64, f64) {
        let (w, h) = (f64::from(self.source_width), f64::from(self.source_height));
        let (x, y) = match self.rotate_cw_deg {
            90 => (h - 1.0 - y, x),
            180 => (w - 1.0 - x, h - 1.0 - y),
            270 => (y, w - 1.0 - x),
            _ => (x, y),
        };
        let (tw, th) = self.turned_size();
        let theta = self.deskew_cw_deg * core::f64::consts::PI / 180.0;
        let (s, c) = (math::sin(theta), math::cos(theta));
        let (dx, dy) = (x - (tw - 1.0) / 2.0, y - (th - 1.0) / 2.0);
        (
            c * dx - s * dy + (f64::from(self.width) - 1.0) / 2.0,
            s * dx + c * dy + (f64::from(self.height) - 1.0) / 2.0,
        )
    }

    /// A point of the prepared frame, in the page as given: the inverse of [`Frame::from_source`].
    pub fn to_source(&self, x: f64, y: f64) -> (f64, f64) {
        let (w, h) = (f64::from(self.source_width), f64::from(self.source_height));
        let (tw, th) = self.turned_size();
        let theta = self.deskew_cw_deg * core::f64::consts::PI / 180.0;
        let (s, c) = (math::sin(theta), math::cos(theta));
        let (dx, dy) = (
            x - (f64::from(self.width) - 1.0) / 2.0,
            y - (f64::from(self.height) - 1.0) / 2.0,
        );
        let (x, y) = (
            c * dx + s * dy + (tw - 1.0) / 2.0,
            -s * dx + c * dy + (th - 1.0) / 2.0,
        );
        match self.rotate_cw_deg {
            90 => (y, h - 1.0 - x),
            180 => (w - 1.0 - x, h - 1.0 - y),
            270 => (w - 1.0 - y, x),
            _ => (x, y),
        }
    }
}

// ------------------------------------------------------------------------------------ results

/// One image to hand to a reader.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    /// Path relative to the output directory: `page.png`, `tiles/tile_r0_c1.png`, `regions/region_000_ocr.png`.
    pub name: String,
    pub raster: Raster,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputInfo {
    pub file: String,
    /// `page`, `tile` or `region`.
    pub kind: String,
    /// For a region: its route (`ocr`, `handwriting`, `table`, `mark`, `vision`, ...); for a page or
    /// tile, `ocr`.
    pub route: String,
    /// For a region: the plan's kind (`text_block`, `table`, ...); else the same as `kind`.
    pub region_kind: String,
    pub priority: u32,
    /// `[x0, y0, x1, y1]` in the prepared frame (the turned and levelled page, before cropping and
    /// scaling); [`Frame::to_source`] maps points back to the page as given.
    pub bbox: [u32; 4],
    /// Factor applied to the pixels of `bbox` (1 when left alone).
    pub scale: f64,
    /// The resolution these pixels amount to: the page's dpi times `scale`.
    pub dpi: u32,
    pub width: u32,
    pub height: u32,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    /// `rotate` or `deskew`.
    pub op: String,
    pub clockwise_deg: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: String,
    /// SHA-256 of the page as given (what the plan calls its `input_sha256`).
    pub source_sha256: String,
    /// SHA-256 of the prepared frame.
    pub frame_sha256: String,
    pub dpi: u16,
    pub frame: Frame,
    pub steps: Vec<Step>,
    /// How many times the page was analysed (1 to `max_analyses`).
    pub analyses: u32,
    pub skip: Option<PlanSkip>,
    pub outputs: Vec<OutputInfo>,
    /// The plan of the prepared frame, whose boxes the outputs refer to.
    pub plan: PagePlan,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    pub manifest: Manifest,
    pub outputs: Vec<Output>,
}

// ------------------------------------------------------------------------------------ pipeline

struct Analysed {
    report: PageReport,
}

fn analyse_raster(img: &Raster, dpi: u16, opts: &str) -> Result<Analysed> {
    let gray = img.gray()?;
    let input = PageInput {
        gray: &gray,
        rgb: (img.channels == 3).then_some(img.data.as_slice()),
        width: img.width as u32,
        height: img.height as u32,
        dpi,
    };
    Ok(Analysed {
        report: analyse_page_report(&input, None, opts)?,
    })
}

/// Analyse `input`, turn and level it as its plan says (analysing again after each change, up to
/// `max_analyses` in all), and cut it into the images the final plan describes. `opts_json` takes the
/// same options as `analyse_page`; the detector list defaults to `prepare.detectors` and always
/// contains `plan`.
pub fn prepare(input: &PageInput<'_>, opts_json: &str) -> Result<Prepared> {
    let mut options: Value = if opts_json.trim().is_empty() {
        Value::Object(Default::default())
    } else {
        serde_json::from_str(opts_json).map_err(|e| Error::InvalidOptions(e.to_string()))?
    };
    let obj = options
        .as_object_mut()
        .ok_or_else(|| Error::InvalidOptions("options must be a JSON object".into()))?;
    let params = match obj.get("params").and_then(Value::as_str) {
        Some(toml) => Params::with_overrides(toml)?,
        None => Params::defaults()?,
    };
    let p = &params.prepare;
    let mut detectors: Vec<Value> = match obj.get("detectors").and_then(Value::as_array) {
        Some(list) => list.clone(),
        None => p
            .detectors
            .iter()
            .map(|d| Value::from(d.as_str()))
            .collect(),
    };
    if !detectors.iter().any(|d| d == "plan") {
        detectors.push(Value::from("plan"));
    }
    obj.insert("detectors".into(), Value::Array(detectors));
    // A quarter turn the caller already knows the page needs, applied before the first look.
    let pre_rotate = match obj.remove("pre_rotate_deg") {
        None | Some(Value::Null) => 0,
        Some(v) => v
            .as_u64()
            .filter(|d| [0, 90, 180, 270].contains(d))
            .ok_or_else(|| {
                Error::InvalidOptions("pre_rotate_deg must be 0, 90, 180 or 270".into())
            })? as u32,
    };
    // Debug output is switched on so the orientation reading can be reported when it is unsure.
    obj.insert("debug".into(), Value::Bool(true));
    let opts = options.to_string();

    let dpi = input.dpi;
    let mut cur = if let Some(rgb) = input.rgb {
        Raster::new(input.width as usize, input.height as usize, 3, rgb.to_vec())?
    } else {
        Raster::new(
            input.width as usize,
            input.height as usize,
            1,
            input.gray.to_vec(),
        )?
    };
    let (source_width, source_height) = (input.width, input.height);
    let mut steps: Vec<Step> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut rotate_total = 0u32;
    if pre_rotate != 0 {
        cur = rotate_quarter(&cur, pre_rotate)?;
        rotate_total = pre_rotate;
        steps.push(Step {
            op: "rotate".into(),
            clockwise_deg: f64::from(pre_rotate),
        });
        notes.push("quarter turn asked for by the caller".into());
    }

    let mut report = analyse_raster(&cur, dpi, &opts)?.report;
    let source_sha256 = plan_of(&report)?.input_sha256.clone();
    // What the orientation detector read on the page as it came, for the note when it was unsure.
    let first_turn = report
        .debug
        .as_ref()
        .and_then(|d| d.get("orientation"))
        .map_or((0.0, 0.0), |o| {
            (
                o.get("rotation_deg").copied().unwrap_or(0.0),
                o.get("confidence").copied().unwrap_or(0.0),
            )
        });
    let mut analyses = 1u32;
    let mut deskew_total = 0.0f64;
    // The page before any levelling: turned by the caller's quarter turn, if any.
    let base = cur.clone();
    let mut turned = pre_rotate != 0;

    loop {
        let plan = plan_of(&report)?;
        if plan.skip.is_some() || analyses >= p.max_analyses {
            break;
        }
        if plan.rotate_clockwise_deg != 0 {
            if turned {
                notes.push(
                    "the page still looked turned after a quarter turn; left as it is".into(),
                );
                break;
            }
            // The turn comes first in the frame, so a levelling done on the sideways page is thrown
            // away and the page is turned from its original pixels.
            if deskew_total != 0.0 {
                notes.push("levelled again after the quarter turn: the first reading was of a sideways page".into());
            }
            turned = true;
            rotate_total = plan.rotate_clockwise_deg;
            deskew_total = 0.0;
            cur = rotate_quarter(&base, rotate_total)?;
            steps.clear();
            steps.push(Step {
                op: "rotate".into(),
                clockwise_deg: f64::from(rotate_total),
            });
        } else if let Some(d) = plan.deskew_clockwise_deg {
            if deskew_total != 0.0 || d.abs() < p.min_deskew_deg {
                break;
            }
            if d.abs() > p.max_deskew_deg {
                notes.push(format!(
                    "measured skew {d:.1} degrees is beyond {} degrees: not levelled",
                    p.max_deskew_deg
                ));
                break;
            }
            deskew_total = d;
            let fill = paper_fill(&cur)?;
            cur = deskew(&cur, d, &fill)?;
            steps.push(Step {
                op: "deskew".into(),
                clockwise_deg: d,
            });
        } else {
            break;
        }
        report = analyse_raster(&cur, dpi, &opts)?.report;
        analyses += 1;
    }

    let plan = plan_of(&report)?.clone();
    if plan.rotate_clockwise_deg == 0 && first_turn.0 != 0.0 {
        notes.push(format!(
            "the page may be turned {:.0} degrees clockwise (confidence {:.2}, too low to act on): check it, or pass pre_rotate_deg",
            first_turn.0, first_turn.1
        ));
    }
    let frame = Frame {
        source_width,
        source_height,
        rotate_cw_deg: rotate_total,
        deskew_cw_deg: deskew_total,
        width: cur.width as u32,
        height: cur.height as u32,
    };
    let mut outputs: Vec<Output> = Vec::new();
    let mut infos: Vec<OutputInfo> = Vec::new();
    if plan.skip.is_none() {
        let mut emit = |name: String,
                        kind: &str,
                        route: &str,
                        region_kind: &str,
                        priority: u32,
                        bbox: [u32; 4],
                        factor: f64|
         -> Result<()> {
            let part = crop(&cur, bbox)?;
            let longest = part.width.max(part.height) as f64;
            let mut f = factor.min(f64::from(p.max_output_side_px) / longest);
            if (f - 1.0).abs() < p.min_scale_change {
                f = 1.0;
            }
            let raster = if f == 1.0 {
                part
            } else {
                let nw = ((part.width as f64 * f).round() as usize).max(1);
                let nh = ((part.height as f64 * f).round() as usize).max(1);
                resize(&part, nw, nh)?
            };
            infos.push(OutputInfo {
                file: name.clone(),
                kind: kind.into(),
                route: route.into(),
                region_kind: region_kind.into(),
                priority,
                bbox,
                scale: math::round_output(f),
                dpi: (f64::from(dpi) * f).round() as u32,
                width: raster.width as u32,
                height: raster.height as u32,
                sha256: raster.sha256(),
            });
            outputs.push(Output { name, raster });
            Ok(())
        };
        let factor = plan.scale.factor;
        if plan.tiles.is_empty() {
            emit(
                "page.png".into(),
                "page",
                "ocr",
                "page",
                0,
                plan.content_bbox,
                factor,
            )?;
        } else {
            for t in &plan.tiles {
                emit(
                    format!("tiles/tile_r{}_c{}.png", t.row, t.column),
                    "tile",
                    "ocr",
                    "tile",
                    0,
                    t.bbox,
                    1.0,
                )?;
            }
        }
        let pad = (p.region_pad_in * f64::from(dpi)).round() as u32;
        let (fw, fh) = (
            frame.width.saturating_sub(1),
            frame.height.saturating_sub(1),
        );
        for (i, r) in plan.regions.iter().enumerate() {
            if p.skip_routes.contains(&r.route) {
                continue;
            }
            let b = [
                r.bbox[0].saturating_sub(pad),
                r.bbox[1].saturating_sub(pad),
                (r.bbox[2] + pad).min(fw),
                (r.bbox[3] + pad).min(fh),
            ];
            emit(
                format!("regions/region_{i:03}_{}.png", r.route),
                "region",
                &r.route,
                &r.kind,
                r.priority,
                b,
                factor,
            )?;
        }
    }

    let manifest = Manifest {
        schema: MANIFEST_ID.into(),
        source_sha256,
        frame_sha256: plan.input_sha256.clone(),
        dpi,
        frame,
        steps,
        analyses,
        skip: plan.skip.clone(),
        outputs: infos,
        plan,
        notes,
    };
    Ok(Prepared { manifest, outputs })
}

fn plan_of(report: &PageReport) -> Result<&PagePlan> {
    report
        .plan
        .as_ref()
        .ok_or_else(|| Error::InvalidOptions("the plan detector did not run".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(w: usize, h: usize, c: usize) -> Raster {
        let data = (0..w * h * c).map(|i| (i * 7 % 251) as u8).collect();
        Raster::new(w, h, c, data).unwrap()
    }

    #[test]
    fn four_quarter_turns_are_the_identity_and_two_make_a_half() {
        for c in [1, 3] {
            let a = ramp(7, 5, c);
            let mut b = a.clone();
            for _ in 0..4 {
                b = rotate_quarter(&b, 90).unwrap();
            }
            assert_eq!(a, b);
            let half = rotate_quarter(&a, 180).unwrap();
            let twice = rotate_quarter(&rotate_quarter(&a, 90).unwrap(), 90).unwrap();
            assert_eq!(half, twice);
            let back = rotate_quarter(&rotate_quarter(&a, 270).unwrap(), 90).unwrap();
            assert_eq!(a, back);
        }
        assert!(rotate_quarter(&ramp(2, 2, 1), 45).is_err());
    }

    #[test]
    fn a_quarter_turn_puts_the_top_left_corner_at_the_top_right() {
        let mut a = ramp(4, 3, 1);
        a.data[0] = 200;
        let b = rotate_quarter(&a, 90).unwrap();
        assert_eq!((b.width, b.height), (3, 4));
        assert_eq!(b.data[2], 200);
    }

    #[test]
    fn levelling_by_nothing_changes_nothing_and_by_a_turn_keeps_the_middle() {
        let a = ramp(31, 17, 3);
        assert_eq!(deskew(&a, 0.0, &[0, 0, 0]).unwrap(), a);
        let b = deskew(&a, 10.0, &[255, 255, 255]).unwrap();
        assert!(b.width > a.width && b.height > a.height);
        let (mx, my) = (b.width / 2, b.height / 2);
        let (x0, y0) = (a.width / 2, a.height / 2);
        for k in 0..3 {
            let d = i32::from(b.px(mx, my)[k]) - i32::from(a.px(x0, y0)[k]);
            assert!(d.abs() <= 40, "channel {k} differs by {d}");
        }
    }

    #[test]
    fn a_constant_image_stays_constant_under_every_operation() {
        let a = Raster::new(40, 30, 1, vec![123; 1200]).unwrap();
        assert!(
            deskew(&a, -7.5, &[123])
                .unwrap()
                .data
                .iter()
                .all(|v| *v == 123)
        );
        for (w, h) in [(20, 15), (13, 9), (80, 60), (41, 33)] {
            assert!(resize(&a, w, h).unwrap().data.iter().all(|v| *v == 123));
        }
    }

    #[test]
    fn halving_averages_blocks_exactly() {
        let a = Raster::new(4, 2, 1, vec![0, 100, 40, 60, 200, 100, 0, 20]).unwrap();
        let b = resize(&a, 2, 1).unwrap();
        assert_eq!(b.data, [100, 30]);
    }

    #[test]
    fn crop_is_inclusive_and_clamped() {
        let a = ramp(10, 8, 1);
        let c = crop(&a, [2, 1, 4, 3]).unwrap();
        assert_eq!((c.width, c.height), (3, 3));
        assert_eq!(c.data[0], a.data[10 + 2]);
        let all = crop(&a, [0, 0, 99, 99]).unwrap();
        assert_eq!(all, a);
    }

    #[test]
    fn the_frame_maps_points_both_ways() {
        for (rot, deg) in [(0u32, 0.0), (90, 0.0), (180, 3.5), (270, -6.0), (0, 12.0)] {
            let (w, h) = (300u32, 200u32);
            let turned = if rot % 180 == 90 { (h, w) } else { (w, h) };
            let theta = deg * core::f64::consts::PI / 180.0;
            let (s, c) = (math::sin(theta).abs(), math::cos(theta).abs());
            let fw = (f64::from(turned.0) * c + f64::from(turned.1) * s).ceil() as u32;
            let fh = (f64::from(turned.0) * s + f64::from(turned.1) * c).ceil() as u32;
            let f = Frame {
                source_width: w,
                source_height: h,
                rotate_cw_deg: rot,
                deskew_cw_deg: deg,
                width: fw,
                height: fh,
            };
            for (x, y) in [(0.0, 0.0), (299.0, 0.0), (10.0, 150.0), (150.0, 100.0)] {
                let (u, v) = f.from_source(x, y);
                let (bx, by) = f.to_source(u, v);
                assert!(
                    (bx - x).abs() < 1e-6 && (by - y).abs() < 1e-6,
                    "{rot} {deg}"
                );
            }
        }
    }

    #[test]
    fn frame_and_pixels_agree_on_where_a_marked_pixel_goes() {
        let mut a = Raster::new(120, 80, 1, vec![0; 9600]).unwrap();
        a.data[20 * 120 + 100] = 255;
        for (rot, deg) in [(90u32, 0.0f64), (180, 0.0), (270, 8.0), (0, -8.0)] {
            let t = rotate_quarter(&a, rot).unwrap();
            let l = if deg == 0.0 {
                t
            } else {
                deskew(&t, deg, &[0]).unwrap()
            };
            let f = Frame {
                source_width: 120,
                source_height: 80,
                rotate_cw_deg: rot,
                deskew_cw_deg: deg,
                width: l.width as u32,
                height: l.height as u32,
            };
            let (u, v) = f.from_source(100.0, 20.0);
            let brightest = l
                .data
                .iter()
                .enumerate()
                .max_by_key(|&(i, v)| (*v, std::cmp::Reverse(i)))
                .map(|(i, _)| ((i % l.width) as f64, (i / l.width) as f64))
                .unwrap();
            assert!(
                (brightest.0 - u).abs() <= 1.5 && (brightest.1 - v).abs() <= 1.5,
                "{rot} {deg}: pixels say {brightest:?}, frame says ({u}, {v})"
            );
        }
    }
}
