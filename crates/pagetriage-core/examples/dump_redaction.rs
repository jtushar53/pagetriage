//! Print the dark rectangles of the redaction detector (module M12) with their measurements:
//! `cargo run --release -p pagetriage-core --example dump_redaction -- page.pgm`
use pagetriage_core::Params;
use pagetriage_core::pixel::{layout, quality, redaction};

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
        for b in redaction::boxes(&l, &l.level_gray(gray), &params.redaction) {
            println!(
                "{{\"bbox\":[{},{},{},{}],\"fill\":{:.3},\"edge\":{:.3},\"sides\":{},\"h_xh\":{:.1},\"w_xh\":{:.1}}}",
                b.bbox.x0,
                b.bbox.y0,
                b.bbox.x1,
                b.bbox.y1,
                b.fill,
                b.edge,
                b.sides,
                b.height_xh,
                b.width_xh
            );
        }
    }
}
