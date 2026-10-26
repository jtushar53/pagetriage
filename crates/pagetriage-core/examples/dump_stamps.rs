//! Print the candidate regions of the stamp detector (module M6) for PPM (P6) or PGM (P5) pages:
//! `cargo run --release -p pagetriage-core --example dump_stamps -- page.ppm`
use pagetriage_core::Params;
use pagetriage_core::pixel::stamp;

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
        assert_eq!(tokens[0], "P6", "a colour page (PPM, P6) is needed");
        let regs = stamp::regions(&bytes[pos..], w, h, 150, &params.stamp).unwrap();
        for r in regs {
            println!(
                "{{\"page\":{path:?},\"bbox\":{:?},\"pixels\":{},\"conc\":{:.3},\"chroma\":{:.1},\"hue\":{:.0},\"ring\":{:.3},\"inner\":{:.3},\"rect\":{:.3},\"tilt\":{:.0},\"ring_interior\":{:.3},\"rect_interior\":{:.3},\"round_box\":{}}}",
                r.bbox,
                r.pixels,
                r.hue_concentration,
                r.mean_chroma,
                r.peak_hue_deg,
                r.frame.ring,
                r.frame.inner_ring,
                r.frame.rectangle,
                r.frame.tilt_deg,
                r.frame.ring_interior,
                r.frame.rect_interior,
                r.round_box
            );
        }
    }
}
