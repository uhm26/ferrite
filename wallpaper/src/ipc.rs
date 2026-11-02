//! One running `wallpaper` per session. The first `wallpaper <image>` becomes the daemon and
//! listens on $XDG_RUNTIME_DIR/wallpaper.sock; later calls send it the path and exit.
//! Protocol: client writes `<absolute path>\n`, daemon answers `ok\n` or `err <why>\n`.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Longest request line we read; a path is never near this.
const MAX_REQUEST: u64 = 4096;

pub enum Role {
    /// Another `wallpaper` is running; this is a connection to it.
    Client(UnixStream),
    /// We are the daemon.
    Daemon(UnixListener),
}

/// No fallback to /tmp: anyone could plant a socket there.
fn socket_path() -> io::Result<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))?;
    Ok(PathBuf::from(dir).join("wallpaper.sock"))
}

/// Connect to a running daemon, or become it.
pub fn claim() -> io::Result<Role> {
    let path = socket_path()?;
    // Held until we return, so two `wallpaper`s starting together can't both take over a
    // stale socket: the second waits, then finds the first one listening.
    let lock = File::create(path.with_extension("lock"))?;
    lock.lock()?;
    match UnixStream::connect(&path) {
        Ok(s) => return Ok(Role::Client(s)),
        // Socket file left over from a daemon that died: take it over.
        Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => std::fs::remove_file(&path)?,
        Err(_) => {}
    }
    let l = UnixListener::bind(&path)?;
    l.set_nonblocking(true)?;
    Ok(Role::Daemon(l))
}

/// Client side: ask the daemon to show `image`. Returns its answer.
pub fn request(mut s: UnixStream, image: &Path) -> io::Result<Result<(), String>> {
    let path = image.as_os_str().as_encoded_bytes();
    if path.contains(&b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "image path contains a newline",
        ));
    }
    s.write_all(&[path, b"\n"].concat())?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line)?;
    Ok(match line.trim_end() {
        "ok" => Ok(()),
        other => Err(other.strip_prefix("err ").unwrap_or(other).to_string()),
    })
}

/// Daemon side: a client asking for a wallpaper.
pub struct Ask {
    pub image: PathBuf,
    stream: UnixStream,
}

/// Accept one waiting client, if any, and read its path. A client that sends nothing
/// usable is dropped and logged; only errors of the listener itself are returned.
pub fn accept(l: &UnixListener) -> io::Result<Option<Ask>> {
    loop {
        let stream = match l.accept() {
            Ok((s, _)) => s,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(e) if e.kind() == io::ErrorKind::ConnectionAborted => continue,
            Err(e) => return Err(e),
        };
        match read_path(&stream) {
            Ok(image) => return Ok(Some(Ask { image, stream })),
            Err(e) => eprintln!("wallpaper: dropped a client: {e}"),
        }
    }
}

fn read_path(stream: &UnixStream) -> io::Result<PathBuf> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    let mut line = Vec::new();
    BufReader::new(stream)
        .take(MAX_REQUEST)
        .read_until(b'\n', &mut line)?;
    if line.pop() != Some(b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "no newline-terminated path",
        ));
    }
    Ok(PathBuf::from(OsString::from_vec(line)))
}

impl Ask {
    pub fn answer(mut self, r: Result<(), String>) {
        let msg = match r {
            Ok(()) => "ok\n".to_string(),
            Err(e) => format!("err {e}\n"),
        };
        // The client may be gone already; nothing to do about it.
        let _ = self.stream.write_all(msg.as_bytes());
    }
}

pub fn remove_socket() {
    if let Ok(p) = socket_path() {
        let _ = std::fs::remove_file(p);
    }
}

/// $XDG_STATE_HOME/wallpaper/last (default ~/.local/state/wallpaper/last): the last shown image.
fn state_path() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_STATE_HOME") {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".local/state"),
    };
    Some(base.join("wallpaper/last"))
}

/// Remember `image` for a bare `wallpaper` (e.g. at login). Failing only costs that.
pub fn remember(image: &Path) {
    let Some(p) = state_path() else { return };
    let saved = p
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&p, image.as_os_str().as_encoded_bytes()));
    if let Err(e) = saved {
        eprintln!("wallpaper: can't remember the wallpaper in {}: {e}", p.display());
    }
}

/// The image `remember` saved last, if any.
pub fn last() -> Option<PathBuf> {
    let mut bytes = std::fs::read(state_path()?).ok()?;
    if bytes.last() == Some(&b'\n') {
        bytes.pop(); // hand-edited file
    }
    Some(PathBuf::from(OsString::from_vec(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Shutdown;

    #[test]
    fn bad_clients_are_dropped_not_fatal() {
        let sock = std::env::temp_dir().join(format!("wallpaper-test-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&sock);
        let l = UnixListener::bind(&sock).unwrap();
        l.set_nonblocking(true).unwrap();
        let silent = UnixStream::connect(&sock).unwrap();
        silent.shutdown(Shutdown::Write).unwrap();
        let mut junk = UnixStream::connect(&sock).unwrap();
        junk.write_all(&[0xff; 5000]).unwrap(); // no newline, over the cap
        let mut good = UnixStream::connect(&sock).unwrap();
        good.write_all(b"/a/\xffb.png\n").unwrap();
        let ask = accept(&l).unwrap().expect("the good client");
        assert_eq!(ask.image.as_os_str().as_encoded_bytes(), b"/a/\xffb.png");
        assert!(accept(&l).unwrap().is_none());
        std::fs::remove_file(sock).unwrap();
    }
}
