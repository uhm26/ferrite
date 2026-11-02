//! Fonts: one family in four styles, plus fontconfig fallback faces for characters
//! the family lacks. Layout (cells, lines, caching) is the app's job.
//! Taken from ../term/src/font/mod.rs at term 7be66f5 (2026-11-01): the face and
//! fallback part; term's cell grid, glyph cache and box drawing stay in term.

mod fc;
pub mod ft;

pub use ft::{Face, Rendered};

use std::collections::HashSet;
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

impl std::error::Error for Error {}

/// Regular, bold, italic, bold italic: index with these (for [`Font::new`]).
pub const REGULAR: usize = 0;
pub const BOLD: usize = 1;
pub const ITALIC: usize = 2;
pub const BOLD_ITALIC: usize = 3;

pub struct Font {
    // Field order matters: faces drop before the library.
    faces: Vec<Face>,
    fallbacks: Vec<(PathBuf, i32, Face)>,
    /// Characters no installed font has (asked fontconfig once already).
    missing: HashSet<char>,
    lib: ft::Library,
    family: String,
    pub size: f32,
}

impl Font {
    /// `family` is a fontconfig name like `Adwaita Sans`; `size` in points at 96 dpi.
    /// Faces in the order of [`REGULAR`], [`BOLD`], [`ITALIC`], [`BOLD_ITALIC`].
    pub fn new(family: &str, size: f32) -> Result<Self, Error> {
        Self::with_styles(family, size, &["", ":bold", ":italic", ":bold:italic"])
    }

    /// One face per fontconfig style suffix (`""`, `":medium"`, `":semibold"` …);
    /// a style index is the position in `styles`.
    pub fn with_styles(family: &str, size: f32, styles: &[&str]) -> Result<Self, Error> {
        let lib = ft::Library::new().ok_or(Error::FreeType)?;
        let mut faces = Vec::with_capacity(styles.len());
        for style in styles {
            let name = format!("{family}{style}");
            let (path, index) = fc::find(&name, None).ok_or(Error::NotFound(name))?;
            let mut face = lib.open(&path, index).ok_or(Error::Load(path))?;
            face.set_size(size);
            faces.push(face);
        }
        Ok(Font {
            faces,
            fallbacks: Vec::new(),
            missing: HashSet::new(),
            lib,
            family: family.to_string(),
            size,
        })
    }

    /// New size for every face (zoom). Cached glyphs are stale after this.
    pub fn resize(&mut self, size: f32) {
        self.size = size;
        for f in &mut self.faces {
            f.set_size(size);
        }
        for (_, _, f) in &mut self.fallbacks {
            f.set_size(size);
        }
    }

    /// The face of `style` itself (for metrics).
    pub fn face(&mut self, style: usize) -> &mut Face {
        let last = self.faces.len() - 1;
        &mut self.faces[style.min(last)]
    }

    /// The styled face if it has `c`, else a fallback font that does. With none, the
    /// styled face (it draws its "missing" box).
    pub fn face_for(&mut self, c: char, style: usize) -> &mut Face {
        let style = style.min(self.faces.len() - 1);
        if self.faces[style].has(c) {
            return &mut self.faces[style];
        }
        if let Some(i) = self.fallbacks.iter().position(|(_, _, f)| f.has(c)) {
            return &mut self.fallbacks[i].2;
        }
        // A fontconfig search costs milliseconds: do it once per character, ever.
        if self.missing.insert(c)
            && let Some((path, index)) = fc::find(&self.family, Some(c))
            && !self
                .fallbacks
                .iter()
                .any(|(p, i, _)| *p == path && *i == index)
            && let Some(mut face) = self.lib.open(&path, index)
            && face.has(c)
            && face.set_size(self.size)
        // bitmap-only fonts (color emoji) can't be sized
        {
            self.missing.remove(&c);
            self.fallbacks.push((path, index, face));
            let last = self.fallbacks.len() - 1;
            return &mut self.fallbacks[last].2;
        }
        &mut self.faces[style]
    }
}
