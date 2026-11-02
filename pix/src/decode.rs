//! Read an image file: sniff its format from the first bytes, decode it into packed
//! 8-bit RGB frames (alpha composited onto a caller-given background color), or keep
//! an SVG as a document to render at any size. PNG/JPEG here (zune); GIF in `gif.rs`,
//! WebP in `webp.rs`, SVG in `svg.rs`.
//! PNG/JPEG copied from ../image/src/image.rs at image 86fc166.

use std::fmt;
use std::path::Path;

use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;

use crate::svg::Svg;

/// All frames of one animation together stay under this (RGB bytes; 16 GB machine).
pub(crate) const MAX_FRAME_BYTES: usize = 1 << 30;

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
            Error::Format => write!(f, "not a PNG, JPEG, GIF, WebP or SVG file"),
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

/// One frame of an animation (stills have one): the whole canvas, shown `ms` long.
pub struct Frame {
    pub img: Rgb,
    /// 0 for a still.
    pub ms: u32,
}

/// What [`open`] found: pixels, or a vector document.
pub enum Image {
    Frames(Vec<Frame>),
    Svg(Svg),
}

/// A frame delay as browsers play it: 0–10 ms means "as fast as possible", which they
/// (and we) show for 100 ms.
pub(crate) fn delay_ms(ms: u32) -> u32 {
    if ms <= 10 { 100 } else { ms }
}

/// Open any supported file. `bg`: the color transparent pixels are blended onto.
pub fn open(path: &Path, bg: [u8; 3]) -> Result<Image, Error> {
    let bytes = std::fs::read(path).map_err(Error::Read)?;
    let still = |img| Image::Frames(vec![Frame { img, ms: 0 }]);
    if bytes.starts_with(b"\x89PNG") {
        png(&bytes, bg).map(still)
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        jpeg(&bytes, bg).map(still)
    } else if bytes.starts_with(b"GIF8") {
        crate::gif::frames(&bytes, bg).map(Image::Frames)
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        crate::webp::frames(&bytes, bg).map(Image::Frames)
    } else if is_svg(&bytes) {
        Svg::new(&bytes).map(Image::Svg)
    } else {
        Err(Error::Format)
    }
}

/// XML text with an `<svg` tag near the start (after any `<?xml`, comments, doctype).
fn is_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(4096)];
    let text = head.strip_prefix(b"\xef\xbb\xbf").unwrap_or(head); // UTF-8 BOM
    text.trim_ascii_start().starts_with(b"<") && text.windows(4).any(|w| w == b"<svg")
}

/// The first frame as pixels; an SVG rendered at its own size.
pub fn load(path: &Path, bg: [u8; 3]) -> Result<Rgb, Error> {
    match open(path, bg)? {
        Image::Frames(mut f) => Ok(f.swap_remove(0).img),
        Image::Svg(s) => {
            let (w, h) = s.size();
            s.render((w.round() as usize, h.round() as usize), bg)
        }
    }
}

fn png(bytes: &[u8], bg: [u8; 3]) -> Result<Rgb, Error> {
    let opts = DecoderOptions::new_fast().png_set_strip_to_8bit(true);
    let mut d = zune_png::PngDecoder::new_with_options(ZCursor::new(bytes), opts);
    d.decode_headers()
        .map_err(|e| Error::Decode(format!("{e:?}")))?;
    let (w, h) = d.dimensions().ok_or(Error::Format)?;
    let cs = d.colorspace().ok_or(Error::Format)?;
    let raw = d
        .decode_raw()
        .map_err(|e| Error::Decode(format!("{e:?}")))?;
    to_rgb(raw, (w, h), cs, bg)
}

fn jpeg(bytes: &[u8], bg: [u8; 3]) -> Result<Rgb, Error> {
    let opts = DecoderOptions::new_fast().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut d = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), opts);
    let px = d.decode().map_err(|e| Error::Decode(format!("{e:?}")))?;
    let info = d.info().ok_or(Error::Format)?;
    let (w, h) = (usize::from(info.width), usize::from(info.height));
    let cs = d.output_colorspace().unwrap_or(ColorSpace::RGB);
    let img = to_rgb(px, (w, h), cs, bg)?;
    // Phones store photos sideways plus a tag. shortcut: JPEG only; PNG eXIf and
    // WebP EXIF tags are rare, read them if one shows up sideways.
    let o = d.exif().map_or(1, |t| crate::orient::exif_orientation(t));
    Ok(if o == 1 {
        img
    } else {
        crate::orient::orient(&img, o)
    })
}

/// Any 8-bit layout the decoders hand back → packed RGB.
pub(crate) fn to_rgb(
    raw: Vec<u8>,
    (w, h): (usize, usize),
    cs: ColorSpace,
    bg: [u8; 3],
) -> Result<Rgb, Error> {
    let n = cs.num_components();
    if raw.len() < w * h * n {
        return Err(Error::Decode("short pixel buffer".into()));
    }
    let px = match cs {
        ColorSpace::RGB => raw,
        ColorSpace::RGBA => rgba_in_place(raw, w * h, bg),
        ColorSpace::Luma => raw.iter().flat_map(|&l| [l, l, l]).collect(),
        ColorSpace::LumaA => raw
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| over_bg([p[0]; 3], p[1], bg))
            .collect(),
        other => return Err(Error::Decode(format!("unsupported color space {other:?}"))),
    };
    Ok(Rgb { w, h, px })
}

/// RGBA → RGB inside the decoder's buffer: no second full-size copy. Pixel `i` is
/// written to `3i..3i+3`, which is never past bytes still to be read (`4i..`).
fn rgba_in_place(mut raw: Vec<u8>, n: usize, bg: [u8; 3]) -> Vec<u8> {
    for i in 0..n {
        let p = [raw[4 * i], raw[4 * i + 1], raw[4 * i + 2]];
        let rgb = over_bg(p, raw[4 * i + 3], bg);
        raw[3 * i..3 * i + 3].copy_from_slice(&rgb);
    }
    raw.truncate(n * 3);
    raw
}

/// shortcut: transparency shows a flat color, no checkerboard; add one if asked.
fn over_bg(rgb: [u8; 3], a: u8, bg: [u8; 3]) -> [u8; 3] {
    let a = u16::from(a);
    let mut out = [0; 3];
    for ((o, c), b) in out.iter_mut().zip(rgb).zip(bg) {
        *o = ((u16::from(c) * a + u16::from(b) * (255 - a) + 127) / 255) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const BG: [u8; 3] = [1, 2, 3];

    #[test]
    fn layouts_become_rgb() {
        let rgba = vec![200, 100, 50, 255, 200, 100, 50, 0];
        let rgba = to_rgb(rgba, (2, 1), ColorSpace::RGBA, BG).unwrap();
        assert_eq!(rgba.px, vec![200, 100, 50, 1, 2, 3]);
        let luma_a = to_rgb(vec![255, 255, 9, 0], (2, 1), ColorSpace::LumaA, BG).unwrap();
        assert_eq!(luma_a.px, vec![255, 255, 255, 1, 2, 3]);
        let luma = to_rgb(vec![9, 8], (2, 1), ColorSpace::Luma, BG).unwrap();
        assert_eq!(luma.px, vec![9, 9, 9, 8, 8, 8]);
        assert!(to_rgb(vec![1], (2, 1), ColorSpace::RGB, BG).is_err());
    }

    #[test]
    fn half_alpha_blends_halfway() {
        assert_eq!(over_bg([255, 0, 100], 128, [0, 255, 100]), [128, 127, 100]);
    }

    #[test]
    fn sniffs_svg_text() {
        assert!(is_svg(
            b"\xef\xbb\xbf<?xml version=\"1.0\"?>\n<!-- x -->\n<svg xmlns=\"\">"
        ));
        assert!(!is_svg(b"hello <svg"));
    }

    #[test]
    fn rejects_other_files() {
        let p = std::env::temp_dir().join("pix-test-not-image.txt");
        std::fs::write(&p, b"hello").unwrap();
        assert!(matches!(load(&p, BG), Err(Error::Format)));
        std::fs::remove_file(p).unwrap();
    }
}
