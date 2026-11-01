use std::path::PathBuf;
use std::process::ExitCode;

use wall::{app, ipc};

const USAGE: &str = "usage: wall [image.png|image.jpg]\n\
    First call stays running and shows the image; later calls switch it.\n\
    No image: show the last one again.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = match args.as_slice() {
        [p] if !p.starts_with('-') => PathBuf::from(p),
        [] => match ipc::last() {
            Some(p) => p,
            None => return usage(),
        },
        _ => return usage(),
    };
    let image = match std::fs::canonicalize(&path) {
        Ok(p) => p,
        Err(e) => return fail(format!("{}: {e}", path.display())),
    };
    match ipc::claim() {
        Ok(ipc::Role::Client(s)) => match ipc::request(s, &image) {
            Ok(Ok(())) => ExitCode::SUCCESS,
            Ok(Err(why)) => fail(why),
            Err(e) => fail(format!("talking to the running wall: {e}")),
        },
        Ok(ipc::Role::Daemon(l)) => match app::run(image, &l) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                ipc::remove_socket();
                fail(e.to_string())
            }
        },
        Err(e) => fail(format!("wall socket: {e}")),
    }
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn fail(msg: String) -> ExitCode {
    eprintln!("wall: {msg}");
    ExitCode::FAILURE
}
