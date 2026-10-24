//! A tolerant PDF content-stream tokenizer: operands, operators and inline images.
//!
//! It never panics and never allocates more than the stream itself plus a bounded operand stack.
//! Unknown tokens become operators the interpreter ignores. It is deliberately not `lopdf`'s
//! content parser: a single bad inline image there fails the whole page, and the operand and
//! nesting limits here are ours to set.

/// One operand.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Num(f64),
    Name(Vec<u8>),
    Str(Vec<u8>),
    Array(Vec<Operand>),
    Dict(Vec<(Vec<u8>, Operand)>),
    /// `true`, `false`, `null`.
    Other,
}

impl Operand {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Operand::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_name(&self) -> Option<&[u8]> {
        match self {
            Operand::Name(n) => Some(n),
            _ => None,
        }
    }
}

/// What [`Tokenizer::next_item`] returns.
#[derive(Debug, Clone, PartialEq)]
pub enum Item<'a> {
    /// An operator keyword (`Tj`, `cm`, `q`, `'`, ...); its operands are in the operand vector.
    Op(&'a [u8]),
    /// `BI ... ID ... EI`: the inline image's dictionary entries (abbreviated keys as written).
    InlineImage(Vec<(Vec<u8>, Operand)>),
}

pub struct Tokenizer<'a> {
    data: &'a [u8],
    pos: usize,
    max_operands: usize,
}

const MAX_NESTING: usize = 6;

fn is_ws(c: u8) -> bool {
    matches!(c, 0 | 9 | 10 | 12 | 13 | 32)
}

fn is_delim(c: u8) -> bool {
    matches!(
        c,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn is_regular(c: u8) -> bool {
    !is_ws(c) && !is_delim(c)
}

impl<'a> Tokenizer<'a> {
    pub fn new(data: &'a [u8], max_operands: usize) -> Self {
        Tokenizer {
            data,
            pos: 0,
            max_operands: max_operands.max(1),
        }
    }

    pub fn is_done(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn skip_ws_and_comments(&mut self) {
        while let Some(&c) = self.data.get(self.pos) {
            if is_ws(c) {
                self.pos += 1;
            } else if c == b'%' {
                while self.pos < self.data.len() && !matches!(self.data[self.pos], b'\n' | b'\r') {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    /// Read operands into `operands` (cleared first) until an operator is found.
    pub fn next_item(&mut self, operands: &mut Vec<Operand>) -> Option<Item<'a>> {
        operands.clear();
        loop {
            self.skip_ws_and_comments();
            let c = *self.data.get(self.pos)?;
            match c {
                b'/' | b'(' | b'[' | b'<' => {
                    let v = self.operand(0)?;
                    self.push(operands, v);
                }
                b'>' | b']' | b')' | b'{' | b'}' => {
                    // A stray closer: skip it.
                    self.pos += 1;
                }
                _ => {
                    let start = self.pos;
                    while self.pos < self.data.len() && is_regular(self.data[self.pos]) {
                        self.pos += 1;
                    }
                    let tok = &self.data[start..self.pos];
                    if let Some(n) = parse_number(tok) {
                        self.push(operands, Operand::Num(n));
                    } else if tok == b"true" || tok == b"false" || tok == b"null" {
                        self.push(operands, Operand::Other);
                    } else if tok == b"BI" {
                        return Some(Item::InlineImage(self.inline_image()));
                    } else {
                        return Some(Item::Op(tok));
                    }
                }
            }
        }
    }

    fn push(&self, operands: &mut Vec<Operand>, v: Operand) {
        if operands.len() >= self.max_operands {
            operands.remove(0);
        }
        operands.push(v);
    }

    fn operand(&mut self, depth: usize) -> Option<Operand> {
        self.skip_ws_and_comments();
        let c = *self.data.get(self.pos)?;
        match c {
            b'/' => {
                self.pos += 1;
                Some(Operand::Name(self.name()))
            }
            b'(' => Some(Operand::Str(self.literal_string())),
            b'<' if self.data.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                Some(self.dict(depth))
            }
            b'<' => Some(Operand::Str(self.hex_string())),
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws_and_comments();
                    match self.data.get(self.pos) {
                        None => break,
                        Some(b']') => {
                            self.pos += 1;
                            break;
                        }
                        Some(_) => {
                            if depth >= MAX_NESTING {
                                // Too deep: skip a token rather than recurse.
                                self.skip_token();
                                continue;
                            }
                            match self.operand(depth + 1) {
                                Some(v) => {
                                    if items.len() < 1 << 16 {
                                        items.push(v);
                                    }
                                }
                                None => break,
                            }
                        }
                    }
                }
                Some(Operand::Array(items))
            }
            _ => {
                let start = self.pos;
                while self.pos < self.data.len() && is_regular(self.data[self.pos]) {
                    self.pos += 1;
                }
                if self.pos == start {
                    self.pos += 1; // a delimiter we do not handle here
                    return Some(Operand::Other);
                }
                let tok = &self.data[start..self.pos];
                Some(match parse_number(tok) {
                    Some(n) => Operand::Num(n),
                    None => Operand::Other,
                })
            }
        }
    }

    fn skip_token(&mut self) {
        let start = self.pos;
        while self.pos < self.data.len() && is_regular(self.data[self.pos]) {
            self.pos += 1;
        }
        if self.pos == start {
            self.pos += 1;
        }
    }

    fn dict(&mut self, depth: usize) -> Operand {
        let mut entries = Vec::new();
        loop {
            self.skip_ws_and_comments();
            match self.data.get(self.pos) {
                None => break,
                Some(b'>') => {
                    self.pos += if self.data.get(self.pos + 1) == Some(&b'>') {
                        2
                    } else {
                        1
                    };
                    break;
                }
                Some(b'/') => {
                    self.pos += 1;
                    let key = self.name();
                    if depth >= MAX_NESTING {
                        self.skip_token();
                        continue;
                    }
                    if let Some(v) = self.operand(depth + 1)
                        && entries.len() < 256
                    {
                        entries.push((key, v));
                    }
                }
                Some(_) => self.skip_token(),
            }
        }
        Operand::Dict(entries)
    }

    fn name(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        while let Some(&c) = self.data.get(self.pos) {
            if !is_regular(c) {
                break;
            }
            self.pos += 1;
            if c == b'#' {
                let h = |i: usize| self.data.get(self.pos + i).copied().and_then(hex_val);
                if let (Some(a), Some(b)) = (h(0), h(1)) {
                    out.push(a * 16 + b);
                    self.pos += 2;
                    continue;
                }
            }
            if out.len() < 4096 {
                out.push(c);
            }
        }
        out
    }

    fn literal_string(&mut self) -> Vec<u8> {
        self.pos += 1; // (
        let mut out = Vec::new();
        let mut depth = 1usize;
        while let Some(&c) = self.data.get(self.pos) {
            self.pos += 1;
            match c {
                b'\\' => {
                    let Some(&e) = self.data.get(self.pos) else {
                        break;
                    };
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'0'..=b'7' => {
                            let mut v = u32::from(e - b'0');
                            for _ in 0..2 {
                                match self.data.get(self.pos) {
                                    Some(&d @ b'0'..=b'7') => {
                                        v = v * 8 + u32::from(d - b'0');
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push((v & 0xFF) as u8);
                        }
                        b'\r' => {
                            if self.data.get(self.pos) == Some(&b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        other => out.push(other),
                    }
                }
                b'(' => {
                    depth += 1;
                    out.push(c);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
        out
    }

    fn hex_string(&mut self) -> Vec<u8> {
        self.pos += 1; // <
        let mut nibbles: Vec<u8> = Vec::new();
        while let Some(&c) = self.data.get(self.pos) {
            self.pos += 1;
            if c == b'>' {
                break;
            }
            if let Some(v) = hex_val(c) {
                nibbles.push(v);
            }
        }
        if nibbles.len() % 2 == 1 {
            nibbles.push(0);
        }
        nibbles.chunks(2).map(|p| p[0] * 16 + p[1]).collect()
    }

    /// After `BI`: dictionary entries up to `ID`, then skip the data up to `EI`.
    fn inline_image(&mut self) -> Vec<(Vec<u8>, Operand)> {
        let mut entries: Vec<(Vec<u8>, Operand)> = Vec::new();
        loop {
            self.skip_ws_and_comments();
            let Some(&c) = self.data.get(self.pos) else {
                return entries;
            };
            if c == b'/' {
                self.pos += 1;
                let key = self.name();
                if let Some(v) = self.operand(1)
                    && entries.len() < 64
                {
                    entries.push((key, v));
                }
            } else {
                let start = self.pos;
                self.skip_token();
                if &self.data[start..self.pos] == b"ID" {
                    break;
                }
            }
        }
        // One whitespace byte follows ID.
        if self.data.get(self.pos).is_some_and(|c| is_ws(*c)) {
            self.pos += 1;
        }
        let exact = inline_data_len(&entries);
        if let Some(n) = exact {
            let end = self.pos.saturating_add(n);
            if end <= self.data.len() {
                let mut j = end;
                while j < self.data.len() && is_ws(self.data[j]) {
                    j += 1;
                }
                if self.data[j..].starts_with(b"EI")
                    && self.data.get(j + 2).is_none_or(|c| !is_regular(*c))
                {
                    self.pos = j + 2;
                    return entries;
                }
            }
        }
        // Search for a plausible EI.
        let mut j = self.pos;
        while j + 1 < self.data.len() {
            if self.data[j] == b'E'
                && self.data[j + 1] == b'I'
                && (j == self.pos || is_ws(self.data[j - 1]))
                && self.data.get(j + 2).is_none_or(|c| is_ws(*c))
            {
                let tail = &self.data[(j + 2).min(self.data.len())..];
                let probe = &tail[..tail.len().min(16)];
                if probe.iter().all(|&b| is_ws(b) || (0x20..0x7F).contains(&b)) {
                    self.pos = j + 2;
                    return entries;
                }
            }
            j += 1;
        }
        self.pos = self.data.len();
        entries
    }
}

/// Byte length of uncompressed inline-image data, when the dictionary gives it away.
fn inline_data_len(entries: &[(Vec<u8>, Operand)]) -> Option<usize> {
    let get = |long: &[u8], short: &[u8]| {
        entries
            .iter()
            .find(|(k, _)| k == long || k == short)
            .map(|(_, v)| v)
    };
    if get(b"Filter", b"F").is_some() {
        return None;
    }
    let w = get(b"Width", b"W")?.as_f64()?;
    let h = get(b"Height", b"H")?.as_f64()?;
    let is_mask = matches!(get(b"ImageMask", b"IM"), Some(Operand::Other));
    let (bpc, comps) = if is_mask {
        (1.0, 1.0)
    } else {
        let bpc = get(b"BitsPerComponent", b"BPC")?.as_f64()?;
        let comps = match get(b"ColorSpace", b"CS")?.as_name()? {
            b"G" | b"DeviceGray" | b"I" | b"Indexed" => 1.0,
            b"RGB" | b"DeviceRGB" => 3.0,
            b"CMYK" | b"DeviceCMYK" => 4.0,
            _ => return None,
        };
        (bpc, comps)
    };
    if !(w >= 1.0 && h >= 1.0 && w <= 1e6 && h <= 1e6 && (1.0..=16.0).contains(&bpc)) {
        return None;
    }
    let row = (w * comps * bpc / 8.0).ceil();
    let total = row * h;
    (total <= 1e10).then_some(total as usize)
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// PDF number syntax, leniently: optional sign(s), digits, at most one dot.
fn parse_number(tok: &[u8]) -> Option<f64> {
    let mut i = 0;
    let mut neg = false;
    while i < tok.len() && (tok[i] == b'+' || tok[i] == b'-') {
        neg |= tok[i] == b'-';
        i += 1;
    }
    if i >= tok.len() {
        return None;
    }
    let (mut int, mut frac, mut scale) = (0f64, 0f64, 1f64);
    let mut seen_dot = false;
    let mut digits = 0;
    for &c in &tok[i..] {
        match c {
            b'0'..=b'9' => {
                digits += 1;
                if seen_dot {
                    if scale < 1e15 {
                        scale *= 10.0;
                        frac = frac * 10.0 + f64::from(c - b'0');
                    }
                } else {
                    int = int * 10.0 + f64::from(c - b'0');
                    if int > 1e18 {
                        int = 1e18;
                    }
                }
            }
            b'.' if !seen_dot => seen_dot = true,
            _ => return None,
        }
    }
    if digits == 0 {
        return None;
    }
    let v = int + frac / scale;
    Some(if neg { -v } else { v })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ops(data: &[u8]) -> Vec<(String, Vec<Operand>)> {
        let mut t = Tokenizer::new(data, 64);
        let mut operands = Vec::new();
        let mut out = Vec::new();
        while let Some(item) = t.next_item(&mut operands) {
            match item {
                Item::Op(o) => {
                    out.push((String::from_utf8_lossy(o).into_owned(), operands.clone()))
                }
                Item::InlineImage(d) => out.push(("BI".into(), vec![Operand::Dict(d)])),
            }
        }
        out
    }

    #[test]
    fn numbers_names_strings_and_arrays() {
        let r = ops(b"q 1 0 0 1 -10.5 +.5 cm /F1 12 Tf 3 Tr [(He) -250 <6c4C> (l\\157)] TJ Q");
        let names: Vec<&str> = r.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["q", "cm", "Tf", "Tr", "TJ", "Q"]);
        assert_eq!(
            r[1].1,
            [1.0, 0.0, 0.0, 1.0, -10.5, 0.5].map(Operand::Num).to_vec()
        );
        assert_eq!(r[2].1[0], Operand::Name(b"F1".to_vec()));
        let Operand::Array(items) = &r[4].1[0] else {
            panic!()
        };
        assert_eq!(items[0], Operand::Str(b"He".to_vec()));
        assert_eq!(items[1], Operand::Num(-250.0));
        assert_eq!(items[2], Operand::Str(b"lL".to_vec()));
        assert_eq!(items[3], Operand::Str(b"lo".to_vec()));
    }

    #[test]
    fn string_escapes_and_nesting() {
        let r = ops(b"(a(b)c\\)\\n\\\n d\\101) Tj");
        assert_eq!(r[0].1[0], Operand::Str(b"a(b)c)\n dA".to_vec()));
    }

    #[test]
    fn comments_stray_closers_and_quote_operators() {
        let r = ops(b"% comment\n) ] > (x) ' 1 2 (y) \" T* f*");
        let names: Vec<&str> = r.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["'", "\"", "T*", "f*"]);
    }

    #[test]
    fn inline_image_with_known_length_and_with_scan() {
        // 2x2 gray, 8 bpc: exactly 4 data bytes that happen to contain "EI".
        let mut data = b"q BI /W 2 /H 2 /BPC 8 /CS /G ID ".to_vec();
        data.extend_from_slice(b"EI\x00\x01");
        data.extend_from_slice(b"\nEI Q 1 2 m");
        let r = ops(&data);
        let names: Vec<&str> = r.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["q", "BI", "Q", "m"]);

        // Filtered (unknown length): scan for EI.
        let mut data = b"BI /W 4 /H 4 /BPC 8 /CS /G /F /Fl ID ".to_vec();
        data.extend_from_slice(&[0x78, 0x9c, 0xff, 0x00, 0x45, 0x49, 0x80]);
        data.extend_from_slice(b"\nEI\nQ");
        let names: Vec<String> = ops(&data).into_iter().map(|x| x.0).collect();
        assert_eq!(names, ["BI", "Q"]);
    }

    #[test]
    fn operand_stack_is_bounded() {
        let mut data = String::new();
        for i in 0..1000 {
            data.push_str(&format!("{i} "));
        }
        data.push('m');
        let mut t = Tokenizer::new(data.as_bytes(), 8);
        let mut operands = Vec::new();
        let item = t.next_item(&mut operands).unwrap();
        assert_eq!(item, Item::Op(b"m"));
        assert_eq!(operands.len(), 8);
        assert_eq!(operands[7], Operand::Num(999.0));
    }

    #[test]
    fn hostile_input_does_not_panic_or_hang() {
        let deep = "[".repeat(10_000) + &"]".repeat(10_000);
        let deep_dict = "<<".repeat(5_000);
        for input in [
            deep.as_bytes(),
            deep_dict.as_bytes(),
            b"BI /W",
            b"BI ID",
            b"BI /W 99999999 /H 99999999 /BPC 8 /CS /G ID abc",
            b"(unterminated",
            b"<4",
            b"/Na#",
            b"/Na#zz",
            b"-",
            b"--5 m",
            b"1.2.3 m",
            b"\x00\xff\xfe",
        ] {
            let _ = ops(input);
        }
    }
}
