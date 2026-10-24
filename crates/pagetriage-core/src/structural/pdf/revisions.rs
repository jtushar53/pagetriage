//! Incremental updates (module M12): find the file's revisions, diff the object sets of
//! consecutive ones, and raise a tamper signal when page content changes after a signature.
//!
//! A revision is a prefix of the file ending at a `%%EOF` marker that parses as a complete PDF on
//! its own (cross-reference data intact, no reconstruction). Prefixes that do not (the first part
//! of a linearised file, a `%%EOF` inside binary data) are skipped, so a linearised file counts
//! as one revision and not two.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};

use lopdf::{Dictionary, Document, LoadOptions, Object, ObjectId};

use super::pages::collect_pages;
use crate::structural::params::StructuralParams;
use crate::structural::types::RevisionInfo;

pub struct RevisionAnalysis {
    pub revisions: Vec<RevisionInfo>,
    pub eof_markers: u32,
    pub truncated: bool,
    /// Valid revisions found (original file included); updates = this minus one.
    pub valid_revisions: u32,
    pub tamper_signal: bool,
    pub tamper_reasons: Vec<String>,
    /// Some update changed page content, images or the page tree (signed or not).
    pub content_changed_in_update: bool,
}

/// Load a PDF from bytes with the decompression cap, converting parser panics into `None`.
pub fn load_guarded(bytes: &[u8], max_decompressed: usize) -> Result<Document, String> {
    let opts = LoadOptions {
        max_decompressed_size: Some(max_decompressed),
        ..LoadOptions::default()
    };
    match catch_unwind(AssertUnwindSafe(|| {
        Document::load_mem_with_options(bytes, opts)
    })) {
        Ok(Ok(doc)) => Ok(doc),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("the PDF parser panicked on this input".to_owned()),
    }
}

fn eof_ends(bytes: &[u8], max: usize) -> (Vec<usize>, u32) {
    let mut ends = Vec::new();
    let mut total = 0u32;
    let needle = b"%%EOF";
    let mut i = 0;
    while let Some(rel) = bytes[i..].windows(needle.len()).position(|w| w == needle) {
        let end = i + rel + needle.len();
        total = total.saturating_add(1);
        if ends.len() < max {
            ends.push(end);
        }
        i = end;
    }
    (ends, total)
}

fn streams_equal(a: &lopdf::Stream, b: &lopdf::Stream) -> bool {
    a.dict == b.dict && a.content == b.content
}

fn objects_equal(a: &Object, b: &Object) -> bool {
    match (a, b) {
        (Object::Stream(x), Object::Stream(y)) => streams_equal(x, y),
        _ => a == b,
    }
}

fn type_of(o: &Object) -> Option<&[u8]> {
    let d = match o {
        Object::Dictionary(d) => d,
        Object::Stream(s) => &s.dict,
        _ => return None,
    };
    d.get(b"Type").ok().and_then(|t| t.as_name().ok())
}

fn dict_of(o: &Object) -> Option<&Dictionary> {
    match o {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

fn is_signature(d: &Dictionary) -> bool {
    d.get(b"Type").ok().and_then(|t| t.as_name().ok()) == Some(b"Sig")
        || (d.has(b"ByteRange") && d.has(b"Contents") && d.has(b"Filter"))
}

fn differing_keys(a: &Dictionary, b: &Dictionary) -> BTreeSet<Vec<u8>> {
    let mut out = BTreeSet::new();
    for (k, v) in a.iter() {
        match b.get(k) {
            Ok(w) if objects_equal(v, w) => {}
            _ => {
                out.insert(k.clone());
            }
        }
    }
    for (k, _) in b.iter() {
        if !a.has(k) {
            out.insert(k.clone());
        }
    }
    out
}

/// Count signature dictionaries in a document.
pub fn count_signatures(doc: &Document) -> u32 {
    doc.objects
        .values()
        .filter(|o| dict_of(o).is_some_and(is_signature))
        .count()
        .min(u32::MAX as usize) as u32
}

struct PageMap {
    /// page object -> 0-based page index
    pages: BTreeMap<ObjectId, u32>,
    /// content stream object -> page index
    contents: BTreeMap<ObjectId, u32>,
}

fn page_map(doc: &Document, p: &StructuralParams) -> PageMap {
    let (pages, _) = collect_pages(doc, p.limits.max_pages, p.limits.max_page_tree_depth);
    let mut map = PageMap {
        pages: BTreeMap::new(),
        contents: BTreeMap::new(),
    };
    for (i, page) in pages.iter().enumerate() {
        let idx = i as u32;
        map.pages.insert(page.id, idx);
        if let Ok(c) = page.dict.get(b"Contents") {
            match c {
                Object::Reference(id) => {
                    // A reference to a stream, or to an array of streams.
                    match doc.objects.get(id) {
                        Some(Object::Array(a)) => {
                            for r in a {
                                if let Ok(rid) = r.as_reference() {
                                    map.contents.insert(rid, idx);
                                }
                            }
                        }
                        _ => {
                            map.contents.insert(*id, idx);
                        }
                    }
                }
                Object::Array(a) => {
                    for r in a {
                        if let Ok(rid) = r.as_reference() {
                            map.contents.insert(rid, idx);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    map
}

/// Kinds of change that rewrite what a page shows.
const CONTENT_KINDS: [&str; 4] = [
    "page_content_changed",
    "image_changed",
    "xobject_changed",
    "page_tree_changed",
];

struct Diff {
    added: u32,
    changed: u32,
    freed: u32,
    kinds: BTreeSet<&'static str>,
    pages: BTreeSet<u32>,
}

fn diff(prev: &Document, cur: &Document, p: &StructuralParams) -> Diff {
    let mut d = Diff {
        added: 0,
        changed: 0,
        freed: 0,
        kinds: BTreeSet::new(),
        pages: BTreeSet::new(),
    };
    let prev_map = page_map(prev, p);
    let cur_map = page_map(cur, p);
    if prev_map.pages.keys().ne(cur_map.pages.keys()) {
        d.kinds.insert("page_tree_changed");
    }
    d.freed = prev
        .objects
        .keys()
        .filter(|k| !cur.objects.contains_key(k))
        .count()
        .min(u32::MAX as usize) as u32;
    let info_id = cur
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|o| o.as_reference().ok());
    let meta_id = cur
        .trailer
        .get(b"Root")
        .ok()
        .and_then(|o| doc_dict(cur, o))
        .and_then(|cat| cat.get(b"Metadata").ok())
        .and_then(|o| o.as_reference().ok());

    for (id, obj) in &cur.objects {
        let before = prev.objects.get(id);
        let is_new = before.is_none();
        if let Some(b) = before {
            if objects_equal(b, obj) {
                continue;
            }
            d.changed += 1;
        } else {
            d.added += 1;
        }
        // Classify, most specific first.
        let dict = dict_of(obj);
        if dict.is_some_and(is_signature) {
            if is_new {
                d.kinds.insert("signature_added");
            }
            continue;
        }
        if Some(*id) == info_id || Some(*id) == meta_id {
            d.kinds.insert("metadata_changed");
            continue;
        }
        if let Some(&page) = cur_map
            .contents
            .get(id)
            .or_else(|| prev_map.contents.get(id))
        {
            d.kinds.insert("page_content_changed");
            d.pages.insert(page);
            continue;
        }
        if let Some(&page) = cur_map.pages.get(id) {
            let keys = match before.and_then(dict_of) {
                Some(b) => differing_keys(b, dict.expect("a page is a dictionary")),
                None => BTreeSet::new(),
            };
            if is_new {
                d.kinds.insert("page_tree_changed");
            } else if keys.iter().any(|k| {
                matches!(
                    k.as_slice(),
                    b"Contents" | b"Resources" | b"MediaBox" | b"CropBox" | b"Rotate"
                )
            }) {
                d.kinds.insert("page_content_changed");
                d.pages.insert(page);
            } else if keys.contains(b"Annots".as_slice()) {
                d.kinds.insert("annotation_changed");
            } else {
                d.kinds.insert("other_changed");
            }
            continue;
        }
        match type_of(obj) {
            Some(b"Annot") => {
                let widget = dict
                    .and_then(|x| x.get(b"Subtype").ok())
                    .and_then(|s| s.as_name().ok())
                    == Some(b"Widget");
                d.kinds.insert(if widget {
                    "form_field_changed"
                } else if is_new {
                    "annotation_added"
                } else {
                    "annotation_changed"
                });
                continue;
            }
            Some(b"Font") | Some(b"FontDescriptor") => {
                d.kinds.insert("font_changed");
                continue;
            }
            Some(b"Pages") => {
                d.kinds.insert("page_tree_changed");
                continue;
            }
            Some(b"Catalog") => {
                d.kinds.insert("catalog_changed");
                continue;
            }
            _ => {}
        }
        if let Object::Stream(s) = obj {
            match s.dict.get(b"Subtype").ok().and_then(|x| x.as_name().ok()) {
                Some(b"Image") => {
                    d.kinds.insert(if is_new {
                        "image_added"
                    } else {
                        "image_changed"
                    });
                    continue;
                }
                Some(b"Form") => {
                    d.kinds.insert(if is_new {
                        "other_changed"
                    } else {
                        "xobject_changed"
                    });
                    continue;
                }
                _ => {}
            }
        }
        // Widgets and annotations are sometimes untyped.
        if dict.is_some_and(|x| x.has(b"Rect") && x.has(b"Subtype")) {
            d.kinds.insert(if is_new {
                "annotation_added"
            } else {
                "annotation_changed"
            });
            continue;
        }
        d.kinds.insert("other_changed");
    }
    d
}

fn doc_dict<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Dictionary> {
    doc.dereference(o).ok().and_then(|(_, x)| x.as_dict().ok())
}

struct Walk {
    out: RevisionAnalysis,
    sig_revision: Option<u32>,
    update_index: u32,
    valid: u32,
}

impl Walk {
    fn step(
        &mut self,
        cur: &Document,
        before: Option<&Document>,
        end: usize,
        p: &StructuralParams,
    ) {
        self.valid += 1;
        let Some(before) = before else {
            if count_signatures(cur) > 0 {
                self.sig_revision = Some(0);
            }
            return;
        };
        self.update_index += 1;
        let df = diff(before, cur, p);
        let kinds: Vec<String> = df.kinds.iter().map(|s| (*s).to_owned()).collect();
        let what: Vec<&str> = CONTENT_KINDS
            .iter()
            .copied()
            .filter(|c| df.kinds.contains(c))
            .collect();
        if !what.is_empty() {
            self.out.content_changed_in_update = true;
            if let Some(j) = self.sig_revision {
                self.out.tamper_signal = true;
                self.out.tamper_reasons.push(format!(
                    "update {} changes page content ({}) after the signature in revision {j}",
                    self.update_index,
                    what.join(", ")
                ));
            }
        }
        if df.kinds.contains("signature_added") && self.sig_revision.is_none() {
            self.sig_revision = Some(self.update_index);
        }
        self.out.revisions.push(RevisionInfo {
            index: self.update_index,
            end_offset: end as u64,
            objects_added: df.added,
            objects_changed: df.changed,
            objects_freed: df.freed,
            kinds,
            content_changed_pages: df.pages.into_iter().collect(),
        });
    }
}

pub fn analyse(bytes: &[u8], final_doc: &Document, p: &StructuralParams) -> RevisionAnalysis {
    let prov = &p.provenance;
    let (mut ends, total) = eof_ends(bytes, prov.max_eof_markers);
    let out = RevisionAnalysis {
        revisions: Vec::new(),
        eof_markers: total,
        truncated: total as usize > prov.max_eof_markers,
        valid_revisions: 1,
        tamper_signal: false,
        tamper_reasons: Vec::new(),
        content_changed_in_update: false,
    };
    if ends.is_empty() {
        return out;
    }
    // The last candidate stands for the whole file unless a lot of data follows it.
    let last = *ends.last().expect("non-empty");
    if last + 1024 < bytes.len() {
        ends.push(bytes.len());
    } else {
        *ends.last_mut().expect("non-empty") = bytes.len();
    }
    // Examine only the most recent prefixes.
    let skip = ends.len().saturating_sub(prov.max_revisions + 1);
    let mut walk = Walk {
        out,
        sig_revision: None,
        update_index: 0,
        valid: 0,
    };
    if skip > 0 {
        walk.out.truncated = true;
    }
    let ends = &ends[skip..];

    let mut prev: Option<Document> = None;
    let mut parsed_bytes = 0usize;
    let n = ends.len();
    for (k, &end) in ends.iter().enumerate() {
        if k + 1 == n {
            walk.step(final_doc, prev.as_ref(), end, p);
            break;
        }
        parsed_bytes = parsed_bytes.saturating_add(end);
        if parsed_bytes > prov.max_revision_parse_bytes {
            walk.out.truncated = true;
            continue;
        }
        match load_guarded(&bytes[..end], p.limits.max_decompressed_bytes) {
            Ok(doc) if doc.xref_recovery.is_none() && doc.catalog().is_ok() => {
                walk.step(&doc, prev.as_ref(), end, p);
                prev = Some(doc);
            }
            _ => {}
        }
    }
    walk.out.valid_revisions = walk.valid.max(1);
    walk.out
}
