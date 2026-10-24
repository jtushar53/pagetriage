"""Minimal PDF object writer for the synthetic corpus: full files and incremental updates.

Standard library only. Nothing here reads the clock, the environment or the filesystem, and nothing
iterates over a `set` of strings (hash order differs per process), so the same calls always give the
same bytes.

A `PdfFile` owns numbered objects. `serialize()` writes a complete file (classic cross-reference
table, or a cross-reference stream with `xref_stream=True`). After that, `begin_update()` starts
a revision: every `set()` is recorded, and `append_update()` appends the changed objects, a new
cross-reference section with a `/Prev` link, a trailer and a fresh `%%EOF`.
"""
from __future__ import annotations

import zlib
from dataclasses import dataclass, field


def name(s: str) -> str:
    """A PDF name object: `/Kruti#20Dev#20010`."""
    out = ["/"]
    for b in s.encode("utf-8"):
        if b < 0x21 or b > 0x7E or chr(b) in "#()<>[]{}/%":
            out.append(f"#{b:02X}")
        else:
            out.append(chr(b))
    return "".join(out)


def lit(data: bytes) -> str:
    """A literal string for arbitrary bytes. Backslash, parentheses and CR/LF are escaped."""
    out = []
    for b in data:
        c = chr(b)
        if c in "\\()":
            out.append("\\" + c)
        elif b == 13:
            out.append("\\r")
        elif b == 10:
            out.append("\\n")
        elif 32 <= b < 127:
            out.append(c)
        else:
            out.append(f"\\{b:03o}")
    return "(" + "".join(out) + ")"


def hexstr(data: bytes) -> str:
    return "<" + data.hex().upper() + ">"


def text_string(s: str, utf16: bool = False) -> str:
    """A PDF text string: PDFDocEncoding (Latin-1 range here) or UTF-16BE with a BOM."""
    if not utf16:
        try:
            return lit(s.encode("latin-1"))
        except UnicodeEncodeError:
            pass
    return hexstr(b"\xfe\xff" + s.encode("utf-16-be"))


def stream_body(entries: str, data: bytes, compress: bool = False, level: int = 6) -> bytes:
    """`<< entries /Length n >> stream ... endstream`, optionally Flate-compressed."""
    entries = entries.strip()
    if compress:
        data = zlib.compress(data, level)
        entries += " /Filter /FlateDecode"
    head = f"<< {entries} /Length {len(data)} >>\nstream\n".encode("latin-1")
    return head + data + b"\nendstream"


def fmt(x: float) -> str:
    """Compact, locale-independent number for content streams."""
    s = f"{x:.3f}".rstrip("0").rstrip(".")
    return "0" if s in ("", "-0") else s


@dataclass
class PdfFile:
    objs: dict[int, bytes] = field(default_factory=dict)
    n: int = 0
    dirty: list[int] = field(default_factory=list)
    _tracking: bool = False

    def new(self) -> int:
        self.n += 1
        return self.n

    def set(self, num: int, body: bytes | str) -> None:
        if isinstance(body, str):
            body = body.encode("latin-1")
        self.objs[num] = body
        if self._tracking and num not in self.dirty:
            self.dirty.append(num)

    def add(self, body: bytes | str) -> int:
        num = self.new()
        self.set(num, body)
        return num

    def add_stream(self, entries: str, data: bytes, compress: bool = False, level: int = 6) -> int:
        return self.add(stream_body(entries, data, compress, level))

    def begin_update(self) -> None:
        self._tracking = True
        self.dirty = []

    # ---- writers ---------------------------------------------------------------------------

    @staticmethod
    def _emit_obj(buf: bytearray, num: int, body: bytes) -> int:
        off = len(buf)
        buf += f"{num} 0 obj\n".encode()
        buf += body
        buf += b"\nendobj\n"
        return off

    def serialize(self, version: str, trailer: str, xref_stream: bool = False) -> tuple[bytearray, int]:
        """Return (file bytes, startxref). `trailer` holds entries such as `/Root 1 0 R /Info 2 0 R`."""
        buf = bytearray(f"%PDF-{version}\n".encode())
        buf += b"%\xe2\xe3\xcf\xd3\n"
        offsets: dict[int, int] = {}
        for num in sorted(self.objs):
            offsets[num] = self._emit_obj(buf, num, self.objs[num])
        startxref = len(buf)
        if xref_stream:
            self._write_xref_stream(buf, offsets, trailer, None)
        else:
            size = self.n + 1
            entries = [b"0000000000 65535 f \n"]
            for num in range(1, size):
                entries.append(f"{offsets[num]:010d} 00000 n \n".encode())
            buf += f"xref\n0 {size}\n".encode() + b"".join(entries)
            buf += f"trailer\n<< /Size {size} {trailer} >>\n".encode()
        buf += f"startxref\n{startxref}\n%%EOF\n".encode()
        return buf, startxref

    def append_update(self, buf: bytearray, prev: int, trailer: str, xref_stream: bool = False) -> int:
        """Append the objects changed since `begin_update()` as one revision. Returns its startxref."""
        offsets: dict[int, int] = {}
        for num in sorted(self.dirty):
            offsets[num] = self._emit_obj(buf, num, self.objs[num])
        startxref = len(buf)
        if xref_stream:
            self._write_xref_stream(buf, offsets, trailer, prev)
        else:
            nums = sorted(offsets)
            size = self.n + 1
            buf += b"xref\n"
            i = 0
            while i < len(nums):
                j = i
                while j + 1 < len(nums) and nums[j + 1] == nums[j] + 1:
                    j += 1
                buf += f"{nums[i]} {j - i + 1}\n".encode()
                for k in range(i, j + 1):
                    buf += f"{offsets[nums[k]]:010d} 00000 n \n".encode()
                i = j + 1
            buf += f"trailer\n<< /Size {size} {trailer} /Prev {prev} >>\n".encode()
        buf += f"startxref\n{startxref}\n%%EOF\n".encode()
        self.dirty = []
        return startxref

    def _write_xref_stream(self, buf: bytearray, offsets: dict[int, int], trailer: str,
                           prev: int | None) -> None:
        """A cross-reference stream (PDF 1.5) holding type-1 entries only, no object streams."""
        xnum = self.new()
        xoff = len(buf)
        nums = sorted(offsets) + [xnum]
        offs = dict(offsets)
        offs[xnum] = xoff
        rows = bytearray()
        if prev is None:
            rows += bytes([0]) + (0).to_bytes(4, "big") + (65535).to_bytes(2, "big")
            nums = [0] + nums
        for num in nums:
            if num == 0:
                continue
            rows += bytes([1]) + offs[num].to_bytes(4, "big") + (0).to_bytes(2, "big")
        runs: list[tuple[int, int]] = []
        start, length = nums[0], 1
        for a, b in zip(nums, nums[1:]):
            if b == a + 1:
                length += 1
            else:
                runs.append((start, length))
                start, length = b, 1
        runs.append((start, length))
        idx = " ".join(f"{a} {b}" for a, b in runs)
        extra = f" /Prev {prev}" if prev is not None else ""
        entries = f"/Type /XRef /Size {xnum + 1} /W [1 4 2] /Index [{idx}] {trailer}{extra}"
        buf += f"{xnum} 0 obj\n".encode() + stream_body(entries, bytes(rows), compress=True)
        buf += b"\nendobj\n"
