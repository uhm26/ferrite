//! Fonts: the cell size, and glyphs rasterized into cell-sized coverage boxes so
//! drawing is a straight copy. Box-drawing and block characters are drawn by
//! [`boxdraw`] so lines meet exactly; everything else comes from FreeType, with a
//! fontconfig fallback for characters the main font lacks.

mod boxdraw;
mod cache;
mod fc;
mod ft;

pub use cache::GlyphCache;

use std::fmt;
use std::path::PathBuf;

#[derive(Debug)]
pub enum Error {
    FreeType,
    NotFound(String),
    Load(PathBuf),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::FreeType => write!(f, "FreeType failed to start"),
            Error::NotFound(n) => write!(f, "no font matches {n:?}"),
            Error::Load(p) => write!(f, "cannot load font {}", p.display()),
        }
    }
}

/// Regular, bold, italic, bold italic.
pub const STYLES: usize = 4;

/// Coverage (0-255) for a box `cols` cells wide and one cell high, row-major.
/// Empty `data` means nothing to draw.
pub struct Glyph {
    pub cols: u8,
    pub data: Box<[u8]>,
}

pub struct Font {
    // Field order matters: faces drop before the library.
    faces: Vec<ft::Face>,
    fallbacks: Vec<(PathBuf, i32, ft::Face)>,
    /// Characters no installed font has (asked fontconfig once already).
    missing: std::collections::HashSet<char>,
    lib: ft::Library,
    family: String,
    pub size: f32,
    pub cell_w: usize,
    pub cell_h: usize,
    /// Pixels from the cell top to the baseline.
    pub baseline: usize,
    /// Line thickness for underlines and box drawing.
    pub thickness: usize,
}

impl Font {
    pub fn new(family: &str, size: f32) -> Result<Self, Error> {
        let lib = ft::Library::new().ok_or(Error::FreeType)?;
        let mut faces = Vec::with_capacity(STYLES);
        for style in ["", ":bold", ":italic", ":bold:italic"] {
            let name = format!("{family}{style}");
            let (path, index) = fc::find(&name, None).ok_or(Error::NotFound(name))?;
            let mut face = lib.open(&path, index).ok_or(Error::Load(path))?;
            face.set_size(size);
            faces.push(face);
        }
        let mut font = Font {
            faces,
            fallbacks: Vec::new(),
            missing: std::collections::HashSet::new(),
            lib,
            family: family.to_string(),
            size,
            cell_w: 1,
            cell_h: 1,
            baseline: 1,
            thickness: 1,
        };
        font.measure();
        Ok(font)
    }

    /// New size for every face (zoom). The glyph cache must be cleared after.
    pub fn resize(&mut self, size: f32) {
        self.size = size;
        for f in &mut self.faces {
            f.set_size(size);
        }
        for (_, _, f) in &mut self.fallbacks {
            f.set_size(size);
        }
        self.measure();
    }

    fn measure(&mut self) {
        let (asc, desc) = self.faces[0].metrics();
        let advance = self.faces[0].render('M').map_or(self.size as i64 * 40, |(_, a)| a);
        self.cell_w = ((advance + 32) / 64).max(1) as usize;
        let asc = (asc + 63) / 64;
        let desc = (-desc + 63) / 64;
        self.cell_h = (asc + desc).max(1) as usize;
        self.baseline = asc.max(0) as usize;
        self.thickness = (self.cell_h / 16).max(1);
    }

    /// Rasterize `c` in `style` into a box `cols` cells wide.
    pub fn glyph(&mut self, c: char, style: usize, cols: u8) -> Glyph {
        let (w, h) = (self.cell_w * usize::from(cols), self.cell_h);
        if let Some(data) = boxdraw::draw(c, w, h, self.thickness) {
            return Glyph { cols, data };
        }
        if c == ' ' {
            return Glyph { cols, data: Box::new([]) };
        }
        let baseline = self.baseline as i32;
        let face = self.face_for(c, style);
        let mut data = vec![0u8; w * h];
        let mut any = false;
        if let Some((r, _)) = face.render(c) {
            any = blit(&mut data, (w, h), &r, baseline);
        }
        Glyph {
            cols,
            data: if any { data.into() } else { Box::new([]) },
        }
    }

    /// The styled face if it has `c`, else a fallback font that does.
    fn face_for(&mut self, c: char, style: usize) -> &mut ft::Face {
        let style = style.min(STYLES - 1);
        if self.faces[style].has(c) {
            return &mut self.faces[style];
        }
        if let Some(i) = self.fallbacks.iter().position(|(_, _, f)| f.has(c)) {
            return &mut self.fallbacks[i].2;
        }
        // A fontconfig search costs milliseconds: do it once per character, ever.
        if self.missing.insert(c)
            && let Some((path, index)) = fc::find(&self.family, Some(c))
            && !self.fallbacks.iter().any(|(p, i, _)| *p == path && *i == index)
            && let Some(mut face) = self.lib.open(&path, index)
            && face.has(c)
            && face.set_size(self.size) // bitmap-only fonts (color emoji) can't be sized
        {
            self.missing.remove(&c);
            self.fallbacks.push((path, index, face));
            let last = self.fallbacks.len() - 1;
            return &mut self.fallbacks[last].2;
        }
        &mut self.faces[style] // draws the font's "missing" box
    }
}

/// Copy a rendered bitmap into the cell box, placed on the baseline, clipped.
/// Returns whether any pixel landed.
fn blit(dst: &mut [u8], (w, h): (usize, usize), r: &ft::Rendered, baseline: i32) -> bool {
    let top = baseline - r.top;
    let mut any = false;
    for y in 0..r.rows {
        let dy = top + y as i32;
        if dy < 0 || dy >= h as i32 {
            continue;
        }
        let src = &r.bitmap[y * r.pitch..y * r.pitch + r.width];
        for (x, &a) in src.iter().enumerate() {
            let dx = r.left + x as i32;
            if a != 0 && dx >= 0 && (dx as usize) < w {
                dst[dy as usize * w + dx as usize] = a;
                any = true;
            }
        }
    }
    any
}
