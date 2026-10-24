//! A tiny PDF assembler for tests: classic cross-reference tables, incremental updates, and a
//! one-page document builder. Everything here is synthetic.
#![allow(dead_code)]

use std::collections::BTreeMap;

pub struct Builder {
    pub out: Vec<u8>,
    offsets: BTreeMap<u32, usize>,
    pending: Vec<u32>,
    /// Offset of the last cross-reference section written (for `/Prev`).
    pub last_xref: Option<usize>,
}

impl Builder {
    pub fn new() -> Self {
        let mut out = b"%PDF-1.7\n".to_vec();
        out.extend_from_slice(b"%\xE2\xE3\xCF\xD3\n");
        Self {
            out,
            offsets: BTreeMap::new(),
            pending: Vec::new(),
            last_xref: None,
        }
    }

    pub fn obj(&mut self, num: u32, body: &str) {
        self.obj_bytes(num, body.as_bytes());
    }

    pub fn obj_bytes(&mut self, num: u32, body: &[u8]) {
        self.offsets.insert(num, self.out.len());
        self.pending.push(num);
        self.out
            .extend_from_slice(format!("{num} 0 obj\n").as_bytes());
        self.out.extend_from_slice(body);
        self.out.extend_from_slice(b"\nendobj\n");
    }

    /// A stream object; `dict` is the dictionary body without `/Length`.
    pub fn stream(&mut self, num: u32, dict: &str, data: &[u8]) {
        let mut body = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\nendstream");
        self.obj_bytes(num, &body);
    }

    /// Write a cross-reference section for the objects added since the last one, and a trailer.
    pub fn finish(&mut self, root: u32, info: Option<u32>, size: u32) {
        let xref_at = self.out.len();
        let mut nums = std::mem::take(&mut self.pending);
        nums.sort_unstable();
        nums.dedup();
        let first_revision = self.last_xref.is_none();
        let mut x = String::from("xref\n");
        if first_revision {
            x.push_str("0 1\n0000000000 65535 f \n");
        }
        let mut i = 0;
        while i < nums.len() {
            let mut j = i;
            while j + 1 < nums.len() && nums[j + 1] == nums[j] + 1 {
                j += 1;
            }
            x.push_str(&format!("{} {}\n", nums[i], j - i + 1));
            for n in &nums[i..=j] {
                x.push_str(&format!("{:010} 00000 n \n", self.offsets[n]));
            }
            i = j + 1;
        }
        x.push_str(&format!("trailer\n<< /Size {size} /Root {root} 0 R"));
        if let Some(info) = info {
            x.push_str(&format!(" /Info {info} 0 R"));
        }
        if let Some(prev) = self.last_xref {
            x.push_str(&format!(" /Prev {prev}"));
        }
        x.push_str(&format!(" >>\nstartxref\n{xref_at}\n%%EOF\n"));
        self.out.extend_from_slice(x.as_bytes());
        self.last_xref = Some(xref_at);
    }
}

pub struct Page {
    pub content: String,
    /// Text of the page's /Resources dictionary body (without the surrounding `<< >>`).
    pub resources: String,
    pub media_box: &'static str,
    pub extra: Vec<(u32, String)>,
    pub extra_streams: Vec<(u32, String, Vec<u8>)>,
}

impl Page {
    pub fn new(content: &str) -> Self {
        Self {
            content: content.to_owned(),
            resources: "/Font << /F1 5 0 R >>".to_owned(),
            media_box: "[0 0 612 792]",
            extra: vec![(
                5,
                "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
                    .to_owned(),
            )],
            extra_streams: Vec::new(),
        }
    }

    pub fn resources(mut self, r: &str) -> Self {
        self.resources = r.to_owned();
        self
    }

    pub fn object(mut self, num: u32, body: &str) -> Self {
        self.extra.retain(|(n, _)| *n != num);
        self.extra.push((num, body.to_owned()));
        self
    }

    pub fn stream(mut self, num: u32, dict: &str, data: &[u8]) -> Self {
        self.extra_streams
            .push((num, dict.to_owned(), data.to_vec()));
        self
    }

    /// Objects: 1 catalog, 2 pages, 3 page, 4 content; extras as given.
    pub fn build(&self, producer: Option<&str>) -> Vec<u8> {
        let mut b = Builder::new();
        b.obj(1, "<< /Type /Catalog /Pages 2 0 R >>");
        b.obj(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
        b.obj(
            3,
            &format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox {} /Contents 4 0 R /Resources << {} >> >>",
                self.media_box, self.resources
            ),
        );
        b.stream(4, "", self.content.as_bytes());
        let mut max = 4;
        for (n, body) in &self.extra {
            b.obj(*n, body);
            max = max.max(*n);
        }
        for (n, dict, data) in &self.extra_streams {
            b.stream(*n, dict, data);
            max = max.max(*n);
        }
        let info = producer.map(|p| {
            max += 1;
            b.obj(max, &format!("<< /Producer ({p}) /Creator (Test) >>"));
            max
        });
        b.finish(1, info, max + 1);
        b.out
    }
}

/// A raw 8-bit gray image XObject of the given size (all mid-gray).
pub fn gray_image(w: u32, h: u32) -> (String, Vec<u8>) {
    (
        format!(
            "/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray /BitsPerComponent 8"
        ),
        vec![128u8; (w * h) as usize],
    )
}
