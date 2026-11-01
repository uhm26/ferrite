use std::process::ExitCode;

fn main() -> ExitCode {
    match clip::daemon::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("clip: {e}");
            ExitCode::FAILURE
        }
    }
}
