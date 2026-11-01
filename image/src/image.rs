//! Decode a PNG or JPEG file into packed 8-bit RGB. Alpha is composited onto the
//! window color. Copied from ../wall/src/image.rs at wall 3795cf6.

use std::fmt;
use std::path::Path;

use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;

/// One channel of frame's window color (#111111): what shows behind transparency.
const BG: u8 = 0x11;

#[derive(Debug)]
pub enum Error {
    Read(std::io::Error),
    Format,
    Decode(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Read(e) => write!(f, "can't read image: {e}"),
            Error::Format => write!(f, "not a PNG or JPEG file"),
            Error::Decode(e) => write!(f, "can't decode image: {e}"),
        }
    }
}

/// Packed RGB, 3 bytes per pixel, rows top to bottom.
pub struct Rgb {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

pub fn load(path: &Path) -> Result<Rgb, Error> {
    let bytes = std::fs::read(path).map_err(Error::Read)?;
    if bytes.starts_with(b"\x89PNG") {
        png(&bytes)
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        jpeg(&bytes)
    } else {
        Err(Error::Format)
    }
}

fn png(bytes: &[u8]) -> Result<Rgb, Error> {
    let opts = DecoderOptions::new_fast().png_set_strip_to_8bit(true);
    let mut d = zune_png::PngDecoder::new_with_options(ZCursor::new(bytes), opts);
    d.decode_headers()
        .map_err(|e| Error::Decode(format!("{e:?}")))?;
    let (w, h) = d.dimensions().ok_or(Error::Format)?;
    let cs = d.colorspace().ok_or(Error::Format)?;
    let raw = d
        .decode_raw()
        .map_err(|e| Error::Decode(format!("{e:?}")))?;
    to_rgb(raw, w, h, cs)
}

fn jpeg(bytes: &[u8]) -> Result<Rgb, Error> {
    let opts = DecoderOptions::new_fast().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut d = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), opts);
    let px = d.decode().map_err(|e| Error::Decode(format!("{e:?}")))?;
    let info = d.info().ok_or(Error::Format)?;
    let (w, h) = (usize::from(info.width), usize::from(info.height));
    to_rgb(px, w, h, d.output_colorspace().unwrap_or(ColorSpace::RGB))
}

/// Any 8-bit layout the decoders hand back → packed RGB.
fn to_rgb(raw: Vec<u8>, w: usize, h: usize, cs: ColorSpace) -> Result<Rgb, Error> {
    let n = cs.num_components();
    if raw.len() < w * h * n {
        return Err(Error::Decode("short pixel buffer".into()));
    }
    let px = match cs {
        ColorSpace::RGB => raw,
        ColorSpace::RGBA => rgba_in_place(raw, w * h),
        ColorSpace::Luma => raw.iter().flat_map(|&l| [l, l, l]).collect(),
        ColorSpace::LumaA => raw
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| over_bg([p[0]; 3], p[1]))
            .collect(),
        other => return Err(Error::Decode(format!("unsupported color space {other:?}"))),
    };
    Ok(Rgb { w, h, px })
}

/// RGBA → RGB inside the decoder's buffer: no second full-size copy. Pixel `i` is
/// written to `3i..3i+3`, which is never past bytes still to be read (`4i..`).
fn rgba_in_place(mut raw: Vec<u8>, n: usize) -> Vec<u8> {
    for i in 0..n {
        let p = [raw[4 * i], raw[4 * i + 1], raw[4 * i + 2]];
        let rgb = over_bg(p, raw[4 * i + 3]);
        raw[3 * i..3 * i + 3].copy_from_slice(&rgb);
    }
    raw.truncate(n * 3);
    raw
}

/// shortcut: transparency shows the window gray, no checkerboard; add one if asked.
fn over_bg(rgb: [u8; 3], a: u8) -> [u8; 3] {
    let (a, bg) = (u16::from(a), u16::from(BG));
    rgb.map(|c| ((u16::from(c) * a + bg * (255 - a) + 127) / 255) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_become_rgb() {
        let rgba = to_rgb(
            vec![200, 100, 50, 255, 200, 100, 50, 0],
            2,
            1,
            ColorSpace::RGBA,
        )
        .unwrap();
        assert_eq!(rgba.px, vec![200, 100, 50, 17, 17, 17]);
        let luma = to_rgb(vec![9, 8], 2, 1, ColorSpace::Luma).unwrap();
        assert_eq!(luma.px, vec![9, 9, 9, 8, 8, 8]);
        assert!(to_rgb(vec![1], 2, 1, ColorSpace::RGB).is_err());
    }

    #[test]
    fn rejects_other_files() {
        let p = std::env::temp_dir().join("image-test-not-image.txt");
        std::fs::write(&p, b"hello").unwrap();
        assert!(matches!(load(&p), Err(Error::Format)));
        std::fs::remove_file(p).unwrap();
    }
}
