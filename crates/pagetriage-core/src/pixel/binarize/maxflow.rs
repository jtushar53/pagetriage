//! Min-cut on a 4-connected pixel grid by the Boykov-Kolmogorov augmenting-path algorithm.
//!
//! Source: Y. Boykov and V. Kolmogorov, "An experimental comparison of min-cut/max-flow algorithms
//! for energy minimization in vision", IEEE TPAMI 26(9):1124-1137, 2004. This is a from-scratch
//! implementation of the algorithm as published, specialised to grids: arcs are the four
//! neighbours of each pixel, kept as four residual capacities per node, instead of a general arc
//! list. Capacities are integers, so the cut is exact and identical on every target.
//!
//! Conventions: the source side is "ink" (label 1), the sink side is "paper" (label 0). A pixel
//! with `terminal > 0` is tied to the source with that capacity, and one with `terminal < 0` to the
//! sink with the opposite. Free pixels (reached by neither search tree at the end, which can go
//! either way at equal cost) are put on the sink side.

use std::collections::VecDeque;

/// Parent codes below 4 are the direction of the arc to the parent.
const TERMINAL: u8 = 4;
const ORPHAN: u8 = 5;
const NONE: u8 = 6;
const INFINITE_D: u32 = u32::MAX;

/// Directions: 0 = +x, 1 = -x, 2 = +y, 3 = -y. The reverse of `d` is `d ^ 1`.
#[derive(Clone, Copy, Default)]
struct Node {
    /// Residual capacity of source -> node when positive, of node -> sink (negated) when negative.
    tr_cap: i32,
    /// Residual capacity of the arc from this node in each direction.
    cap: [i32; 4],
    ts: u32,
    dist: u32,
    parent: u8,
    sink: bool,
    active: bool,
}

/// Result of [`min_cut`].
pub struct Cut {
    /// Value of the maximum flow, which equals the minimum cut. It excludes the constant part of
    /// the unary terms, since `terminal` holds only the difference of the two label costs.
    pub flow: i64,
    /// `true` for pixels on the source (ink) side, row-major.
    pub source_side: Vec<bool>,
}

/// Solve the min-cut of a `width * height` grid.
///
/// - `terminal[p]`: signed terminal capacity of pixel `p` (see the module docs).
/// - `right[p]`: capacity of the pair `p` -- `p + 1` (ignored on the last column).
/// - `down[p]`: capacity of the pair `p` -- `p + width` (ignored on the last row).
///
/// Pair capacities must be non-negative.
pub fn min_cut(width: usize, height: usize, terminal: &[i32], right: &[i32], down: &[i32]) -> Cut {
    let n = width * height;
    assert!(terminal.len() == n && right.len() == n && down.len() == n);
    debug_assert!(right.iter().chain(down).all(|&c| c >= 0));
    let mut solver = Solver::new(width, height, terminal, right, down);
    solver.run();
    let source_side = (0..n)
        .map(|p| {
            let node = &solver.nodes[solver.padded(p)];
            node.parent != NONE && !node.sink
        })
        .collect();
    Cut {
        flow: solver.flow,
        source_side,
    }
}

struct Solver {
    width: usize,
    nodes: Vec<Node>,
    offsets: [isize; 4],
    active: VecDeque<usize>,
    orphans: VecDeque<usize>,
    time: u32,
    flow: i64,
}

impl Solver {
    fn new(width: usize, height: usize, terminal: &[i32], right: &[i32], down: &[i32]) -> Self {
        // One node of padding all round, with no capacity, so neighbour arithmetic never leaves
        // the array and the search never enters the border.
        let stride = width + 2;
        let mut nodes = vec![
            Node {
                parent: NONE,
                ..Node::default()
            };
            stride * (height + 2)
        ];
        let mut active = VecDeque::new();
        for y in 0..height {
            for x in 0..width {
                let p = y * width + x;
                let i = (y + 1) * stride + x + 1;
                let tr = terminal[p];
                nodes[i].tr_cap = tr;
                if tr != 0 {
                    nodes[i].parent = TERMINAL;
                    nodes[i].sink = tr < 0;
                    nodes[i].dist = 1;
                    nodes[i].active = true;
                    active.push_back(i);
                }
                if x + 1 < width && right[p] > 0 {
                    nodes[i].cap[0] = right[p];
                    nodes[i + 1].cap[1] = right[p];
                }
                if y + 1 < height && down[p] > 0 {
                    nodes[i].cap[2] = down[p];
                    nodes[i + stride].cap[3] = down[p];
                }
            }
        }
        Self {
            width,
            nodes,
            offsets: [1, -1, stride as isize, -(stride as isize)],
            active,
            orphans: VecDeque::new(),
            time: 0,
            flow: 0,
        }
    }

    fn padded(&self, p: usize) -> usize {
        (p / self.width + 1) * (self.width + 2) + p % self.width + 1
    }

    fn nbr(&self, i: usize, d: u8) -> usize {
        (i as isize + self.offsets[usize::from(d)]) as usize
    }

    fn set_active(&mut self, i: usize) {
        if !self.nodes[i].active {
            self.nodes[i].active = true;
            self.active.push_back(i);
        }
    }

    fn next_active(&mut self) -> Option<usize> {
        while let Some(i) = self.active.pop_front() {
            self.nodes[i].active = false;
            // A node is active only while it belongs to a tree.
            if self.nodes[i].parent != NONE {
                return Some(i);
            }
        }
        None
    }

    fn set_orphan_front(&mut self, i: usize) {
        self.nodes[i].parent = ORPHAN;
        self.orphans.push_front(i);
    }

    fn set_orphan_rear(&mut self, i: usize) {
        self.nodes[i].parent = ORPHAN;
        self.orphans.push_back(i);
    }

    fn run(&mut self) {
        let mut current: Option<usize> = None;
        loop {
            // After an augmentation the same node is grown again before taking a new one.
            let mut node = current.take();
            if let Some(n) = node {
                self.nodes[n].active = false;
                if self.nodes[n].parent == NONE {
                    node = None;
                }
            }
            let i = match node.or_else(|| self.next_active()) {
                Some(i) => i,
                None => break,
            };
            let found = self.grow(i);
            self.time += 1;
            if let Some((u, d)) = found {
                self.nodes[i].active = true;
                current = Some(i);
                self.augment(u, d);
                while let Some(o) = self.orphans.pop_front() {
                    self.process_orphan(o);
                }
            }
        }
    }

    /// Grow the search tree through `i`. Returns the arc `(u, d)` from the source tree into the
    /// sink tree if one is found.
    fn grow(&mut self, i: usize) -> Option<(usize, u8)> {
        let sink = self.nodes[i].sink;
        for d in 0..4u8 {
            let j = self.nbr(i, d);
            // The arc that must have room: i -> j when growing from the source tree, j -> i when
            // growing from the sink tree.
            let residual = if sink {
                self.nodes[j].cap[usize::from(d ^ 1)]
            } else {
                self.nodes[i].cap[usize::from(d)]
            };
            if residual <= 0 {
                continue;
            }
            if self.nodes[j].parent == NONE {
                let (ts, dist) = (self.nodes[i].ts, self.nodes[i].dist);
                let nj = &mut self.nodes[j];
                nj.sink = sink;
                nj.parent = d ^ 1;
                nj.ts = ts;
                nj.dist = dist + 1;
                self.set_active(j);
            } else if self.nodes[j].sink != sink {
                return Some(if sink { (j, d ^ 1) } else { (i, d) });
            } else if self.nodes[j].ts <= self.nodes[i].ts
                && self.nodes[j].dist > self.nodes[i].dist
            {
                // Same tree: re-parent j through i if that makes its path to the root shorter.
                let (ts, dist) = (self.nodes[i].ts, self.nodes[i].dist);
                let nj = &mut self.nodes[j];
                nj.parent = d ^ 1;
                nj.ts = ts;
                nj.dist = dist + 1;
            }
        }
        None
    }

    /// Push the bottleneck flow along source ~> u -> v ~> sink, where `v` is `u`'s neighbour in
    /// direction `d`, and orphan every node whose tree link saturates.
    fn augment(&mut self, u: usize, d: u8) {
        let v = self.nbr(u, d);
        let mut bottleneck = self.nodes[u].cap[usize::from(d)];

        let mut x = u;
        while self.nodes[x].parent != TERMINAL {
            let p = self.nodes[x].parent;
            let y = self.nbr(x, p);
            bottleneck = bottleneck.min(self.nodes[y].cap[usize::from(p ^ 1)]);
            x = y;
        }
        let source_root = x;
        bottleneck = bottleneck.min(self.nodes[source_root].tr_cap);

        x = v;
        while self.nodes[x].parent != TERMINAL {
            let p = self.nodes[x].parent;
            bottleneck = bottleneck.min(self.nodes[x].cap[usize::from(p)]);
            x = self.nbr(x, p);
        }
        let sink_root = x;
        bottleneck = bottleneck.min(-self.nodes[sink_root].tr_cap);

        self.nodes[u].cap[usize::from(d)] -= bottleneck;
        self.nodes[v].cap[usize::from(d ^ 1)] += bottleneck;

        x = u;
        while x != source_root {
            let p = self.nodes[x].parent;
            let y = self.nbr(x, p);
            self.nodes[y].cap[usize::from(p ^ 1)] -= bottleneck;
            self.nodes[x].cap[usize::from(p)] += bottleneck;
            if self.nodes[y].cap[usize::from(p ^ 1)] == 0 {
                self.set_orphan_front(x);
            }
            x = y;
        }
        self.nodes[source_root].tr_cap -= bottleneck;
        if self.nodes[source_root].tr_cap == 0 {
            self.set_orphan_front(source_root);
        }

        x = v;
        while x != sink_root {
            let p = self.nodes[x].parent;
            let y = self.nbr(x, p);
            self.nodes[y].cap[usize::from(p ^ 1)] += bottleneck;
            self.nodes[x].cap[usize::from(p)] -= bottleneck;
            if self.nodes[x].cap[usize::from(p)] == 0 {
                self.set_orphan_front(x);
            }
            x = y;
        }
        self.nodes[sink_root].tr_cap += bottleneck;
        if self.nodes[sink_root].tr_cap == 0 {
            self.set_orphan_front(sink_root);
        }

        self.flow += i64::from(bottleneck);
    }

    /// Find a new parent for the orphan `i` in its own tree, or release it and its children.
    fn process_orphan(&mut self, i: usize) {
        let sink = self.nodes[i].sink;
        let mut best_d = INFINITE_D;
        let mut best_dir: Option<u8> = None;

        for d in 0..4u8 {
            let j = self.nbr(i, d);
            let residual = if sink {
                self.nodes[i].cap[usize::from(d)]
            } else {
                self.nodes[j].cap[usize::from(d ^ 1)]
            };
            if residual <= 0 || self.nodes[j].parent == NONE || self.nodes[j].sink != sink {
                continue;
            }
            // Does j still reach a terminal without passing through an orphan? Count the steps,
            // reusing distances already verified in this round (stamped with the current time).
            let mut steps: u32 = 0;
            let mut k = j;
            loop {
                if self.nodes[k].ts == self.time {
                    steps = steps.saturating_add(self.nodes[k].dist);
                    break;
                }
                let p = self.nodes[k].parent;
                steps = steps.saturating_add(1);
                if p == TERMINAL {
                    self.nodes[k].ts = self.time;
                    self.nodes[k].dist = 1;
                    break;
                }
                if p == ORPHAN {
                    steps = INFINITE_D;
                    break;
                }
                k = self.nbr(k, p);
            }
            if steps < INFINITE_D {
                if steps < best_d {
                    best_d = steps;
                    best_dir = Some(d);
                }
                // Stamp the path just verified.
                let mut k = j;
                let mut dist = steps;
                while self.nodes[k].ts != self.time {
                    self.nodes[k].ts = self.time;
                    self.nodes[k].dist = dist;
                    dist -= 1;
                    let p = self.nodes[k].parent;
                    k = self.nbr(k, p);
                }
            }
        }

        if let Some(d) = best_dir {
            let node = &mut self.nodes[i];
            node.parent = d;
            node.ts = self.time;
            node.dist = best_d + 1;
            return;
        }

        // No parent: i leaves the tree. Its neighbours in the same tree may now grow into the
        // space, and its children become orphans.
        self.nodes[i].parent = NONE;
        for d in 0..4u8 {
            let j = self.nbr(i, d);
            if self.nodes[j].parent == NONE || self.nodes[j].sink != sink {
                continue;
            }
            let residual = if sink {
                self.nodes[i].cap[usize::from(d)]
            } else {
                self.nodes[j].cap[usize::from(d ^ 1)]
            };
            if residual > 0 {
                self.set_active(j);
            }
            let p = self.nodes[j].parent;
            if p != TERMINAL && p != ORPHAN && self.nbr(j, p) == i {
                self.set_orphan_rear(j);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// xorshift32: a tiny fixed generator so the tests are reproducible.
    struct Rng(u32);
    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            self.0
        }
        fn range(&mut self, lo: i32, hi: i32) -> i32 {
            lo + (self.next() % (hi - lo + 1) as u32) as i32
        }
    }

    /// Energy whose minimum equals the max-flow: the cost of the labelling in the s-t graph, i.e.
    /// `max(tr, 0)` per sink-side pixel, `max(-tr, 0)` per source-side pixel, plus the capacity of
    /// every cut pair.
    fn cut_value(
        w: usize,
        h: usize,
        terminal: &[i32],
        right: &[i32],
        down: &[i32],
        ink: &[bool],
    ) -> i64 {
        let mut total = 0i64;
        for p in 0..w * h {
            total += i64::from(if ink[p] {
                (-terminal[p]).max(0)
            } else {
                terminal[p].max(0)
            });
            if p % w + 1 < w && ink[p] != ink[p + 1] {
                total += i64::from(right[p]);
            }
            if p / w + 1 < h && ink[p] != ink[p + w] {
                total += i64::from(down[p]);
            }
        }
        total
    }

    fn brute_force_min(w: usize, h: usize, terminal: &[i32], right: &[i32], down: &[i32]) -> i64 {
        (0..1u32 << (w * h))
            .map(|mask| {
                let ink: Vec<bool> = (0..w * h).map(|p| mask >> p & 1 == 1).collect();
                cut_value(w, h, terminal, right, down, &ink)
            })
            .min()
            .unwrap()
    }

    fn random_problem(
        rng: &mut Rng,
        w: usize,
        h: usize,
        spread: i32,
    ) -> (Vec<i32>, Vec<i32>, Vec<i32>) {
        let n = w * h;
        let terminal = (0..n).map(|_| rng.range(-spread, spread)).collect();
        let right = (0..n).map(|_| rng.range(0, spread / 2)).collect();
        let down = (0..n).map(|_| rng.range(0, spread / 2)).collect();
        (terminal, right, down)
    }

    #[test]
    fn matches_exhaustive_search_on_random_small_grids() {
        let mut rng = Rng(0x1234_5678);
        for case in 0..300 {
            let (w, h) = [(3, 3), (4, 3), (2, 5), (5, 2), (1, 6), (6, 1), (4, 4)][case % 7];
            let spread = [3, 10, 40, 200][case % 4];
            let (t, r, d) = random_problem(&mut rng, w, h, spread);
            let cut = min_cut(w, h, &t, &r, &d);
            let best = brute_force_min(w, h, &t, &r, &d);
            assert_eq!(cut.flow, best, "case {case}: flow vs exhaustive minimum");
            assert_eq!(
                cut_value(w, h, &t, &r, &d, &cut.source_side),
                best,
                "case {case}: returned labelling is not optimal"
            );
        }
    }

    #[test]
    fn larger_random_grids_return_a_labelling_whose_cost_equals_the_flow() {
        let mut rng = Rng(0xdead_beef);
        for case in 0..40 {
            let (w, h) = (30 + case % 7, 20 + case % 5);
            let (t, r, d) = random_problem(&mut rng, w, h, [5, 50, 500][case % 3]);
            let cut = min_cut(w, h, &t, &r, &d);
            assert_eq!(
                cut_value(w, h, &t, &r, &d, &cut.source_side),
                cut.flow,
                "case {case}"
            );
        }
    }

    #[test]
    fn strong_terminals_win_over_weak_links_and_strong_links_smooth() {
        // 1x5 strip: left two pixels want ink, right two want paper, the middle is neutral.
        let t = [50, 50, 0, -50, -50];
        let weak = min_cut(5, 1, &t, &[10; 5], &[0; 5]);
        assert_eq!(weak.source_side, [true, true, false, false, false]);
        // With strong links, one weak contrary terminal (cost 1) is cheaper to override than
        // cutting a link (cost 100): the whole strip follows the strong terminal.
        let t2 = [50, 0, 0, 0, -1];
        let strong = min_cut(5, 1, &t2, &[100; 5], &[0; 5]);
        assert!(strong.source_side.iter().all(|&s| s));
        assert_eq!(strong.flow, 1);
    }

    #[test]
    fn no_terminals_means_no_flow_and_everything_is_paper() {
        let cut = min_cut(4, 4, &[0; 16], &[5; 16], &[5; 16]);
        assert_eq!(cut.flow, 0);
        assert!(cut.source_side.iter().all(|&s| !s));
    }

    #[test]
    fn identical_input_gives_identical_output() {
        let mut rng = Rng(7);
        let (t, r, d) = random_problem(&mut rng, 25, 25, 100);
        let a = min_cut(25, 25, &t, &r, &d);
        let b = min_cut(25, 25, &t, &r, &d);
        assert_eq!((a.flow, a.source_side), (b.flow, b.source_side));
    }
}
