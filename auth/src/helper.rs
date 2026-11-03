//! One password check through polkit's helper. polkit 128 runs the helper per connection on
//! a systemd socket; we send the user name and cookie, then answer its PAM prompts. The
//! helper checks the password and tells polkitd itself; we only relay text.

use std::io::{self, ErrorKind, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;

const SOCKET: &str = "/run/polkit/agent-helper.socket";
/// Longest line we keep (prompts are short); more means a broken helper.
const MAX_LINE: usize = 4096;

/// What the helper said, one line each.
#[derive(Debug, PartialEq)]
pub enum Line {
    /// Asks for input; `echo` = show what is typed (a user name, not a password).
    Prompt { echo: bool, text: String },
    Error(String),
    Info(String),
    /// The check is over: true = authenticated.
    Done(bool),
}

pub struct Helper {
    sock: UnixStream,
    buf: Vec<u8>,
}

impl Helper {
    pub fn start(user: &str, cookie: &str) -> io::Result<Helper> {
        let mut sock = UnixStream::connect(SOCKET)?;
        sock.write_all(format!("{user}\n{cookie}\n").as_bytes())?;
        sock.set_nonblocking(true)?;
        Ok(Helper { sock, buf: Vec::new() })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.sock.as_fd()
    }

    /// All whole lines that arrived; call when `fd` is readable. Hang-up before `Done` is
    /// reported as `Done(false)`.
    pub fn read(&mut self) -> io::Result<Vec<Line>> {
        let mut chunk = [0u8; 1024];
        let mut closed = false;
        loop {
            match self.sock.read(&mut chunk) {
                Ok(0) => break closed = true,
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
            if self.buf.len() > MAX_LINE * 4 {
                return Err(io::Error::other("helper sent too much"));
            }
        }
        let mut lines = Vec::new();
        while let Some(end) = self.buf.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = self.buf.drain(..=end).collect();
            lines.extend(parse(&String::from_utf8_lossy(&raw[..end])));
        }
        if closed && !lines.iter().any(|l| matches!(l, Line::Done(_))) {
            lines.push(Line::Done(false));
        }
        Ok(lines)
    }

    /// Sends one answer and wipes it. Blocking: the answer is tiny and the helper waits for it.
    pub fn answer(&mut self, text: &mut Vec<u8>) -> io::Result<()> {
        text.push(b'\n');
        self.sock.set_nonblocking(false)?;
        let sent = self.sock.write_all(text);
        wipe(text);
        self.sock.set_nonblocking(true)?;
        sent
    }
}

/// Zeroes a secret so it doesn't linger in freed memory.
pub fn wipe(secret: &mut Vec<u8>) {
    for b in secret.iter_mut() {
        // SAFETY: `b` is a valid, exclusive reference; volatile keeps the write.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    secret.clear();
}

fn parse(line: &str) -> Option<Line> {
    let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
    let text = unescape(rest);
    Some(match word {
        "PAM_PROMPT_ECHO_OFF" => Line::Prompt { echo: false, text },
        "PAM_PROMPT_ECHO_ON" => Line::Prompt { echo: true, text },
        "PAM_ERROR_MSG" => Line::Error(text),
        "PAM_TEXT_INFO" => Line::Info(text),
        "SUCCESS" => Line::Done(true),
        "FAILURE" => Line::Done(false),
        _ => return None,
    })
}

/// Undoes the helper's C-style escaping (g_strescape): `\n`, `\t`, `\\`, `\"`, `\NNN` octal.
fn unescape(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let mut bytes = s.bytes().peekable();
    while let Some(b) = bytes.next() {
        if b != b'\\' {
            out.push(b);
            continue;
        }
        match bytes.next() {
            Some(d @ b'0'..=b'7') => {
                let mut n = u32::from(d - b'0');
                for _ in 0..2 {
                    match bytes.peek() {
                        Some(&d @ b'0'..=b'7') => {
                            n = n * 8 + u32::from(d - b'0');
                            bytes.next();
                        }
                        _ => break,
                    }
                }
                out.push(n as u8); // shortcut: \400+ wraps; g_strescape never writes those
            }
            Some(b'n') => out.push(b'\n'),
            Some(b't') => out.push(b'\t'),
            Some(b'r') => out.push(b'\r'),
            Some(c) => out.push(c),
            None => out.push(b'\\'),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_parse_and_unescape() {
        let p = |s| parse(s);
        let prompt = Line::Prompt { echo: false, text: "Password: ".into() };
        assert_eq!(p("PAM_PROMPT_ECHO_OFF Password: "), Some(prompt));
        assert_eq!(p("PAM_ERROR_MSG a\\nb \\\"c\\\" \\303\\274"), Some(Line::Error("a\nb \"c\" ü".into())));
        assert_eq!(p("SUCCESS"), Some(Line::Done(true)));
        assert_eq!(p("FAILURE"), Some(Line::Done(false)));
        assert_eq!(p("junk"), None);
    }

    #[test]
    fn wipe_zeroes_and_clears() {
        let mut s = b"hunter2".to_vec();
        wipe(&mut s);
        assert!(s.is_empty());
    }
}
