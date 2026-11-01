//! Keys to bytes for the shell, xterm style: arrows and function keys with
//! modifier parameters (`CSI 1;5D` = Ctrl+Left), application cursor mode, Alt as
//! an ESC prefix. Keysym values from /usr/include/xkbcommon/xkbcommon-keysyms.h.

use crate::xkb::{Key, Mods};

const BACKSPACE: u32 = 0xff08;
const TAB: u32 = 0xff09;
const RETURN: u32 = 0xff0d;
const ESCAPE: u32 = 0xff1b;
const HOME: u32 = 0xff50;
const LEFT: u32 = 0xff51;
const UP: u32 = 0xff52;
const RIGHT: u32 = 0xff53;
const DOWN: u32 = 0xff54;
const PAGE_UP: u32 = 0xff55;
const PAGE_DOWN: u32 = 0xff56;
const END: u32 = 0xff57;
const INSERT: u32 = 0xff63;
const KP_ENTER: u32 = 0xff8d;
const KP_HOME: u32 = 0xff95;
const KP_DELETE: u32 = 0xff9f;
const F1: u32 = 0xffbe;
const F12: u32 = 0xffc9;
const DELETE: u32 = 0xffff;
const ISO_LEFT_TAB: u32 = 0xfe20;
const SPACE: u32 = 0x20;

/// The xterm modifier parameter: 1 + shift + 2·alt + 4·ctrl.
fn param(m: Mods) -> u8 {
    1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.ctrl)
}

/// Bytes for `key`, or None if it sends nothing (a lone modifier, say).
pub fn encode(key: &Key, app_cursor: bool) -> Option<Vec<u8>> {
    let m = key.mods;
    let sym = normalize_keypad(key.sym);
    if let Some(seq) = special(sym, m, app_cursor) {
        return Some(seq.into_bytes());
    }
    let mut out = Vec::new();
    let text = match sym {
        RETURN | KP_ENTER => "\r",
        // Ctrl+W: "delete word" in fish, bash, readline, Claude Code and vim.
        BACKSPACE if m.ctrl => "\x17",
        BACKSPACE => "\x7f",
        TAB => "\t",
        ESCAPE => "\x1b",
        SPACE if m.ctrl => "\0",
        _ => key.text.as_str(),
    };
    if text.is_empty() {
        return None;
    }
    // Shift+Enter goes out as Alt+Enter (ESC CR): without the kitty keyboard protocol
    // it would be a plain CR, and fish and Claude Code read ESC CR as "new line".
    let shift_enter = m.shift && matches!(sym, RETURN | KP_ENTER);
    if m.alt || shift_enter {
        out.push(0x1b);
    }
    out.extend_from_slice(text.as_bytes());
    Some(out)
}

/// Keypad navigation keys (NumLock off) act like the main ones.
fn normalize_keypad(sym: u32) -> u32 {
    const KP: [u32; 11] = [
        HOME, LEFT, UP, RIGHT, DOWN, PAGE_UP, PAGE_DOWN, END, 0xff58, INSERT, DELETE,
    ];
    match sym {
        KP_HOME..=KP_DELETE => KP[(sym - KP_HOME) as usize],
        s => s,
    }
}

/// Cursor, editing and function keys as escape sequences.
fn special(sym: u32, m: Mods, app_cursor: bool) -> Option<String> {
    let p = param(m);
    let letter = match sym {
        UP => Some('A'),
        DOWN => Some('B'),
        RIGHT => Some('C'),
        LEFT => Some('D'),
        HOME => Some('H'),
        END => Some('F'),
        F1..=0xffc1 => Some(['P', 'Q', 'R', 'S'][(sym - F1) as usize]),
        _ => None,
    };
    if let Some(c) = letter {
        let is_f = sym >= F1;
        return Some(match (p, app_cursor || is_f) {
            (1, true) => format!("\x1bO{c}"),
            (1, false) => format!("\x1b[{c}"),
            _ => format!("\x1b[1;{p}{c}"),
        });
    }
    let num = match sym {
        INSERT => 2,
        DELETE => 3,
        PAGE_UP => 5,
        PAGE_DOWN => 6,
        0xffc2..=F12 => [15, 17, 18, 19, 20, 21, 23, 24][(sym - 0xffc2) as usize],
        ISO_LEFT_TAB => return Some("\x1b[Z".into()),
        _ => return None,
    };
    Some(match p {
        1 => format!("\x1b[{num}~"),
        _ => format!("\x1b[{num};{p}~"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(sym: u32, text: &str, mods: Mods) -> Key {
        Key {
            sym,
            text: text.into(),
            mods,
        }
    }

    fn enc(sym: u32, text: &str, mods: Mods, app: bool) -> String {
        String::from_utf8(encode(&key(sym, text, mods), app).unwrap()).unwrap()
    }

    #[test]
    fn plain_and_modified() {
        let none = Mods::default();
        let ctrl = Mods { ctrl: true, ..none };
        let alt = Mods { alt: true, ..none };
        assert_eq!(enc(0x61, "a", none, false), "a");
        assert_eq!(enc(0x63, "\x03", ctrl, false), "\x03");
        assert_eq!(enc(0x62, "b", alt, false), "\x1bb");
        assert_eq!(enc(LEFT, "", none, false), "\x1b[D");
        assert_eq!(enc(LEFT, "", none, true), "\x1bOD");
        assert_eq!(enc(LEFT, "", ctrl, false), "\x1b[1;5D");
        assert_eq!(enc(DELETE, "", none, false), "\x1b[3~");
        assert_eq!(enc(0xffc2, "", none, false), "\x1b[15~"); // F5
        assert_eq!(enc(F1, "", none, false), "\x1bOP");
        assert_eq!(enc(BACKSPACE, "\x08", none, false), "\x7f");
        assert_eq!(enc(BACKSPACE, "", alt, false), "\x1b\x7f");
        assert_eq!(enc(ISO_LEFT_TAB, "", none, false), "\x1b[Z");
        let shift = Mods {
            shift: true,
            ..none
        };
        assert_eq!(enc(RETURN, "\r", shift, false), "\x1b\r");
        assert_eq!(enc(RETURN, "\r", none, false), "\r");
        assert_eq!(enc(BACKSPACE, "", ctrl, false), "\x17");
        assert_eq!(enc(0xff96, "", none, false), "\x1b[D"); // KP_Left
        assert!(encode(&key(0xffe1, "", none), false).is_none()); // Shift_L
    }
}
