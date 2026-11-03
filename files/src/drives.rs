//! Drives: USB sticks, SD cards and other disks with a filesystem, from udisks2 on
//! the system bus (our own D-Bus client). A worker thread lists them, mounts and
//! ejects; it relists whenever udisks reports a change, and wakes the window through
//! a pipe. Without udisks2 the list stays empty.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use frame::sys::{POLLIN, PollFd};

use frame::dbus::{Conn, Error, Value};

const UDISKS: &str = "org.freedesktop.UDisks2";
const ROOT: &str = "/org/freedesktop/UDisks2";
const BLOCK: &str = "org.freedesktop.UDisks2.Block";
const FS: &str = "org.freedesktop.UDisks2.Filesystem";
const DRIVE: &str = "org.freedesktop.UDisks2.Drive";

/// A filesystem we can mount.
#[derive(Clone, Debug, PartialEq)]
pub struct Drive {
    /// "Backup" (its label), else "32 GB Volume".
    pub name: String,
    /// udisks' object for the filesystem, and for the disk it is on.
    block: String,
    disk: String,
    /// Where it is mounted, if it is.
    pub mount: Option<PathBuf>,
    /// The disk can be unplugged (eject powers it off).
    pub removable: bool,
}

enum Req {
    Mount(String),
    /// Filesystem, disk, and its name for the report.
    Eject(String, String, String),
}

/// What the worker reports.
pub enum Report {
    List(Vec<Drive>),
    /// A mount finished: go there.
    Mounted(PathBuf),
    /// Safe to unplug now (its name).
    Ejected(String),
    /// An internal one, unmounted (its name).
    Unmounted(String),
    Failed(String),
}

pub struct Drives {
    pub list: Vec<Drive>,
    tx: Sender<Req>,
    rx: Receiver<Report>,
    /// Wakes the worker for a request; the worker wakes us back.
    poke: File,
    wake_r: File,
}

impl Drives {
    pub fn new() -> io::Result<Self> {
        let (wake_r, wake_w) = frame::sys::pipe()?;
        let (poke_r, poke_w) = frame::sys::pipe()?;
        frame::sys::set_nonblocking(wake_r.as_fd())?;
        frame::sys::set_nonblocking(poke_r.as_fd())?;
        let (tx, req_rx) = channel();
        let (rep_tx, rx) = channel();
        let (poke_r, wake_w) = (File::from(poke_r), File::from(wake_w));
        std::thread::spawn(move || {
            if let Ok(conn) = Conn::system() {
                work(conn, (&req_rx, poke_r), (&rep_tx, wake_w));
            }
        });
        Ok(Drives {
            list: Vec::new(),
            tx,
            rx,
            poke: File::from(poke_w),
            wake_r: File::from(wake_r),
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.wake_r.as_fd()
    }

    /// Reports since last asked; the list is taken in here.
    pub fn reports(&mut self) -> Vec<Report> {
        let mut sink = [0u8; 64];
        while matches!(self.wake_r.read(&mut sink), Ok(n) if n > 0) {}
        let all: Vec<Report> = self.rx.try_iter().collect();
        for r in &all {
            if let Report::List(l) = r {
                self.list.clone_from(l);
            }
        }
        all
    }

    pub fn mount(&mut self, d: &Drive) {
        self.ask(Req::Mount(d.block.clone()));
    }

    pub fn eject(&mut self, d: &Drive) {
        // Only an unpluggable disk is powered off: never the internal one (it may hold /).
        let disk = if d.removable {
            d.disk.clone()
        } else {
            "/".into()
        };
        self.ask(Req::Eject(d.block.clone(), disk, d.name.clone()));
    }

    fn ask(&mut self, r: Req) {
        if self.tx.send(r).is_ok() {
            let _ = self.poke.write_all(&[1]);
        }
    }
}

type Inbox<'a> = (&'a Receiver<Req>, File);
type Outbox<'a> = (&'a Sender<Report>, File);

/// The worker: list, then wait for udisks signals or our requests.
fn work(mut conn: Conn, (rx, mut poke): Inbox, (tx, mut wake): Outbox) {
    let rule = format!("type='signal',sender='{UDISKS}',path_namespace='{ROOT}'");
    if conn.add_match(&rule).is_err() {
        return;
    }
    let mut send = |r: Report| tx.send(r).is_ok() && wake.write_all(&[1]).is_ok();
    loop {
        // udisks2 missing (ServiceUnknown): nothing to show, nothing to do.
        let Ok(list) = list(&mut conn) else {
            return;
        };
        let _ = conn.read_ready(); // signals queued during the call: covered by this list
        if !send(Report::List(list)) {
            return;
        }
        let mut fds = [poll_fd(conn.fd()), poll_fd(poke.as_raw_fd())];
        if frame::sys::wait(&mut fds, -1).is_err() {
            return;
        }
        let mut sink = [0u8; 64];
        while matches!(poke.read(&mut sink), Ok(n) if n > 0) {}
        for req in rx.try_iter() {
            let report = match run(&mut conn, req) {
                Ok(r) => r,
                Err(e) => Report::Failed(e.to_string()),
            };
            if !send(report) {
                return;
            }
        }
        if conn.read_ready().is_err() {
            return;
        }
    }
}

fn poll_fd(fd: i32) -> PollFd {
    PollFd {
        fd,
        events: POLLIN,
        revents: 0,
    }
}

/// Mount or eject (unmount, then power the disk off when it can be).
fn run(conn: &mut Conn, req: Req) -> Result<Report, Error> {
    let none = || [Value::dict_sv(Vec::new())];
    match req {
        Req::Mount(block) => {
            let out = conn.call(UDISKS, &block, &format!("{FS}.Mount"), &none())?;
            let at = out.first().and_then(Value::as_str).unwrap_or("/");
            Ok(Report::Mounted(PathBuf::from(at)))
        }
        Req::Eject(block, disk, name) => {
            conn.call(UDISKS, &block, &format!("{FS}.Unmount"), &none())?;
            if disk == "/" {
                return Ok(Report::Unmounted(name));
            }
            // Some readers can't power off; unmounted is safe enough then.
            let _ = conn.call(UDISKS, &disk, &format!("{DRIVE}.PowerOff"), &none());
            Ok(Report::Ejected(name))
        }
    }
}

/// Every filesystem udisks would let us mount: not the system's own partitions.
fn list(conn: &mut Conn) -> Result<Vec<Drive>, Error> {
    let method = "org.freedesktop.DBus.ObjectManager.GetManagedObjects";
    let out = conn.call(UDISKS, ROOT, method, &[])?;
    Ok(out.first().map(drives).unwrap_or_default())
}

/// The drives in a GetManagedObjects answer (`a{oa{sa{sv}}}`), sorted by name.
fn drives(objects: &Value) -> Vec<Drive> {
    let mut out: Vec<Drive> = objects
        .as_dict()
        .unwrap_or_default()
        .iter()
        .filter_map(|(path, ifaces)| {
            let block = ifaces.get(BLOCK)?;
            let fs = ifaces.get(FS)?;
            let flag = |k| block.get(k).and_then(Value::as_bool).unwrap_or(false);
            let mount = mount_point(fs);
            // Internal partitions show too (an old install, Windows); the system's own
            // mounts (/, /efi, /home …) don't: anything mounted outside the media dirs.
            if flag("HintIgnore") || mount.as_deref().is_some_and(|m| !media(m)) {
                return None;
            }
            let disk = block.get("Drive").and_then(Value::as_str).unwrap_or("/");
            let removable = objects
                .get(disk)
                .and_then(|d| d.get(DRIVE))
                .and_then(|d| d.get("Removable"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Some(Drive {
                name: name(block),
                block: path.as_str()?.to_string(),
                disk: disk.to_string(),
                mount,
                removable,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Mounted where removable and user mounts go (udisks: /run/media/<user>/…).
fn media(at: &Path) -> bool {
    ["/run/media", "/media", "/mnt"]
        .iter()
        .any(|d| at.starts_with(d))
}

/// Its label, else its size: "32 GB Volume".
fn name(block: &Value) -> String {
    let text = |k| block.get(k).and_then(Value::as_str).unwrap_or("").trim();
    for k in ["HintName", "IdLabel"] {
        if !text(k).is_empty() {
            return text(k).to_string();
        }
    }
    let size = block.get("Size").and_then(Value::as_u64).unwrap_or(0);
    format!("{} Volume", crate::dir::human(size))
}

/// The first of `MountPoints` (`aay`, each NUL-terminated).
fn mount_point(fs: &Value) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let first = fs.get("MountPoints")?.as_array()?.first()?.as_bytes()?;
    let bytes = first.strip_suffix(&[0]).unwrap_or(first);
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use frame::dbus::Type;

    fn sv(pairs: Vec<(&str, Value)>) -> Value {
        Value::dict_sv(pairs)
    }

    fn ifaces(pairs: Vec<(&str, Value)>) -> Value {
        let a_sv = Type::Dict(Box::new(Type::Str), Box::new(Type::Variant));
        let entries = pairs.into_iter().map(|(k, v)| (Value::str(k), v)).collect();
        Value::Dict(Type::Str, a_sv, entries)
    }

    #[test]
    fn lists_mountable_filesystems_but_not_system_ones() {
        let mounted = Value::Array(
            Type::Array(Box::new(Type::Byte)),
            vec![Value::Bytes(b"/run/media/gabe/STICK\0".to_vec())],
        );
        let stick = ifaces(vec![
            (
                BLOCK,
                sv(vec![
                    ("IdLabel", Value::str("STICK")),
                    ("Drive", Value::path("/d/usb")),
                ]),
            ),
            (FS, sv(vec![("MountPoints", mounted)])),
        ]);
        let at = |p: &[u8]| Value::Array(Type::Array(Box::new(Type::Byte)), vec![Value::Bytes(p.to_vec())]);
        let root = ifaces(vec![
            (BLOCK, sv(vec![("HintSystem", Value::Bool(true))])),
            (FS, sv(vec![("MountPoints", at(b"/\0"))])),
        ]);
        let old = ifaces(vec![
            (BLOCK, sv(vec![("HintSystem", Value::Bool(true)), ("IdLabel", Value::str("Arch"))])),
            (FS, sv(vec![])),
        ]);
        let usb = ifaces(vec![(DRIVE, sv(vec![("Removable", Value::Bool(true))]))]);
        let nameless = ifaces(vec![
            (BLOCK, sv(vec![("Size", Value::U64(32 << 30))])),
            (FS, sv(vec![])),
        ]);
        let inner = ifaces(vec![]).ty();
        let objects = Value::Dict(
            Type::Path,
            inner,
            vec![
                (Value::path("/b/sdb1"), stick),
                (Value::path("/b/nvme0n1p2"), root),
                (Value::path("/b/nvme0n1p5"), old),
                (Value::path("/d/usb"), usb),
                (Value::path("/b/sdc1"), nameless),
            ],
        );
        let got = drives(&objects);
        let names: Vec<&str> = got.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["32 GB Volume", "Arch", "STICK"], "/ stays out, an old install shows");
        assert_eq!(got[2].mount, Some(PathBuf::from("/run/media/gabe/STICK")));
        assert!(got[2].removable && !got[0].removable);
        assert_eq!(got[0].mount, None);
    }
}
