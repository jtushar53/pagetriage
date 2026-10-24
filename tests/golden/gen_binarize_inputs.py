#!/usr/bin/env python3
"""Synthetic pages for the binariser golden test (tests/golden/binarize_inputs/*.pgm).

Unlike the tiny report fixtures, these have what separates the binarisers: an illumination ramp,
soft-edged strokes, noise, and (in `ghosted`) a faint mirrored copy of the ink standing in for
bleed-through. Integer arithmetic only, so the files are identical on every machine. Synthetic: no
real document content. Run with `python3 -I tests/golden/gen_binarize_inputs.py`.
"""
import pathlib

OUT = pathlib.Path(__file__).resolve().parent / "binarize_inputs"
W, H = 160, 112


class Lcg:
    def __init__(self, seed):
        self.s = seed & 0xFFFFFFFF

    def next(self, n):
        self.s = (self.s * 1664525 + 1013904223) & 0xFFFFFFFF
        return (self.s >> 8) % n


def ink_layer(rng):
    """Coverage 0..255: lines of word-like strokes (bars and ticks)."""
    cov = [[0] * W for _ in range(H)]
    y = 10
    while y + 12 < H - 6:
        x = 8
        while x < W - 14:
            word = 3 + rng.next(5)
            for _ in range(word):
                if x + 7 >= W - 4:
                    break
                kind = rng.next(3)
                if kind == 0:  # vertical stem
                    for yy in range(y, y + 9):
                        for xx in (x, x + 1):
                            cov[yy][xx] = 255
                elif kind == 1:  # bar
                    for xx in range(x, x + 5):
                        for yy in (y + 3, y + 4):
                            cov[yy][xx] = 255
                else:  # short tick
                    for yy in range(y + 4, y + 9):
                        cov[yy][x + 1] = 255
                x += 6
            x += 5
        y += 16
    return cov


def blur(layer):
    """Two passes of the binomial [1 2 1] x [1 2 1] / 16, edges replicated."""
    for _ in range(2):
        out = [[0] * W for _ in range(H)]
        for y in range(H):
            for x in range(W):
                acc = 0
                for dy, wy in ((-1, 1), (0, 2), (1, 1)):
                    for dx, wx in ((-1, 1), (0, 2), (1, 1)):
                        yy = min(max(y + dy, 0), H - 1)
                        xx = min(max(x + dx, 0), W - 1)
                        acc += wy * wx * layer[yy][xx]
                out[y][x] = acc // 16
        layer = out
    return layer


def render(seed, ghost):
    rng = Lcg(seed)
    cov = blur(ink_layer(rng))
    noise = Lcg(seed ^ 0x5EED)
    pixels = bytearray()
    for y in range(H):
        for x in range(W):
            paper = 215 + x * 35 // W - y * 30 // H  # lighting ramp, 185..250
            value = paper - cov[y][x] * 130 // 255
            if ghost:  # faint mirrored strokes from the "other side"
                value -= cov[y][W - 1 - x] * 38 // 255
            value += noise.next(9) - 4
            pixels.append(min(max(value, 0), 255))
    return bytes(pixels)


def main():
    OUT.mkdir(exist_ok=True)
    for name, seed, ghost in (("scan_like", 7, False), ("ghosted", 11, True)):
        (OUT / f"{name}.pgm").write_bytes(f"P5\n{W} {H}\n255\n".encode() + render(seed, ghost))
        print(f"wrote {name}.pgm")


if __name__ == "__main__":
    main()
