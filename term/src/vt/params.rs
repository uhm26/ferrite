//! CSI parameters: numbers split by `;`, with `:` sub-parameters (SGR 4:3, 38:2::r:g:b).

/// More parameters than this are dropped (the sequence still runs).
const MAX: usize = 32;

#[derive(Default, Clone)]
pub struct Params {
    vals: [u16; MAX],
    /// Bit i set: value i is a `:` sub-parameter of the value before it.
    sub: u32,
    len: usize,
    /// A digit was seen for the value being built.
    open: bool,
    /// More than MAX values came: the rest are ignored.
    overflow: bool,
}

impl Params {
    pub fn clear(&mut self) {
        self.overflow = false;
        self.len = 0;
        self.sub = 0;
        self.open = false;
    }

    pub fn digit(&mut self, d: u8) {
        if !self.open {
            self.push(false);
        }
        if self.overflow {
            return; // past MAX values: dropped, never glued onto the last one
        }
        if let Some(v) = self.vals.get_mut(self.len.wrapping_sub(1)) {
            *v = v.saturating_mul(10).saturating_add(u16::from(d - b'0'));
        }
    }

    /// `;` or `:` ends the current value (an empty one counts as 0).
    pub fn separator(&mut self, colon: bool) {
        if !self.open {
            self.push(false);
        }
        self.open = false;
        if colon {
            // The next value is a sub-parameter.
            self.push(true);
            self.open = true;
        }
    }

    fn push(&mut self, sub: bool) {
        self.overflow = self.len == MAX;
        if self.len < MAX {
            self.vals[self.len] = 0;
            if sub {
                self.sub |= 1 << self.len;
            }
            self.len += 1;
        }
        self.open = true;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Value `i`, or `default` when missing or 0 (the VT rule for counts).
    pub fn arg(&self, i: usize, default: u16) -> u16 {
        match self.get(i) {
            0 => default,
            v => v,
        }
    }

    /// Value `i`; missing → 0.
    pub fn get(&self, i: usize) -> u16 {
        if i < self.len { self.vals[i] } else { 0 }
    }

    /// Groups: each top-level value with its `:` sub-parameters.
    pub fn groups(&self) -> Groups<'_> {
        Groups { p: self, at: 0 }
    }
}

pub struct Groups<'a> {
    p: &'a Params,
    at: usize,
}

impl<'a> Iterator for Groups<'a> {
    type Item = &'a [u16];

    fn next(&mut self) -> Option<&'a [u16]> {
        if self.at >= self.p.len {
            return None;
        }
        let start = self.at;
        self.at += 1;
        while self.at < self.p.len && self.p.sub & (1 << self.at) != 0 {
            self.at += 1;
        }
        Some(&self.p.vals[start..self.at])
    }
}
