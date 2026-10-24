"""Synthetic text in the scripts the corpus covers. Nothing here is real document text.

Latin text is built from legal-sounding templates with generic placeholders (no names, no real
addresses). Indic text is meaningless syllable soup built from the Unicode blocks, plus a handful
of ordinary Hindi words for Devanagari. Legacy-font "text" is Latin code points that read as
gibberish, imitating Devanagari typed in a pre-Unicode font such as Kruti Dev.

Every function takes a `random.Random`, so the output depends only on its seed.
"""
from __future__ import annotations

import random
import unicodedata

SCRIPTS = ["latin", "devanagari", "bengali", "gurmukhi", "gujarati", "tamil", "telugu", "kannada",
           "malayalam", "arabic"]

# Unicode block starts. Offsets inside the Indic blocks are parallel (ISCII layout): consonants
# 0x15-0x39, independent vowels 0x05-0x14, vowel signs 0x3E-0x4C, virama 0x4D, digits 0x66-0x6F.
BLOCK = {"devanagari": 0x0900, "bengali": 0x0980, "gurmukhi": 0x0A00, "gujarati": 0x0A80,
         "tamil": 0x0B80, "telugu": 0x0C00, "kannada": 0x0C80, "malayalam": 0x0D00}
UNICODE_NAME = {"devanagari": "DEVANAGARI", "bengali": "BENGALI", "gurmukhi": "GURMUKHI",
                "gujarati": "GUJARATI", "tamil": "TAMIL", "telugu": "TELUGU", "kannada": "KANNADA",
                "malayalam": "MALAYALAM", "arabic": "ARABIC"}
# Scripts whose letters hang from a continuous headline (shirorekha); used by the pseudo-glyph renderer.
HEADLINE = {"devanagari", "bengali", "gurmukhi"}


def _assigned(cps, category_prefix):
    out = []
    for cp in cps:
        ch = chr(cp)
        if unicodedata.name(ch, None) and unicodedata.category(ch).startswith(category_prefix):
            out.append(ch)
    return out


def inventory(script: str) -> dict[str, list[str] | str]:
    b = BLOCK[script]
    return {
        "consonants": _assigned(range(b + 0x15, b + 0x3A), "L"),
        "vowels": _assigned(range(b + 0x05, b + 0x15), "L"),
        "signs": _assigned(range(b + 0x3E, b + 0x4D), "M"),
        "anusvara": _assigned([b + 0x02], "M"),
        "virama": chr(b + 0x4D),
        "digits": _assigned(range(b + 0x66, b + 0x70), "N"),
    }


_INV = {s: inventory(s) for s in BLOCK}

ARABIC_LETTERS = [chr(c) for c in range(0x0627, 0x064B) if c not in (0x0640,)
                  and unicodedata.category(chr(c)).startswith("L")]
ARABIC_DIGITS = [chr(c) for c in range(0x0660, 0x066A)]

HINDI_WORDS = ["विक्रय", "पत्र", "भूमि", "संपत्ति", "क्रेता", "विक्रेता", "न्यायालय", "प्रमाणित",
               "हस्ताक्षर", "दिनांक", "गवाह", "पंजीकरण", "मूल्य", "रुपये", "खसरा", "नंबर", "तहसील",
               "जिला", "गाँव", "अधिकार", "शर्तें", "सहमति"]
HINDI_DEED_TITLES = ["विक्रय पत्र", "किरायानामा", "बंधक पत्र", "दान पत्र"]


def word(rng: random.Random, script: str) -> str:
    if script == "arabic":
        return "".join(rng.choice(ARABIC_LETTERS) for _ in range(rng.randint(2, 7)))
    inv = _INV[script]
    if script == "devanagari" and rng.random() < 0.35:
        return rng.choice(HINDI_WORDS)
    parts = []
    for i in range(rng.randint(1, 4)):
        if i == 0 and rng.random() < 0.12:
            parts.append(rng.choice(inv["vowels"]))
        else:
            parts.append(rng.choice(inv["consonants"]))
            if rng.random() < 0.10:  # conjunct: consonant + virama + consonant
                parts.append(inv["virama"] + rng.choice(inv["consonants"]))
            if rng.random() < 0.6:
                parts.append(rng.choice(inv["signs"]))
        if rng.random() < 0.06 and inv["anusvara"]:
            parts.append(inv["anusvara"][0])
    return "".join(parts)


def number(rng: random.Random, script: str) -> str:
    n = str(rng.randint(1, 9999))
    if script in _INV and _INV[script]["digits"] and rng.random() < 0.5:
        d = _INV[script]["digits"]
        return "".join(d[int(c)] for c in n)
    if script == "arabic" and rng.random() < 0.5:
        return "".join(ARABIC_DIGITS[int(c)] for c in n)
    return n


def sentence(rng: random.Random, script: str) -> str:
    if script == "latin":
        return latin_sentence(rng)
    # Arabic text carries no digits: bidi reordering of digit runs is not what the corpus tests.
    numeric = 0.0 if script == "arabic" else 0.08
    words = [word(rng, script) if rng.random() >= numeric else number(rng, script)
             for _ in range(rng.randint(5, 14))]
    stop = {"devanagari": "।", "bengali": "।", "gurmukhi": "।", "arabic": "."}.get(script, ".")
    return " ".join(words) + " " + stop


PLACES = ["Alpha Nagar", "Beta Pura", "Gamma Wadi", "Delta Peth", "Sampleville", "Testganj"]
PARTIES = [("the Vendor", "the Purchaser"), ("the Lessor", "the Lessee"),
           ("the Mortgagor", "the Mortgagee"), ("the Donor", "the Donee")]
MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September",
          "October", "November", "December"]
LATIN_TEMPLATES = [
    "This Deed of {deed} is made on this {day}th day of {month}, {year} at {place}.",
    "WHEREAS {a} is the absolute owner of the property bearing Survey No. {n}/{m} situated at {place}.",
    "{A} hereby agrees to transfer and {b} agrees to accept the said property for a total "
    "consideration of Rs. {amount}/- (Rupees {amount} only) paid in full.",
    "{A} shall deliver vacant and peaceful possession of the said property to {b} on or before "
    "{day} {month} {year}.",
    "All taxes, cesses and outgoings up to the date of this Deed shall be borne by {a}.",
    "{A} declares that the property is free from all encumbrances, charges, liens and litigation.",
    "The boundaries of the property are as follows: on the East by Plot No. {n}, on the West by "
    "a road {m} feet wide, on the North by Survey No. {m}, and on the South by Plot No. {day}.",
    "Any dispute arising out of this Deed shall be referred to the courts at {place} only.",
    "In witness whereof the parties have set their hands on the day and year first written above.",
    "The stamp duty of Rs. {amount}/- has been paid under Article {day} of the Schedule.",
    "Clause {day}.{m} – neither party may assign its rights under this Deed without prior written consent.",
    "{A} shall bear the registration charges and {b}’s legal fees shall be borne by {b}.",
    "Item {n}: an undivided share of {m} square feet in the land described in the Schedule.",
]
DEEDS = ["Sale", "Lease", "Mortgage", "Gift", "Partition", "Release", "Settlement"]


def amount(rng: random.Random) -> str:
    n = rng.randint(10, 9999) * 1000
    s = str(n)
    head, tail = s[:-3], s[-3:]
    groups = []
    while len(head) > 2:
        groups.insert(0, head[-2:])
        head = head[:-2]
    if head:
        groups.insert(0, head)
    return ",".join(groups + [tail])


def latin_sentence(rng: random.Random) -> str:
    a, b = rng.choice(PARTIES)
    t = rng.choice(LATIN_TEMPLATES)
    return t.format(deed=rng.choice(DEEDS), day=rng.randint(1, 28), month=rng.choice(MONTHS),
                    year=rng.randint(1985, 2024), place=rng.choice(PLACES), a=a, b=b,
                    A=a[0].upper() + a[1:], n=rng.randint(1, 999), m=rng.randint(1, 99),
                    amount=amount(rng))


def paragraph(rng: random.Random, script: str, sentences: int | None = None) -> str:
    return " ".join(sentence(rng, script) for _ in range(sentences or rng.randint(2, 5)))


def title(rng: random.Random, script: str) -> str:
    if script == "latin":
        return rng.choice(["DEED OF " + d.upper() for d in DEEDS])
    if script == "devanagari":
        return rng.choice(HINDI_DEED_TITLES)
    return " ".join(word(rng, script) for _ in range(rng.randint(2, 3)))


def break_line(cur: str, w: str) -> tuple[str, str]:
    """Close the current line before word `w`. A line never ends in a hyphen: pdftotext treats that
    as a soft hyphen and joins the next line onto it, which would make the expected text depend on
    the extractor, so a trailing "-" word moves down with `w`."""
    if cur.endswith("-") and " " in cur:
        head, tail = cur.rsplit(" ", 1)
        return head, tail + " " + w
    return cur, w


def wrap(text: str, max_chars: int) -> list[str]:
    lines, cur = [], ""
    for w in text.split(" "):
        if cur and len(cur) + 1 + len(w) > max_chars:
            done, cur = break_line(cur, w)
            lines.append(done)
        else:
            cur = (cur + " " + w) if cur else w
    if cur:
        lines.append(cur)
    return lines


# ---- legacy-encoded Devanagari ------------------------------------------------------------------
# Latin code points as typed in a Kruti-Dev-style font. A few words I recall from the Kruti Dev
# layout (UNVERIFIED: the exact mapping is irrelevant here; only "Latin code points under a
# legacy-named font, reading as gibberish" matters), plus random syllable soup in the same style.
LEGACY_WORDS = ["fgUnh", "dkuwu", "Hkkjr", "ljdkj", "U;k;ky;", "foØ;", "lEifRr", "izfr",
                "dk;kZy;", "izekf.kr", "LFkku", "tuin", "rglhy", "[kljk", "uEcj", "fnukad",
                "gLrk{kj", "xokg", "ewY;", ":i;s", "Hkwfe", "Ø;", "'krsZa", "lgefr"]
LEGACY_SYLLABLES = ["d", "[k", "x", "?k", "p", "N", "t", "V", "B", "M", "r", "Fk", "n", "/k", "u", "i",
                    "Q", "c", "Hk", "e", ";", "j", "y", "o", "'k", "l", "g", "ks", "kS", "k", "h", "s",
                    "S", "q", "w", "a", "Z", "f"]


def legacy_word(rng: random.Random) -> str:
    if rng.random() < 0.4:
        return rng.choice(LEGACY_WORDS)
    return "".join(rng.choice(LEGACY_SYLLABLES) for _ in range(rng.randint(3, 8)))


def legacy_word_of_length(rng: random.Random, n: int) -> str:
    """Legacy-style gibberish of about `n` characters (n >= 1)."""
    out = ""
    while len(out) < n:
        out += rng.choice(LEGACY_SYLLABLES)
    return out[:max(1, n)]


def legacy_sentence(rng: random.Random) -> str:
    return " ".join(legacy_word(rng) for _ in range(rng.randint(5, 12))) + " A"


def legacy_paragraph(rng: random.Random, sentences: int | None = None) -> str:
    return " ".join(legacy_sentence(rng) for _ in range(sentences or rng.randint(2, 4)))


def script_of(text: str) -> set[str]:
    """Scripts present in `text`, by Unicode name prefix. Used by the tests."""
    found = set()
    for ch in text:
        if ch in "\u0964\u0965":  # danda and double danda are shared by several scripts
            continue
        n = unicodedata.name(ch, "")
        for s, prefix in UNICODE_NAME.items():
            if n.startswith(prefix):
                found.add(s)
        if n.startswith("LATIN"):
            found.add("latin")
    return found
