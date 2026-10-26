//! Print the per-word measurements of the handwriting detector (module M5) for PGM pages, one JSON
//! object per word, for tuning and for `bench/handwriting_synth_eval.py --features`.
//!
//!     cargo run --release -p pagetriage-core --example dump_words -- page1.pgm page2.pgm

use pagetriage_core::pixel::{handwriting, layout};
use pagetriage_core::{Params, pixel::quality};

fn read_pgm(bytes: &[u8]) -> (usize, usize, Vec<u8>) {
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
    (
        tokens[1].parse().unwrap(),
        tokens[2].parse().unwrap(),
        bytes[pos..].to_vec(),
    )
}

fn main() {
    let params = Params::defaults().unwrap();
    for path in std::env::args().skip(1) {
        let (w, h, gray) = read_pgm(&std::fs::read(&path).unwrap());
        let skew = quality::assess(&gray, w, h, 150, &params)
            .unwrap()
            .quality
            .skew_deg;
        let l = layout::segment(&gray, w, h, skew, &params).unwrap();
        for word in handwriting::words(&l, &params.handwriting) {
            let b = l.rect_to_page(&word.bbox);
            println!(
                "{{\"page\":{path:?},\"bbox\":[{},{},{},{}],\"glyphs\":{},\"baseline_dev\":{:.4},\"axis_share\":{:.4},\"stroke_cv\":{:.4},\"height_cv\":{},\"p\":{:.4}}}",
                b[0],
                b[1],
                b[2],
                b[3],
                word.glyphs,
                word.baseline_dev,
                word.axis_share,
                word.stroke_cv,
                word.height_cv
                    .map_or("null".to_string(), |v| format!("{v:.4}")),
                word.probability
            );
        }
    }
}
