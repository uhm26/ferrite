use std::process::ExitCode;

use term::app::App;

const USAGE: &str = "usage: term [-e command [args…]]";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let command: Vec<String> = match args.next().as_deref() {
        None => Vec::new(),
        Some("-e") => args.collect(),
        Some("--version") => {
            println!("term {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some(_) => {
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    term::sys::init_locale();
    // Config file + the glass switch; both stay watched while running.
    let cfg = term::app::settings();
    match App::new(cfg, command).and_then(|mut app| app.run()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("term: {e}");
            ExitCode::FAILURE
        }
    }
}
