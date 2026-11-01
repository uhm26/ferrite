//! files — our own Wayland file explorer on frame.
//! [`dir`] reads folders, [`browse`] moves between them, [`view`] draws them
//! Nemo-style, [`sys`] holds the libc calls std lacks,
//! [`app`] runs the window loop.

mod app;
mod bookmarks;
mod browse;
mod dir;
mod ops;
mod sys;
mod trash;
mod uri;
mod view;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::run()
}
