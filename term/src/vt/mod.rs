//! The VT byte-stream parser: Paul Williams' DEC state machine
//! (<https://vt100.net/emu/dec_ansi_parser>) with UTF-8 text, OSC ended by BEL or
//! ST, and 7-bit introducers only. It gives bytes no meaning; a [`Perform`] does.
//! Fast path: runs of printable ASCII reach [`Perform::print_ascii`] in one call.

mod params;
#[cfg(test)]
mod tests;

pub use params::Params;

const MAX_INTER: usize = 2;
/// OSC strings longer than this are cut (titles are short; we skip OSC 52).
const MAX_OSC: usize = 4096;

/// Receives what the parser recognizes.
pub trait Perform {
    /// Printable ASCII, 0x20..=0x7e only.
    fn print_ascii(&mut self, s: &[u8]);
    /// Any other printable character.
    fn print(&mut self, c: char);
    /// A C0 control (0x00..=0x1f, 0x7f).
    fn execute(&mut self, byte: u8);
    /// `CSI <inter> <params> <action>`. Private markers (`?`, `>`) are in `inter`.
    fn csi(&mut self, p: &Params, inter: &[u8], action: u8);
    /// `ESC <inter> <action>`.
    fn esc(&mut self, inter: &[u8], action: u8);
    /// `OSC <data> BEL|ST`; `bel` says which ended it (replies use the same).
    fn osc(&mut self, data: &[u8], bel: bool);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Ground,
    Escape,
    EscInter,
    CsiEntry,
    CsiParam,
    CsiInter,
    CsiIgnore,
    Osc,
    /// DCS, SOS, PM, APC: swallowed until ST.
    Swallow,
}

#[derive(Default)]
struct Utf8 {
    cp: u32,
    need: u8,
}

pub struct Parser {
    state: State,
    params: Params,
    inter: [u8; MAX_INTER],
    n_inter: usize,
    ignore: bool,
    osc: Vec<u8>,
    utf8: Utf8,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            params: Params::default(),
            inter: [0; MAX_INTER],
            n_inter: 0,
            ignore: false,
            osc: Vec::new(),
            utf8: Utf8::default(),
        }
    }

    pub fn feed<P: Perform>(&mut self, p: &mut P, bytes: &[u8]) {
        let mut i = 0;
        while i < bytes.len() {
            if self.state == State::Ground && self.utf8.need == 0 {
                let run = bytes[i..]
                    .iter()
                    .position(|b| !(0x20..0x7f).contains(b))
                    .unwrap_or(bytes.len() - i);
                if run > 0 {
                    p.print_ascii(&bytes[i..i + run]);
                    i += run;
                    continue;
                }
            }
            self.byte(p, bytes[i]);
            i += 1;
        }
    }

    fn byte<P: Perform>(&mut self, p: &mut P, b: u8) {
        // "Anywhere" transitions.
        match b {
            0x18 | 0x1a => {
                self.state = State::Ground;
                return;
            }
            0x1b if !matches!(self.state, State::Osc | State::Swallow) => {
                self.enter_escape();
                return;
            }
            _ => {}
        }
        match self.state {
            State::Ground => self.ground(p, b),
            State::Escape | State::EscInter => self.escape(p, b),
            State::CsiEntry | State::CsiParam | State::CsiInter | State::CsiIgnore => {
                self.csi(p, b)
            }
            State::Osc => self.osc(p, b),
            State::Swallow => {
                if b == 0x1b {
                    self.enter_escape();
                }
            }
        }
    }

    fn enter_escape(&mut self) {
        self.state = State::Escape;
        self.n_inter = 0;
        self.ignore = false;
        self.utf8 = Utf8::default();
    }

    fn collect(&mut self, b: u8) {
        if self.n_inter < MAX_INTER {
            self.inter[self.n_inter] = b;
            self.n_inter += 1;
        } else {
            self.ignore = true;
        }
    }

    fn ground<P: Perform>(&mut self, p: &mut P, b: u8) {
        if self.utf8.need > 0 && b & 0xc0 != 0x80 {
            // Broken sequence: U+FFFD, then `b` counts as a fresh byte.
            self.utf8 = Utf8::default();
            p.print(char::REPLACEMENT_CHARACTER);
        }
        match b {
            0x00..=0x1f | 0x7f => p.execute(b),
            0x20..=0x7e => p.print_ascii(&[b]),
            _ => {
                if let Some(c) = self.utf8_byte(b) {
                    p.print(c);
                }
            }
        }
    }

    /// Feed one byte ≥ 0x80 to the decoder.
    fn utf8_byte(&mut self, b: u8) -> Option<char> {
        let u = &mut self.utf8;
        if u.need > 0 {
            u.cp = (u.cp << 6) | u32::from(b & 0x3f);
            u.need -= 1;
            if u.need > 0 {
                return None;
            }
            return Some(char::from_u32(u.cp).unwrap_or(char::REPLACEMENT_CHARACTER));
        }
        let (need, bits) = match b {
            0xc2..=0xdf => (1, b & 0x1f),
            0xe0..=0xef => (2, b & 0x0f),
            0xf0..=0xf4 => (3, b & 0x07),
            _ => return Some(char::REPLACEMENT_CHARACTER),
        };
        self.utf8 = Utf8 {
            cp: u32::from(bits),
            need,
        };
        None
    }

    fn escape<P: Perform>(&mut self, p: &mut P, b: u8) {
        match b {
            0x00..=0x1f => p.execute(b),
            0x20..=0x2f => {
                self.collect(b);
                self.state = State::EscInter;
            }
            b'[' if self.state == State::Escape => {
                self.params.clear();
                self.state = State::CsiEntry;
            }
            b']' if self.state == State::Escape => {
                self.osc.clear();
                self.state = State::Osc;
            }
            b'P' | b'X' | b'^' | b'_' if self.state == State::Escape => {
                self.state = State::Swallow;
            }
            0x30..=0x7e => {
                if !self.ignore {
                    p.esc(&self.inter[..self.n_inter], b);
                }
                self.state = State::Ground;
            }
            _ => {} // DEL is ignored here
        }
    }

    fn csi<P: Perform>(&mut self, p: &mut P, b: u8) {
        match b {
            0x00..=0x1f => p.execute(b),
            0x7f => {}
            b'0'..=b'9' | b';' | b':' => match self.state {
                State::CsiEntry | State::CsiParam => {
                    self.state = State::CsiParam;
                    match b {
                        b';' => self.params.separator(false),
                        b':' => self.params.separator(true),
                        d => self.params.digit(d),
                    }
                }
                _ => self.state = State::CsiIgnore,
            },
            0x3c..=0x3f => {
                if self.state == State::CsiEntry {
                    self.collect(b); // private marker
                } else {
                    self.state = State::CsiIgnore;
                }
            }
            0x20..=0x2f => {
                if self.state != State::CsiIgnore {
                    self.collect(b);
                    self.state = State::CsiInter;
                }
            }
            0x40..=0x7e => {
                if self.state != State::CsiIgnore && !self.ignore {
                    p.csi(&self.params, &self.inter[..self.n_inter], b);
                }
                self.state = State::Ground;
            }
            _ => self.state = State::CsiIgnore,
        }
    }

    fn osc<P: Perform>(&mut self, p: &mut P, b: u8) {
        match b {
            0x07 => {
                p.osc(&self.osc, true);
                self.state = State::Ground;
            }
            0x1b => {
                // ST is ESC \; the `\` then reaches `esc` as a no-op.
                p.osc(&self.osc, false);
                self.enter_escape();
            }
            0x00..=0x1f => {}
            _ => {
                if self.osc.len() < MAX_OSC {
                    self.osc.push(b);
                }
            }
        }
    }
}
