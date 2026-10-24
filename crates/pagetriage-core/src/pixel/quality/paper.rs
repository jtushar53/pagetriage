//! Paper level on a coarse block grid: per-block sums of the gray values of "clear paper" pixels
//! (pixels that are neither ink nor next to ink). Shared by the bleed-through and fold-line
//! measures. Sums are exact integers, so every value is identical on every target.

/// Sums and counts of clear-paper pixels per `block x block` cell.
pub struct BlockGrid {
    pub block: usize,
    pub blocks_x: usize,
    pub blocks_y: usize,
    sum: Vec<u64>,
    count: Vec<u64>,
}

impl BlockGrid {
    /// `clear[i] != 0` marks pixels that count as paper.
    pub fn new(gray: &[u8], clear: &[u8], width: usize, height: usize, block: usize) -> Self {
        let block = block.max(1);
        let blocks_x = width.div_ceil(block);
        let blocks_y = height.div_ceil(block);
        let mut sum = vec![0u64; blocks_x * blocks_y];
        let mut count = vec![0u64; blocks_x * blocks_y];
        for y in 0..height {
            let row = (y / block) * blocks_x;
            for x in 0..width {
                let i = y * width + x;
                if clear[i] != 0 {
                    sum[row + x / block] += u64::from(gray[i]);
                    count[row + x / block] += 1;
                }
            }
        }
        Self {
            block,
            blocks_x,
            blocks_y,
            sum,
            count,
        }
    }

    /// Mean gray of the clear pixels of block `(i, j)`, if it has at least `min_pixels`.
    pub fn mean(&self, i: usize, j: usize, min_pixels: u64) -> Option<f64> {
        let k = j * self.blocks_x + i;
        if self.count[k] >= min_pixels.max(1) {
            Some(self.sum[k] as f64 / self.count[k] as f64)
        } else {
            None
        }
    }

    /// Local paper level for every block: the mean over a `(2 radius + 1)` square of blocks,
    /// weighted by pixel counts and rounded to an integer; `fallback` where no paper is near.
    pub fn smoothed(&self, radius: usize, fallback: i32) -> Vec<i32> {
        let (bx, by) = (self.blocks_x, self.blocks_y);
        let stride = bx + 1;
        let mut isum = vec![0u64; stride * (by + 1)];
        let mut icnt = vec![0u64; stride * (by + 1)];
        for j in 0..by {
            let (mut row_sum, mut row_cnt) = (0u64, 0u64);
            for i in 0..bx {
                row_sum += self.sum[j * bx + i];
                row_cnt += self.count[j * bx + i];
                isum[(j + 1) * stride + i + 1] = isum[j * stride + i + 1] + row_sum;
                icnt[(j + 1) * stride + i + 1] = icnt[j * stride + i + 1] + row_cnt;
            }
        }
        let mut level = vec![fallback; bx * by];
        for j in 0..by {
            for i in 0..bx {
                let (i0, i1) = (i.saturating_sub(radius), (i + radius + 1).min(bx));
                let (j0, j1) = (j.saturating_sub(radius), (j + radius + 1).min(by));
                let at = |t: &[u64], x: usize, y: usize| t[y * stride + x];
                let s =
                    at(&isum, i1, j1) + at(&isum, i0, j0) - at(&isum, i1, j0) - at(&isum, i0, j1);
                let c =
                    at(&icnt, i1, j1) + at(&icnt, i0, j0) - at(&icnt, i1, j0) - at(&icnt, i0, j1);
                if let Some(mean) = (s + c / 2).checked_div(c) {
                    level[j * bx + i] = mean as i32;
                }
            }
        }
        level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_means_ignore_masked_pixels() {
        // 4x2 image, 2x2 blocks: left block {10, 10, 10, 200(masked)}, right block all 50.
        let gray = [10, 10, 50, 50, 10, 200, 50, 50];
        let clear = [1, 1, 1, 1, 1, 0, 1, 1];
        let g = BlockGrid::new(&gray, &clear, 4, 2, 2);
        assert_eq!((g.blocks_x, g.blocks_y), (2, 1));
        assert_eq!(g.mean(0, 0, 1), Some(10.0));
        assert_eq!(g.mean(1, 0, 1), Some(50.0));
        assert_eq!(g.mean(0, 0, 4), None, "only 3 clear pixels");
    }

    #[test]
    fn smoothing_averages_blocks_by_pixel_count_and_falls_back() {
        let gray = [10, 10, 50, 50, 10, 10, 50, 50];
        let g = BlockGrid::new(&gray, &[1u8; 8], 4, 2, 2);
        assert_eq!(g.smoothed(0, 99), [10, 50]);
        assert_eq!(g.smoothed(1, 99), [30, 30]);
        let none = BlockGrid::new(&gray, &[0u8; 8], 4, 2, 2);
        assert_eq!(none.smoothed(1, 99), [99, 99]);
    }
}
