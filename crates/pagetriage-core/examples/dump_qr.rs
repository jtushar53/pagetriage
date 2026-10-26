//! Print the finder patterns and codes of the QR detector (module M11), and with `--mask x0 y0 x1 y1`
//! the Sauvola mask of a region: `cargo run --release -p pagetriage-core --example dump_qr -- page.pgm`
use pagetriage_core::Params;
use pagetriage_core::pixel::binarize::local;
use pagetriage_core::pixel::qr;

fn main() {
    let params = Params::defaults().unwrap();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bytes = std::fs::read(&args[0]).unwrap();
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
    let mask = local::sauvola(gray, w, h, &params.sauvola);
    let fs = qr::finders(&mask, w, h, &params.qr);
    for f in &fs {
        println!("finder x={:.1} y={:.1} module={:.2}", f.x, f.y, f.module);
    }
    for c in qr::codes(&fs, w, h, &params.qr) {
        println!("{c:?}");
    }
    if args.len() == 6 && args[1] == "--mask" {
        let n: Vec<usize> = args[2..6].iter().map(|s| s.parse().unwrap()).collect();
        for y in n[1]..n[3] {
            let row: String = (n[0]..n[2])
                .map(|x| if mask[y * w + x] == 1 { '#' } else { '.' })
                .collect();
            println!("{row}");
        }
    }
}
