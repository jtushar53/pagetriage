#!/usr/bin/env python3
"""Regenerate tests/golden/inputs/*.pgm.

Synthetic pages only (never real documents: they contain PII). Pure standard library and integer
arithmetic, so the bytes are identical on every machine. After changing a fixture, regenerate the
expected reports with `UPDATE_GOLDEN=1 cargo test -p pagetriage-core --test golden` and review the
diff: a changed report is a behaviour change.
"""
import pathlib

W, H = 64, 80
OUT = pathlib.Path(__file__).resolve().parent / "inputs"
# Module M3 fixtures are bigger than the 64 x 80 pages above, so they live apart: the binariser
# goldens (which run the slow Howe method on every page in inputs/) do not take them.
QUALITY_OUT = pathlib.Path(__file__).resolve().parent / "quality_inputs"
# Layout segmentation fixtures (the opt-in `layout` detector): pages with known columns, rulings and
# graphics. Checked by tests/layout_golden.rs and the Python and WASM runners.
LAYOUT_OUT = pathlib.Path(__file__).resolve().parent / "layout_inputs"


class Lcg:
    """32-bit linear congruential generator (Numerical Recipes constants)."""

    def __init__(self, seed):
        self.state = seed & 0xFFFFFFFF

    def next(self, bound):
        self.state = (self.state * 1664525 + 1013904223) & 0xFFFFFFFF
        return (self.state >> 16) % bound


def write_pgm(name, pixels, w=W, h=H, out=None):
    assert len(pixels) == w * h, name
    out = out or OUT
    out.mkdir(parents=True, exist_ok=True)
    (out / f"{name}.pgm").write_bytes(f"P5\n{w} {h}\n255\n".encode() + bytes(pixels))


def blank_speckle():
    """Paper around 245 with sensor noise and 5 dark specks (0.098% ink): blank."""
    rng = Lcg(1)
    px = [245 + rng.next(7) - 3 for _ in range(W * H)]
    for _ in range(5):
        px[rng.next(W * H)] = 30
    return px


def text_like():
    """Lines of dark word blobs on paper: about a fifth of the page is ink."""
    rng = Lcg(2)
    px = [240 + rng.next(3) for _ in range(W * H)]
    for line_top in range(8, 72, 8):
        x = 6
        while x < 58:
            word = 3 + rng.next(9)
            for row in range(line_top + 2, line_top + 5):
                for dx in range(word):
                    if x + dx < 58:
                        px[row * W + x + dx] = 25 + rng.next(20)
            x += word + 2 + rng.next(2)
    return px


def borderline():
    """Flat paper with exactly 10 ink pixels (0.195%), just under the 0.2% blank threshold."""
    rng = Lcg(3)
    px = [250] * (W * H)
    placed = set()
    while len(placed) < 10:
        placed.add(rng.next(W * H))
    for i in sorted(placed):
        px[i] = 0
    return px


def faint_strokes():
    """Strokes only 45 gray levels below paper: inside ink_delta, so not counted as ink."""
    rng = Lcg(4)
    px = [200 + rng.next(7) for _ in range(W * H)]
    for line_top in range(8, 72, 8):
        for row in range(line_top + 2, line_top + 5):
            for x in range(6, 58):
                px[row * W + x] = 155 + rng.next(3)
    return px


def dark_fill():
    """Solid dark region over the top 62% of the page, clean paper below: the dark fill becomes
    the 'paper level', so the page reads as blank. A known failure, pinned here."""
    rng = Lcg(5)
    return [(25 + rng.next(3)) if (i // W) < 50 else (245 + rng.next(3)) for i in range(W * H)]


def _text_page(seed):
    """`text_like`'s layout with other words: same line positions, different word lengths."""
    rng = Lcg(seed)
    px = [240 + rng.next(3) for _ in range(W * H)]
    for line_top in range(8, 72, 8):
        x = 6
        while x < 58:
            word = 3 + rng.next(9)
            for row in range(line_top + 2, line_top + 5):
                for dx in range(word):
                    if x + dx < 58:
                        px[row * W + x + dx] = 25 + rng.next(20)
            x += word + 2 + rng.next(2)
    return px


def _shift(px, dx, dy, fill):
    """Move the page content right by dx and down by dy, filling the exposed edge with `fill`."""
    out = [fill] * (W * H)
    for y in range(H):
        for x in range(W):
            sx, sy = x - dx, y - dy
            if 0 <= sx < W and 0 <= sy < H:
                out[y * W + x] = px[sy * W + sx]
    return out


def text_like_rescan():
    """`text_like` rescanned at the same registration: 5% less gain and sensor noise. A
    near-duplicate of it."""
    rng = Lcg(11)
    return [min(255, max(0, v * 243 // 256 + rng.next(5) - 2)) for v in text_like()]


def text_like_shift1():
    """`text_like` moved 1 px right. On this 64 px wide page that is 1.6% of the width, which is
    already enough to flip several hash bits; on a real 1500 px page 1 px is 0.07%."""
    return _shift(text_like(), 1, 0, 240)


def text_like_shifted():
    """`text_like` moved 10 px right and 8 px down (16% and 10% of the page). pHash is not shift
    invariant, so this is a known miss: the same page, but far from it in Hamming distance."""
    return _shift(text_like(), 10, 8, 240)


CROP_W, CROP_H = 61, 76  # 90.5% of the page area


def text_like_cropped():
    """`text_like` cropped to its central 90% by area (61 x 76, offset 1,2), as a rescan with
    different margins would be. The hash resizes any size to 32 x 32, so the crop is stretched."""
    px = text_like()
    return [px[(y + 2) * W + x + 1] for y in range(CROP_H) for x in range(CROP_W)]


def text_like_other():
    """A different page with the same layout (same lines, other words): a typed form or the next
    deed page. A known false positive: low-frequency hashes see the layout, not the words."""
    return _text_page(7)


def flat_blank():
    """Paper with sensor noise only: no structure, so both hashes are reported as all zeros."""
    rng = Lcg(12)
    return [245 + rng.next(5) - 2 for _ in range(W * H)]


# ---- quality fixtures (module M3) -------------------------------------------------------------
# Pages with a known amount of one degradation each. Text is built from 2-px stems in 6-px cells
# (8 px tall), so every page is "text-like" at the scale the binariser and the quality measures
# expect. Integer arithmetic only: skew is a floor-division shear, blur an integer box filter.

QW, QH = 200, 256  # a small page crop
QFW = 460  # the fold page is wider: a fold must be at least 3 inches (450 px at 150 dpi) long


def _quality_ink(seed, w, h, headline):
    """Ink pixels of 14 lines of text-like words. With `headline`, every word gets a 2-px bar
    along its top, as Devanagari, Bengali and Gurmukhi do (the shirorekha)."""
    rng = Lcg(seed)
    ink = set()
    y = 22
    while y + 10 < h - 16:
        x = 14
        while x < w - 20:
            glyphs = 2 + rng.next(6)
            start = x
            for g in range(glyphs):
                gx = x + g * 6
                kind = rng.next(4)
                for dy in range(8):
                    ink.update({(gx, y + dy), (gx + 1, y + dy)})
                if kind == 1:
                    for dy in range(8):
                        ink.update({(gx + 3, y + dy), (gx + 4, y + dy)})
                elif kind == 2:
                    for dx in range(5):
                        ink.update({(gx + dx, y + 3), (gx + dx, y + 4)})
                elif kind == 3:
                    for dx in range(5):
                        ink.update({(gx + dx, y + 6), (gx + dx, y + 7)})
            end = x + glyphs * 6
            if headline:
                for dx in range(start, end):
                    ink.update({(dx, y), (dx, y + 1)})
            x = end + 6 + rng.next(4)
        y += 17
    return ink


def _quality_page(seed, w, h, headline=False, shear=None, ghost=None, paper_seed=21):
    """Paper (235 + noise) with the ink of `_quality_ink`, optionally sheared by `shear(x)` rows and
    with a faint mirrored `ghost` layer (a set of ink coordinates) underneath."""
    rng = Lcg(paper_seed)
    px = [235 + rng.next(5) for _ in range(w * h)]
    if ghost:
        for x, y in sorted(ghost):
            mx = w - 1 - x
            yy = y + 8  # offset from the front lines, as a real verso's lines are
            if 0 <= mx < w and 0 <= yy < h:
                px[yy * w + mx] = 185 + rng.next(5)
    for x, y in sorted(_quality_ink(seed, w, h, headline)):
        yy = y + (shear(x) if shear else 0)
        if 0 <= x < w and 0 <= yy < h:
            px[yy * w + x] = 30 + rng.next(20)
    return px


def _box_blur(px, w, h):
    """3x3 mean with edge replication, integer (rounded to nearest)."""
    out = []
    for y in range(h):
        for x in range(w):
            t = 0
            for dy in (-1, 0, 1):
                for dx in (-1, 0, 1):
                    t += px[min(h - 1, max(0, y + dy)) * w + min(w - 1, max(0, x + dx))]
            out.append((t + 4) // 9)
    return out


def quality_clean():
    """Upright, clean text-like page: skew 0, no blur, no specks, no folds, grade A."""
    return _quality_page(31, QW, QH)


def quality_skew_p3():
    """The same layout rotated counter-clockwise by atan(1/19) = 3.01 degrees (text rises to the
    right): skew_deg should read +3.0."""
    return _quality_page(31, QW, QH, shear=lambda x: ((QW // 2 - x) * 1) // 19)


def quality_deva_skew_m2():
    """Words with a headline bar, rotated clockwise by atan(1/29) = 1.98 degrees (text descends to
    the right): skew_deg should read -2.0. The headline bars are the sharpest feature."""
    return _quality_page(32, QW, QH, headline=True, shear=lambda x: -((QW // 2 - x) * 1) // 29)


def quality_blurred():
    """`quality_clean` through two 3x3 mean filters (about 1.1 px of Gaussian blur): the edge
    sharpness drops and the blur index rises well above the clean page's."""
    px = _quality_page(31, QW, QH)
    return _box_blur(_box_blur(px, QW, QH), QW, QH)


def quality_speckled():
    """`quality_clean` plus 100 isolated dark specks (1 or 2 px wide) at LCG positions: a speckle
    density of 100 / (200 * 256) = 0.00195."""
    px = _quality_page(31, QW, QH)
    rng = Lcg(41)
    for i in range(100):
        x, y = rng.next(QW - 2), rng.next(QH - 2)
        px[y * QW + x] = 60
        if i % 2:
            px[y * QW + x + 1] = 60
    return px


def quality_bleed():
    """`quality_clean` with a faint mirrored second set of lines underneath (gray 185 on paper 237:
    about a quarter of the ink's contrast), as show-through from the other side looks."""
    return _quality_page(31, QW, QH, ghost=_quality_ink(33, QW, QH, False))


def quality_fold():
    """A wide page with one horizontal crease across the middle, in the gap between two lines of
    text: a 2-px valley 22 levels deep with 6 levels of shade on the 20 rows below it. fold_lines
    should read 1. (A crease that crosses a line of text is harder: see the unit tests.)"""
    px = _quality_page(31, QFW, QH)
    k = QH // 2 + 8  # in the gap between two lines of text (rows 132-138)
    for y in range(QH):
        drop = 22 if y in (k, k + 1) else 6 if k + 1 < y < k + 22 else 0
        if drop:
            for x in range(QFW):
                px[y * QFW + x] -= drop
    return px


LW, LH = 320, 260


def _ink_lines(rng, ink, x0, x1, y, count):
    """`count` lines of text-like words between x0 and x1, 17 px apart from row y (same glyph
    shapes as `_quality_ink`)."""
    for _ in range(count):
        x = x0
        while x < x1 - 40:
            glyphs = 2 + rng.next(5)
            for g in range(glyphs):
                gx = x + g * 6
                kind = rng.next(4)
                for dy in range(8):
                    ink.update({(gx, y + dy), (gx + 1, y + dy)})
                if kind == 1:
                    for dy in range(8):
                        ink.update({(gx + 3, y + dy), (gx + 4, y + dy)})
                elif kind == 2:
                    for dx in range(5):
                        ink.update({(gx + dx, y + 3), (gx + dx, y + 4)})
                elif kind == 3:
                    for dx in range(5):
                        ink.update({(gx + dx, y + 6), (gx + dx, y + 7)})
            x += glyphs * 6 + 6 + rng.next(4)
        y += 17


def _layout_page(ink):
    rng = Lcg(51)
    px = [235 + rng.next(5) for _ in range(LW * LH)]
    for x, y in sorted(ink):
        px[y * LW + x] = 30 + rng.next(20)
    return px


def layout_one_column():
    """One paragraph of 10 lines and, below a gap, a second paragraph of 3."""
    rng = Lcg(52)
    ink = set()
    _ink_lines(rng, ink, 20, 300, 16, 10)
    _ink_lines(rng, ink, 20, 300, 16 + 10 * 17 + 24, 3)
    return _layout_page(ink)


def layout_two_columns():
    """A heading line over two columns of 9 lines with a 40-px gutter."""
    rng = Lcg(53)
    ink = set()
    _ink_lines(rng, ink, 20, 300, 12, 1)
    _ink_lines(rng, ink, 14, 130, 50, 9)
    _ink_lines(rng, ink, 186, 306, 50, 9)
    return _layout_page(ink)


def layout_form():
    """Text, a ruled box (table frame with a rule through it) and a filled rectangle (a stamp)."""
    rng = Lcg(54)
    ink = set()
    _ink_lines(rng, ink, 20, 300, 12, 3)
    for x in range(20, 300):
        ink.update({(x, 80), (x, 81), (x, 150), (x, 151), (x, 118)})
    for y in range(80, 152):
        ink.update({(20, y), (21, y), (298, y), (299, y)})
    _ink_lines(rng, ink, 30, 150, 90, 3)
    for y in range(180, 236):
        for x in range(220, 290):
            ink.add((x, y))
    _ink_lines(rng, ink, 20, 200, 180, 3)
    for x in range(20, 200):
        ink.update({(x, 60), (x, 61)})  # two free-standing rules
        ink.update({(x, 244), (x, 245)})
    return _layout_page(ink)


def _cursive_ink(seed, w, h, y0=22, y1=None):
    """Lines of joined pseudo-letters: words are 2-px baselines under 2-4 letters of one kind each
    (stem, arch, bowl, teeth), with 2x2 dots above some letters, as the Arabic script has."""
    rng = Lcg(seed)
    ink = set()
    y = y0
    y1 = y1 if y1 is not None else h - 16
    while y + 12 < y1:
        x = 14
        while x < w - 20:
            letters = 2 + rng.next(4)
            for g in range(letters):
                gx = x + g * 6
                for dx in range(7):
                    ink.update({(gx + dx, y + 6), (gx + dx, y + 7)})
                kind = rng.next(4)
                if kind == 0:
                    for dy in range(7):
                        ink.update({(gx + 2, y + dy), (gx + 3, y + dy)})
                elif kind == 1:
                    for dx in range(1, 6):
                        ink.update({(gx + dx, y + 3 + (dx in (1, 5)) * 2), (gx + dx, y + 4 + (dx in (1, 5)) * 2)})
                elif kind == 2:
                    for dx in range(1, 6):
                        ink.update({(gx + dx, y + 10 - (dx in (1, 5)) * 2), (gx + dx, y + 9 - (dx in (1, 5)) * 2)})
                else:
                    for tx in (0, 3, 6):
                        for dy in range(4, 6):
                            ink.update({(gx + tx, y + dy)})
                if rng.next(5) < 3:
                    ink.update({(gx + 2, y - 3), (gx + 3, y - 3), (gx + 2, y - 2), (gx + 3, y - 2)})
            x += letters * 6 + 8 + rng.next(4)
        y += 20
    return ink


def script_cursive():
    """Joined pseudo-letters with dots on a baseline: the cursive family."""
    rng = Lcg(61)
    px = [235 + rng.next(5) for _ in range(QW * QH)]
    for x, y in sorted(_cursive_ink(62, QW, QH)):
        px[y * QW + x] = 30 + rng.next(20)
    return px


def script_mixed():
    """Headline words in the top half, separate letters in the bottom half."""
    rng = Lcg(63)
    px = [235 + rng.next(5) for _ in range(QW * QH)]
    top = _quality_ink(64, QW, QH // 2 + 12, True)
    bottom = _quality_ink(65, QW, QH // 2 + 12, False)
    for x, y in sorted(top):
        if y < QH // 2 - 6:
            px[y * QW + x] = 30 + rng.next(20)
    for x, y in sorted(bottom):
        if y >= 22 and y + QH // 2 - 6 < QH - 8:
            px[(y + QH // 2 - 6) * QW + x] = 30 + rng.next(20)
    return px


def _line(ink, x0, y0, x1, y1, width):
    """Integer Bresenham line, `width` px thick (a square pen)."""
    dx, dy = abs(x1 - x0), -abs(y1 - y0)
    sx, sy = (1 if x0 < x1 else -1), (1 if y0 < y1 else -1)
    err = dx + dy
    while True:
        for ox in range(width):
            for oy in range(width):
                ink.add((x0 + ox, y0 + oy))
        if x0 == x1 and y0 == y1:
            return
        e2 = 2 * err
        if e2 >= dy:
            err += dy
            x0 += sx
        if e2 <= dx:
            err += dx
            y0 += sy


def _scribble_word(rng, ink, x, base, letters):
    """A word of looping pen strokes: each letter is 6 random points within a 7 x 10 cell, joined by
    lines of 1-2 px, the baseline wandering by up to 2 px. Integer arithmetic only."""
    wander = 0
    prev = None
    for _ in range(letters):
        wander = max(-2, min(2, wander + rng.next(3) - 1))
        pts = [(x + rng.next(8), base + wander - rng.next(11)) for _ in range(6)]
        pts.sort(key=lambda p: p[0])
        if prev is not None:
            pts.insert(0, prev)
        for a, b in zip(pts, pts[1:]):
            _line(ink, a[0], a[1], b[0], b[1], 1 + rng.next(2))
        prev = pts[-1]
        x += 6 + rng.next(3)
    return x


def layout_hand_form():
    """Three printed text lines, then a form row: a printed label with a rule and a scribbled
    fill-in on it, twice, and a line of scribble on its own."""
    rng = Lcg(71)
    ink = set()
    _ink_lines(rng, ink, 20, 300, 12, 3)
    for row, base in enumerate((90, 130)):
        _ink_lines(rng, ink, 20, 90, base - 8, 1)
        for x in range(100, 300):
            ink.add((x, base + 4))
        x = 104
        for _ in range(2):
            x = _scribble_word(rng, ink, x, base, 5) + 8
    x = 20
    for _ in range(4):
        x = _scribble_word(rng, ink, x, 190, 5) + 10
    _ink_lines(rng, ink, 20, 300, 215, 2)
    return _layout_page(ink)


RW, RH = 320, 500


def redaction_page():
    """Nine lines of text-like words with a word-sized black box over one line and a block of two
    lines blacked out. Ink next to a box is cleared first (a real redaction removes what is under it
    and the text beside it is not pressed against it)."""
    rng = Lcg(121)
    ink = set()
    _ink_lines(rng, ink, 14, 306, 12, 11)
    px = [235 + rng.next(5) for _ in range(LW * LH)]
    boxes = [(100, 41, 150, 58), (30, 112, 240, 144)]  # a word; two lines
    for x0, y0, x1, y1 in boxes:
        ink = {(x, y) for x, y in ink if not (x0 - 3 <= x <= x1 + 3 and y0 - 3 <= y <= y1 + 3)}
    for x, y in sorted(ink):
        if 0 <= x < LW and 0 <= y < LH:
            px[y * LW + x] = 30 + rng.next(20)
    for x0, y0, x1, y1 in boxes:
        for y in range(y0, y1 + 1):
            for x in range(x0, x1 + 1):
                px[y * LW + x] = rng.next(8)
    return px


def qr_page():
    """Six lines of text-like words and, bottom right, a QR-like symbol: 25 modules of 3 px with
    the three finder patterns, separators and timing patterns, and pseudo-random data modules (not
    decodable). A grid of small boxes on the left must not be taken for one."""
    rng = Lcg(131)
    px = [235 + rng.next(5) for _ in range(LW * LH)]
    ink = set()
    _ink_lines(rng, ink, 14, 300, 6, 7)
    n, sc, ox, oy = 25, 3, 220, 150
    grid = [[rng.next(2) == 1 for _ in range(n)] for _ in range(n)]
    for fx, fy in ((0, 0), (n - 7, 0), (0, n - 7)):
        for dy in range(-1, 8):
            for dx in range(-1, 8):
                x, y = fx + dx, fy + dy
                if 0 <= x < n and 0 <= y < n:
                    ring = dx in (0, 6) or dy in (0, 6) or (2 <= dx <= 4 and 2 <= dy <= 4)
                    grid[y][x] = ring and -1 < dx < 7 and -1 < dy < 7
    for i in range(8, n - 8):
        grid[6][i] = grid[i][6] = i % 2 == 0
    for y in range(n):
        for x in range(n):
            if grid[y][x]:
                for yy in range(sc):
                    for xx in range(sc):
                        ink.add((ox + x * sc + xx, oy + y * sc + yy))
    for r in range(4):  # checkbox grid
        for c in range(5):
            x0, y0 = 20 + c * 24, 160 + r * 24
            for k in range(14):
                for a, b in ((x0 + k, y0), (x0 + k, y0 + 13), (x0, y0 + k), (x0 + 13, y0 + k)):
                    ink.add((a, b))
    for x, y in sorted(ink):
        if 0 <= x < LW and 0 <= y < LH:
            px[y * LW + x] = 30 + rng.next(20)
    return px


def photo_page():
    """Six lines of text-like words and a picture: a 1.1 x 1.0 in (160 x 90 px at 150 dpi) area of smooth
    mid-gray with a gentle gradient and grain. Integer arithmetic only."""
    rng = Lcg(151)
    px = [235 + rng.next(5) for _ in range(LW * LH)]
    ink = set()
    _ink_lines(rng, ink, 14, 300, 6, 7)
    for x, y in sorted(ink):
        if 0 <= x < LW and 0 <= y < LH:
            px[y * LW + x] = 30 + rng.next(20)
    for y in range(140, 236):
        for x in range(60, 236):
            px[y * LW + x] = 80 + (x - 60) // 2 + (y - 140) // 2 + rng.next(6)
    return px


def emboss_page():
    """A blank-looking page with a blind-embossed seal: a rim 12 px wide and an inner ring 6 px wide,
    lit from the left, as gray-level shading of 8 levels around the paper (no ink). The shading of a
    raised ring is its slope times the cosine of the angle to the light, so rising and falling halves
    of each ring, and the left and right sides of the seal, have opposite signs. Integer arithmetic
    and math.isqrt only."""
    import math
    rng = Lcg(141)
    px = [240 + rng.next(3) - 1 for _ in range(LW * LH)]
    cx, cy, radius = 196, 130, 72
    for y in range(LH):
        for x in range(LW):
            dx, dy = x - cx, y - cy
            d = math.isqrt(dx * dx + dy * dy)
            if d == 0 or d > radius:
                continue
            slope = 0
            if radius - 12 <= d < radius - 6:
                slope = 1
            elif radius - 6 <= d <= radius:
                slope = -1
            elif radius - 40 <= d < radius - 37:
                slope = 1
            elif radius - 37 <= d < radius - 34:
                slope = -1
            if slope:
                px[y * LW + x] += 8 * slope * dx // d
    return px


DOC_OUT = pathlib.Path(__file__).resolve().parent / "document_inputs"


def document_pages():
    """Page reports of a five-page synthetic deed for the document-level pass: a stamp in the same
    corner of pages 0, 1, 2 and 4 (an endorsement), a seal on page 2 alone, a signature in the same
    place on pages 1 and 2 only, and the same hashes on pages 0 and 4 (near-duplicates)."""
    import json

    def finding(kind, bbox, confidence=0.9):
        return {"kind": kind, "confidence": confidence, "bbox": bbox, "polygon": None, "attributes": {}, "evidence": []}

    hashes = ["f0f0f0f0f0f0f0f0", "0123456789abcdef", "fedcba9876543210", "aaaaaaaa55555555", "f0f0f0f0f0f0f0f0"]
    pages = []
    for i in range(5):
        findings = []
        if i != 3:
            findings.append(finding("stamp", [1020 + i, 1540, 1180 + i, 1680]))
        if i == 2:
            findings.append(finding("seal", [200, 300, 360, 460], 0.8))
        if i in (1, 2):
            findings.append(finding("signature", [400, 1500, 640, 1600], 0.7))
        pages.append({
            "index": i, "size_pt": [595.0, 842.0], "analysis_dpi": 150, "timings_ms": None,
            "text_layer": None, "quality": None, "findings": findings, "page_type": None,
            "phash": hashes[i], "dhash": hashes[i], "near_duplicate_of": None, "debug": None,
        })
    return json.dumps(pages, sort_keys=True, separators=(",", ":")) + "\n"


def thumb_page():
    """Text lines, a thumb-like impression (concentric elliptical ridges, 4 px period, on the right)
    and, on the left, hatching with the same period that must not be taken for one. Integer
    arithmetic and math.isqrt only, so the bytes are the same everywhere."""
    import math
    rng = Lcg(111)
    px = [235 + rng.next(5) for _ in range(LW * LH)]
    ink = set()
    _ink_lines(rng, ink, 14, 300, 8, 3)
    for y in range(120, 190):  # hatching block
        if (y // 2) % 2 == 0:
            for x in range(20, 100):
                ink.add((x, y))
    cx, cy, a, b = 220, 160, 60, 75
    for y in range(cy - b, cy + b + 1):
        for x in range(cx - a, cx + a + 1):
            dx, dy = x - cx, y - cy
            if 1000 * 1000 * dx * dx // (a * a) + 1000 * 1000 * dy * dy // (b * b) <= 1000 * 1000:
                r = math.isqrt(9 * dx * dx + 4 * dy * dy) // 3
                if r % 4 < 2:
                    ink.add((x, y))
    for x, y in sorted(ink):
        if 0 <= x < LW and 0 <= y < LH:
            px[y * LW + x] = 45 + rng.next(15)
    return px


def signature_page():
    """Three lines of text, a flourish of six loops with an underswing (a signature), and a black
    double-ringed stamp that must not be taken for one."""
    rng = Lcg(101)
    ink = set()
    _ink_lines(rng, ink, 20, 300, 12, 3)
    loop = [(22, 0), (11, 19), (-11, 19), (-22, 0), (-11, -19), (11, -19)]
    prev = (30, 125)
    for i in range(6):
        cx = 50 + i * 28
        for dx, dy in loop + [loop[0]]:
            p = (cx + dx + rng.next(7) - 3, 120 + dy + rng.next(7) - 3)
            _line(ink, prev[0], prev[1], p[0], p[1], 2)
            prev = p
    _line(ink, prev[0], prev[1], 60, 160, 2)
    _line(ink, 60, 160, 230, 150, 2)
    for r2 in (46 * 46, 33 * 33):
        for y in range(205 - 50, 205 + 51):
            for x in range(250 - 50, 250 + 51):
                d = (x - 250) ** 2 + (y - 205) ** 2
                if abs(d - r2) <= (3 * 46 if r2 > 1500 else 3 * 33):
                    ink.add((x, y))
    for k in range(5):
        for dy in range(10):
            ink.update({(236 + k * 8 + dx, 200 + dy) for dx in range(3)})
    return _layout_page(ink)


# Colour fixtures for the stamp detector (module M6) are PPM (P6) files in stamp_inputs/.
STAMP_OUT = pathlib.Path(__file__).resolve().parent / "stamp_inputs"
SW, SH = 360, 300


def write_ppm(name, rgb, w, h, out):
    assert len(rgb) == w * h, name
    out.mkdir(parents=True, exist_ok=True)
    data = bytearray()
    for r, g, b in rgb:
        data += bytes((r, g, b))
    (out / f"{name}.ppm").write_bytes(f"P6\n{w} {h}\n255\n".encode() + bytes(data))


def _colour_page(seed):
    """Cream paper with a little noise, and an `ink` set of dark text-like pixels drawn black."""
    rng = Lcg(seed)
    return [(250 - rng.next(4), 247 - rng.next(4), 238 - rng.next(4)) for _ in range(SW * SH)]


def _paint(px, pixels, colour, rng):
    for x, y in sorted(pixels):
        if 0 <= x < SW and 0 <= y < SH:
            r, g, b = colour
            px[y * SW + x] = (r + rng.next(9) - 4, g + rng.next(9) - 4, b + rng.next(9) - 4)


def _ring_pixels(cx, cy, r, t):
    lo, hi = (2 * r - t) ** 2, (2 * r + t) ** 2  # squared, in half pixels
    return {(x, y) for y in range(cy - r - t, cy + r + t + 1) for x in range(cx - r - t, cx + r + t + 1)
            if lo <= 4 * ((x - cx) ** 2 + (y - cy) ** 2) <= hi}


def _words_pixels(cx, cy):
    out = set()
    for row, n in ((cy - 12, 5), (cy + 14, 6)):
        for k in range(n):
            for dx in range(7):
                for dy in range(10):
                    out.add((cx - 30 + k * 11 + dx, row + dy))
    return out


def stamp_circle_over_text():
    """Lines of black text with a blue ring stamp and two lines of words pressed over them."""
    rng = Lcg(91)
    px = _colour_page(90)
    ink = set()
    _ink_lines(rng, ink, 14, 340, 14, 14)
    _paint(px, ink, (35, 35, 40), rng)
    stamp = _ring_pixels(200, 160, 70, 4) | _words_pixels(200, 160)
    _paint(px, stamp, (25, 55, 170), rng)
    return px


def stamp_violet_seal_and_red_rectangle():
    """A violet double-ringed seal on the left, a red rectangle stamp (tilted by a 5:1 slope) on
    the right, nothing else."""
    rng = Lcg(92)
    px = _colour_page(93)
    seal = _ring_pixels(95, 150, 75, 4) | _ring_pixels(95, 150, 52, 3) | _words_pixels(95, 150)
    _paint(px, seal, (110, 40, 150), rng)
    # Rectangle with sides u = (120, -24) and v = (12, 60), which are perpendicular.
    frame = set()
    c = (200, 140)
    corners = [c, (c[0] + 120, c[1] - 24), (c[0] + 132, c[1] + 36), (c[0] + 12, c[1] + 60)]
    for a, b in zip(corners, corners[1:] + corners[:1]):
        _line(frame, a[0], a[1], b[0], b[1], 3)
    _paint(px, frame | {(x + 120, y + 6) for x, y in _words_pixels(130, 150)}, (185, 30, 40), rng)
    return px


def stamp_confusers():
    """What is not a stamp: a bare red frame, a solid green block, a black ring with words, and a
    block of two hues."""
    rng = Lcg(94)
    px = _colour_page(95)
    frame = set()
    for a, b in (((20, 20), (150, 20)), ((150, 20), (150, 100)), ((150, 100), (20, 100)), ((20, 100), (20, 20))):
        _line(frame, a[0], a[1], b[0], b[1], 3)
    _paint(px, frame, (185, 30, 40), rng)
    _paint(px, {(x, y) for x in range(200, 330) for y in range(20, 110)}, (30, 140, 60), rng)
    _paint(px, _ring_pixels(90, 210, 60, 4) | _words_pixels(90, 210), (30, 30, 30), rng)
    _paint(px, {(x, y) for x in range(200, 262) for y in range(170, 260)}, (200, 40, 40), rng)
    _paint(px, {(x, y) for x in range(266, 330) for y in range(170, 260)}, (40, 70, 190), rng)
    return px


def rulings_table():
    """A 4 x 3 ruled table with a word in each top-row cell, a row of five checkboxes (two ticked),
    a free-standing underline, and a faint vertical crease down the right margin."""
    rng = Lcg(81)
    ink = set()
    for y in (24, 64, 104, 144, 184):
        for x in range(20, 262):
            ink.update({(x, y), (x, y + 1)})
    for x in (20, 100, 180, 260):
        for y in range(24, 186):
            ink.update({(x, y), (x + 1, y)})
    for col in range(3):
        _ink_lines(rng, ink, 20 + col * 80 + 8, 20 + col * 80 + 78, 36, 1)
    for i in range(5):
        x, y = 30 + 40 * i, 220  # 18-px boxes: M9 ignores anything under 0.11 in (16.5 px at 150 dpi)
        for k in range(18):
            ink.update({(x + k, y), (x + k, y + 1), (x + k, y + 16), (x + k, y + 17)})
            ink.update({(x, y + k), (x + 1, y + k), (x + 16, y + k), (x + 17, y + k)})
        if i in (1, 3):
            for k in range(12):
                ink.update({(x + 3 + k, y + 3 + k), (x + 4 + k, y + 3 + k)})
                ink.update({(x + 14 - k, y + 3 + k), (x + 15 - k, y + 3 + k)})
    _ink_lines(rng, ink, 20, 300, 250, 2)
    for x in range(20, 201):
        ink.update({(x, 320), (x, 321)})
    px = [235 + rng.next(5) for _ in range(RW * RH)]
    for y in range(5, 496):
        for x in (290, 291, 292):
            px[y * RW + x] = 175 + rng.next(3)
    for x, y in sorted(ink):
        px[y * RW + x] = 30 + rng.next(20)
    return px


if __name__ == "__main__":
    for fn in (blank_speckle, text_like, borderline, faint_strokes, dark_fill):
        write_pgm(fn.__name__, fn())
        print("wrote", fn.__name__)
    for fn in (text_like_rescan, text_like_shift1, text_like_shifted, text_like_other, flat_blank):
        write_pgm(fn.__name__, fn())
        print("wrote", fn.__name__)
    write_pgm("text_like_cropped", text_like_cropped(), CROP_W, CROP_H)
    print("wrote text_like_cropped")
    for fn in (quality_clean, quality_skew_p3, quality_deva_skew_m2, quality_blurred,
               quality_speckled, quality_bleed):
        write_pgm(fn.__name__, fn(), QW, QH, QUALITY_OUT)
        print("wrote", fn.__name__)
    write_pgm("quality_fold", quality_fold(), QFW, QH, QUALITY_OUT)
    print("wrote quality_fold")
    for fn in (layout_one_column, layout_two_columns, layout_form):
        write_pgm(fn.__name__, fn(), LW, LH, LAYOUT_OUT)
        print("wrote", fn.__name__)
    for fn in (script_cursive, script_mixed):
        write_pgm(fn.__name__, fn(), QW, QH, LAYOUT_OUT)
        print("wrote", fn.__name__)
    write_pgm("layout_hand_form", layout_hand_form(), LW, LH, LAYOUT_OUT)
    print("wrote layout_hand_form")
    write_pgm("rulings_table", rulings_table(), RW, RH, LAYOUT_OUT)
    print("wrote rulings_table")
    for fn in (stamp_circle_over_text, stamp_violet_seal_and_red_rectangle, stamp_confusers):
        write_ppm(fn.__name__, fn(), SW, SH, STAMP_OUT)
        print("wrote", fn.__name__)
    write_pgm("signature_page", signature_page(), LW, LH, LAYOUT_OUT)
    print("wrote signature_page")
    write_pgm("thumb_page", thumb_page(), LW, LH, LAYOUT_OUT)
    print("wrote thumb_page")
    write_pgm("redaction_page", redaction_page(), LW, LH, LAYOUT_OUT)
    print("wrote redaction_page")
    write_pgm("qr_page", qr_page(), LW, LH, LAYOUT_OUT)
    print("wrote qr_page")
    write_pgm("emboss_page", emboss_page(), LW, LH, LAYOUT_OUT)
    print("wrote emboss_page")
    write_pgm("photo_page", photo_page(), LW, LH, LAYOUT_OUT)
    print("wrote photo_page")
    DOC_OUT.mkdir(exist_ok=True)
    (DOC_OUT / "deed_pages.json").write_text(document_pages(), encoding="utf-8")
    print("wrote deed_pages")
