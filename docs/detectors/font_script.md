# `structural.font_script` (module M2)

Reports, per PDF page, which script the text layer is in, and flags the India-specific trap: legacy
non-Unicode fonts (Kruti Dev, DV-TTYogesh, Shree-Lipi, Chanakya, ...) that put Devanagari glyphs on
Latin code points. A page set in one of these *looks* Hindi and *extracts* as Latin gibberish
(`;g foys[k fu"iknu`), so any consumer of the text layer must know. Part of `pagetriage structure` and
`analyse_pdf_structure` (needs the `pdf` cargo feature); the pure pieces (`structural::unicode`,
`cmap`, `encoding`, `legacy`, `font_script`) compile without it.

Output per page, in `pages[i].script` (`ScriptInfo`): `dominant` (machine id), `name` (display, e.g.
`Devanagari (legacy-encoded)`), `shares` (per script, over decoded letters, sums to 1), `letters`,
`glyphs_undecodable`, `legacy_encoded`, `legacy_script`, `legacy_glyph_share`, the fonts used, and
the evidence. `text_layer.reliable` is `false` when the layer is legacy-encoded or undecodable.

## Algorithm

1. **Which fonts.** Only fonts that actually showed a glyph on the page (a `Tf` with no text after
   it, or a font only listed in shared `Resources`, does not count). Fonts are identified by object
   id (inline font dictionaries by order of first use), so the output order is deterministic.
2. **Decode** each text string into code points:
   * a `ToUnicode` CMap when present (`bfchar`, `bfrange` with a start value or an array, codespace
     ranges of one to four bytes, UTF-16BE destinations with surrogates), bounded by
     `max_cmap_entries`;
   * otherwise a simple font's base encoding (`WinAnsi`; `MacRoman` as recalled from memory;
     `Standard` approximated; no `/Encoding` is treated as WinAnsi) with `/Differences` glyph names
     (`uniXXXX`, `uXXXX`, Latin names, and Indic names by suffix such as `kadeva`; the suffix rule is
     recalled, not checked against the Adobe Glyph List);
   * a composite font (Type0, Identity-H) without a `ToUnicode` map, a Type3 font with no map or
     names, or code points in the private-use area: **undecodable**. Counted, not guessed.
3. **Histogram** the code points by Unicode block: Latin, Devanagari U+0900-097F, Bengali
   U+0980-09FF, Gurmukhi U+0A00-0A7F, Gujarati U+0A80-0AFF, Oriya U+0B00-0B7F, Tamil U+0B80-0BFF,
   Telugu U+0C00-0C7F, Kannada U+0C80-0CFF, Malayalam U+0D00-0D7F, Arabic U+0600-06FF (plus the
   supplement and presentation forms), CJK, other. Digits, punctuation, danda and spaces are left
   out of the shares.
4. **Legacy-font verdict**, per font, as a log-odds sum
   `prior_logit + name_llr + bigram_llr + mid_caps_llr + marker_llr`, legacy when
   `sigmoid >= emit_min_confidence`:
   * **Name**: the normalised font name (subset prefix `ABCDEF+` dropped, lower-cased, only letters
     and digits) against the curated list in `params/font_script.toml` (`prefix` or `contains`,
     `strong` or `weak`). The list is **domain knowledge written from memory and not checked
     against any published list**; names are taken from `/BaseFont`, the descriptor's `/FontName` and
     `/FontFamily`, and the descendant font.
   * **Text statistics** on what the font's codes decode to, used when the sample has at least
     `min_sample_letters` letters: the share of adjacent in-word letter pairs found in a list of the
     50 most common English bigrams (a list recalled from letter-frequency tables, approximate);
     the share of 3+ letter words with a capital after a lower-case letter (`lEifRr`); the share of
     words with a character that prose does not put mid-word (`[ ] { } \ ^ ~ |`, a flanked `;` `<` `=`,
     accented Latin-1 letters between ASCII letters). Each is a piecewise-linear llr between a `lo`
     and a `hi` in the TOML.
5. **Page**: a legacy font's Latin letters are counted under the script it writes (the entry's script,
   or `default_legacy_script` = Devanagari when only the statistics fired). `legacy_encoded` is true if
   any such font showed glyphs; `text_layer.reliable` is `false` if legacy glyphs are at least
   `unreliable_glyph_share` (10%) of the page's glyphs, or undecodable glyphs at least
   `undecodable_unreliable_share` (30%). `dominant` is `unknown` when there are glyphs but no
   decodable letter, `none` when there is no text.
6. **Consistency rule**: a page with no text layer (`raster_only`, `vector_only`, `empty`) cannot be
   set in a legacy font, because a legacy font exists only where text is drawn. If both appear
   (typically one or two stray glyphs, below `min_glyphs`, in a legacy font on a scan) the page carries
   `conflicts: ["raster_only+legacy_devanagari"]` and nothing is silently dropped.

Evidence for the golden legacy page (`tests/golden/pdf/scripts_legacy.pdf`, page 2):

```
Devanagari (legacy-encoded), reliable=false
  legacy_font_name          1.0     thr 1.0   llr +5.0    font F0 matches curated legacy devanagari entry "krutidev" (Strong)
  english_bigram_coverage   0.021   thr 0.3   llr +3.0
  mid_word_capital_rate     0.143   thr 0.02  llr +2.78
  mid_word_marker_rate      0.333   thr 0.02  llr +3.0
  text_layer / legacy_glyph_share  1.0  thr 0.1  llr -11.78   100% of glyphs are set in legacy-encoded devanagari fonts
```

## Parameters

`crates/pagetriage-core/params/font_script.toml`. Honest provenance of its numbers:

* the log-odds weights, prior and the three piecewise thresholds are **hand-set**, not fitted; the
  comments say so. They were checked (not tuned) with `bench/tools/legacy_probe.py`, below;
* the 50-bigram list and the font list are **recalled from memory** and unverified;
* the per-feature coefficients plus the prior and weights number well under the project's budget of
  about 50 interpretable coefficients (the 50-entry bigram list is a lexicon, listed here because it
  sits close to that budget).

## Measured results

**Corpus** (`bench/structure_eval.py`, synthetic, written by us; weak evidence of real-world accuracy):

| Check | Result |
|---|---|
| Dominant script of the text layer, 300 pages | 300 / 300. Label counts: latin 119, none 77, devanagari 60, bengali 13, telugu 8, tamil 7, gujarati 7, kannada 3, gurmukhi 3, arabic 2, malayalam 1 (the small scripts rest on very few pages) |
| Legacy-encoded Devanagari pages flagged (target >= 95%) | 17 / 17 = 100% (5 documents), all with `text_layer.reliable = false` |
| False positives on the other 283 pages | 0 |
| Consistency conflicts reported | 0 (none arise in this corpus) |

Seen and unseen: all 17 legacy pages are in the even-numbered (held-out) documents, so the legacy
detector was **not** developed or tuned against any corpus legacy page. The held-out split was
scored once, after the analyser was finished (see `text_layer.md` for the protocol). The corpus's
legacy strings and font names were also written from memory, by someone else, so this is two sets of
recollections agreeing, not a check against real Kruti Dev files, which we do not have.

**Statistics alone** (`python -I bench/tools/legacy_probe.py`): Hindi sentences written for the
probe, encoded into an approximate Kruti Dev layout (also from memory), in a font with an ordinary
name, against English and transliterated-Indian-English sentences (names, `S/O`, `Rs. 10,00,000/-`,
survey numbers, all-capitals recitals):

| Text unit | Legacy-style flagged | English-style flagged |
|---|---|---|
| 1 sentence (60-80 letters), tuning half / test half | 12 / 12 and 12 / 12 | 0 / 12 and 0 / 12 |
| 3 sentences, both halves | 4 / 4 and 4 / 4 | 0 / 4 and 0 / 4 |

The probe is 24 Hindi and 24 English sentences. It shows the statistics separate the two on this
approximation; it says little about real Kruti Dev text or about other Latin-script languages.

## Failure gallery

| Case | What happens | Why |
|---|---|---|
| Legacy font that is **not** on the list, with little text (under 40 letters) | Missed | No name hit and no statistics. A short heading in an unlisted font stays `Latin`. |
| Legacy font on the list, but the text it shows reads as English | Not flagged | A strong name adds 5 to the prior of -2; English-looking statistics subtract about 5, giving -2. A sample under 40 letters has no statistics, so the name alone flags it. Rare in practice. |
| Unicode font whose name contains a listed word (`Kiran`, `Agra`, `Priyanka`, `Nudi`) | Weak entries only add 1.5; not flagged without bad statistics | They are marked `weak` for this reason. Strong entries (`krutidev`, `dvtt`, `chanakya`, ...) are not ambiguous in practice, but that is a belief, not a measurement. |
| Hindi typed in Roman letters (Hinglish) | Probably flagged | Low English-bigram coverage and capital runs look like legacy text. Untested. |
| Other Latin-script languages (French, Portuguese, Vietnamese) | Probably fine for French/Portuguese, doubtful for heavily accented text | The accented-letter marker could fire on dense accents. Untested. |
| Legacy fonts of other Indian scripts (Bengali Bijoy/SutonnyMJ, Gurmukhi Anmol Lipi, Tamil Bamini, ...) | Flagged by name only; script from the entry | The statistics and `default_legacy_script` assume Hindi-like keyboard layouts. Names recalled from memory. |
| Composite fonts (Identity-H) without `ToUnicode` | `dominant: unknown`, `reliable: false` | Right, but it also covers harmless fonts whose map was stripped; the output cannot tell the two apart. |
| Legacy font that is a Type0 with a bogus `ToUnicode` mapping to private-use code points | `unknown` / undecodable | Counted as undecodable, not as legacy. |
| Mixed page: English body, Kruti Dev heading | `legacy_encoded: true`, `reliable: true`, `dominant: latin` | Under the 10% share. Intended: the body text is usable; the heading is not. |
| Right-to-left text | Shares computed, order ignored | Only the code points are counted. |
| Ligature and half-form glyphs in Unicode Devanagari fonts without `ToUnicode` | `unknown` | No glyph-to-Unicode recovery from the font program is attempted. |

## What is not covered

Real legacy-font PDFs (the font names, the keystroke layouts and the bigram list are all from
memory), embedded font programs (never parsed), `ToUnicode` maps that are wrong rather than absent,
predefined CJK CMaps, Type3 fonts with real maps, mixed-script lines in one font, and scripts outside
the thirteen blocks.
