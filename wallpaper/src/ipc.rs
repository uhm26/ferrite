//! One running `wall` per session. The first `wall <image>` becomes the daemon and
//! listens on $XDG_RUNTIME_DIR/wall.sock; later calls send it the path and exit.
//! Protocol: client writes `<absolute path>\n`, daemon answers `ok\n` or `err <why>\n`.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub enum Role {
    /// Another `wall` is running; this is a connection to it.
    Client(UnixStream),
    /// We are the daemon.
    Daemon(UnixListener),
}

fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").unwrap_or_else(|| "/tmp".into());
    PathBuf::from(dir).join("wall.sock")
}

/// Connect to a running daemon, or become it.
pub fn claim() -> io::Result<Role> {
    let path = socket_path();
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
    writeln!(s, "{}", image.display())?;
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

/// Accept one waiting client, if any, and read its path.
pub fn accept(l: &UnixListener) -> io::Result<Option<Ask>> {
    let stream = match l.accept() {
        Ok((s, _)) => s,
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
        Err(e) => return Err(e),
    };
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    Ok(Some(Ask {
        image: PathBuf::from(line.trim_end()),
        stream,
    }))
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
    let _ = std::fs::remove_file(socket_path());
}

/// $XDG_STATE_HOME/wall/last (default ~/.local/state/wall/last): the last shown image.
fn state_path() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_STATE_HOME") {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".local/state"),
    };
    Some(base.join("wall/last"))
}

/// Remember `image` for a bare `wall` (e.g. at login). Failing only costs that.
pub fn remember(image: &Path) {
    let Some(p) = state_path() else { return };
    let saved = p
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&p, image.as_os_str().as_encoded_bytes()));
    if let Err(e) = saved {
        eprintln!("wall: can't remember the wallpaper in {}: {e}", p.display());
    }
}

/// The image `remember` saved last, if any.
pub fn last() -> Option<PathBuf> {
    let text = std::fs::read_to_string(state_path()?).ok()?;
    Some(PathBuf::from(text.trim_end()))
}
