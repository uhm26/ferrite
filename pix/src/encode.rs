//! Write images: PNG bytes, e.g. for the clipboard (apps paste `image/png`).

use zune_core::bit_depth::BitDepth;
use zune_core::colorspace::ColorSpace;
use zune_core::options::EncoderOptions;

use crate::decode::{Error, Rgb};

/// `img` as a PNG file's bytes.
pub fn png(img: &Rgb) -> Result<Vec<u8>, Error> {
    let opts = EncoderOptions::new(img.w, img.h, ColorSpace::RGB, BitDepth::Eight);
    let mut out = Vec::new();
    zune_png::PngEncoder::new(&img.px, opts)
        .encode(&mut out)
        .map_err(|e| Error::Decode(format!("png encode: {e:?}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn png_round_trips() {
        let img = crate::Rgb { w: 3, h: 2, px: (0..18).collect() };
        let bytes = super::png(&img).unwrap();
        let p = std::env::temp_dir().join(format!("pix-encode-{}.png", std::process::id()));
        std::fs::write(&p, bytes).unwrap();
        let back = crate::load(&p, [0; 3]);
        std::fs::remove_file(&p).unwrap();
        let back = back.unwrap();
        assert_eq!((back.w, back.h, back.px), (3, 2, img.px));
    }
}
