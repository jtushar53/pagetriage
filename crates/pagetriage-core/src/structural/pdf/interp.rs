//! A small content-stream interpreter: just enough of the PDF graphics model to measure what
//! module M1 needs (text render modes, glyph counts and positions, image rectangles, path
//! counts) and what module M2 needs (which fonts show which bytes). It draws nothing.
//!
//! Approximations, all documented in `docs/detectors/text_layer.md`: glyph advances use a flat
//! `approx_advance_em` instead of font widths (only the start of each text-show operation is
//! tested against image boxes); clipping paths, transparency groups, optional content and blend
//! modes are ignored; white or off-page text counts as visible.

use std::collections::BTreeMap;
use std::rc::Rc;

use lopdf::{Dictionary, Document, Object, ObjectId};

use super::fonts::{FontCache, FontModel};
use crate::structural::content::{Item, Operand, Tokenizer};
use crate::structural::encoding::Decoded;
use crate::structural::params::StructuralParams;
use crate::structural::text_layer::PageObservation;
use crate::structural::unicode::Histogram;

type M = [f64; 6];
const IDENT: M = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `a` applied first, then `b` (PDF's row-vector convention: `a x b`).
fn mul(a: M, b: M) -> M {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

fn apply(m: M, x: f64, y: f64) -> (f64, f64) {
    (x * m[0] + y * m[2] + m[4], x * m[1] + y * m[3] + m[5])
}

/// Identity of a font within one page scan; ordered so output is deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FontKey {
    /// Text shown before any `Tf`.
    None,
    Obj(ObjectId),
    /// A font dictionary written directly in the resources, numbered in order of first use.
    Inline(u32),
}

/// What one font showed on the page.
pub struct FontUse {
    pub model: Rc<FontModel>,
    pub glyphs: u32,
    pub undecodable: u32,
    pub hist: Histogram,
    /// Decoded characters, words separated by spaces, capped (for the legacy-font statistics).
    pub sample: String,
}

/// An image drawn on the page.
#[derive(Debug, Clone)]
pub struct ImageUse {
    pub id: Option<ObjectId>,
    pub width: u32,
    pub height: u32,
    pub filter: String,
    pub color_space: String,
    pub bits: u32,
    pub is_mask: bool,
    pub has_mask: bool,
    pub cover: f64,
    pub dpi: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
pub struct PageBox {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl PageBox {
    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }
    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }
}

/// Everything measured on one page.
pub struct PageScan {
    pub obs: PageObservation,
    pub images: Vec<ImageUse>,
    pub fonts: BTreeMap<FontKey, FontUse>,
    pub notes: Vec<String>,
}

/// Document-wide content budget shared by all pages.
pub struct Budget {
    pub content_bytes_left: u64,
}

#[derive(Clone)]
struct Gs {
    ctm: M,
    font: Option<(FontKey, Rc<FontModel>)>,
    size: f64,
    tc: f64,
    tw: f64,
    th: f64,
    tl: f64,
    render: i64,
}

#[derive(Default)]
struct PathState {
    bbox: Option<[f64; 4]>,
    segments: u32,
}

struct TextRecord {
    x: f64,
    y: f64,
    glyphs: u32,
    visible: bool,
    occluded: bool,
}

const MAX_AABBS: usize = 2048;
const MAX_OCCLUSION_SCANS: u32 = 64;

struct Interp<'d, 'c> {
    doc: &'d Document,
    p: &'c StructuralParams,
    page: PageBox,
    fonts: &'c mut FontCache,
    budget: &'c mut Budget,
    gs: Gs,
    stack: Vec<Gs>,
    tm: M,
    tlm: M,
    path: PathState,
    scan: PageScan,
    ops: usize,
    content_bytes: usize,
    forms_done: usize,
    visiting: Vec<ObjectId>,
    records: Vec<TextRecord>,
    aabbs: Vec<[f64; 4]>,
    glyphs_total: u64,
    glyphs_recorded: u64,
    max_single_cover: f64,
    occlusion_scans: u32,
    inline_font_ids: BTreeMap<usize, u32>,
    fallback: Rc<FontModel>,
    stopped: bool,
}

/// Interpret a page's content. `contents` are the page's already-decoded content streams in order;
/// `resources` are the page's (inherited) resources.
pub fn scan_page<'d>(
    doc: &'d Document,
    p: &StructuralParams,
    page: PageBox,
    resources: Option<&'d Dictionary>,
    contents: &[Vec<u8>],
    fonts: &mut FontCache,
    budget: &mut Budget,
) -> PageScan {
    let mut it = Interp {
        doc,
        p,
        page,
        fonts,
        budget,
        gs: Gs {
            ctm: IDENT,
            font: None,
            size: 0.0,
            tc: 0.0,
            tw: 0.0,
            th: 1.0,
            tl: 0.0,
            render: 0,
        },
        stack: Vec::new(),
        tm: IDENT,
        tlm: IDENT,
        path: PathState::default(),
        scan: PageScan {
            obs: PageObservation::default(),
            images: Vec::new(),
            fonts: BTreeMap::new(),
            notes: Vec::new(),
        },
        ops: 0,
        content_bytes: 0,
        forms_done: 0,
        visiting: Vec::new(),
        records: Vec::new(),
        aabbs: Vec::new(),
        glyphs_total: 0,
        glyphs_recorded: 0,
        max_single_cover: 0.0,
        occlusion_scans: 0,
        inline_font_ids: BTreeMap::new(),
        fallback: Rc::new(FontModel::fallback()),
        stopped: false,
    };
    for c in contents {
        if it.stopped {
            break;
        }
        it.content_bytes = it.content_bytes.saturating_add(c.len());
        // A content stream may end inside a q/BT block; each stream continues the same state.
        it.run(c, resources, 0);
    }
    it.finish()
}

impl<'d> Interp<'d, '_> {
    fn limits(&self) -> &crate::structural::params::LimitsParams {
        &self.p.limits
    }

    fn truncate(&mut self, why: &str) {
        if !self.scan.obs.truncated {
            self.scan.notes.push(why.to_owned());
        }
        self.scan.obs.truncated = true;
        self.stopped = true;
    }

    fn run(&mut self, content: &[u8], res: Option<&'d Dictionary>, depth: usize) {
        let mut tok = Tokenizer::new(content, self.limits().max_operands);
        let mut operands: Vec<Operand> = Vec::new();
        while !self.stopped {
            let Some(item) = tok.next_item(&mut operands) else {
                break;
            };
            self.ops += 1;
            if self.ops > self.limits().max_ops_per_page {
                self.truncate("operator limit reached; page content cut short");
                break;
            }
            match item {
                Item::InlineImage(dict) => self.inline_image(&dict),
                Item::Op(op) => self.op(op, &operands, res, depth),
            }
        }
    }

    fn num(operands: &[Operand], i: usize) -> f64 {
        operands.get(i).and_then(Operand::as_f64).unwrap_or(0.0)
    }

    fn op(&mut self, op: &[u8], a: &[Operand], res: Option<&'d Dictionary>, depth: usize) {
        match op {
            b"q" => {
                if self.stack.len() < self.limits().max_q_depth {
                    self.stack.push(self.gs.clone());
                }
            }
            b"Q" => {
                if let Some(g) = self.stack.pop() {
                    self.gs = g;
                }
            }
            b"cm" if a.len() >= 6 => {
                let m = [
                    Self::num(a, 0),
                    Self::num(a, 1),
                    Self::num(a, 2),
                    Self::num(a, 3),
                    Self::num(a, 4),
                    Self::num(a, 5),
                ];
                self.gs.ctm = mul(m, self.gs.ctm);
            }
            b"BT" => {
                self.tm = IDENT;
                self.tlm = IDENT;
            }
            b"Tf" if a.len() >= 2 => {
                self.gs.size = Self::num(a, 1);
                if let Some(name) = a[0].as_name() {
                    self.set_font(res, name);
                }
            }
            b"Tc" => self.gs.tc = Self::num(a, 0),
            b"Tw" => self.gs.tw = Self::num(a, 0),
            b"Tz" => self.gs.th = Self::num(a, 0) / 100.0,
            b"TL" => self.gs.tl = Self::num(a, 0),
            b"Tr" => self.gs.render = Self::num(a, 0) as i64,
            b"Td" => self.td(Self::num(a, 0), Self::num(a, 1)),
            b"TD" => {
                self.gs.tl = -Self::num(a, 1);
                self.td(Self::num(a, 0), Self::num(a, 1));
            }
            b"Tm" if a.len() >= 6 => {
                self.tm = [
                    Self::num(a, 0),
                    Self::num(a, 1),
                    Self::num(a, 2),
                    Self::num(a, 3),
                    Self::num(a, 4),
                    Self::num(a, 5),
                ];
                self.tlm = self.tm;
            }
            b"T*" => self.td(0.0, -self.gs.tl),
            b"Tj" => {
                if let Some(Operand::Str(s)) = a.first() {
                    self.show(std::slice::from_ref(&Showing::Text(s)));
                }
            }
            b"'" => {
                self.td(0.0, -self.gs.tl);
                if let Some(Operand::Str(s)) = a.first() {
                    self.show(std::slice::from_ref(&Showing::Text(s)));
                }
            }
            b"\"" => {
                self.gs.tw = Self::num(a, 0);
                self.gs.tc = Self::num(a, 1);
                self.td(0.0, -self.gs.tl);
                if let Some(Operand::Str(s)) = a.get(2) {
                    self.show(std::slice::from_ref(&Showing::Text(s)));
                }
            }
            b"TJ" => {
                if let Some(Operand::Array(items)) = a.first() {
                    let parts: Vec<Showing<'_>> = items
                        .iter()
                        .filter_map(|o| match o {
                            Operand::Str(s) => Some(Showing::Text(s)),
                            Operand::Num(n) => Some(Showing::Adjust(*n)),
                            _ => None,
                        })
                        .collect();
                    self.show(&parts);
                }
            }
            b"Do" => {
                if let Some(name) = a.first().and_then(Operand::as_name) {
                    self.do_xobject(res, name, depth);
                }
            }
            // Path construction.
            b"m" | b"l" => {
                let (x, y) = apply(self.gs.ctm, Self::num(a, 0), Self::num(a, 1));
                self.path_point(x, y);
                self.path.segments += 1;
            }
            b"c" => {
                for i in 0..3 {
                    let (x, y) = apply(self.gs.ctm, Self::num(a, 2 * i), Self::num(a, 2 * i + 1));
                    self.path_point(x, y);
                }
                self.path.segments += 1;
            }
            b"v" | b"y" => {
                for i in 0..2 {
                    let (x, y) = apply(self.gs.ctm, Self::num(a, 2 * i), Self::num(a, 2 * i + 1));
                    self.path_point(x, y);
                }
                self.path.segments += 1;
            }
            b"re" => {
                let (x, y, w, h) = (
                    Self::num(a, 0),
                    Self::num(a, 1),
                    Self::num(a, 2),
                    Self::num(a, 3),
                );
                for (px, py) in [(x, y), (x + w, y), (x + w, y + h), (x, y + h)] {
                    let (qx, qy) = apply(self.gs.ctm, px, py);
                    self.path_point(qx, qy);
                }
                self.path.segments += 4;
            }
            // Painting.
            b"S" | b"s" => self.paint(false),
            b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" => self.paint(true),
            b"n" => self.path = PathState::default(),
            b"sh" => self.scan.obs.paint_ops = self.scan.obs.paint_ops.saturating_add(1),
            _ => {}
        }
    }

    fn path_point(&mut self, x: f64, y: f64) {
        self.path.bbox = Some(match self.path.bbox {
            None => [x, y, x, y],
            Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        });
    }

    fn paint(&mut self, fill: bool) {
        self.scan.obs.paint_ops = self.scan.obs.paint_ops.saturating_add(1);
        if fill && let Some([x0, y0, x1, y1]) = self.path.bbox {
            let t = &self.p.text_layer;
            let (w, h) = (x1 - x0, y1 - y0);
            if w <= t.small_path_max_page_fraction * self.page.width()
                && h <= t.small_path_max_page_fraction * self.page.height()
                && self.path.segments >= t.small_path_min_segments
            {
                self.scan.obs.small_filled_paths =
                    self.scan.obs.small_filled_paths.saturating_add(1);
            }
        }
        self.path = PathState::default();
    }

    fn td(&mut self, tx: f64, ty: f64) {
        self.tlm = mul([1.0, 0.0, 0.0, 1.0, tx, ty], self.tlm);
        self.tm = self.tlm;
    }

    // --- fonts ---------------------------------------------------------------------------------

    fn set_font(&mut self, res: Option<&'d Dictionary>, name: &[u8]) {
        let found = res
            .and_then(|r| r.get(b"Font").ok())
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .and_then(|d| d.get(name).ok())
            .and_then(|o| self.doc.dereference(o).ok());
        let Some((id, Object::Dictionary(dict))) = found else {
            self.gs.font = None;
            return;
        };
        let key = match id {
            Some(id) => FontKey::Obj(id),
            None => {
                let addr = std::ptr::from_ref(dict) as usize;
                let next = self.inline_font_ids.len() as u32;
                FontKey::Inline(*self.inline_font_ids.entry(addr).or_insert(next))
            }
        };
        if !self.scan.fonts.contains_key(&key)
            && self.scan.fonts.len() >= self.limits().max_fonts_per_page
        {
            self.gs.font = None;
            return;
        }
        let model = match id {
            Some(id) => {
                self.fonts
                    .get_or_build(self.doc, id, dict, &self.p.font_script, &self.p.limits)
            }
            None => Rc::new(FontModel::from_dict(
                self.doc,
                dict,
                &self.p.font_script,
                &self.p.limits,
            )),
        };
        self.gs.font = Some((key, model));
    }

    // --- text ----------------------------------------------------------------------------------

    fn show(&mut self, parts: &[Showing<'_>]) {
        let (x, y) = {
            let (tx, ty) = (self.tm[4], self.tm[5]);
            apply(self.gs.ctm, tx, ty)
        };
        let render = self.gs.render;
        let visible = !matches!(render, 3 | 7);
        let space_gap = self.p.font_script.tj_space_adjustment;
        let mut glyphs: u32 = 0;
        for part in parts {
            match part {
                Showing::Text(bytes) => {
                    let (n_glyphs, n_codes, n_spaces) = self.show_string(bytes);
                    glyphs = glyphs.saturating_add(n_glyphs);
                    let adv = (f64::from(n_codes)
                        * (self.p.text_layer.approx_advance_em * self.gs.size + self.gs.tc)
                        + f64::from(n_spaces) * self.gs.tw)
                        * self.gs.th;
                    self.advance(adv);
                }
                Showing::Adjust(n) => {
                    let adv = -n / 1000.0 * self.gs.size * self.gs.th;
                    self.advance(adv);
                    if *n <= space_gap
                        && let Some((key, _)) = &self.gs.font
                        && let Some(u) = self.scan.fonts.get_mut(key)
                        && !u.sample.ends_with(' ')
                        && u.sample.len() < self.p.font_script.max_sample_chars
                    {
                        u.sample.push(' ');
                    }
                }
            }
        }
        if glyphs == 0 {
            return;
        }
        let obs = &mut self.scan.obs;
        obs.text_ops = obs.text_ops.saturating_add(1);
        obs.glyphs = obs.glyphs.saturating_add(glyphs);
        if !visible {
            obs.glyphs_invisible_mode = obs.glyphs_invisible_mode.saturating_add(glyphs);
        }
        self.glyphs_total += u64::from(glyphs);
        if self.records.len() < self.p.text_layer.max_text_records {
            self.glyphs_recorded += u64::from(glyphs);
            self.records.push(TextRecord {
                x,
                y,
                glyphs,
                visible,
                occluded: false,
            });
        }
    }

    fn advance(&mut self, tx: f64) {
        self.tm[4] += tx * self.tm[0];
        self.tm[5] += tx * self.tm[1];
    }

    /// Decode one string with the current font. Returns (glyphs, codes, spaces).
    fn show_string(&mut self, bytes: &[u8]) -> (u32, u32, u32) {
        let (key, font) = match &self.gs.font {
            Some((k, f)) => (*k, Rc::clone(f)),
            None => (FontKey::None, Rc::clone(&self.fallback)),
        };
        let cap = self.p.font_script.max_sample_chars;
        let usage = self.scan.fonts.entry(key).or_insert_with(|| FontUse {
            model: Rc::clone(&font),
            glyphs: 0,
            undecodable: 0,
            hist: Histogram::default(),
            sample: String::new(),
        });
        let (mut glyphs, mut codes, mut spaces) = (0u32, 0u32, 0u32);
        font.decode(bytes, |g| {
            codes += 1;
            if g.is_space {
                spaces += 1;
                if !usage.sample.ends_with(' ') && usage.sample.len() < cap {
                    usage.sample.push(' ');
                }
                return;
            }
            glyphs += 1;
            usage.glyphs = usage.glyphs.saturating_add(1);
            let mut known = false;
            for v in g.values {
                match v {
                    Decoded::Cp(cp) => {
                        usage.hist.add(*cp);
                        known |= !matches!(
                            crate::structural::unicode::classify(*cp),
                            crate::structural::unicode::CodeClass::PrivateUse
                        );
                        if usage.sample.len() < cap
                            && let Some(c) = char::from_u32(*cp)
                        {
                            usage.sample.push(c);
                        }
                    }
                    Decoded::ScriptLetter(s) => {
                        usage.hist.letters[s.index()] += 1;
                        known = true;
                    }
                    Decoded::Unknown => {}
                }
            }
            if !known {
                usage.undecodable = usage.undecodable.saturating_add(1);
            }
        });
        (glyphs, codes, spaces)
    }

    // --- XObjects and images -------------------------------------------------------------------

    fn do_xobject(&mut self, res: Option<&'d Dictionary>, name: &[u8], depth: usize) {
        let found = res
            .and_then(|r| r.get(b"XObject").ok())
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .and_then(|d| d.get(name).ok())
            .and_then(|o| self.doc.dereference(o).ok());
        let Some((id, Object::Stream(stream))) = found else {
            return;
        };
        match stream
            .dict
            .get(b"Subtype")
            .ok()
            .and_then(|o| o.as_name().ok())
        {
            Some(b"Image") => {
                let info = image_dict_info(self.doc, id, &stream.dict);
                self.draw_image(info);
            }
            Some(b"Form") => self.do_form(id, stream, res, depth),
            _ => {}
        }
    }

    fn do_form(
        &mut self,
        id: Option<ObjectId>,
        stream: &'d lopdf::Stream,
        res: Option<&'d Dictionary>,
        depth: usize,
    ) {
        let l = &self.p.limits;
        if depth >= l.max_form_depth || self.forms_done >= l.max_forms_per_page {
            self.scan
                .notes
                .push("form XObject limit reached; nested content skipped".into());
            return;
        }
        if let Some(id) = id
            && self.visiting.contains(&id)
        {
            return; // a form that draws itself
        }
        let room = l
            .max_content_bytes_per_page
            .saturating_sub(self.content_bytes)
            .min(usize::try_from(self.budget.content_bytes_left).unwrap_or(usize::MAX));
        if room == 0 {
            self.truncate("content byte budget exhausted; page content cut short");
            return;
        }
        let Ok(data) = stream.decompressed_content_with_limit(room.min(l.max_decompressed_bytes))
        else {
            self.scan
                .notes
                .push("a form XObject could not be decoded".into());
            return;
        };
        self.content_bytes = self.content_bytes.saturating_add(data.len());
        self.budget.content_bytes_left = self
            .budget
            .content_bytes_left
            .saturating_sub(data.len() as u64);
        self.forms_done += 1;

        let matrix = stream
            .dict
            .get(b"Matrix")
            .ok()
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok())
            .filter(|a| a.len() >= 6)
            .map(|a| {
                let mut m = IDENT;
                for (slot, o) in m.iter_mut().zip(a.iter()) {
                    *slot = o.as_float().map_or(*slot, f64::from);
                }
                m
            })
            .unwrap_or(IDENT);
        let form_res = stream
            .dict
            .get(b"Resources")
            .ok()
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .or(res);

        let saved_gs = self.gs.clone();
        let saved_stack = self.stack.len();
        let (saved_tm, saved_tlm) = (self.tm, self.tlm);
        let saved_path = std::mem::take(&mut self.path);
        self.gs.ctm = mul(matrix, self.gs.ctm);
        if let Some(id) = id {
            self.visiting.push(id);
        }
        self.run(&data, form_res, depth + 1);
        if id.is_some() {
            self.visiting.pop();
        }
        self.stack.truncate(saved_stack);
        self.gs = saved_gs;
        self.tm = saved_tm;
        self.tlm = saved_tlm;
        self.path = saved_path;
    }

    fn inline_image(&mut self, dict: &[(Vec<u8>, Operand)]) {
        let get = |long: &[u8], short: &[u8]| {
            dict.iter()
                .find(|(k, _)| k == long || k == short)
                .map(|(_, v)| v)
        };
        let int =
            |o: Option<&Operand>| o.and_then(Operand::as_f64).map_or(0, |v| v.max(0.0) as u32);
        let name = |o: Option<&Operand>| match o {
            Some(Operand::Name(n)) => Some(n.clone()),
            Some(Operand::Array(a)) => a.last().and_then(|x| x.as_name().map(<[u8]>::to_vec)),
            _ => None,
        };
        let filter = name(get(b"Filter", b"F")).map_or_else(
            || "none".to_owned(),
            |n| match n.as_slice() {
                b"AHx" => "ASCIIHexDecode".into(),
                b"A85" => "ASCII85Decode".into(),
                b"LZW" => "LZWDecode".into(),
                b"Fl" => "FlateDecode".into(),
                b"RL" => "RunLengthDecode".into(),
                b"CCF" => "CCITTFaxDecode".into(),
                b"DCT" => "DCTDecode".into(),
                other => String::from_utf8_lossy(other).chars().take(32).collect(),
            },
        );
        let cs = name(get(b"ColorSpace", b"CS")).map_or_else(
            || "unknown".to_owned(),
            |n| match n.as_slice() {
                b"G" => "DeviceGray".into(),
                b"RGB" => "DeviceRGB".into(),
                b"CMYK" => "DeviceCMYK".into(),
                b"I" => "Indexed".into(),
                other => String::from_utf8_lossy(other).chars().take(32).collect(),
            },
        );
        let is_mask = matches!(get(b"ImageMask", b"IM"), Some(Operand::Other));
        let info = RawImage {
            id: None,
            width: int(get(b"Width", b"W")),
            height: int(get(b"Height", b"H")),
            filter,
            color_space: cs,
            bits: if is_mask {
                1
            } else {
                int(get(b"BitsPerComponent", b"BPC"))
            },
            is_mask,
            has_mask: false,
        };
        self.draw_image(info);
    }

    fn draw_image(&mut self, info: RawImage) {
        let m = self.gs.ctm;
        let corners = [
            apply(m, 0.0, 0.0),
            apply(m, 1.0, 0.0),
            apply(m, 0.0, 1.0),
            apply(m, 1.0, 1.0),
        ];
        let mut aabb = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for (x, y) in corners {
            aabb = [
                aabb[0].min(x),
                aabb[1].min(y),
                aabb[2].max(x),
                aabb[3].max(y),
            ];
        }
        if !aabb.iter().all(|v| v.is_finite()) {
            return;
        }
        let page_area = self.page.width() * self.page.height();
        let ix = (aabb[2].min(self.page.x1) - aabb[0].max(self.page.x0)).max(0.0);
        let iy = (aabb[3].min(self.page.y1) - aabb[1].max(self.page.y0)).max(0.0);
        let cover = if page_area > 0.0 {
            (ix * iy / page_area).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let obs = &mut self.scan.obs;
        obs.image_count = obs.image_count.saturating_add(1);
        self.max_single_cover = self.max_single_cover.max(cover);
        if self.aabbs.len() < MAX_AABBS {
            self.aabbs.push(aabb);
        }

        let t = &self.p.text_layer;
        if !info.is_mask
            && !info.has_mask
            && cover >= t.occluding_image_cover_fraction
            && self.occlusion_scans < MAX_OCCLUSION_SCANS
        {
            self.occlusion_scans += 1;
            for r in &mut self.records {
                if r.visible
                    && !r.occluded
                    && r.x >= aabb[0]
                    && r.x <= aabb[2]
                    && r.y >= aabb[1]
                    && r.y <= aabb[3]
                {
                    r.occluded = true;
                }
            }
        }

        let draw_w = (m[0] * m[0] + m[1] * m[1]).sqrt();
        let dpi =
            (draw_w > 1e-9 && info.width > 0).then(|| f64::from(info.width) / (draw_w / 72.0));
        if self.scan.images.len() < self.p.limits.max_images_per_page {
            self.scan.images.push(ImageUse {
                id: info.id,
                width: info.width,
                height: info.height,
                filter: info.filter,
                color_space: info.color_space,
                bits: info.bits,
                is_mask: info.is_mask,
                has_mask: info.has_mask,
                cover,
                dpi: dpi.filter(|d| d.is_finite()),
            });
        }
    }

    // --- result --------------------------------------------------------------------------------

    fn finish(mut self) -> PageScan {
        let n = self.p.text_layer.coverage_grid as usize;
        let (w, h) = (self.page.width(), self.page.height());
        let mut covered = vec![false; n * n];
        let mut cells = 0usize;
        if w > 0.0 && h > 0.0 && !self.aabbs.is_empty() {
            for gy in 0..n {
                let cy = self.page.y0 + (gy as f64 + 0.5) * h / n as f64;
                for gx in 0..n {
                    let cx = self.page.x0 + (gx as f64 + 0.5) * w / n as f64;
                    if self
                        .aabbs
                        .iter()
                        .any(|a| cx >= a[0] && cx <= a[2] && cy >= a[1] && cy <= a[3])
                    {
                        covered[gy * n + gx] = true;
                        cells += 1;
                    }
                }
            }
        }
        let obs = &mut self.scan.obs;
        // The grid underestimates small images by up to a cell; a single image's exact cover is a
        // lower bound on the union, so take the larger of the two.
        obs.image_cover = (cells as f64 / (n * n) as f64).max(self.max_single_cover);

        let (mut under, mut in_image) = (0u64, 0u64);
        for r in &self.records {
            if r.visible && r.occluded {
                under += u64::from(r.glyphs);
            }
            if w > 0.0 && h > 0.0 {
                let gx = ((r.x - self.page.x0) / w * n as f64).floor();
                let gy = ((r.y - self.page.y0) / h * n as f64).floor();
                if gx >= 0.0
                    && gy >= 0.0
                    && (gx as usize) < n
                    && (gy as usize) < n
                    && covered[gy as usize * n + gx as usize]
                {
                    in_image += u64::from(r.glyphs);
                }
            }
        }
        // If the record cap was hit, scale the geometric counts to all glyphs.
        let scale = |v: u64| -> u32 {
            if self.glyphs_recorded == 0 || self.glyphs_recorded == self.glyphs_total {
                v.min(u64::from(u32::MAX)) as u32
            } else {
                (v * self.glyphs_total / self.glyphs_recorded).min(u64::from(u32::MAX)) as u32
            }
        };
        obs.glyphs_under_image = scale(under);
        obs.glyphs_in_image = scale(in_image);
        obs.glyphs_under_image = obs
            .glyphs_under_image
            .min(obs.glyphs.saturating_sub(obs.glyphs_invisible_mode));
        obs.glyphs_in_image = obs.glyphs_in_image.min(obs.glyphs);
        if self.records.len() >= self.p.text_layer.max_text_records {
            self.scan
                .notes
                .push("text record cap reached; image overlap counts are scaled".into());
        }
        if self.occlusion_scans >= MAX_OCCLUSION_SCANS {
            self.scan.notes.push(
                "many page-covering images; later ones were not tested for hiding text".into(),
            );
        }
        self.scan
    }
}

enum Showing<'a> {
    Text(&'a [u8]),
    Adjust(f64),
}

/// Fields read from an image XObject dictionary or an inline image dictionary.
pub struct RawImage {
    pub id: Option<ObjectId>,
    pub width: u32,
    pub height: u32,
    pub filter: String,
    pub color_space: String,
    pub bits: u32,
    pub is_mask: bool,
    pub has_mask: bool,
}

/// Describe an image XObject from its dictionary.
pub fn image_dict_info(doc: &Document, id: Option<ObjectId>, dict: &Dictionary) -> RawImage {
    let int = |key: &[u8]| {
        dict.get(key)
            .ok()
            .and_then(|o| doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_i64().ok())
            .map_or(0, |v| v.clamp(0, i64::from(u32::MAX)) as u32)
    };
    let filter = dict
        .get(b"Filter")
        .ok()
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| match o {
            Object::Name(n) => Some(n.clone()),
            Object::Array(a) => a
                .last()
                .and_then(|x| doc.dereference(x).ok())
                .and_then(|(_, x)| x.as_name().ok().map(<[u8]>::to_vec)),
            _ => None,
        })
        .map_or_else(
            || "none".to_owned(),
            |n| String::from_utf8_lossy(&n).chars().take(32).collect(),
        );
    let color_space = dict
        .get(b"ColorSpace")
        .ok()
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| match o {
            Object::Name(n) => Some(n.clone()),
            Object::Array(a) => a.first().and_then(|x| x.as_name().ok().map(<[u8]>::to_vec)),
            _ => None,
        })
        .map_or_else(
            || "unknown".to_owned(),
            |n| String::from_utf8_lossy(&n).chars().take(32).collect(),
        );
    let is_mask = dict
        .get(b"ImageMask")
        .ok()
        .and_then(|o| doc.dereference(o).ok())
        .is_some_and(|(_, o)| matches!(o, Object::Boolean(true)));
    RawImage {
        id,
        width: int(b"Width"),
        height: int(b"Height"),
        filter,
        color_space,
        bits: if is_mask { 1 } else { int(b"BitsPerComponent") },
        is_mask,
        has_mask: dict.has(b"SMask") || dict.has(b"Mask"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_multiplication_order() {
        // Translate by (10, 0) then scale by 2: a point at the origin ends up at (20, 0).
        let t = [1.0, 0.0, 0.0, 1.0, 10.0, 0.0];
        let s = [2.0, 0.0, 0.0, 2.0, 0.0, 0.0];
        assert_eq!(apply(mul(t, s), 0.0, 0.0), (20.0, 0.0));
        assert_eq!(apply(mul(s, t), 0.0, 0.0), (10.0, 0.0));
    }
}
