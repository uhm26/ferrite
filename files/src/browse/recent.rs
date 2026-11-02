//! Recent files: what apps noted in ~/.local/share/recently-used.xbel (GTK, Firefox,
//! …), newest first. Shown like search results (the Recent place in the sidebar).

use std::path::PathBuf;

use crate::dir::{self, Entry};
use crate::uri;

/// Files listed at most.
const MAX: usize = 200;

/// The recent files that still exist, newest first, named like `~/Pictures/a.png`.
pub fn list(hidden: bool) -> Vec<Entry> {
    let home = crate::view::home();
    let file = home.join(".local/share/recently-used.xbel");
    let text = std::fs::read_to_string(file).unwrap_or_default();
    parse(&text)
        .into_iter()
        .filter(|p| p.symlink_metadata().is_ok())
        .take(MAX)
        .map(|p| {
            let shown = match p.strip_prefix(&home) {
                Ok(rest) => format!("~/{}", rest.display()),
                Err(_) => p.display().to_string(),
            };
            dir::entry(&p, (p.clone().into_os_string(), shown), hidden)
        })
        .collect()
}

/// The file paths in an xbel document, newest `modified` first.
// shortcut: scans for the two attributes instead of parsing XML; fine for the files
// GLib writes, switch to a real parser if other writers break it.
fn parse(xml: &str) -> Vec<PathBuf> {
    let mut found: Vec<(&str, PathBuf)> = xml
        .split("<bookmark") // keeps the space before the first attribute
        .skip(1)
        .filter_map(|b| {
            let tag = &b[..b.find('>')?];
            let href = attr(tag, "href")?.replace("&amp;", "&");
            Some((
                attr(tag, "modified").unwrap_or(""),
                uri::from_file_uri(&href)?,
            ))
        })
        .collect();
    // ISO 8601 times sort as text.
    found.sort_by(|a, b| b.0.cmp(a.0));
    found.into_iter().map(|(_, p)| p).collect()
}

/// The value of `name="…"` in a tag.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let at = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let len = tag[at..].find('"')?;
    Some(&tag[at..at + len])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_and_decoded() {
        let xml = r#"<xbel>
  <bookmark href="file:///a/old%20one.txt" added="x" modified="2026-01-01T00:00:00Z">
  </bookmark>
  <bookmark href="https://example.com/" modified="2026-12-01T00:00:00Z"></bookmark>
  <bookmark href="file:///b/new&amp;more.png" modified="2026-11-01T09:00:00Z"/>
</xbel>"#;
        assert_eq!(
            parse(xml),
            [
                PathBuf::from("/b/new&more.png"),
                PathBuf::from("/a/old one.txt")
            ]
        );
    }
}
