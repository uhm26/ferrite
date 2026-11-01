//! `file://` URIs and percent-encoding, as GTK bookmarks and the FreeDesktop trash
//! (`Path=` in .trashinfo) write paths. Bytes, not chars: paths need not be UTF-8.

use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

/// Characters kept as they are (RFC 3986 unreserved, plus `/`).
fn plain(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-._~/".contains(&b)
}

/// Percent-encode a path's bytes.
pub fn encode(p: &Path) -> String {
    p.as_os_str()
        .as_bytes()
        .iter()
        .map(|&b| {
            if plain(b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Undo [`encode`]; a broken `%` sequence is kept as written.
pub fn decode(s: &str) -> PathBuf {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = b
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok());
        match (b[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    PathBuf::from(OsString::from_vec(out))
}

/// `file:///a%20b` → `/a b`; None for other schemes.
pub fn from_file_uri(s: &str) -> Option<PathBuf> {
    s.strip_prefix("file://").map(decode)
}

pub fn to_file_uri(p: &Path) -> String {
    format!("file://{}", encode(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_odd_paths() {
        let odd = PathBuf::from(OsString::from_vec(b"/home/g/caf\xe9 a+b%".to_vec()));
        let uri = to_file_uri(&odd);
        assert_eq!(uri, "file:///home/g/caf%E9%20a%2Bb%25");
        assert_eq!(from_file_uri(&uri), Some(odd));
        assert_eq!(from_file_uri("sftp://x"), None);
        assert_eq!(decode("50%"), PathBuf::from("50%"));
    }
}
