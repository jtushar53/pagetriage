/// Bound one evidence term's llr to `[-clip, clip]` so a single saturated feature cannot decide a
/// finding on its own.
pub fn clip_llr(llr: f64, clip: f64) -> f64 {
    llr.clamp(-clip, clip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_both_sides_and_passes_through_inside() {
        assert_eq!(clip_llr(20.0, 8.0), 8.0);
        assert_eq!(clip_llr(-20.0, 8.0), -8.0);
        assert_eq!(clip_llr(1.25, 8.0), 1.25);
    }
}
