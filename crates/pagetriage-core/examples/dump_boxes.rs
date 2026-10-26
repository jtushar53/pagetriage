//! Print the checkboxes found on PGM pages: `cargo run --release -p pagetriage-core --example dump_boxes -- page.pgm`
use pagetriage_core::Params;
use pagetriage_core::pixel::{layout, quality, rulings};

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
        if let Ok(v) = std::env::var("REGION") {
            // x0,y0,w,h in the levelled frame, plus the rules found.
            let n: Vec<usize> = v.split(',').map(|t| t.parse().unwrap()).collect();
            println!("levelled {}x{}, skew {skew:.2}, rules:", l.width, l.height);
            for r in &l.rules {
                println!("  rule {:?}", (r.x0, r.y0, r.x1, r.y1));
            }
            for y in n[1]..n[1] + n[3] {
                let row: String = (n[0]..n[0] + n[2])
                    .map(|x| {
                        if l.ink[y * l.width + x] != 0 {
                            '#'
                        } else {
                            '.'
                        }
                    })
                    .collect();
                println!("{row}");
            }
        }
        if std::env::var_os("ASCII").is_some() {
            // The first three glyph components of box size, drawn from the levelled ink mask.
            for g in l
                .glyphs
                .iter()
                .filter(|g| g.bbox.width() > 14 && g.bbox.width() < 40)
                .take(3)
            {
                let r = g.bbox;
                println!("glyph {:?} skew {skew:.2}", (r.x0, r.y0, r.x1, r.y1));
                for y in r.y0.saturating_sub(2)..=r.y1 + 2 {
                    let row: String = (r.x0.saturating_sub(2)..=r.x1 + 2)
                        .map(|x| {
                            if l.ink[y as usize * l.width + x as usize] != 0 {
                                '#'
                            } else {
                                '.'
                            }
                        })
                        .collect();
                    println!("{row}");
                }
            }
        }
        for b in rulings::boxes(&l, 150, &params.rulings) {
            println!(
                "{{\"bbox\":[{},{},{},{}],\"checked\":{},\"edge\":{:.3}}}",
                b.bbox.x0, b.bbox.y0, b.bbox.x1, b.bbox.y1, b.checked, b.edge_coverage
            );
        }
    }
}
