#!/usr/bin/env python3
"""Probe the statistics-only half of the legacy-font detector (module M2).

The detector combines a font-name list with statistics of the Latin-looking text a legacy font
produces. This script isolates the statistics: every font here has an unremarkable name
(`Fnt1`), so only the text can give a legacy font away. Hindi sentences written for this probe are
encoded into an *approximation* of the Kruti Dev layout (recalled from memory, simplified, not
checked against the real font) and compared with English and transliterated-Indian-English text.

    python -I bench/tools/legacy_probe.py [SENTENCES_PER_TEXT]   (default 3)

The thresholds in params/font_script.toml were set by hand before this probe was written; it
checks them (it did not tune them). The sentences are split into two halves only so that both can
be reported. The numbers say how the heuristic behaves on this approximation, not on real Kruti Dev
PDFs, which we do not have.
"""
import importlib.util
import json
import pathlib
import sys

import pagetriage_py

ROOT = pathlib.Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("gen_pdfs", ROOT / "tests/golden/gen_pdfs.py")
g = importlib.util.module_from_spec(spec)
spec.loader.exec_module(g)

HINDI = [
    "यह विक्रय विलेख आज दिनांक पन्द्रह मार्च को पुणे में निष्पादित किया गया है",
    "विक्रेता उक्त सम्पत्ति का एकमात्र स्वामी है और उस पर कोई भार नहीं है",
    "क्रेता ने विक्रेता को सम्पूर्ण प्रतिफल की राशि का भुगतान कर दिया है",
    "सम्पत्ति का क्षेत्रफल दो सौ पचास वर्ग मीटर है जो गाँव के सर्वे नम्बर में स्थित है",
    "स्टाम्प शुल्क और पंजीकरण का सम्पूर्ण व्यय क्रेता द्वारा वहन किया जाएगा",
    "इस विलेख के अन्तर्गत कोई भी विवाद उत्पन्न होने पर न्यायालय का क्षेत्राधिकार होगा",
    "विक्रेता अपने उत्तराधिकारियों सहित इस विलेख की शर्तों से बाध्य रहेगा",
    "भूमि पर स्थित सभी वृक्ष और निर्माण इस विक्रय में सम्मिलित हैं",
    "दोनों पक्षों ने इस विलेख को पढ़कर और समझकर साक्षियों के सामने हस्ताक्षर किए",
    "यह दस्तावेज़ मूल रूप में उप पंजीयक कार्यालय में प्रस्तुत किया गया",
    "राज्य सरकार द्वारा निर्धारित दर के अनुसार मूल्यांकन किया गया है",
    "उपरोक्त सम्पत्ति का कब्ज़ा आज ही क्रेता को सौंप दिया गया है",
    "सभी कर और उपकर आज की तिथि तक विक्रेता द्वारा चुकाए गए हैं",
    "इस विलेख की एक प्रति क्रेता और एक प्रति विक्रेता के पास रहेगी",
    "विवाद की स्थिति में मध्यस्थ की नियुक्ति दोनों पक्षों की सहमति से होगी",
    "अचल सम्पत्ति के स्वामित्व का हस्तान्तरण इसी विलेख द्वारा पूर्ण होता है",
    "गवाहों के नाम और पते इस विलेख के अन्त में दिए गए हैं",
    "विक्रेता घोषणा करता है कि सम्पत्ति किसी न्यायालय में विचाराधीन नहीं है",
    "क्रेता को सम्पत्ति के नामान्तरण का पूर्ण अधिकार प्राप्त होगा",
    "यह सम्पूर्ण विलेख स्वेच्छा से और बिना किसी दबाव के निष्पादित हुआ है",
    "सम्पत्ति के चारों ओर की सीमाएँ पूर्व पश्चिम उत्तर और दक्षिण में वर्णित हैं",
    "विक्रेता की पत्नी और पुत्रों ने भी इस विक्रय के लिए अपनी सहमति दी है",
    "बिजली और पानी के सभी बकाया बिल विक्रेता चुका चुका है",
    "इस सम्बन्ध में पूर्व में किया गया कोई भी करार अब निरस्त माना जाएगा",
]

ENGLISH = [
    "This Deed of Sale is executed at Pune on this fifteenth day of March between the Vendor and the Purchaser",
    "The Vendor is the sole and absolute owner of the said property free from all encumbrances",
    "The Purchaser has paid the entire consideration amount to the Vendor and the Vendor acknowledges the receipt",
    "The property admeasures two hundred and fifty square metres situated at Survey No. 45/2A of the village",
    "Stamp duty and registration charges shall be borne entirely by the Purchaser",
    "Any dispute arising out of this Deed shall be subject to the jurisdiction of the courts at Pune",
    "The Vendor shall be bound by the terms of this Deed along with his legal heirs and successors",
    "All trees and constructions standing on the land are included in this sale",
    "Both parties have read and understood this Deed and have signed it in the presence of witnesses",
    "This document was presented in original at the office of the Sub-Registrar",
    "The valuation has been made at the rate prescribed by the State Government",
    "Possession of the said property has been handed over to the Purchaser today",
    "All taxes and cesses up to the date of this Deed have been paid by the Vendor",
    "One copy of this Deed shall remain with the Purchaser and one with the Vendor",
    "In case of a dispute an arbitrator shall be appointed with the consent of both parties",
    "Transfer of ownership of the immovable property is complete by this Deed",
    "Names and addresses of the witnesses are given at the end of this Deed",
    "The Vendor declares that the property is not under litigation in any court",
    "SHRI RAMESH KUMAR S/O LATE SHRI MOHAN LAL R/O 12 GANDHI NAGAR PUNE 411001 AGED 54 YEARS",
    "Rs. 10,00,000/- (Rupees Ten Lakhs only) paid by cheque no. 004512 drawn on SBI Pune Camp branch",
    "Plot No. 17, Sector 9, Navi Mumbai, bounded on the East by 12 metre wide road and on the West by Plot No. 18",
    "Smt. Sunita Devi W/o Shri Anil Patil, Adhar Card No. XXXX XXXX 4521, PAN ABCDE1234F",
    "WHEREAS the Vendor is seized and possessed of the property described in the Schedule hereunder written",
    "NOW THIS DEED WITNESSETH that in consideration of the sum aforesaid the Vendor doth hereby convey",
]

# Unicode Devanagari -> approximate Kruti Dev 010 keystrokes. Simplified and recalled from memory.
CONS = {"क": "d", "ख": "[k", "ग": "x", "घ": "?k", "ङ": "³", "च": "p", "छ": "N", "ज": "t", "झ": ">", "ञ": "¥",
        "ट": "V", "ठ": "B", "ड": "M", "ढ": "<", "ण": ".k", "त": "r", "थ": "Fk", "द": "n", "ध": "/k", "न": "u",
        "प": "i", "फ": "Q", "ब": "c", "भ": "Hk", "म": "e", "य": ";", "र": "j", "ल": "y", "व": "o", "श": "'k",
        "ष": "\"k", "स": "l", "ह": "g", "ळ": "G"}
HALF = {"क": "D", "ख": "[", "ग": "X", "घ": "?", "च": "P", "ज": "T", "त": "R", "थ": "F", "ध": "/", "न": "U",
        "प": "I", "फ": "¶", "ब": "C", "भ": "H", "म": "E", "य": "¸", "ल": "Y", "व": "O", "श": "'", "ष": "\"",
        "स": "L", "ह": "²"}
MATRA = {"ा": "k", "ी": "h", "ु": "q", "ू": "w", "ृ": "`", "े": "s", "ै": "S", "ो": "ks", "ौ": "kS", "ं": "a",
         "ः": "%", "ँ": "¡", "़": "+"}
VOWEL = {"अ": "v", "आ": "vk", "इ": "b", "ई": "bZ", "उ": "m", "ऊ": "Å", "ऋ": "_", "ए": ",", "ऐ": ",s", "ओ": "vks", "औ": "vkS"}


def kruti(text):
    out, i = [], 0
    chars = list(text)
    while i < len(chars):
        c = chars[i]
        nxt = chars[i + 1] if i + 1 < len(chars) else ""
        if c in CONS:
            # conjunct: consonant + virama + consonant -> half form + full
            if nxt == "्" and i + 2 < len(chars) and chars[i + 2] in CONS:
                out.append(HALF.get(c, CONS[c]))
                i += 2
                continue
            base = CONS[c]
            if nxt == "ि":  # short i matra is written before the consonant
                out.append("f" + base)
                i += 2
                continue
            out.append(base)
            i += 1
            continue
        if c == "्":
            i += 1
            continue
        if c in MATRA:
            out.append(MATRA[c])
        elif c in VOWEL:
            out.append(VOWEL[c])
        elif c == "ि":
            out.append("f")
        else:
            out.append(c)
        i += 1
    return "".join(out)


def pdf_with(text):
    pdf = g.Pdf()
    cp = text.encode("cp1252", "replace").decode("cp1252")
    esc = "".join(f"\\{ord(ch.encode('cp1252', 'replace')):03o}" if ord(ch) > 126 or ch in "()\\" else ch for ch in cp)
    content = f"BT /F1 12 Tf 72 700 Td ({esc}) Tj ET\n"
    info = g.pages_doc(
        pdf,
        [(content, "/Font << /F1 100 0 R >>")],
        extra_objects=lambda p: p.obj(100, "<< /Type /Font /Subtype /TrueType /BaseFont /Fnt1 /Encoding /WinAnsiEncoding >>"),
    )
    return pdf.finish(1, 101, info)


def paragraphs(sentences, size=3):
    return [" ".join(sentences[i : i + size]) for i in range(0, len(sentences) - size + 1, size)]


def flagged(text):
    r = json.loads(pagetriage_py.analyse_pdf_structure(pdf_with(text)))
    return r["pages"][0]["script"]["legacy_encoded"]


def main():
    size = int(sys.argv[1]) if len(sys.argv) > 1 else 3  # sentences per text
    legacy = [kruti(p) for p in paragraphs(HINDI, size)]
    english = paragraphs(ENGLISH, size)
    print("example legacy-style paragraph:", legacy[0][:100])
    for split, sl in (("tuning", slice(0, len(legacy) // 2)), ("test", slice(len(legacy) // 2, None))):
        l, e = legacy[sl], english[sl]
        tp = sum(flagged(t) for t in l)
        fp = sum(flagged(t) for t in e)
        print(f"{split:<7} legacy-style text flagged {tp}/{len(l)}   English text flagged {fp}/{len(e)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
