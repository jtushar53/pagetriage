"""QR-like symbols for the finder-pattern detector. Not decodable: the three finder patterns, the
separators, the timing patterns and an alignment pattern are drawn as in ISO/IEC 18004, and the data
area is filled with random modules. That is all the detector looks at."""
from __future__ import annotations

import random

from PIL import Image, ImageDraw


def qr_like(rng: random.Random, modules: int, scale: int, quiet: int = 4) -> Image.Image:
    """Grayscale image, 0 = dark, with a quiet zone of `quiet` modules."""
    n = modules
    grid = [[rng.random() < 0.5 for _ in range(n)] for _ in range(n)]

    def finder(x0, y0):
        for dy in range(-1, 8):
            for dx in range(-1, 8):
                x, y = x0 + dx, y0 + dy
                if not (0 <= x < n and 0 <= y < n):
                    continue
                if dx in (-1, 7) or dy in (-1, 7):
                    grid[y][x] = False
                elif dx in (0, 6) or dy in (0, 6) or (2 <= dx <= 4 and 2 <= dy <= 4):
                    grid[y][x] = True
                else:
                    grid[y][x] = False

    finder(0, 0)
    finder(n - 7, 0)
    finder(0, n - 7)
    for i in range(8, n - 8):
        grid[6][i] = grid[i][6] = i % 2 == 0
    side = (n + 2 * quiet) * scale
    img = Image.new("L", (side, side), 255)
    d = ImageDraw.Draw(img)
    for y in range(n):
        for x in range(n):
            if grid[y][x]:
                px, py = (x + quiet) * scale, (y + quiet) * scale
                d.rectangle([px, py, px + scale - 1, py + scale - 1], fill=0)
    return img


def real_qr(rng: random.Random, version: int, level: str, scale: int, text: str, quiet: int = 4) -> Image.Image:
    """A decodable QR code made with the `qrcode` package (a development-time dependency), as a
    grayscale image, 0 = dark, with a quiet zone."""
    import qrcode

    levels = {"L": qrcode.constants.ERROR_CORRECT_L, "M": qrcode.constants.ERROR_CORRECT_M,
              "Q": qrcode.constants.ERROR_CORRECT_Q, "H": qrcode.constants.ERROR_CORRECT_H}
    qr = qrcode.QRCode(version=version, error_correction=levels[level], box_size=1, border=0,
                       mask_pattern=rng.randrange(8))
    qr.add_data(text)
    qr.make(fit=False)
    m = qr.get_matrix()
    n = len(m)
    side = (n + 2 * quiet) * scale
    img = Image.new("L", (side, side), 255)
    d = ImageDraw.Draw(img)
    for y, row in enumerate(m):
        for x, dark in enumerate(row):
            if dark:
                px, py = (x + quiet) * scale, (y + quiet) * scale
                d.rectangle([px, py, px + scale - 1, py + scale - 1], fill=0)
    return img
