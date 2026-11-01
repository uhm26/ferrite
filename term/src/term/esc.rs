//! C0 controls, plain ESC sequences, and OSC strings (title, color queries).

use super::Term;
use super::cell::{Cell, Flags};
use crate::color;

impl Term {
    pub(super) fn control(&mut self, byte: u8) {
        match byte {
            0x07 => {} // bell: ignored
            0x08 => self.backspace(),
            0x09 => self.tab(1),
            0x0a..=0x0c => {
                self.linefeed();
                if self.modes.newline {
                    self.carriage_return();
                }
            }
            0x0d => self.carriage_return(),
            0x0e => self.gl = 1,
            0x0f => self.gl = 0,
            _ => {}
        }
    }

    pub(super) fn esc_dispatch(&mut self, inter: &[u8], action: u8) {
        match (inter, action) {
            ([], b'7') => self.save_cursor(),
            ([], b'8') => self.restore_cursor(),
            ([], b'D') => self.linefeed(),
            ([], b'E') => {
                self.linefeed();
                self.carriage_return();
            }
            ([], b'H') => self.tabs[self.cursor.col] = true,
            ([], b'M') => self.reverse_index(),
            ([], b'c') => self.full_reset(),
            ([], b'=') => self.modes.app_keypad = true,
            ([], b'>') => self.modes.app_keypad = false,
            ([b'('], c) => self.charsets[0] = c == b'0',
            ([b')'], c) => self.charsets[1] = c == b'0',
            ([b'#'], b'8') => self.align_test(),
            _ => {} // includes `ESC \` (the ST after an OSC)
        }
    }

    /// DECALN: fill the screen with `E`.
    fn align_test(&mut self) {
        for y in 0..self.rows {
            self.grid.row_mut(y).cells.fill(Cell {
                ch: 'E',
                flags: Flags(0),
                ..Cell::BLANK
            });
        }
        self.goto(0, 0);
    }

    pub(super) fn osc_dispatch(&mut self, data: &[u8], bel: bool) {
        let text = String::from_utf8_lossy(data);
        let (num, rest) = text.split_once(';').unwrap_or((&text, ""));
        let end = if bel { "\x07" } else { "\x1b\\" };
        match num {
            "0" | "2" => self.title = Some(rest.chars().take(256).collect()),
            "4" => self.osc_palette(rest, end),
            "10" | "11" | "12" => self.osc_dynamic(num, rest, end),
            "104" => self.reset_palette(rest),
            "110" => self.set_dynamic(10, self.base_palette.fg),
            "111" => self.set_dynamic(11, self.base_palette.bg),
            "112" => self.set_dynamic(12, self.base_palette.cursor),
            _ => {} // 7 (cwd), 8 (links), 52 (clipboard), 133 (prompts): not yet
        }
    }

    /// OSC 4 ; index ; spec|? [; index ; spec|? …]
    fn osc_palette(&mut self, rest: &str, end: &str) {
        let parts: Vec<&str> = rest.split(';').collect();
        for [index, value] in parts.as_chunks::<2>().0 {
            let Ok(i) = index.parse::<u8>() else {
                continue;
            };
            if *value == "?" {
                let v = color::spec(self.palette.ansi[usize::from(i)]);
                self.reply(&format!("\x1b]4;{i};{v}{end}"));
            } else if let Some(v) = color::parse(value) {
                self.palette.ansi[usize::from(i)] = v;
                self.recolor = true;
            }
        }
    }

    /// OSC 10/11/12: default fg, bg, cursor. Each following value is the next one.
    fn osc_dynamic(&mut self, num: &str, rest: &str, end: &str) {
        let Ok(mut n) = num.parse::<u16>() else {
            return;
        };
        for spec in rest.split(';') {
            if spec == "?" {
                let v = match n {
                    10 => self.palette.fg,
                    11 => self.palette.bg,
                    _ => self.palette.cursor,
                };
                self.reply(&format!("\x1b]{n};{}{end}", color::spec(v)));
            } else if let Some(v) = color::parse(spec) {
                self.set_dynamic(n, v);
            }
            n += 1;
            if n > 12 {
                break;
            }
        }
    }

    fn set_dynamic(&mut self, n: u16, v: u32) {
        match n {
            10 => self.palette.fg = v,
            11 => self.palette.bg = v,
            12 => self.palette.cursor = v,
            _ => return,
        }
        self.recolor = true;
    }

    fn reset_palette(&mut self, rest: &str) {
        if rest.is_empty() {
            self.palette.ansi = self.base_palette.ansi;
        }
        for i in rest.split(';').filter_map(|s| s.parse::<u8>().ok()) {
            self.palette.ansi[usize::from(i)] = self.base_palette.ansi[usize::from(i)];
        }
        self.recolor = true;
    }
}
