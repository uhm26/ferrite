//! auth: a polkit authentication agent (the password popup) for Wayland.
//! [`agent`] talks to polkit, [`user`] picks who answers, [`helper`] relays the password
//! check, [`prompt`] is the popup, [`session`] runs one request, [`run`] the daemon loop.

mod agent;
mod helper;
mod prompt;
mod run;
mod session;
mod user;

fn main() {
    if let Err(e) = run::run() {
        eprintln!("auth: {e}");
        std::process::exit(1);
    }
}
