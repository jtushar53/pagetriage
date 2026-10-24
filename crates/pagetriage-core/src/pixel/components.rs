//! 8-connected component labelling of a binary mask, with a union-find over provisional labels.
//!
//! Deterministic by construction: the scan order is fixed (raster), a union always keeps the
//! smaller label as the root, and final labels are numbered in raster order of each component's
//! first pixel. All arithmetic is integer, so every target gives the same labels and statistics.
//!
//! Method: two-pass connected-component labelling with equivalence classes resolved by union-find
//! (A. Rosenfeld and J. L. Pfaltz, "Sequential operations in digital picture processing", J. ACM
//! 13(4):471-494, 1966; the 8-neighbour mask used here is the four already-visited neighbours
//! W, NW, N, NE).

/// Statistics of one connected component. Bounding-box corners are inclusive pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Component {
    pub area: u32,
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
    pub sum_x: u64,
    pub sum_y: u64,
}

impl Component {
    pub fn width(&self) -> u32 {
        self.x1 - self.x0 + 1
    }

    pub fn height(&self) -> u32 {
        self.y1 - self.y0 + 1
    }

    /// Fraction of the bounding box that is ink.
    pub fn fill(&self) -> f64 {
        f64::from(self.area) / (f64::from(self.width()) * f64::from(self.height()))
    }
}

/// A labelled mask. `labels[i]` is 0 for background and `k + 1` for `components[k]`.
#[derive(Debug, Clone)]
pub struct Labelling {
    pub width: usize,
    pub height: usize,
    pub labels: Vec<u32>,
    pub components: Vec<Component>,
}

fn find(parent: &mut [u32], mut i: u32) -> u32 {
    // Path halving.
    while parent[i as usize] != i {
        let grand = parent[parent[i as usize] as usize];
        parent[i as usize] = grand;
        i = grand;
    }
    i
}

fn union(parent: &mut [u32], a: u32, b: u32) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        // The smaller label wins, so the root of a class is always its first-created label.
        let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
        parent[hi as usize] = lo;
    }
}

/// Label the 8-connected components of `mask` (non-zero is foreground), `width * height`
/// row-major.
pub fn label_8(mask: &[u8], width: usize, height: usize) -> Labelling {
    debug_assert_eq!(mask.len(), width * height);
    let mut labels = vec![0u32; mask.len()];
    // parent[0] is the background sentinel; provisional labels start at 1.
    let mut parent: Vec<u32> = vec![0];

    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            if mask[i] == 0 {
                continue;
            }
            let w = if x > 0 { labels[i - 1] } else { 0 };
            let (nw, n, ne) = if y > 0 {
                let up = i - width;
                (
                    if x > 0 { labels[up - 1] } else { 0 },
                    labels[up],
                    if x + 1 < width { labels[up + 1] } else { 0 },
                )
            } else {
                (0, 0, 0)
            };
            let mut label = 0u32;
            for l in [w, nw, n, ne] {
                if l != 0 {
                    if label == 0 {
                        label = l;
                    } else if l != label {
                        union(&mut parent, label, l);
                        // Keep the smaller one as the working label.
                        label = label.min(l);
                    }
                }
            }
            if label == 0 {
                label = parent.len() as u32;
                parent.push(label);
            }
            labels[i] = label;
        }
    }

    // Resolve every provisional label to a final number, in order of the root's creation.
    let mut final_of = vec![0u32; parent.len()];
    let mut count = 0u32;
    for l in 1..parent.len() as u32 {
        let root = find(&mut parent, l);
        if root == l {
            count += 1;
            final_of[l as usize] = count;
        } else {
            // Roots are smaller than their members, so the root's number is already set.
            final_of[l as usize] = final_of[root as usize];
        }
    }

    let mut components = vec![
        Component {
            area: 0,
            x0: u32::MAX,
            y0: u32::MAX,
            x1: 0,
            y1: 0,
            sum_x: 0,
            sum_y: 0,
        };
        count as usize
    ];
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            let l = labels[i];
            if l == 0 {
                continue;
            }
            let f = final_of[l as usize];
            labels[i] = f;
            let c = &mut components[(f - 1) as usize];
            let (xu, yu) = (x as u32, y as u32);
            c.area += 1;
            c.x0 = c.x0.min(xu);
            c.y0 = c.y0.min(yu);
            c.x1 = c.x1.max(xu);
            c.y1 = c.y1.max(yu);
            c.sum_x += u64::from(xu);
            c.sum_y += u64::from(yu);
        }
    }

    Labelling {
        width,
        height,
        labels,
        components,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn from_rows(rows: &[&str]) -> (Vec<u8>, usize, usize) {
        let h = rows.len();
        let w = rows[0].len();
        let mask = rows
            .iter()
            .flat_map(|r| r.bytes().map(|b| u8::from(b == b'#')))
            .collect();
        (mask, w, h)
    }

    #[test]
    fn diagonal_neighbours_connect_under_8_connectivity() {
        let (m, w, h) = from_rows(&["#..", ".#.", "..#"]);
        let l = label_8(&m, w, h);
        assert_eq!(l.components.len(), 1);
        assert_eq!(l.components[0].area, 3);
    }

    #[test]
    fn separate_blobs_get_raster_order_labels_and_exact_stats() {
        let (m, w, h) = from_rows(&["##....#", "##....#", "......#", ".###...", ".#.#..."]);
        let l = label_8(&m, w, h);
        assert_eq!(l.components.len(), 3);
        assert_eq!(l.labels[0], 1);
        assert_eq!(l.labels[6], 2);
        assert_eq!(l.labels[3 * w + 1], 3);
        let a = l.components[0];
        assert_eq!((a.area, a.x0, a.y0, a.x1, a.y1), (4, 0, 0, 1, 1));
        assert_eq!((a.sum_x, a.sum_y), (2, 2));
        let b = l.components[1];
        assert_eq!((b.area, b.width(), b.height()), (3, 1, 3));
        let c = l.components[2];
        assert_eq!((c.area, c.x0, c.y0, c.x1, c.y1), (5, 1, 3, 3, 4));
    }

    #[test]
    fn u_shape_merges_when_the_arms_meet_late() {
        // The two arms get different provisional labels and join along the bottom row.
        let (m, w, h) = from_rows(&["#...#", "#...#", "#####"]);
        let l = label_8(&m, w, h);
        assert_eq!(l.components.len(), 1);
        assert_eq!(l.components[0].area, 9);
        assert!(l.labels.iter().all(|&v| v <= 1));
    }

    #[test]
    fn ne_neighbour_links_a_late_arriving_branch() {
        // Pixel (2,1) touches (3,0) only through its NE neighbour.
        let (m, w, h) = from_rows(&["#..#", ".##."]);
        let l = label_8(&m, w, h);
        assert_eq!(l.components.len(), 1);
    }

    #[test]
    fn empty_and_full_masks() {
        let l = label_8(&[0u8; 12], 4, 3);
        assert!(l.components.is_empty());
        let l = label_8(&[1u8; 12], 4, 3);
        assert_eq!(l.components.len(), 1);
        assert_eq!(l.components[0].area, 12);
        assert_eq!(l.components[0].fill(), 1.0);
    }

    #[test]
    fn checkerboard_is_one_component_under_8_connectivity() {
        let mask: Vec<u8> = (0..64)
            .map(|i| u8::from((i / 8 + i % 8) % 2 == 0))
            .collect();
        let l = label_8(&mask, 8, 8);
        assert_eq!(l.components.len(), 1);
        assert_eq!(l.components[0].area, 32);
    }

    #[test]
    fn labelling_is_repeatable() {
        let mask: Vec<u8> = (0..400u32)
            .map(|i| u8::from(i.wrapping_mul(2_654_435_761) >> 29 & 1 == 1))
            .collect();
        let a = label_8(&mask, 20, 20);
        let b = label_8(&mask, 20, 20);
        assert_eq!(a.labels, b.labels);
        assert_eq!(a.components, b.components);
    }
}
