//! One run: parse args, capture, encode, write.
//! Usage: screenshot [-o OUTPUT | -s] [FILE | -]   (default ~/Pictures/screenshot-<date>.png; - is stdout)
//! -s: freeze the screen and pick a region or window; saved PNGs also go to the clipboard.

use std::fmt;
use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::select::{self, Pick};
use crate::{capture, clipboard, compose::Canvas, png, sys};

pub enum Error {
    Usage,
    Capture(capture::Error),
    Write(PathBuf, io::Error),
    Encode(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Usage => write!(f, "usage: screenshot [-o OUTPUT | -s] [FILE | -]"),
            Error::Capture(e) => write!(f, "{e}"),
            Error::Write(p, e) => write!(f, "{}: {e}", p.display()),
            Error::Encode(e) => write!(f, "encoding the PNG: {e}"),
        }
    }
}

impl From<capture::Error> for Error {
    fn from(e: capture::Error) -> Self {
        Error::Capture(e)
    }
}

struct Args {
    select: bool,
    output: Option<String>,
    file: Option<String>,
}

fn parse(mut it: impl Iterator<Item = String>) -> Result<Args, Error> {
    let mut a = Args {
        select: false,
        output: None,
        file: None,
    };
    while let Some(s) = it.next() {
        match s.as_str() {
            "-s" => a.select = true,
            "-o" => a.output = Some(it.next().ok_or(Error::Usage)?),
            f if (f == "-" || !f.starts_with('-')) && a.file.is_none() => a.file = Some(s),
            _ => return Err(Error::Usage),
        }
    }
    if a.select && a.output.is_some() {
        return Err(Error::Usage); // -s always shows every output
    }
    Ok(a)
}

pub fn run(args: impl Iterator<Item = String>) -> Result<(), Error> {
    let a = parse(args)?;
    if a.select {
        return pick(a.file);
    }
    let mut session = capture::Session::connect()?;
    let shots = session.grab_outputs(a.output.as_deref())?;
    let canvas = Canvas::new(&shots);
    let fill = |y, row: &mut [u8]| canvas.row(y, row);
    if a.file.as_deref() == Some("-") {
        let err = |e| Error::Write("stdout".into(), e);
        return png::encode(canvas.w, canvas.h, fill, &mut io::stdout().lock()).map_err(err);
    }
    let (path, file) = open(a.file);
    let err = |e| Error::Write(path.clone(), e);
    png::encode(canvas.w, canvas.h, fill, &mut file.map_err(err)?).map_err(err)
}

/// `-s`: freeze, pick, then save and serve the PNG on the clipboard until replaced.
fn pick(file: Option<String>) -> Result<(), Error> {
    let wins = std::thread::spawn(select::hypr::windows); // IPC runs during the capture
    let mut s = capture::Session::connect()?;
    let shots = s.grab_outputs(None)?;
    let wins = wins.join().ok().and_then(Result::ok).unwrap_or_default();
    let Some(pick) = select::run(&mut s, &shots, wins)? else {
        return Ok(());
    };
    let window;
    let canvas = match pick {
        Pick::Region(r) => Canvas::crop(&shots, r),
        Pick::Window(handle) => {
            window = [s.grab_window(handle)?];
            Canvas::new(&window)
        }
    };
    let mut out = Vec::new();
    let fill = |y, row: &mut [u8]| canvas.row(y, row);
    png::encode(canvas.w, canvas.h, fill, &mut out).map_err(Error::Encode)?;
    if file.as_deref() == Some("-") {
        // stdout is the destination (e.g. a pipe): no clipboard, or the pipe never closes.
        let mut o = io::stdout().lock();
        return o
            .write_all(&out)
            .map_err(|e| Error::Write("stdout".into(), e));
    }
    let (path, f) = open(file);
    f.and_then(|mut f| f.write_all(&out))
        .map_err(|e| Error::Write(path, e))?;
    Ok(clipboard::serve(&mut s, out)?)
}

/// The named file, or a new one in ~/Pictures.
fn open(file: Option<String>) -> (PathBuf, io::Result<File>) {
    match file {
        Some(f) => (PathBuf::from(&f), File::create(&f)),
        None => new_file(),
    }
}

/// ~/Pictures/screenshot-<local time>.png; never overwrites an older shot from the same second.
fn new_file() -> (PathBuf, io::Result<File>) {
    let dir = PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Pictures");
    let stem = format!("screenshot-{}", sys::local_time(c"%Y-%m-%d_%H-%M-%S"));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return (dir, Err(e));
    }
    let mut n = 1;
    loop {
        let name = if n == 1 {
            format!("{stem}.png")
        } else {
            format!("{stem}-{n}.png")
        };
        let path = dir.join(name);
        match File::create_new(&path) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => n += 1,
            r => return (path, r),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Result<Args, Error> {
        parse(s.split_whitespace().map(String::from))
    }

    #[test]
    fn parses() {
        let a = args("-o DP-2 -").ok().unwrap();
        assert_eq!(
            (a.output.as_deref(), a.file.as_deref()),
            (Some("DP-2"), Some("-"))
        );
        assert!(args("-x").is_err());
        assert!(args("a.png b.png").is_err());
        assert!(args("-o").is_err());
        assert!(args("-s -o DP-2").is_err());
        assert!(args("-s").ok().unwrap().select);
    }
}
