//! The file picker other apps get (Firefox, Discord, …): an xdg-desktop-portal
//! backend. `files --portal` serves org.freedesktop.impl.portal.FileChooser on the
//! session bus (D-Bus starts it on first use, see install.sh). Each request opens a
//! picker window (`files --pick …`, view/pick.rs) whose answer goes back to the app.
//! It quits after a few idle minutes.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::channel;

use frame::sys::{POLLIN, PollFd};

use frame::dbus::{Conn, Kind, Message, Type, Value};
use crate::uri;

const NAME: &str = "org.freedesktop.impl.portal.desktop.files";
const IFACE: &str = "org.freedesktop.impl.portal.FileChooser";
/// Quit after this long without requests, ms.
const IDLE_MS: i32 = 5 * 60 * 1000;

type Answer = (Message, Option<Vec<PathBuf>>);

/// Serve picker requests until idle.
pub fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let mut conn = Conn::session()?;
    conn.request_name(NAME)?;
    let (wake_r, wake_w) = frame::sys::pipe()?;
    frame::sys::set_nonblocking(wake_r.as_fd())?;
    let (mut wake_r, wake_w) = (File::from(wake_r), File::from(wake_w));
    let (tx, rx) = channel::<Answer>();
    let mut pending = 0usize;
    loop {
        let poll = |fd| PollFd {
            fd,
            events: POLLIN,
            revents: 0,
        };
        let mut fds = [poll(conn.fd()), poll(wake_r.as_raw_fd())];
        frame::sys::wait(&mut fds, if pending == 0 { IDLE_MS } else { -1 })?;
        let idle = fds.iter().all(|f| !f.ready(POLLIN));
        if idle && pending == 0 {
            return Ok(());
        }
        let mut sink = [0u8; 64];
        while matches!(wake_r.read(&mut sink), Ok(n) if n > 0) {}
        for (msg, picked) in rx.try_iter() {
            pending -= 1;
            conn.reply(&msg, &answer(&msg, picked))?;
        }
        for msg in conn.read_ready()? {
            if msg.kind != Kind::Call {
                continue;
            }
            match (msg.interface.as_deref(), msg.member.as_deref()) {
                (Some(IFACE), Some("OpenFile" | "SaveFile" | "SaveFiles")) => {
                    pending += 1;
                    let (tx, mut wake) = (tx.clone(), wake_w.try_clone()?);
                    std::thread::spawn(move || {
                        let picked = pick(&msg);
                        let _ = tx.send((msg, picked));
                        let _ = wake.write_all(&[1]);
                    });
                }
                _ => {
                    let e = "org.freedesktop.DBus.Error.UnknownMethod";
                    conn.reply_error(&msg, e, "files serves FileChooser only")?;
                }
            }
        }
    }
}

/// `(u response, a{sv} results)`: 0 and the picked files' uris, or 1 (cancelled).
fn answer(msg: &Message, picked: Option<Vec<PathBuf>>) -> Vec<Value> {
    let Some(mut paths) = picked else {
        return vec![Value::U32(1), Value::dict_sv(Vec::new())];
    };
    if msg.member.as_deref() == Some("SaveFiles") {
        // A folder was picked: the files go into it under their own names.
        let dir = paths.pop().unwrap_or_default();
        let names = opt(msg, "files")
            .and_then(Value::as_array)
            .unwrap_or_default();
        paths = names
            .iter()
            .filter_map(Value::as_bytes)
            .map(|n| dir.join(bytes_path(n)))
            .collect();
    }
    let uris = paths
        .iter()
        .map(|p| Value::str(&uri::to_file_uri(p)))
        .collect();
    let uris = Value::Array(Type::Str, uris);
    vec![Value::U32(0), Value::dict_sv(vec![("uris", uris)])]
}

/// An option of the request (`a{sv}`, the 5th argument).
fn opt<'a>(msg: &'a Message, key: &str) -> Option<&'a Value> {
    msg.body.get(4)?.get(key)
}

/// A NUL-terminated byte path (`ay`).
fn bytes_path(b: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    let b = b.strip_suffix(&[0]).unwrap_or(b);
    PathBuf::from(std::ffi::OsStr::from_bytes(b))
}

/// Our binary, for the picker windows. This backend lives for the whole session, so the
/// file may have been replaced by an update since it started: Linux then names it
/// "<path> (deleted)", which can't be run; the new binary at <path> can.
fn own_exe() -> Option<PathBuf> {
    Some(live_path(std::env::current_exe().ok()?))
}

fn live_path(p: PathBuf) -> PathBuf {
    match p.to_str().and_then(|s| s.strip_suffix(" (deleted)")) {
        Some(s) if !p.exists() => PathBuf::from(s),
        _ => p,
    }
}

/// Run a picker window for the request; the paths picked, None if cancelled.
fn pick(msg: &Message) -> Option<Vec<PathBuf>> {
    let out = Command::new(own_exe()?)
        .args(pick_args(msg))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let paths: Vec<PathBuf> = out
        .stdout
        .split(|&b| b == 0)
        .filter(|p| !p.is_empty())
        .map(bytes_path)
        .collect();
    (!paths.is_empty()).then_some(paths)
}

/// `files --pick` arguments for a request.
// shortcut: filters (file types) are ignored, every file shows; add when an app's
// picker needs them.
fn pick_args(msg: &Message) -> Vec<String> {
    let member = msg.member.as_deref().unwrap_or("");
    let flag = |k| opt(msg, k).and_then(Value::as_bool).unwrap_or(false);
    let text = |k| opt(msg, k).and_then(Value::as_str).map(str::to_string);
    let mode = match member {
        "SaveFile" => "save",
        "SaveFiles" => "dir",
        _ if flag("directory") => "dir",
        _ => "open",
    };
    let mut a = vec!["--pick".to_string(), mode.into()];
    if flag("multiple") {
        a.push("--multiple".into());
    }
    let title = msg.body.get(3).and_then(Value::as_str).unwrap_or("");
    let more = [
        ("--title", Some(title.to_string())),
        ("--accept", text("accept_label")),
    ];
    let name = if member == "SaveFile" {
        text("current_name")
    } else {
        None
    };
    for (f, v) in more.into_iter().chain([("--name", name)]) {
        if let Some(v) = v.filter(|v| !v.is_empty()) {
            a.extend([f.to_string(), v]);
        }
    }
    let folder = opt(msg, "current_folder").and_then(Value::as_bytes);
    a.push(
        folder
            .map_or_else(crate::view::home, bytes_path)
            .display()
            .to_string(),
    );
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replaced_binary_is_run_from_its_path() {
        let gone = PathBuf::from("/nonexistent/files (deleted)");
        assert_eq!(live_path(gone), PathBuf::from("/nonexistent/files"));
        // A real file whose name ends that way is left alone.
        let dir = std::env::temp_dir().join(format!("files-exe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let odd = dir.join("x (deleted)");
        std::fs::write(&odd, "").unwrap();
        let kept = live_path(odd.clone());
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(kept, odd);
    }

    fn request(member: &str, opts: Vec<(&str, Value)>) -> Message {
        let body = vec![
            Value::path("/r/1"),
            Value::str("firefox"),
            Value::str(""),
            Value::str("Upload"),
            Value::dict_sv(opts),
        ];
        let mut m = Message::call(NAME, "/org/freedesktop/portal/desktop", IFACE, member, body);
        m.serial = 7;
        m
    }

    #[test]
    fn requests_become_picker_arguments() {
        let open = request(
            "OpenFile",
            vec![
                ("multiple", Value::Bool(true)),
                ("current_folder", Value::Bytes(b"/tmp\0".to_vec())),
            ],
        );
        assert_eq!(
            pick_args(&open),
            ["--pick", "open", "--multiple", "--title", "Upload", "/tmp"]
        );
        let save = request("SaveFile", vec![("current_name", Value::str("a.png"))]);
        let args = pick_args(&save);
        assert_eq!(&args[..2], ["--pick", "save"]);
        assert!(args.windows(2).any(|w| w == ["--name", "a.png"]));
    }

    #[test]
    fn answers_carry_uris_or_cancel() {
        let open = request("OpenFile", vec![]);
        let ok = answer(&open, Some(vec![PathBuf::from("/a b.txt")]));
        let uris = ok[1].get("uris").and_then(Value::as_array).unwrap();
        assert_eq!(uris[0].as_str(), Some("file:///a%20b.txt"));
        assert_eq!(answer(&open, None)[0].as_u64(), Some(1));
        let names = Value::Array(
            Type::Array(Box::new(Type::Byte)),
            vec![Value::Bytes(b"x.txt\0".to_vec())],
        );
        let many = request("SaveFiles", vec![("files", names)]);
        let ok = answer(&many, Some(vec![PathBuf::from("/d")]));
        let uris = ok[1].get("uris").and_then(Value::as_array).unwrap();
        assert_eq!(uris[0].as_str(), Some("file:///d/x.txt"));
    }
}
