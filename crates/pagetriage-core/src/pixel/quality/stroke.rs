//! Distance transform and stroke ridges of the binarised text.
//!
//! The 3-4 chamfer transform (G. Borgefors, "Distance transformations in digital images", CVGIP
//! 34(3):344-371, 1986) is two integer raster passes; a pixel's value is three times its distance
//! to the nearest paper pixel, accurate to about 8%. Ridge pixels (local maxima of the transform)
//! form a one-pixel-wide centre line of each stroke, and twice the ridge value gives the stroke
//! width. Both the skew estimate and the photocopy-generation estimate use them.

/// Distance value standing for "far away" (a component that never meets paper).
const FAR: u16 = 30_000;

/// Chamfer units per pixel of axis-aligned distance.
pub const UNITS_PER_PX: u16 = 3;

const STEP_AXIS: u16 = 3;
const STEP_DIAGONAL: u16 = 4;

/// 3-4 chamfer distance of every ink pixel (`mask != 0`) to the nearest paper pixel. Paper is 0.
/// Outside the page counts as ink, so a stroke cut by the page edge keeps its thickness.
pub fn chamfer_distance(mask: &[u8], width: usize, height: usize) -> Vec<u16> {
    debug_assert_eq!(mask.len(), width * height);
    let mut d: Vec<u16> = mask.iter().map(|&m| if m != 0 { FAR } else { 0 }).collect();
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            if d[i] == 0 {
                continue;
            }
            let mut best = d[i];
            if x > 0 {
                best = best.min(d[i - 1].saturating_add(STEP_AXIS));
            }
            if y > 0 {
                let up = i - width;
                best = best.min(d[up].saturating_add(STEP_AXIS));
                if x > 0 {
                    best = best.min(d[up - 1].saturating_add(STEP_DIAGONAL));
                }
                if x + 1 < width {
                    best = best.min(d[up + 1].saturating_add(STEP_DIAGONAL));
                }
            }
            d[i] = best;
        }
    }
    for y in (0..height).rev() {
        for x in (0..width).rev() {
            let i = y * width + x;
            if d[i] == 0 {
                continue;
            }
            let mut best = d[i];
            if x + 1 < width {
                best = best.min(d[i + 1].saturating_add(STEP_AXIS));
            }
            if y + 1 < height {
                let down = i + width;
                best = best.min(d[down].saturating_add(STEP_AXIS));
                if x > 0 {
                    best = best.min(d[down - 1].saturating_add(STEP_DIAGONAL));
                }
                if x + 1 < width {
                    best = best.min(d[down + 1].saturating_add(STEP_DIAGONAL));
                }
            }
            d[i] = best;
        }
    }
    d
}

/// True where `d` is positive and at least every 8-neighbour (a ridge or a plateau of one).
pub fn is_ridge(d: &[u16], width: usize, height: usize, x: usize, y: usize) -> bool {
    let v = d[y * width + x];
    if v == 0 || v >= FAR {
        return false;
    }
    let x0 = x.saturating_sub(1);
    let x1 = (x + 1).min(width - 1);
    let y0 = y.saturating_sub(1);
    let y1 = (y + 1).min(height - 1);
    for yy in y0..=y1 {
        for xx in x0..=x1 {
            if d[yy * width + xx] > v {
                return false;
            }
        }
    }
    true
}

/// Stroke width in pixels from a ridge value: the ridge sits at half the width, and a pixel's
/// distance to the nearest paper pixel is half the width plus a half pixel.
pub fn width_px(ridge: u16) -> f64 {
    2.0 * f64::from(ridge) / f64::from(UNITS_PER_PX) - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(w: usize, h: usize, thickness: usize) -> Vec<u8> {
        let mut m = vec![0u8; w * h];
        let top = (h - thickness) / 2;
        for y in top..top + thickness {
            for x in 2..w - 2 {
                m[y * w + x] = 1;
            }
        }
        m
    }

    #[test]
    fn distance_of_a_bar_peaks_at_its_centre_line() {
        let m = bar(30, 15, 5);
        let d = chamfer_distance(&m, 30, 15);
        // Rows 5..10 are ink; row 7 is the centre, 3 px from paper (3 units per pixel).
        assert_eq!(d[7 * 30 + 15], 9);
        assert_eq!(d[5 * 30 + 15], 3);
        assert_eq!(d[0], 0);
        assert!(is_ridge(&d, 30, 15, 15, 7));
        assert!(!is_ridge(&d, 30, 15, 15, 5));
    }

    #[test]
    fn stroke_width_recovers_odd_widths_exactly() {
        for t in [1usize, 3, 5, 7] {
            let m = bar(40, 21, t);
            let d = chamfer_distance(&m, 40, 21);
            let centre = d[10 * 40 + 20];
            assert!((width_px(centre) - t as f64).abs() < 1e-9, "t={t}");
        }
    }

    #[test]
    fn distance_is_never_below_the_axis_distance_to_paper() {
        let mut m = vec![1u8; 11 * 11];
        m[5 * 11 + 5] = 0;
        let d = chamfer_distance(&m, 11, 11);
        assert_eq!(d[5 * 11 + 5], 0);
        assert_eq!(d[5 * 11 + 6], 3);
        assert_eq!(d[6 * 11 + 6], 4);
        assert_eq!(d[5 * 11 + 8], 9);
    }

    #[test]
    fn all_ink_page_is_far_everywhere_and_has_no_ridge() {
        let d = chamfer_distance(&[1u8; 16], 4, 4);
        assert!(d.iter().all(|&v| v == FAR));
        assert!(!is_ridge(&d, 4, 4, 1, 1));
    }
}
