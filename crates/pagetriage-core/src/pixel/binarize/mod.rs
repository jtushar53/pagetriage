//! Binarisers: Otsu, Sauvola, Wolf-Jolion and Howe. Each takes the gray page and returns a mask
//! with `1` for ink and `0` for paper. Module M3 picks among them from the page's degradation.

use crate::error::{Error, Result};
use crate::params::Params;
use crate::schema::Binarizer;

pub mod canny;
pub mod filters;
pub mod howe;
pub mod local;
pub mod maxflow;

/// Binarise a `width * height` row-major gray page with the chosen method and parameters.
pub fn binarize(
    gray: &[u8],
    width: usize,
    height: usize,
    method: Binarizer,
    params: &Params,
) -> Result<Vec<u8>> {
    if width == 0 || height == 0 || gray.len() != width * height {
        return Err(Error::InvalidInput(format!(
            "gray buffer of {} bytes does not match {width}x{height}",
            gray.len()
        )));
    }
    Ok(match method {
        Binarizer::Otsu => local::otsu(gray),
        Binarizer::Sauvola => local::sauvola(gray, width, height, &params.sauvola),
        Binarizer::Wolf => local::wolf(gray, width, height, &params.wolf),
        Binarizer::Howe => howe::binarize(gray, width, height, &params.howe).ink,
    })
}

/// The method's snake_case name as used in reports and options.
pub fn parse_method(name: &str) -> Result<Binarizer> {
    match name {
        "otsu" => Ok(Binarizer::Otsu),
        "sauvola" => Ok(Binarizer::Sauvola),
        "wolf" => Ok(Binarizer::Wolf),
        "howe" => Ok(Binarizer::Howe),
        other => Err(Error::InvalidOptions(format!(
            "unknown binariser {other:?}; expected otsu, sauvola, wolf or howe"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_method_returns_one_label_per_pixel() {
        let params = Params::defaults().unwrap();
        let gray: Vec<u8> = (0..30 * 20).map(|i| (i * 13 % 256) as u8).collect();
        for m in ["otsu", "sauvola", "wolf", "howe"] {
            let mut p = params.clone();
            p.howe.mode = crate::params::HoweMode::Static;
            let out = binarize(&gray, 30, 20, parse_method(m).unwrap(), &p).unwrap();
            assert_eq!(out.len(), gray.len(), "{m}");
            assert!(out.iter().all(|&v| v <= 1), "{m}");
        }
    }

    #[test]
    fn bad_buffers_and_names_are_rejected() {
        let params = Params::defaults().unwrap();
        assert!(binarize(&[0; 5], 2, 2, Binarizer::Otsu, &params).is_err());
        assert!(binarize(&[], 0, 0, Binarizer::Otsu, &params).is_err());
        assert!(parse_method("niblack").is_err());
    }
}
