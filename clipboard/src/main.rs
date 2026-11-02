use std::process::ExitCode;

fn main() -> ExitCode {
    match clipboard::daemon::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("clipboard: {e}");
            ExitCode::FAILURE
        }
    }
}
