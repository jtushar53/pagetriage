//! Module M1 `structural.text_layer`: the per-page text-layer state, decided from the counts the
//! content-stream interpreter measured. This file is pure (no PDF parsing), so the rules are
//! unit-testable with hand-made observations.
//!
//! Rules, in order (all thresholds in `params/text_layer.toml`):
//!
//! * fewer than `min_glyphs` glyphs: no text layer. Image coverage >= `raster_min_cover_fraction`
//!   is `raster_only`; many path-painting operators is `vector_only`; otherwise `empty`.
//! * hidden glyphs (render mode 3/7, or covered by a later opaque image) >= `hidden_glyph_fraction`:
//!   `invisible_ocr_under_image` if an image covers at least `raster_min_cover_fraction`, else the
//!   layer has nothing to sit under and the page is `vector_only` or `empty`.
//! * visible glyphs >= `hidden_glyph_fraction`: `mixed` if images cover at least
//!   `mixed_image_cover_fraction`, else `native_text`.
//! * anything in between: `mixed`.
//!
//! Confidence is `sigmoid(base_logit + sum(llr))` over the evidence the rule used, each term
//! `+-weight` (placeholder weights, see the params file).

use super::params::TextLayerParams;
use crate::error::Result;
use crate::math;
use crate::schema::{Evidence, TextLayer, TextLayerState};

const DETECTOR_ID: &str = "structural.text_layer";
const CITATION: &str = "module M1: text render mode 3 over >=90% of glyphs; image Do covering >=85% of the page box; glyph/image overlap; producer fingerprint";

/// What the interpreter measured on one page.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PageObservation {
    pub glyphs: u32,
    pub glyphs_invisible_mode: u32,
    pub glyphs_under_image: u32,
    pub glyphs_in_image: u32,
    pub text_ops: u32,
    pub image_cover: f64,
    pub image_count: u32,
    pub paint_ops: u32,
    pub small_filled_paths: u32,
    pub truncated: bool,
}

impl PageObservation {
    pub fn hidden_glyphs(&self) -> u32 {
        self.glyphs_invisible_mode
            .saturating_add(self.glyphs_under_image)
    }

    pub fn hidden_fraction(&self) -> f64 {
        ratio(self.hidden_glyphs(), self.glyphs)
    }

    pub fn overlap_fraction(&self) -> f64 {
        ratio(self.glyphs_in_image, self.glyphs)
    }
}

fn ratio(a: u32, b: u32) -> f64 {
    if b == 0 {
        0.0
    } else {
        f64::from(a.min(b)) / f64::from(b)
    }
}

/// Document-level facts a page decision may use.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocContext {
    /// The producer or creator names a text-layer (OCR) tool.
    pub ocr_producer: bool,
}

struct Builder<'p> {
    p: &'p TextLayerParams,
    evidence: Vec<Evidence>,
    logit: f64,
}

impl Builder<'_> {
    fn add(
        &mut self,
        feature: &str,
        value: f64,
        threshold: Option<f64>,
        llr: f64,
        note: String,
    ) -> Result<()> {
        self.logit += llr;
        self.evidence.push(Evidence::new(
            DETECTOR_ID,
            feature,
            value,
            threshold,
            llr,
            note,
            CITATION,
        )?);
        Ok(())
    }

    fn glyph_count(&mut self, obs: &PageObservation, want_text: bool) -> Result<()> {
        let have = obs.glyphs >= self.p.min_glyphs;
        let w = self.p.weights.glyph_count;
        self.add(
            "glyph_count",
            f64::from(obs.glyphs),
            Some(f64::from(self.p.min_glyphs)),
            if have == want_text { w } else { -w },
            if want_text {
                format!("{} non-space glyphs on the page", obs.glyphs)
            } else if obs.glyphs == 0 {
                "no text-showing operators produced glyphs".to_owned()
            } else {
                format!(
                    "only {} stray glyphs, below the {} minimum",
                    obs.glyphs, self.p.min_glyphs
                )
            },
        )
    }

    /// Image coverage as evidence for "a scan fills the page": `-w` at the raster floor, `+w` at
    /// the 85% threshold, linear between.
    fn image_cover_supports(&mut self, cov: f64) -> Result<()> {
        let (lo, hi) = (
            self.p.raster_min_cover_fraction,
            self.p.image_cover_fraction,
        );
        let t = if hi > lo {
            ((cov - lo) / (hi - lo)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let w = self.p.weights.image_cover;
        self.add(
            "image_cover",
            cov,
            Some(hi),
            -w + 2.0 * w * t,
            format!(
                "images cover {:.1}% of the page box (full at {:.0}%, floor {:.0}%)",
                cov * 100.0,
                hi * 100.0,
                lo * 100.0
            ),
        )
    }
}

/// Decide the text-layer state of one page.
pub fn decide(obs: &PageObservation, ctx: &DocContext, p: &TextLayerParams) -> Result<TextLayer> {
    let hidden = obs.hidden_fraction();
    let cov = obs.image_cover;
    let has_text = obs.glyphs >= p.min_glyphs;
    let w = &p.weights;
    let mut b = Builder {
        p,
        evidence: Vec::new(),
        logit: w.base_logit,
    };

    let state = if !has_text {
        if cov >= p.raster_min_cover_fraction {
            b.image_cover_supports(cov)?;
            b.glyph_count(obs, false)?;
            if ctx.ocr_producer {
                b.add(
                    "producer_ocr",
                    1.0,
                    None,
                    0.0,
                    "the document's producer is an OCR tool but this page has no text layer"
                        .to_owned(),
                )?;
            }
            TextLayerState::RasterOnly
        } else if obs.small_filled_paths >= p.outlined_text_min_paths
            || obs.paint_ops >= p.vector_min_paint_ops
        {
            vector_evidence(&mut b, obs, cov)?;
            TextLayerState::VectorOnly
        } else {
            empty_evidence(&mut b, obs, cov)?;
            TextLayerState::Empty
        }
    } else if hidden >= p.hidden_glyph_fraction {
        if cov >= p.raster_min_cover_fraction {
            b.glyph_count(obs, true)?;
            b.add(
                "hidden_glyph_fraction",
                hidden,
                Some(p.hidden_glyph_fraction),
                w.hidden_fraction,
                format!(
                    "{} of {} glyphs are hidden: {} in render mode 3/7, {} under a later opaque image",
                    obs.hidden_glyphs(),
                    obs.glyphs,
                    obs.glyphs_invisible_mode,
                    obs.glyphs_under_image
                ),
            )?;
            b.image_cover_supports(cov)?;
            let overlap = obs.overlap_fraction();
            let ok = overlap >= p.glyph_overlap_fraction;
            b.add(
                "glyph_image_overlap",
                overlap,
                Some(p.glyph_overlap_fraction),
                if ok {
                    w.glyph_overlap
                } else {
                    -w.glyph_overlap
                },
                format!(
                    "{:.0}% of glyph origins lie inside an image box",
                    overlap * 100.0
                ),
            )?;
            b.add(
                "producer_ocr",
                f64::from(u8::from(ctx.ocr_producer)),
                Some(1.0),
                if ctx.ocr_producer {
                    w.producer_ocr
                } else {
                    0.0
                },
                if ctx.ocr_producer {
                    "producer or creator names an OCR tool".to_owned()
                } else {
                    "no OCR producer fingerprint (no penalty: many OCR tools leave none)".to_owned()
                },
            )?;
            TextLayerState::InvisibleOcrUnderImage
        } else if obs.paint_ops >= p.vector_min_paint_ops {
            b.add(
                "hidden_glyph_fraction",
                hidden,
                Some(p.hidden_glyph_fraction),
                w.hidden_fraction,
                "all text is hidden and no image lies under it; vector content only".to_owned(),
            )?;
            vector_evidence(&mut b, obs, cov)?;
            TextLayerState::VectorOnly
        } else {
            b.add(
                "hidden_glyph_fraction",
                hidden,
                Some(p.hidden_glyph_fraction),
                w.hidden_fraction,
                "all text is hidden and nothing visible is on the page".to_owned(),
            )?;
            empty_evidence(&mut b, obs, cov)?;
            TextLayerState::Empty
        }
    } else if hidden <= 1.0 - p.hidden_glyph_fraction {
        b.glyph_count(obs, true)?;
        b.add(
            "hidden_glyph_fraction",
            hidden,
            Some(1.0 - p.hidden_glyph_fraction),
            w.hidden_fraction,
            format!("{:.0}% of glyphs are visible", (1.0 - hidden) * 100.0),
        )?;
        if cov >= p.mixed_image_cover_fraction {
            b.add(
                "image_cover",
                cov,
                Some(p.mixed_image_cover_fraction),
                w.mixed,
                format!(
                    "visible text together with images covering {:.1}% of the page",
                    cov * 100.0
                ),
            )?;
            TextLayerState::Mixed
        } else {
            b.add(
                "image_cover",
                cov,
                Some(p.mixed_image_cover_fraction),
                w.no_image,
                format!("images cover only {:.1}% of the page", cov * 100.0),
            )?;
            TextLayerState::NativeText
        }
    } else {
        b.glyph_count(obs, true)?;
        b.add(
            "hidden_glyph_fraction",
            hidden,
            Some(p.hidden_glyph_fraction),
            w.mixed,
            format!(
                "{:.0}% of glyphs are hidden: part visible text, part hidden layer",
                hidden * 100.0
            ),
        )?;
        TextLayerState::Mixed
    };

    if obs.truncated {
        b.add(
            "truncated",
            1.0,
            None,
            0.0,
            "content was cut short by a limit; counts are lower bounds".to_owned(),
        )?;
    }

    let confidence = math::sigmoid(b.logit);
    Ok(TextLayer {
        state,
        confidence,
        reliable: None,
        evidence: b.evidence,
    })
}

fn vector_evidence(b: &mut Builder<'_>, obs: &PageObservation, cov: f64) -> Result<()> {
    let p = b.p;
    let w = &p.weights;
    let paints = obs.paint_ops >= p.vector_min_paint_ops;
    b.add(
        "paint_ops",
        f64::from(obs.paint_ops),
        Some(f64::from(p.vector_min_paint_ops)),
        if paints { w.paint_ops } else { -w.paint_ops },
        format!("{} path-painting operators", obs.paint_ops),
    )?;
    let outlined = obs.small_filled_paths >= p.outlined_text_min_paths;
    b.add(
        "glyph_like_paths",
        f64::from(obs.small_filled_paths),
        Some(f64::from(p.outlined_text_min_paths)),
        if outlined { w.vector_density } else { 0.0 },
        if outlined {
            "many small filled outlines: text drawn as paths is suspected (needs OCR)".to_owned()
        } else {
            "few small filled outlines".to_owned()
        },
    )?;
    b.add(
        "image_cover",
        cov,
        Some(p.raster_min_cover_fraction),
        if cov < p.raster_min_cover_fraction {
            w.no_image
        } else {
            -w.no_image
        },
        format!("images cover {:.1}% of the page", cov * 100.0),
    )?;
    b.glyph_count(obs, false)
}

fn empty_evidence(b: &mut Builder<'_>, obs: &PageObservation, cov: f64) -> Result<()> {
    let p = b.p;
    let w = &p.weights;
    b.glyph_count(obs, false)?;
    b.add(
        "image_cover",
        cov,
        Some(p.raster_min_cover_fraction),
        if cov < p.raster_min_cover_fraction {
            w.no_image
        } else {
            -w.no_image
        },
        format!("images cover {:.1}% of the page", cov * 100.0),
    )?;
    let few = obs.paint_ops < p.vector_min_paint_ops;
    b.add(
        "paint_ops",
        f64::from(obs.paint_ops),
        Some(f64::from(p.vector_min_paint_ops)),
        if few { w.paint_ops } else { -w.paint_ops },
        format!("{} path-painting operators", obs.paint_ops),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structural::params::StructuralParams;

    fn p() -> TextLayerParams {
        StructuralParams::defaults().unwrap().text_layer
    }

    fn run(obs: PageObservation, ocr: bool) -> TextLayer {
        decide(&obs, &DocContext { ocr_producer: ocr }, &p()).unwrap()
    }

    fn base() -> PageObservation {
        PageObservation::default()
    }

    #[test]
    fn native_text_page() {
        let t = run(
            PageObservation {
                glyphs: 1800,
                text_ops: 60,
                image_cover: 0.0,
                ..base()
            },
            false,
        );
        assert_eq!(t.state, TextLayerState::NativeText);
        assert!(t.confidence > 0.95, "{}", t.confidence);
        assert!(
            t.evidence
                .iter()
                .all(|e| e.detector == "structural.text_layer")
        );
    }

    #[test]
    fn hidden_text_under_a_covering_image_is_ocr_under_image() {
        let obs = PageObservation {
            glyphs: 1500,
            glyphs_invisible_mode: 1500,
            glyphs_in_image: 1500,
            image_cover: 1.0,
            image_count: 1,
            ..base()
        };
        let t = run(obs, true);
        assert_eq!(t.state, TextLayerState::InvisibleOcrUnderImage);
        assert!(t.confidence > 0.99);
        // Without the producer fingerprint it is still decided, with a little less confidence.
        let t2 = run(obs, false);
        assert_eq!(t2.state, TextLayerState::InvisibleOcrUnderImage);
        assert!(t2.confidence < t.confidence);
    }

    #[test]
    fn text_drawn_under_a_later_image_counts_as_hidden() {
        let t = run(
            PageObservation {
                glyphs: 900,
                glyphs_under_image: 900,
                glyphs_in_image: 900,
                image_cover: 0.99,
                image_count: 1,
                ..base()
            },
            false,
        );
        assert_eq!(t.state, TextLayerState::InvisibleOcrUnderImage);
    }

    #[test]
    fn scan_without_text_is_raster_only_and_an_ocr_producer_is_noted() {
        let obs = PageObservation {
            image_cover: 1.0,
            image_count: 1,
            ..base()
        };
        let t = run(obs, true);
        assert_eq!(t.state, TextLayerState::RasterOnly);
        assert!(
            t.evidence
                .iter()
                .any(|e| e.feature == "producer_ocr" && e.llr == 0.0)
        );
        assert!(t.confidence > 0.95);
    }

    #[test]
    fn partial_cover_lowers_confidence() {
        let full = run(
            PageObservation {
                image_cover: 0.95,
                image_count: 1,
                ..base()
            },
            false,
        );
        let part = run(
            PageObservation {
                image_cover: 0.50,
                image_count: 1,
                ..base()
            },
            false,
        );
        assert_eq!(part.state, TextLayerState::RasterOnly);
        assert!(part.confidence < full.confidence);
    }

    #[test]
    fn visible_text_over_a_large_image_is_mixed() {
        let t = run(
            PageObservation {
                glyphs: 500,
                image_cover: 0.6,
                image_count: 1,
                ..base()
            },
            false,
        );
        assert_eq!(t.state, TextLayerState::Mixed);
        let half = run(
            PageObservation {
                glyphs: 400,
                glyphs_invisible_mode: 200,
                image_cover: 0.0,
                ..base()
            },
            false,
        );
        assert_eq!(half.state, TextLayerState::Mixed);
    }

    #[test]
    fn small_photo_next_to_text_stays_native() {
        let t = run(
            PageObservation {
                glyphs: 800,
                image_cover: 0.1,
                image_count: 1,
                ..base()
            },
            false,
        );
        assert_eq!(t.state, TextLayerState::NativeText);
    }

    #[test]
    fn vector_only_and_outlined_text_guard() {
        let rules = run(
            PageObservation {
                paint_ops: 40,
                ..base()
            },
            false,
        );
        assert_eq!(rules.state, TextLayerState::VectorOnly);
        let outlined = run(
            PageObservation {
                paint_ops: 5000,
                small_filled_paths: 4000,
                ..base()
            },
            false,
        );
        assert_eq!(outlined.state, TextLayerState::VectorOnly);
        assert!(
            outlined
                .evidence
                .iter()
                .any(|e| e.feature == "glyph_like_paths" && e.llr > 0.0)
        );
        assert!(
            rules
                .evidence
                .iter()
                .any(|e| e.feature == "glyph_like_paths" && e.llr == 0.0),
            "plain vector art is not flagged as outlined text"
        );
    }

    #[test]
    fn empty_page_and_stray_marks() {
        assert_eq!(run(base(), false).state, TextLayerState::Empty);
        let stray = run(
            PageObservation {
                glyphs: 1,
                text_ops: 1,
                ..base()
            },
            false,
        );
        assert_eq!(stray.state, TextLayerState::Empty);
        let tiny_logo = run(
            PageObservation {
                image_cover: 0.02,
                image_count: 1,
                ..base()
            },
            false,
        );
        assert_eq!(tiny_logo.state, TextLayerState::Empty);
    }

    #[test]
    fn hidden_text_with_nothing_beneath_is_not_an_ocr_layer() {
        let t = run(
            PageObservation {
                glyphs: 300,
                glyphs_invisible_mode: 300,
                ..base()
            },
            false,
        );
        assert_eq!(t.state, TextLayerState::Empty);
    }

    #[test]
    fn scan_with_a_page_number_stamp_is_still_raster_only() {
        let t = run(
            PageObservation {
                glyphs: 2,
                image_cover: 1.0,
                image_count: 1,
                ..base()
            },
            false,
        );
        assert_eq!(t.state, TextLayerState::RasterOnly);
    }

    #[test]
    fn evidence_is_finite_and_confidence_in_unit_interval() {
        for g in [0u32, 1, 3, 50, 5000] {
            for inv in [0u32, 25, 100] {
                for cov in [0.0, 0.3, 0.9, 1.0] {
                    let glyphs = g;
                    let obs = PageObservation {
                        glyphs,
                        glyphs_invisible_mode: glyphs * inv / 100,
                        image_cover: cov,
                        ..base()
                    };
                    let t = run(obs, false);
                    assert!((0.0..=1.0).contains(&t.confidence));
                    assert!(
                        t.evidence
                            .iter()
                            .all(|e| e.value.is_finite() && e.llr.is_finite())
                    );
                }
            }
        }
    }
}
