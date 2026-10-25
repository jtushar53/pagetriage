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
