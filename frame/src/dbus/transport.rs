//! Getting a socket: bus addresses and the SASL `EXTERNAL` login.

use super::Error;
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::linux::net::SocketAddrExt;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{SocketAddr, UnixStream};

unsafe extern "C" {
    /// libc: real user id of this process (`uid_t` is `u32` on Linux).
    fn getuid() -> u32;
}

pub fn system_address() -> String {
    std::env::var("DBUS_SYSTEM_BUS_ADDRESS")
        .unwrap_or_else(|_| "unix:path=/run/dbus/system_bus_socket".to_string())
}

/// `DBUS_SESSION_BUS_ADDRESS`, else `$XDG_RUNTIME_DIR/bus` like sd-bus.
pub fn session_address() -> Result<String, Error> {
    if let Ok(a) = std::env::var("DBUS_SESSION_BUS_ADDRESS") {
        return Ok(a);
    }
    let dir = std::env::var("XDG_RUNTIME_DIR")
        .map_err(|_| Error::Address("no DBUS_SESSION_BUS_ADDRESS or XDG_RUNTIME_DIR".into()))?;
    Ok(format!("unix:path={dir}/bus"))
}

/// Connects to the first usable `unix:` entry of a `;`-separated address list.
pub fn connect(addrs: &str) -> Result<UnixStream, Error> {
    let mut last = Error::Address(format!("no unix: address in {addrs:?}"));
    for params in addrs.split(';').filter_map(|a| a.strip_prefix("unix:")) {
        match open_unix(params) {
            Ok(sock) => return Ok(sock),
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn open_unix(params: &str) -> Result<UnixStream, Error> {
    for kv in params.split(',') {
        let (key, value) = kv.split_once('=').unwrap_or((kv, ""));
        match key {
            "path" => return Ok(UnixStream::connect(OsStr::from_bytes(&unescape(value)?))?),
            "abstract" => {
                let addr = SocketAddr::from_abstract_name(unescape(value)?)?;
                return Ok(UnixStream::connect_addr(&addr)?);
            }
            _ => {}
        }
    }
    Err(Error::Address(format!(
        "unix address without path= or abstract=: {params:?}"
    )))
}

/// Undoes `%xx` escapes in an address value.
pub fn unescape(s: &str) -> Result<Vec<u8>, Error> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while let Some(&c) = s.as_bytes().get(i) {
        if c == b'%' {
            let hex = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            out.push(hex.ok_or_else(|| Error::Address(format!("bad escape in {s:?}")))?);
            i += 3;
        } else {
            out.push(c);
            i += 1;
        }
    }
    Ok(out)
}

/// NUL byte, `AUTH EXTERNAL <hex uid>`, expect `OK <guid>`, then `BEGIN`.
pub fn authenticate(sock: &mut UnixStream) -> Result<(), Error> {
    // SAFETY: getuid takes no arguments, touches no memory and cannot fail.
    let uid = unsafe { getuid() };
    let hex: String = uid
        .to_string()
        .bytes()
        .map(|b| format!("{b:02x}"))
        .collect();
    sock.write_all(format!("\0AUTH EXTERNAL {hex}\r\n").as_bytes())?;
    let answer = read_line(sock)?;
    if !answer.starts_with("OK ") {
        return Err(Error::Auth(answer));
    }
    sock.write_all(b"BEGIN\r\n")?;
    Ok(())
}

/// One `\r\n`-terminated line, byte by byte so nothing after it is consumed.
fn read_line(sock: &mut UnixStream) -> Result<String, Error> {
    let mut line = Vec::new();
    let mut byte = [0u8];
    while !line.ends_with(b"\r\n") {
        if line.len() > 512 {
            return Err(Error::Auth("auth answer too long".into()));
        }
        if sock.read(&mut byte)? == 0 {
            return Err(Error::Closed);
        }
        line.push(byte[0]);
    }
    line.truncate(line.len() - 2);
    Ok(String::from_utf8_lossy(&line).into_owned())
}
