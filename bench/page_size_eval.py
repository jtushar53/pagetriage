#!/usr/bin/env python3
"""Run the layout, script and quality readouts on one synthetic text page rendered at many page sizes
(A4, Letter, Legal, A5, A3, landscape, a 6 in square, a 3.1 in receipt strip), many resolutions (72 to
600 dpi), tilted, at other type sizes, with a wrong dpi, and at poster size, and print what comes out
next to the truth (lines drawn, skew applied).

    python -I bench/page_size_eval.py

Nothing in the core is tied to A4 or 150 dpi: sizes come from the input and the layout works in
x-heights. This prints the evidence (and the cost: time and memory grow with the pixel count). The
detectors other than layout and script were tuned on A4 at 150 dpi (see each docs page) and have not
been swept over sizes. `crates/pagetriage-core/tests/page_sizes.rs` pins the layout result in the test
suite. Needs the Python binding, numpy and Pillow.
"""
import json
import random
import resource
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "synth"))
import numpy as np  # noqa: E402
import pagetriage_py  # noqa: E402
import scanimg  # noqa: E402
import synthtext  # noqa: E402
IN={"A4":(8.27,11.69),"Letter":(8.5,11),"Legal":(8.5,14),"A5":(5.83,8.27),"A3":(11.69,16.54),"A1":(23.4,33.1),"receipt":(3.1,11),"landscape A4":(11.69,8.27),"square 6in":(6,6)}
def run(name,dpi,skew=0.0,pt=11.0,paras=6):
    w,h=round(IN[name][0]*dpi),round(IN[name][1]*dpi)
    rng,np_rng=random.Random(1),np.random.default_rng(1)
    ps=[synthtext.paragraph(rng,"latin",6) for _ in range(paras)]
    s=scanimg.render_scan(rng,np_rng,w,h,dpi,"latin",ps,pt,(0.08,0.06,0.08,0.06),skew,2.0,0.0,0.0,0.0)
    truth=len([l for l in s.lines if l])
    opts=json.dumps({"detectors":["layout","script","qr","photo","rulings","redaction"],"debug":True})
    t=time.perf_counter()
    r=json.loads(pagetriage_py.analyse_page(s.image.tobytes(),w,h,dpi,None,None,opts))
    ms=(time.perf_counter()-t)*1000
    q=r["quality"]; L=r["debug"]["layout"]
    fam=[f["attributes"]["family"] for f in r["findings"] if f["kind"]=="script"][:1]
    spur=[f["kind"] for f in r["findings"] if f["kind"] in ("qr_code","photo","table","redaction_box","ruling_lines")]
    print(f"{name:<13}{dpi:>4} dpi {w:>5}x{h:<5}{q['grade']:>3} skew {q['skew_deg']:>5.1f} (true {skew}) lines {int(L['lines']):>4} ({truth:>3}) {fam} spurious {spur} {ms:>7.0f} ms")
for name in ("A4","Letter","Legal","A5","A3","landscape A4","square 6in","receipt"):
    run(name,150)
print()
for dpi in (72,100,200,300,600):
    run("A4",dpi)
print()
run("A4",150,skew=3.0); run("Letter",300,skew=3.0); run("A3",100,skew=3.0)
print()
run("A1",100,paras=14)
for pt in (6.0,8.0,14.0,24.0):
    run("A4",150,pt=pt); 
print("--- wrong dpi: an A4 page scanned at 150 dpi, told it is 75, 150, 300, 600")
rng,np_rng=random.Random(1),np.random.default_rng(1)
ps=[synthtext.paragraph(rng,"latin",6) for _ in range(6)]
s=scanimg.render_scan(rng,np_rng,1240,1754,150,"latin",ps,11.0,(0.08,0.06,0.08,0.06),0.0,2.0,0.0,0.0,0.0)
truth=len([l for l in s.lines if l])
for dpi in (75,150,300,600):
    r=json.loads(pagetriage_py.analyse_page(s.image.tobytes(),1240,1754,dpi,None,None,json.dumps({"detectors":["layout","script","rulings","photo"],"debug":True})))
    print(dpi,"lines",int(r["debug"]["layout"]["lines"]),"(true",truth,") grade",r["quality"]["grade"],"size_pt",[round(x) for x in r["size_pt"]])
print("--- very large")
for name,dpi in (("A1",200),("A1",300)):
    w,h=round(IN[name][0]*dpi),round(IN[name][1]*dpi)
    img=np.full((h,w),255,np.uint8); img[200:260,200:w-200]=40
    t=time.perf_counter()
    try:
        r=json.loads(pagetriage_py.analyse_page(img.tobytes(),w,h,dpi,None,None,json.dumps({"detectors":["layout"]})))
        print(name,dpi,w*h/1e6,"Mpx ok",round(time.perf_counter()-t,1),"s","peak MB",resource.getrusage(resource.RUSAGE_SELF).ru_maxrss//1024)
    except Exception as e: print(name,dpi,w*h/1e6,"Mpx ERROR",str(e)[:100])
