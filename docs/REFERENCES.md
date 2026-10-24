# References

Papers behind the detectors in `pagetriage`, grouped by the module that uses them.

**How to read the links.** A link appears only where our research notes recorded a DOI, an arXiv ID
or a URL. A `—` in the Link column means we have the citation but have not yet resolved a link; it
is not a statement that the paper is unavailable. Entries marked *unverified* carry a caveat from
the research notes that should be re-checked before the paper is quoted.

Accuracy figures quoted in the notes mostly come from systems that used a trained classifier (MLP,
SVM, random forest). `pagetriage` reuses the papers' *features* and replaces the classifier with
hand-set rules and a few frozen logistic coefficients, so expect lower numbers than the papers report.

## Binarisation and image quality (M3)

| Paper | Venue | Link | Used for |
|---|---|---|---|
| N. R. Howe, "Document binarization with automatic parameter tuning" | IJDAR 16:247–258, 2013 | [doi:10.1007/s10032-012-0192-x](https://doi.org/10.1007/s10032-012-0192-x); [author's manuscript](http://www.science.smith.edu/~nhowe/research/pubs/divseg-ijdar.pdf) (CC-BY) | Howe binarisation (graph cut), chosen when bleed-through or low contrast is flagged. Implemented from this text; Tables 1 and 3 are the published numbers our DIBCO evaluation is compared with |
| N. R. Howe, "A Laplacian energy for document binarization" | ICDAR 2011, pp. 6–10 | — | The base method that the 2013 paper tunes |
| N. Otsu, "A threshold selection method from gray-level histograms" | IEEE Trans. SMC 9(1):62–66, 1979 | — | Global threshold |
| J. Sauvola, M. Pietikäinen, "Adaptive document image binarization" | Pattern Recognition 33(2):225–236, 2000 | — | Local threshold |
| C. Wolf, J.-M. Jolion, "Extraction and recognition of artificial text in multimedia documents" | Formal Pattern Analysis & Applications 6(4):309–326, 2004 | — | Local threshold (*unverified*: citation and formula recalled, not re-checked) |
| J. Canny, "A computational approach to edge detection" | IEEE TPAMI 8(6):679–698, 1986 | — | Edge map that places the Howe cut |
| SauvolaNet: learning adaptive Sauvola network for degraded document binarization | arXiv 2105.05521 | [arXiv:2105.05521](https://arxiv.org/abs/2105.05521) | Secondary source for the Howe vs Sauvola DIBCO F-measure tables |
| DeepOtsu: document enhancement and binarization using iterative deep learning | arXiv 1901.06081 | [arXiv:1901.06081](https://arxiv.org/abs/1901.06081) | Secondary comparison source for DIBCO results |
| (secondary source for Howe's H-DIBCO 2012 score; *unverified*) | arXiv 1305.2949 | [arXiv:1305.2949](https://arxiv.org/abs/1305.2949) | Re-measure in our harness rather than trust |
| (secondary source for Howe's H-DIBCO 2012 score; *unverified*) | arXiv 1709.01782 | [arXiv:1709.01782](https://arxiv.org/abs/1709.01782) | Re-measure in our harness rather than trust |
| (secondary source for "1st in HDIBCO 2012, 2014, 2016"; *unverified*) | arXiv 1708.03276 | [arXiv:1708.03276](https://arxiv.org/abs/1708.03276) | Re-measure in our harness rather than trust |
| B. B. Chaudhuri, U. Pal, "Skew angle detection of digitized Indian script documents" | IEEE TPAMI 19(2), 1997 | — | Headline-based skew estimation |
| Pech-Pacheco et al., variance of the Laplacian as a focus measure | 2000 | — | Blur measure (*unverified*: full citation not recorded) |
| W. Postl, "Detection of linear oblique structures and skew in digitized documents" | ICPR 1986 | — | Projection-profile variance as the skew criterion (*unverified*: recalled from memory, not checked) |
| G. Borgefors, "Distance transformations in digital images" | Computer Vision, Graphics and Image Processing 34(3):344–371, 1986 | — | The 3-4 chamfer distance transform behind stroke widths and ridges (*unverified*: recalled from memory, not checked) |
| A. Rosenfeld, J. L. Pfaltz, "Sequential operations in digital picture processing" | J. ACM 13(4):471–494, 1966 | — | Two-pass connected-component labelling (*unverified*: recalled from memory, not checked) |
| E. Peli, "Contrast in complex images" | J. Optical Society of America A 7(10):2032–2040, 1990 | — | RMS contrast definition (*unverified*: recalled from memory, not checked) |
| Y. Boykov, V. Kolmogorov, "An experimental comparison of min-cut/max-flow algorithms for energy minimization in vision" | IEEE TPAMI 26(9):1124–1137, 2004 | — | Max-flow for the Howe graph cut |

## Script identification (M4)

| Paper | Venue | Link | Used for |
|---|---|---|---|
| U. Pal, B. B. Chaudhuri, "Script line separation from Indian multi-script documents" | ICDAR 1999, pp. 406–409; IETE J. Research 49(1):3–11 | — | Headline (shirorekha) tier; hand-built tree classifier |
| U. Pal, S. Sinha, B. B. Chaudhuri, "Multi-script line identification from Indian documents" | ICDAR 2003, pp. 880–884 | — | Water-reservoir, contour and profile features across 12 Indian scripts |
| A. L. Spitz, "Determination of the script and language content of document images" | IEEE TPAMI 19(3):235–245, 1997 | — | Upward-concavity distribution; Han vs Latin-like scripts |
| J. Hochberg et al., "Automatic script identification from document images using cluster-based templates" | IEEE TPAMI 19(2):176–181, 1997 | — | Design reference only (its templates are learned codebooks) |
| D. Dhanya, A. G. Ramakrishnan, P. B. Pati, "Script identification in printed bilingual documents" | Sādhanā 27(1):73–82, 2002 | [ias.ac.in](https://www.ias.ac.in/article/fulltext/sadh/027/01/0073-0082) | Gabor texture tier |
| G. D. Joshi, S. Garg, J. Sivaswamy, "A generalised framework for script identification" | IJDAR 10(2):55–68, 2007 | — | Hierarchical log-Gabor texture framework |
| T. N. Tan, rotation-invariant texture features and their use in script identification | 1998 | — | Rotation-invariant Gabor features (*unverified*: full citation not recorded) |

## Handwriting vs print (M5)

| Paper | Venue | Link | Used for |
|---|---|---|---|
| Lu et al., "Handwritten and Printed Text Segmentation via Region-Aware Human-Writing Descriptor Engineering" | arXiv 2607.15936, 2026 | [arXiv:2607.15936](https://arxiv.org/abs/2607.15936) | SCCS segmentation and the RHD feature vector; benchmark line vs its random-forest result (*unverified*: dataset and code availability) |
| M. Seuret, M. Liwicki, R. Ingold, "Pixel level handwritten and printed content discrimination in scanned documents" | ICFHR 2014, pp. 423–428 | — | Neighbourhood features for the overlay case (the paper used an MLP) |
| A. Belaïd, K. C. Santosh, V. Poulain d'Andecy, "Handwritten and printed text separation in real document" | arXiv 1303.4614 | [arXiv:1303.4614](https://arxiv.org/abs/1303.4614) | RLSA pseudo-words; neighbour context propagation (copied deterministically) |
| B. M. Garlapati, S. R. Chalamala, "A system for handwritten and printed text classification" | UKSim 2017 | — | Cheap word-level intensity features |
| Gholamian, Vahidian, "Handwritten and Printed Text Segmentation: A Signature Case Study" (SignaTR6K) | arXiv 2307.07887 | [arXiv:2307.07887](https://arxiv.org/abs/2307.07887) | Closest public evaluation set for the overlay case (*unverified*: dataset licence) |

## Stamps and seals (M6)

| Paper | Venue | Link | Used for |
|---|---|---|---|
| B. Micenková, J. van Beusekom, "Stamp detection in color document images" | ICDAR 2011, pp. 1125–1129 | [doi:10.1109/ICDAR.2011.227](https://doi.org/10.1109/ICDAR.2011.227) | Colour-cluster stamp detection; the StaVer dataset |
| G. Zhu, S. Jaeger, D. Doermann, "A robust stamp detection framework on degraded documents" | Proc. SPIE 6067, 60670B, 2006 | [doi:10.1117/12.643537](https://doi.org/10.1117/12.643537) | Shape path for black or photocopied stamps |
| P. P. Roy, U. Pal, J. Lladós, "Document seal detection using GHT and character proximity graphs" | Pattern Recognition 44(6):1282–1295, 2011 | [author PDF](https://www.isical.ac.in/~umapada/pdf_papers/Partha_PR.pdf) | Generalised-Hough seal matching (Phase 4, optional) |
| Y. Bhalgat, M. Kulkarni, S. Karande, S. Lodha, "Stamp processing with examplar features" | arXiv 1609.05001 | [arXiv:1609.05001](https://arxiv.org/abs/1609.05001) | Benchmark comparison and design reference only (learned patch codebook) |
| D. H. Ballard, "Generalizing the Hough transform to detect arbitrary shapes" | Pattern Recognition 13(2), 1981 | — | Generalised Hough transform |

## Signatures (M7)

| Paper | Venue | Link | Used for |
|---|---|---|---|
| R. Mandal, P. P. Roy, U. Pal, M. Blumenstein, "Bag-of-visual-words for signature-based multi-script document retrieval" | arXiv 1807.06772; Neural Computing and Applications, 2019 | [arXiv:1807.06772](https://arxiv.org/abs/1807.06772), [doi:10.1007/s00521-018-3444-y](https://doi.org/10.1007/s00521-018-3444-y) | Deterministic foreground/background features (loops, water reservoirs); the paper itself used SIFT BoVW plus an SVM |
| G. Zhu, Y. Zheng, D. Doermann, S. Jaeger, "Multi-scale structural saliency for signature detection" | CVPR 2007 | — | Curvilinear-stroke saliency prior |

## Thumb impressions (M8)

| Paper | Venue | Link | Used for |
|---|---|---|---|
| A. M. Bazen, S. H. Gerez, "Segmentation of fingerprint images" | ProRISC 2001, pp. 276–280 | — | Coherence, mean and variance with a linear classifier |

## Page layout (M10)

| Paper | Venue | Link | Used for |
|---|---|---|---|
| K. Y. Wong, R. G. Casey, F. M. Wahl, "Document analysis system" | IBM J. Research and Development 26(6), 1982 | — | RLSA |
| G. Nagy, S. Seth, "Hierarchical representation of optically scanned documents" | ICPR 1984, pp. 347–349 | — | Recursive XY-cut |
| L. O'Gorman, "The document spectrum for page layout analysis" | IEEE TPAMI 15(11), 1993 | — | Docstrum, for skewed text |

## PDF structure (M1, M2, M12)

| Source | Venue | Link | Used for |
|---|---|---|---|
| ISO 32000-1:2008 (PDF 1.7) and ISO 32000-2:2020 | ISO | — | Content-stream operators, text render modes, `ToUnicode` CMaps, cross-reference tables and streams, incremental updates (*unverified*: section numbers are not cited because they were recalled, not re-read) |
| ITU-T T.81 / ISO/IEC 10918-1, "Digital compression and coding of continuous-tone still images" | 1992 | — | JPEG marker segments and the Annex K example quantisation tables |
| Independent JPEG Group, libjpeg `jpeg_set_quality` / `jpeg_quality_scaling` | IJG libjpeg 6b and later | — | The quality-to-quantisation-table scaling that the quality estimate inverts. Checked against Pillow's (libjpeg) output for qualities 1 to 100 |
| `lopdf` (J.-F. Liu et al.), MIT | crates.io 0.46 | [github.com/J-F-Liu/lopdf](https://github.com/J-F-Liu/lopdf) | PDF object parsing, stream decoding with a size cap; the only PDF parser dependency (behind the `pdf` feature) |
| `pdf-inspector` (Firecrawl), MIT | crates.io 1.25.2 | [github.com/firecrawl/pdf-inspector](https://github.com/firecrawl/pdf-inspector) | Evaluated and **not** used in the core; compared in `bench/compare_pdf_inspector.py`. See `detectors/text_layer.md` |

## Provenance (M12)

| Paper | Venue | Link | Used for |
|---|---|---|---|
| H. Farid, JPEG quantisation-table ballistics (Dartmouth technical reports) | 2006–2008 | — | Scanner and encoder family from JPEG tables (*unverified*: exact citation not recorded; confirm before quoting) |
| C. Zauner, "Implementation and Benchmarking of Perceptual Image Hash Functions" | Master's thesis, Upper Austria University of Applied Sciences, Hagenberg, 2010 | — | The DCT perceptual hash (pHash) (*unverified*: recalled from memory, not re-checked) |
| N. Krawetz, "Kind of Like That", The Hacker Factor Blog | 2013 | — | The difference hash (dHash) (*unverified*: recalled from memory, not re-checked) |
| N. Ahmed, T. Natarajan, K. R. Rao, "Discrete cosine transform" | IEEE Trans. Computers C-23(1):90–93, 1974 | — | The DCT-II behind pHash (*unverified*: recalled from memory, not re-checked) |

## Synthetic degradation (test data)

| Paper | Venue | Link | Used for |
|---|---|---|---|
| Groleau et al., Augraphy | ICDAR 2023, arXiv 2208.14558 | [arXiv:2208.14558](https://arxiv.org/abs/2208.14558) | Generating degraded synthetic deed pages with exact ground truth |
