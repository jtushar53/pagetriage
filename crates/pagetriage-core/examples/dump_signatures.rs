//! Print the signature candidates (module M7) of PGM pages with every measurement:
//! `cargo run --release -p pagetriage-core --example dump_signatures -- page.pgm`
use pagetriage_core::Params;
use pagetriage_core::pixel::{layout, quality, signature};

fn main() {
    let params = Params::defaults().unwrap();
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path).unwrap();
        let mut pos = 0;
        let mut tokens = Vec::new();
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
        let (w, h): (usize, usize) = (tokens[1].parse().unwrap(), tokens[2].parse().unwrap());
        let gray = &bytes[pos..];
        let skew = quality::assess(gray, w, h, 150, &params)
            .unwrap()
            .quality
            .skew_deg;
        let l = layout::segment(gray, w, h, skew, &params).unwrap();
        if std::env::var_os("LAYOUT").is_some() {
            println!(
                "xheight {} graphics {:?}",
                l.xheight,
                l.graphics
                    .iter()
                    .map(|r| (r.x0, r.y0, r.x1, r.y1))
                    .collect::<Vec<_>>()
            );
            for g in l.glyphs.iter().filter(|g| g.bbox.height() > 20) {
                println!(
                    "glyph {:?} line {:?}",
                    (g.bbox.x0, g.bbox.y0, g.bbox.x1, g.bbox.y1),
                    g.line
                );
            }
        }
        for c in signature::candidates(&l, 150, &params.signature) {
            let b = l.rect_to_page(&c.bbox);
            println!(
                "{{\"page\":{path:?},\"bbox\":{b:?},\"w_in\":{:.2},\"h_in\":{:.2},\"fill\":{:.3},\"hole\":{:.3},\"tort\":{:.2},\"loops\":{},\"axis\":{:.3},\"h_xh\":{:.2}}}",
                c.width_in,
                c.height_in,
                c.fill,
                c.max_hole,
                c.tortuosity,
                c.loops,
                c.axis_share,
                c.height_xh
            );
        }
    }
}
