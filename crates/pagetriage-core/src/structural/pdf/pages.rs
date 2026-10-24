//! Page-tree traversal with cycle, depth and count limits, collecting the inheritable page
//! attributes (`Resources`, `MediaBox`, `CropBox`, `Rotate`).

use std::collections::BTreeSet;

use lopdf::{Dictionary, Document, Object, ObjectId};

use super::interp::PageBox;

/// One page leaf with its inherited attributes resolved.
pub struct PageRef<'d> {
    pub id: ObjectId,
    pub dict: &'d Dictionary,
    pub resources: Option<&'d Dictionary>,
    pub media_box: Option<[f64; 4]>,
    pub crop_box: Option<[f64; 4]>,
    pub rotate: i64,
}

#[derive(Clone, Copy, Default)]
struct Inherited<'d> {
    resources: Option<&'d Dictionary>,
    media_box: Option<[f64; 4]>,
    crop_box: Option<[f64; 4]>,
    rotate: Option<i64>,
}

fn rect_of(doc: &Document, o: &Object) -> Option<[f64; 4]> {
    let (_, o) = doc.dereference(o).ok()?;
    let a = o.as_array().ok()?;
    if a.len() < 4 {
        return None;
    }
    let mut v = [0.0f64; 4];
    for (slot, item) in v.iter_mut().zip(a.iter()) {
        let (_, item) = doc.dereference(item).ok()?;
        *slot = f64::from(item.as_float().ok()?);
    }
    if !v.iter().all(|x| x.is_finite()) {
        return None;
    }
    Some([
        v[0].min(v[2]),
        v[1].min(v[3]),
        v[0].max(v[2]),
        v[1].max(v[3]),
    ])
}

/// Walk the page tree from the catalog's `/Pages`. Returns the pages in order and whether the
/// walk hit a limit (`max_pages` or the depth/cycle guards cut something off).
pub fn collect_pages<'d>(
    doc: &'d Document,
    max_pages: usize,
    max_depth: usize,
) -> (Vec<PageRef<'d>>, bool) {
    let mut out = Vec::new();
    let mut truncated = false;
    let root = doc
        .trailer
        .get(b"Root")
        .ok()
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_dict().ok())
        .and_then(|catalog| catalog.get(b"Pages").ok());
    let Some(root) = root else {
        return (out, false);
    };
    let Ok((Some(root_id), _)) = doc.dereference(root) else {
        return (out, false);
    };
    let mut seen: BTreeSet<ObjectId> = BTreeSet::new();
    // Explicit stack (depth-first, first kid first) instead of recursion.
    let mut stack: Vec<(ObjectId, usize, Inherited<'d>)> = vec![(root_id, 0, Inherited::default())];
    while let Some((id, depth, inh)) = stack.pop() {
        if out.len() >= max_pages {
            truncated = true;
            break;
        }
        if depth > max_depth || !seen.insert(id) {
            truncated |= depth > max_depth;
            continue;
        }
        let Some(Object::Dictionary(dict)) = doc.objects.get(&id) else {
            continue;
        };
        let mut here = inh;
        if let Some(r) = dict
            .get(b"Resources")
            .ok()
            .and_then(|o| doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
        {
            here.resources = Some(r);
        }
        if let Some(b) = dict.get(b"MediaBox").ok().and_then(|o| rect_of(doc, o)) {
            here.media_box = Some(b);
        }
        if let Some(b) = dict.get(b"CropBox").ok().and_then(|o| rect_of(doc, o)) {
            here.crop_box = Some(b);
        }
        if let Some(r) = dict
            .get(b"Rotate")
            .ok()
            .and_then(|o| doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_i64().ok())
        {
            here.rotate = Some(r);
        }
        let kids = dict
            .get(b"Kids")
            .ok()
            .and_then(|o| doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok());
        let is_pages_node = kids.is_some()
            && dict
                .get(b"Type")
                .ok()
                .and_then(|o| o.as_name().ok())
                .is_none_or(|t| t != b"Page");
        if is_pages_node {
            if let Some(kids) = kids {
                for kid in kids.iter().rev() {
                    if let Ok(kid_id) = kid.as_reference() {
                        stack.push((kid_id, depth + 1, here));
                    }
                }
            }
        } else {
            out.push(PageRef {
                id,
                dict,
                resources: here.resources,
                media_box: here.media_box,
                crop_box: here.crop_box,
                rotate: here.rotate.unwrap_or(0),
            });
        }
    }
    (out, truncated)
}

/// Pages found by scanning every object for `/Type /Page`, in object-number order. Used only when
/// the page tree gives nothing.
pub fn scan_for_pages(doc: &Document, max_pages: usize) -> Vec<PageRef<'_>> {
    let mut out = Vec::new();
    for (id, obj) in &doc.objects {
        if out.len() >= max_pages {
            break;
        }
        if let Object::Dictionary(d) = obj
            && d.get(b"Type").ok().and_then(|o| o.as_name().ok()) == Some(b"Page")
        {
            out.push(PageRef {
                id: *id,
                dict: d,
                resources: d
                    .get(b"Resources")
                    .ok()
                    .and_then(|o| doc.dereference(o).ok())
                    .and_then(|(_, o)| o.as_dict().ok()),
                media_box: d.get(b"MediaBox").ok().and_then(|o| rect_of(doc, o)),
                crop_box: d.get(b"CropBox").ok().and_then(|o| rect_of(doc, o)),
                rotate: d
                    .get(b"Rotate")
                    .ok()
                    .and_then(|o| o.as_i64().ok())
                    .unwrap_or(0),
            });
        }
    }
    out
}

/// The page box used for coverage (CropBox clipped to MediaBox; Letter if nothing usable) and
/// the display size in points after rotation.
pub fn page_geometry(p: &PageRef<'_>) -> (PageBox, [f64; 2], u32, bool) {
    let default = [0.0, 0.0, 612.0, 792.0];
    let mut used_default = false;
    let media = p.media_box.unwrap_or_else(|| {
        used_default = true;
        default
    });
    let b = match p.crop_box {
        Some(c) => {
            let r = [
                c[0].max(media[0]),
                c[1].max(media[1]),
                c[2].min(media[2]),
                c[3].min(media[3]),
            ];
            if r[2] > r[0] && r[3] > r[1] { r } else { media }
        }
        None => media,
    };
    let b = if b[2] - b[0] > 0.0 && b[3] - b[1] > 0.0 {
        b
    } else {
        used_default = true;
        default
    };
    let page = PageBox {
        x0: b[0],
        y0: b[1],
        x1: b[2],
        y1: b[3],
    };
    let rot = p.rotate.rem_euclid(360) as u32;
    let rot = if rot.is_multiple_of(90) { rot } else { 0 };
    let (w, h) = (page.width(), page.height());
    let size = if rot == 90 || rot == 270 {
        [h, w]
    } else {
        [w, h]
    };
    (page, size, rot, used_default)
}
