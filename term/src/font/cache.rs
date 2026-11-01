//! Rasterized glyphs, kept until the font changes (zoom, live config). Printable ASCII is a direct
//! array index; everything else is a hash map with a cheap integer hash.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use super::{Font, Glyph, STYLES};

const ASCII: usize = 0x7f - 0x20;

/// Multiply-shift hash for u32 keys (keys are char | style | width bits).
#[derive(Default)]
pub struct IntHasher(u64);

impl Hasher for IntHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
    }

    fn write_u32(&mut self, v: u32) {
        self.0 = u64::from(v).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
}

#[derive(Default)]
pub struct GlyphCache {
    ascii: Vec<Option<Glyph>>,
    other: HashMap<u32, Glyph, BuildHasherDefault<IntHasher>>,
}

impl GlyphCache {
    pub fn clear(&mut self) {
        self.ascii.clear();
        self.other.clear();
    }

    pub fn get(&mut self, font: &mut Font, c: char, style: usize, cols: u8) -> &Glyph {
        let code = c as usize;
        if cols == 1 && (0x20..0x7f).contains(&code) {
            if self.ascii.is_empty() {
                self.ascii.resize_with(ASCII * STYLES, || None);
            }
            let slot = &mut self.ascii[(code - 0x20) * STYLES + style];
            return slot.get_or_insert_with(|| font.glyph(c, style, cols));
        }
        let key = c as u32 | (style as u32) << 21 | u32::from(cols - 1) << 23;
        self.other
            .entry(key)
            .or_insert_with(|| font.glyph(c, style, cols))
    }

    /// Bytes held by cached glyph bitmaps (for memory checks).
    pub fn bytes(&self) -> usize {
        let a: usize = self.ascii.iter().flatten().map(|g| g.data.len()).sum();
        a + self.other.values().map(|g| g.data.len()).sum::<usize>()
    }
}
