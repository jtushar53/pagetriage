//! Print the circles the embossed-seal detector (module M11) keeps, with their measurements:
//! `cargo run --release -p pagetriage-core --example dump_emboss -- page.pgm [dpi] [all]`
use pagetriage_core::Params;
use pagetriage_core::pixel::emboss;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // A third argument `all` drops the antisymmetry and contrast tests, to see what they reject.
    let params = if args.get(2).is_some_and(|a| a == "all") {
        Params::with_overrides(
            "[emboss]\nantisymmetry_min = -1.0\nmin_contrast = 0.0\nmax_seals = 20\n",
        )
        .unwrap()
    } else {
        Params::defaults().unwrap()
    };
    let bytes = std::fs::read(&args[0]).unwrap();
    let dpi: u16 = args.get(1).map_or(150, |s| s.parse().unwrap());
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
    for s in emboss::seals(&bytes[pos..], w, h, dpi, &params.emboss) {
        println!(
            "centre=({:.0},{:.0}) r={:.0} votes={:.2} antisymmetry={:.2} contrast={:.2}",
            s.cx, s.cy, s.radius, s.vote_share, s.antisymmetry, s.contrast
        );
    }
}
