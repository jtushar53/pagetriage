//! Pixel-buffer helpers. Integer arithmetic only, so results are identical on every target.

use crate::error::{Error, Result};

/// Gray from interleaved RGB using Rec. 601 luma weights in fixed point:
/// `(299 R + 587 G + 114 B + 500) / 1000`, rounded to nearest.
pub fn rgb_to_gray(rgb: &[u8]) -> Result<Vec<u8>> {
    let (pixels, remainder) = rgb.as_chunks::<3>();
    if !remainder.is_empty() {
        return Err(Error::InvalidInput(format!(
            "rgb buffer length {} is not a multiple of 3",
            rgb.len()
        )));
    }
    Ok(pixels
        .iter()
        .map(|px| {
            let y = 299 * u32::from(px[0]) + 587 * u32::from(px[1]) + 114 * u32::from(px[2]) + 500;
            // Weights sum to 1000, so y / 1000 <= 255.
            (y / 1000) as u8
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luma_of_primaries_and_extremes() {
        let gray = rgb_to_gray(&[0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 255]).unwrap();
        assert_eq!(gray, [0, 255, 76, 150, 29]);
    }

    #[test]
    fn rejects_ragged_rgb() {
        assert!(rgb_to_gray(&[1, 2, 3, 4]).is_err());
    }
}
