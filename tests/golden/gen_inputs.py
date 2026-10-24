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


class Lcg:
    """32-bit linear congruential generator (Numerical Recipes constants)."""

    def __init__(self, seed):
        self.state = seed & 0xFFFFFFFF

    def next(self, bound):
        self.state = (self.state * 1664525 + 1013904223) & 0xFFFFFFFF
        return (self.state >> 16) % bound


def write_pgm(name, pixels, w=W, h=H):
    assert len(pixels) == w * h, name
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / f"{name}.pgm").write_bytes(f"P5\n{w} {h}\n255\n".encode() + bytes(pixels))


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


if __name__ == "__main__":
    for fn in (blank_speckle, text_like, borderline, faint_strokes, dark_fill):
        write_pgm(fn.__name__, fn())
        print("wrote", fn.__name__)
    for fn in (text_like_rescan, text_like_shift1, text_like_shifted, text_like_other, flat_blank):
        write_pgm(fn.__name__, fn())
        print("wrote", fn.__name__)
    write_pgm("text_like_cropped", text_like_cropped(), CROP_W, CROP_H)
    print("wrote text_like_cropped")
