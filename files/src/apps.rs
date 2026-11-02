//! Apps that can open a file: .desktop entries (XDG data dirs) whose MimeType lists the
//! file's type, the xdg-mime default first. Launching fills Exec's field codes in;
//! Terminal=true apps run in our terminal (`term -e`). Types and defaults go through
//! xdg-mime, so Nemo, xdg-open and we agree.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    /// Desktop file id, e.g. `firefox.desktop`.
    pub id: String,
    pub name: String,
    exec: String,
    terminal: bool,
}

/// The file's MIME type, as xdg-mime sees it.
pub fn mime_of(p: &Path) -> Option<String> {
    let out = Command::new("xdg-mime")
        .args(["query", "filetype"])
        .arg(p)
        .output()
        .ok()?;
    let m = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !m.is_empty()).then_some(m)
}

fn default_for(mime: &str) -> Option<String> {
    let out = Command::new("xdg-mime")
        .args(["query", "default", mime])
        .output()
        .ok()?;
    let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!id.is_empty()).then_some(id)
}

/// Make `app` open `mime` files from now on (xdg-open, Nemo and we follow it).
pub fn set_default(app: &App, mime: &str) -> io::Result<()> {
    let ok = Command::new("xdg-mime")
        .args(["default", &app.id, mime])
        .status()?
        .success();
    if ok {
        Ok(())
    } else {
        Err(io::Error::other("xdg-mime default failed"))
    }
}

/// `applications/` under `$XDG_DATA_HOME`, then each of `$XDG_DATA_DIRS`.
fn dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::view::home().join(".local/share"));
    let rest =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    std::iter::once(home)
        .chain(rest.split(':').map(PathBuf::from))
        .map(|d| d.join("applications"))
        .collect()
}

/// Apps that open `mime` (text/* also gets text/plain apps), the default first, then
/// by name, and whether the first is the default.
pub fn for_mime(mime: &str) -> (Vec<App>, bool) {
    let mut seen = std::collections::HashSet::new();
    let mut apps: Vec<App> = Vec::new();
    for d in dirs() {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        for e in rd.filter_map(Result::ok) {
            let id = e.file_name().to_string_lossy().into_owned();
            if !id.ends_with(".desktop") || !seen.insert(id.clone()) {
                continue; // earlier dirs win (user entries override system ones)
            }
            let Ok(text) = std::fs::read_to_string(e.path()) else {
                continue;
            };
            if let Some((app, mimes)) = parse(&id, &text)
                && mimes.iter().any(|m| fits(m, mime))
            {
                apps.push(app);
            }
        }
    }
    apps.sort_by_key(|a| a.name.to_lowercase());
    let default = default_for(mime);
    let at = apps.iter().position(|a| Some(&a.id) == default.as_ref());
    if let Some(i) = at {
        let d = apps.remove(i);
        apps.insert(0, d);
    }
    (apps, at.is_some())
}

/// Does an app listing `listed` open `mime`?
fn fits(listed: &str, mime: &str) -> bool {
    listed == mime
        || listed
            .strip_suffix("/*")
            .is_some_and(|t| mime.split('/').next() == Some(t))
        || (mime.starts_with("text/") && listed == "text/plain")
}

/// An app and its MIME types from a .desktop file; None if hidden or not an app.
fn parse(id: &str, text: &str) -> Option<(App, Vec<String>)> {
    let mut entry = false;
    let (mut name, mut exec, mut mimes, mut terminal) = (None, None, Vec::new(), false);
    for line in text.lines() {
        if line.starts_with('[') {
            entry = line == "[Desktop Entry]";
            continue;
        }
        let Some((k, v)) = line.split_once('=').filter(|_| entry) else {
            continue;
        };
        match k.trim() {
            "Name" => name = Some(v.trim().to_string()),
            "Exec" => exec = Some(v.trim().to_string()),
            "MimeType" => {
                mimes = v
                    .split(';')
                    .filter(|m| !m.is_empty())
                    .map(String::from)
                    .collect()
            }
            "Terminal" => terminal = v.trim() == "true",
            "NoDisplay" | "Hidden" if v.trim() == "true" => return None,
            "Type" if v.trim() != "Application" => return None,
            _ => {}
        }
    }
    let app = App {
        id: id.to_string(),
        name: name?,
        exec: exec?,
        terminal,
    };
    Some((app, mimes))
}

/// Exec as argv for opening `files`: quotes honored, `%f %u` → the first file,
/// `%F %U` → all, `%%` → `%`, other codes dropped; no file code → files appended.
fn argv(exec: &str, files: &[PathBuf]) -> Vec<String> {
    let mut out = Vec::new();
    let mut used = false;
    for word in split(exec) {
        match word.as_str() {
            "%f" | "%u" => {
                out.extend(files.first().map(|p| p.display().to_string()));
                used = true;
            }
            "%F" | "%U" => {
                out.extend(files.iter().map(|p| p.display().to_string()));
                used = true;
            }
            w if w.len() == 2 && w.starts_with('%') && w != "%%" => {}
            w => out.push(w.replace("%%", "%")),
        }
    }
    if !used {
        out.extend(files.iter().map(|p| p.display().to_string()));
    }
    out
}

/// Split an Exec line into words (double quotes group, backslash escapes inside).
fn split(s: &str) -> Vec<String> {
    let (mut words, mut cur, mut quoted, mut any) = (Vec::new(), String::new(), false, false);
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => (quoted, any) = (!quoted, true),
            '\\' if quoted => cur.extend(chars.next()),
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        words.push(cur);
    }
    words
}

/// Open `files` with `app` (detached).
pub fn launch(app: &App, files: &[PathBuf]) -> io::Result<()> {
    let mut words = argv(&app.exec, files);
    if app.terminal {
        words.splice(0..0, ["term".to_string(), "-e".to_string()]);
    }
    let (prog, args) = words
        .split_first()
        .ok_or_else(|| io::Error::other("empty Exec"))?;
    let mut child = Command::new(prog).args(args).spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Run a typed command on `files` ("nvim" → `nvim <files>`), through sh so the owner
/// can write pipes or flags, in `dir`, detached.
pub fn run_command(cmd: &str, files: &[PathBuf], dir: &Path) -> io::Result<()> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(format!("{cmd} \"$@\""))
        .arg("sh")
        .args(files)
        .current_dir(dir)
        .spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_lines_become_argv() {
        let f = [PathBuf::from("/a b.md"), PathBuf::from("/c.md")];
        assert_eq!(argv("zed %F", &f), ["zed", "/a b.md", "/c.md"]);
        assert_eq!(argv("firefox %u", &f), ["firefox", "/a b.md"]);
        assert_eq!(
            argv("\"/opt/my app/run\" --x %i", &f[..1]),
            ["/opt/my app/run", "--x", "/a b.md"]
        );
        assert_eq!(argv("echo 100%%", &f[..1]), ["echo", "100%", "/a b.md"]);
    }

    #[test]
    fn desktop_entries_parse_and_match() {
        let text = "[Desktop Entry]\nType=Application\nName=Zed\nExec=zed %F\nMimeType=text/plain;text/markdown;\n[Desktop Action new]\nName=New Window\n";
        let (app, mimes) = parse("zed.desktop", text).unwrap();
        assert_eq!((app.name.as_str(), app.exec.as_str()), ("Zed", "zed %F"));
        assert!(
            mimes.iter().any(|m| fits(m, "text/x-rust")),
            "text/* gets text/plain apps"
        );
        assert!(fits("image/*", "image/png") && !fits("image/png", "video/mp4"));
        assert!(
            parse(
                "x.desktop",
                "[Desktop Entry]\nName=X\nExec=x\nNoDisplay=true\n"
            )
            .is_none()
        );
    }
}
