//! One screen cell: 16 bytes, no heap. Colors stay symbolic (palette index or
//! "default") until drawing, so palette changes recolor old text too.

/// A cell color: 24-bit RGB, a palette index, or the default fg/bg.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Color(u32);

const TAG_RGB: u32 = 1 << 24;
const TAG_INDEX: u32 = 2 << 24;
const TAG_DEFAULT: u32 = 3 << 24;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Rgb(u32),
    Index(u8),
    Default,
}

impl Color {
    pub const DEFAULT: Color = Color(TAG_DEFAULT);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color(TAG_RGB | (r as u32) << 16 | (g as u32) << 8 | b as u32)
    }

    pub const fn index(i: u8) -> Self {
        Color(TAG_INDEX | i as u32)
    }

    pub fn kind(self) -> Kind {
        match self.0 & 0xff00_0000 {
            TAG_RGB => Kind::Rgb(self.0 & 0xff_ffff),
            TAG_INDEX => Kind::Index(self.0 as u8),
            _ => Kind::Default,
        }
    }
}

/// Cell attributes, one bit each.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Flags(pub u32);

impl Flags {
    pub const BOLD: u32 = 1;
    pub const DIM: u32 = 1 << 1;
    pub const ITALIC: u32 = 1 << 2;
    pub const UNDERLINE: u32 = 1 << 3;
    pub const DOUBLE_UL: u32 = 1 << 4;
    pub const CURLY_UL: u32 = 1 << 5;
    pub const REVERSE: u32 = 1 << 6;
    pub const HIDDEN: u32 = 1 << 7;
    pub const STRIKE: u32 = 1 << 8;
    /// First half of a 2-column character.
    pub const WIDE: u32 = 1 << 9;
    /// Second half of a 2-column character (drawn by the first half).
    pub const SPACER: u32 = 1 << 10;
    pub const ANY_UL: u32 = Self::UNDERLINE | Self::DOUBLE_UL | Self::CURLY_UL;
    /// Bits above this are free for the renderer's own marks (cursor, selection).
    pub const TERM_MASK: u32 = (1 << 16) - 1;

    pub fn has(self, bit: u32) -> bool {
        self.0 & bit != 0
    }

    pub fn set(&mut self, bit: u32, on: bool) {
        if on {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub flags: Flags,
}

/// Cells compare as one 128-bit value: one load and compare instead of four
/// (scrollback trimming and the renderer diff compare cells in bulk).
impl PartialEq for Cell {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.bits() == other.bits()
    }
}

impl Eq for Cell {}

impl Default for Cell {
    fn default() -> Self {
        Cell::BLANK
    }
}

impl Cell {
    #[inline]
    fn bits(&self) -> u128 {
        u128::from(self.ch as u32)
            | u128::from(self.fg.0) << 32
            | u128::from(self.bg.0) << 64
            | u128::from(self.flags.0) << 96
    }

    pub const BLANK: Cell = Cell {
        ch: ' ',
        fg: Color::DEFAULT,
        bg: Color::DEFAULT,
        flags: Flags(0),
    };

    /// An erased cell: blank, keeping the pen's background (BCE).
    pub fn erased(pen: &Pen) -> Cell {
        Cell {
            bg: pen.bg,
            ..Cell::BLANK
        }
    }
}

/// The attributes new text is written with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pen {
    pub fg: Color,
    pub bg: Color,
    pub flags: Flags,
}

impl Default for Pen {
    fn default() -> Self {
        Pen {
            fg: Color::DEFAULT,
            bg: Color::DEFAULT,
            flags: Flags(0),
        }
    }
}

impl Pen {
    pub fn cell(&self, ch: char) -> Cell {
        Cell {
            ch,
            fg: self.fg,
            bg: self.bg,
            flags: self.flags,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_is_16_bytes() {
        assert_eq!(size_of::<Cell>(), 16);
    }

    #[test]
    fn color_kinds() {
        assert_eq!(Color::rgb(1, 2, 3).kind(), Kind::Rgb(0x010203));
        assert_eq!(Color::index(200).kind(), Kind::Index(200));
        assert_eq!(Color::DEFAULT.kind(), Kind::Default);
    }
}
