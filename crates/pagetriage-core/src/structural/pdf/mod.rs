//! Reading a PDF with `lopdf` and producing a [`StructureReport`]. Compiled only with the `pdf`
//! feature.

mod fonts;
mod interp;
mod meta;
mod pages;
mod revisions;

use std::collections::BTreeMap;

use lopdf::{Document, Object, ObjectId};

use self::fonts::FontCache;
use self::interp::{Budget, ImageUse, PageScan, scan_page};
use self::pages::{PageRef, collect_pages, page_geometry, scan_for_pages};
use super::font_script::{self, FontObservation};
use super::jpeg;
use super::params::StructuralParams;
use super::text_layer::{self, DocContext};
use super::types::{
    DocumentStructure, ImageInfo, JpegInfo, PageFeatures, PageStructure, STRUCTURE_SCHEMA_ID,
    StructureReport,
};
use crate::error::{Error, Result};
use crate::hash::sha256_hex;
use crate::schema::DocumentInfo;

/// Analyse the structure of a PDF held in memory.
pub fn analyse(bytes: &[u8], p: &StructuralParams) -> Result<StructureReport> {
    let limits = &p.limits;
    if bytes.len() > limits.max_input_bytes {
        return Err(Error::InvalidInput(format!(
            "PDF is {} bytes; the limit is {}",
            bytes.len(),
            limits.max_input_bytes
        )));
    }
    let head = &bytes[..bytes.len().min(1024)];
    if !head.windows(5).any(|w| w == b"%PDF-") {
        return Err(Error::InvalidInput("not a PDF: no %PDF- header".into()));
    }
    let doc = revisions::load_guarded(bytes, limits.max_decompressed_bytes)
        .map_err(|e| Error::InvalidInput(format!("cannot read PDF: {e}")))?;
    if doc.objects.len() > limits.max_objects {
        return Err(Error::InvalidInput(format!(
            "PDF has {} objects; the limit is {}",
            doc.objects.len(),
            limits.max_objects
        )));
    }

    let mut warnings: Vec<String> = Vec::new();
    let mut degraded = false;
    let mut meta = meta::read_meta(&doc, bytes, &p.provenance, limits.max_decompressed_bytes);
    // A trailer that still names /Encrypt after loading means the empty password did not open it.
    let locked = doc.trailer.has(b"Encrypt");
    if locked {
        degraded = true;
        warnings.push(
            "the PDF is encrypted and the empty password does not open it; page content was not analysed"
                .into(),
        );
    } else if meta.encrypted {
        warnings.push("the PDF is encrypted; it opened with the empty password".into());
    }
    if let Some(r) = &doc.xref_recovery {
        degraded = true;
        warnings.push(format!(
            "the cross-reference data was damaged and was repaired ({r:?}); objects may be missing or stale"
        ));
    }

    // Pages.
    let (mut page_refs, truncated) =
        collect_pages(&doc, limits.max_pages, limits.max_page_tree_depth);
    if truncated {
        degraded = true;
        warnings.push(format!(
            "page tree cut short at {} pages or depth {}",
            limits.max_pages, limits.max_page_tree_depth
        ));
    }
    if page_refs.is_empty() && !locked {
        page_refs = scan_for_pages(&doc, limits.max_pages);
        if !page_refs.is_empty() {
            degraded = true;
            warnings.push(
                "page tree unusable; pages found by scanning objects (order may differ)".into(),
            );
        }
    }

    let ctx = DocContext {
        ocr_producer: meta.ocr_producer,
    };
    let mut cache = FontCache::default();
    let mut jpeg_cache: BTreeMap<ObjectId, Option<JpegInfo>> = BTreeMap::new();
    let mut budget = Budget {
        content_bytes_left: limits.max_content_bytes_total,
    };
    let mut pages_out: Vec<PageStructure> = Vec::with_capacity(page_refs.len());
    for (i, page) in page_refs.iter().enumerate() {
        let ps = if locked {
            let (_, size, rot, _) = page_geometry(page);
            PageStructure {
                index: i as u32,
                size_pt: size,
                rotation: rot,
                text_layer: None,
                script: None,
                features: None,
                images: Vec::new(),
                conflicts: Vec::new(),
                notes: vec!["encrypted: content not readable".into()],
            }
        } else {
            analyse_page(
                &doc,
                page,
                i as u32,
                &ctx,
                p,
                &mut cache,
                &mut jpeg_cache,
                &mut budget,
            )?
        };
        if ps.text_layer.is_none() || ps.features.as_ref().is_some_and(|f| f.truncated) {
            degraded = true;
        }
        pages_out.push(ps);
    }
    if budget.content_bytes_left == 0 {
        warnings
            .push("document-wide content budget exhausted; later pages may be incomplete".into());
    }

    // Provenance.
    let rev = if locked {
        None
    } else {
        Some(revisions::analyse(bytes, &doc, p))
    };
    let mut page_states: BTreeMap<String, u32> = BTreeMap::new();
    let mut unreliable = 0u32;
    for ps in &pages_out {
        if let Some(tl) = &ps.text_layer {
            *page_states
                .entry(font_script::state_id(tl.state).to_owned())
                .or_insert(0) += 1;
            if tl.reliable == Some(false) {
                unreliable += 1;
            }
        }
    }
    let mut jpeg_table_hashes: BTreeMap<String, u32> = BTreeMap::new();
    let mut jpeg_images = 0u32;
    for j in jpeg_cache.values().flatten() {
        jpeg_images += 1;
        *jpeg_table_hashes.entry(j.table_hash.clone()).or_insert(0) += 1;
    }
    let (revisions_out, incremental, eofs, rev_truncated, tamper, reasons, content_changed) =
        match &rev {
            Some(r) => (
                r.revisions.clone(),
                r.valid_revisions.saturating_sub(1),
                r.eof_markers,
                r.truncated,
                r.tamper_signal,
                r.tamper_reasons.clone(),
                r.content_changed_in_update,
            ),
            None => (Vec::new(), 0, 0, false, false, Vec::new(), false),
        };
    if rev_truncated {
        warnings.push("more revisions than the limit; older ones were not examined".into());
    }
    meta.encrypted |= locked;

    let document = DocumentInfo {
        sha256: sha256_hex(bytes),
        pages: pages_out.len().min(u32::MAX as usize) as u32,
        source: "pdf".into(),
        producer: meta.producer.clone(),
        incremental_updates: Some(incremental),
    };
    let signature_count = if locked {
        0
    } else {
        revisions::count_signatures(&doc)
    };
    let structure = DocumentStructure {
        pdf_version: meta.version.clone(),
        creator: meta.creator.clone(),
        creation_date: meta.creation_date.clone(),
        mod_date: meta.mod_date.clone(),
        xmp_present: meta.xmp_present,
        xmp_tool: meta.xmp_tool.clone(),
        producer_family: meta.producer_family.clone(),
        ocr_producer: meta.ocr_producer,
        xref_style: meta.xref_style.to_owned(),
        linearized: meta.linearized,
        encrypted: meta.encrypted,
        eof_markers: eofs,
        revisions: revisions_out,
        revisions_truncated: rev_truncated,
        signature_count,
        tamper_signal: tamper,
        tamper_reasons: reasons,
        content_changed_in_update: content_changed,
        jpeg_images,
        jpeg_table_hashes,
        page_states,
        unreliable_pages: unreliable,
    };
    Ok(StructureReport {
        schema: STRUCTURE_SCHEMA_ID.to_owned(),
        document,
        structure,
        pages: pages_out,
        degraded,
        warnings,
    })
}

#[allow(clippy::too_many_arguments)]
fn analyse_page(
    doc: &Document,
    page: &PageRef<'_>,
    index: u32,
    ctx: &DocContext,
    p: &StructuralParams,
    cache: &mut FontCache,
    jpeg_cache: &mut BTreeMap<ObjectId, Option<JpegInfo>>,
    budget: &mut Budget,
) -> Result<PageStructure> {
    let (page_box, size_pt, rotation, defaulted) = page_geometry(page);
    let mut notes: Vec<String> = Vec::new();
    if defaulted {
        notes.push("page box missing or empty; US Letter assumed".into());
    }

    let (contents, undecodable, truncated) = read_contents(doc, page, p, budget, &mut notes);
    if budget.content_bytes_left == 0 && truncated {
        notes.push("document content budget exhausted".into());
    }
    if undecodable > 0 && contents.is_empty() {
        notes.push("page content could not be decoded".into());
        return Ok(PageStructure {
            index,
            size_pt,
            rotation,
            text_layer: None,
            script: None,
            features: None,
            images: Vec::new(),
            conflicts: Vec::new(),
            notes,
        });
    }

    let mut scan: PageScan = scan_page(doc, p, page_box, page.resources, &contents, cache, budget);
    scan.obs.truncated |= truncated;
    notes.append(&mut scan.notes);

    let mut layer = text_layer::decide(&scan.obs, ctx, &p.text_layer)?;

    // Module M2 on the fonts that showed text.
    let observations: Vec<FontObservation> = scan
        .fonts
        .values()
        .map(|u| {
            let m = &u.model;
            let mut names: Vec<String> = m.candidate_names().map(str::to_owned).collect();
            names.retain(|n| !n.is_empty());
            FontObservation {
                names,
                subtype: m.subtype.clone(),
                embedded: m.embedded,
                to_unicode: m.has_to_unicode,
                encoding: m.encoding_label.clone(),
                glyphs: u.glyphs,
                undecodable: u.undecodable,
                hist: u.hist.clone(),
                sample: u.sample.clone(),
            }
        })
        .collect();
    let summary = font_script::summarise(&observations, &p.font_script)?;
    font_script::apply_reliability(&mut layer, &summary, &p.font_script)?;
    let conflicts = font_script::conflicts(layer.state, &summary.info);

    let images = scan
        .images
        .iter()
        .map(|im| image_info(doc, im, p, jpeg_cache))
        .collect();
    let o = &scan.obs;
    let features = PageFeatures {
        glyphs: o.glyphs,
        glyphs_invisible_mode: o.glyphs_invisible_mode,
        glyphs_under_image: o.glyphs_under_image,
        glyphs_in_image: o.glyphs_in_image,
        text_ops: o.text_ops,
        image_cover: o.image_cover,
        image_count: o.image_count,
        paint_ops: o.paint_ops,
        small_filled_paths: o.small_filled_paths,
        outlined_text_suspected: o.glyphs < p.text_layer.min_glyphs
            && o.small_filled_paths >= p.text_layer.outlined_text_min_paths,
        truncated: o.truncated,
    };
    Ok(PageStructure {
        index,
        size_pt,
        rotation,
        text_layer: Some(layer),
        script: Some(summary.info),
        features: Some(features),
        images,
        conflicts,
        notes,
    })
}

/// Decode the page's content streams in order. Returns the streams, how many could not be
/// decoded, and whether a size limit cut the content short.
fn read_contents(
    doc: &Document,
    page: &PageRef<'_>,
    p: &StructuralParams,
    budget: &mut Budget,
    notes: &mut Vec<String>,
) -> (Vec<Vec<u8>>, u32, bool) {
    let l = &p.limits;
    let mut streams: Vec<&lopdf::Stream> = Vec::new();
    if let Ok(c) = page.dict.get(b"Contents") {
        match doc.dereference(c) {
            Ok((_, Object::Stream(s))) => streams.push(s),
            Ok((_, Object::Array(items))) => {
                for item in items.iter().take(4096) {
                    if let Ok((_, Object::Stream(s))) = doc.dereference(item) {
                        streams.push(s);
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    let (mut undecodable, mut truncated) = (0u32, false);
    let mut page_bytes = 0usize;
    for s in streams {
        let room = l
            .max_content_bytes_per_page
            .saturating_sub(page_bytes)
            .min(budget.content_bytes_left);
        if room == 0 {
            truncated = true;
            notes.push("content byte budget exhausted; page content cut short".into());
            break;
        }
        match s.decompressed_content_with_limit(room.min(l.max_decompressed_bytes)) {
            Ok(data) => {
                page_bytes += data.len();
                budget.content_bytes_left = budget.content_bytes_left.saturating_sub(data.len());
                out.push(data);
            }
            Err(lopdf::Error::Decompress(lopdf::DecompressError::MemoryLimitExceeded {
                ..
            })) => {
                truncated = true;
                notes.push("a content stream exceeds the decompression limit".into());
            }
            Err(e) => {
                undecodable += 1;
                notes.push(format!(
                    "a content stream could not be decoded: {}",
                    e.to_string().chars().take(80).collect::<String>()
                ));
            }
        }
    }
    (out, undecodable, truncated)
}

fn coding_name(sof: Option<u8>) -> &'static str {
    match sof {
        Some(0) => "baseline",
        Some(1) => "extended",
        Some(2) => "progressive",
        _ => "other",
    }
}

fn image_info(
    doc: &Document,
    im: &ImageUse,
    p: &StructuralParams,
    cache: &mut BTreeMap<ObjectId, Option<JpegInfo>>,
) -> ImageInfo {
    let jpeg = match im.id {
        Some(id) if im.filter == "DCTDecode" => cache
            .entry(id)
            .or_insert_with(|| jpeg_info(doc, id, p))
            .clone(),
        _ => None,
    };
    ImageInfo {
        object: im.id.map(|(n, _)| n),
        width: im.width,
        height: im.height,
        filter: im.filter.clone(),
        color_space: im.color_space.clone(),
        bits_per_component: im.bits,
        is_mask: im.is_mask,
        has_mask: im.has_mask,
        cover_fraction: im.cover,
        dpi: im.dpi,
        jpeg,
    }
}

fn jpeg_info(doc: &Document, id: ObjectId, p: &StructuralParams) -> Option<JpegInfo> {
    let Some(Object::Stream(s)) = doc.objects.get(&id) else {
        return None;
    };
    // Only a bare DCT stream: other filter chains would need decoding first.
    let filters = s.filters().ok()?;
    if filters.as_slice() != [b"DCTDecode".as_slice()] {
        return None;
    }
    let h = jpeg::parse_header(&s.content, p.provenance.max_jpeg_header_bytes)?;
    if h.tables.is_empty() {
        return None;
    }
    let luma = h
        .tables
        .iter()
        .find(|t| t.id == 0)
        .or_else(|| h.tables.first());
    let chroma = h.tables.iter().find(|t| t.id == 1);
    let luma_fit = luma.map(|t| jpeg::fit_ijg_quality(&t.values, false));
    let chroma_fit = chroma.map(|t| jpeg::fit_ijg_quality(&t.values, true));
    Some(JpegInfo {
        table_hash: jpeg::table_hash(&h.tables),
        tables: h.tables.len() as u32,
        luma_quality: luma_fit.map(|f| f.quality),
        luma_fit_error: luma_fit.map(|f| f.mean_abs_error),
        ijg_scaled: luma_fit
            .is_some_and(|f| f.mean_abs_error <= p.provenance.ijg_fit_mean_abs_error),
        chroma_quality: chroma_fit.map(|f| f.quality),
        width: h.width,
        height: h.height,
        components: u32::from(h.components),
        coding: coding_name(h.sof_type).to_owned(),
        app_markers: h.app_markers.clone(),
    })
}
