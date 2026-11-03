//! files — our own Wayland file explorer on frame.
//! [`dir`] reads folders, [`browse`] moves between them, [`view`] draws them
//! Nemo-style, [`sys`] holds the libc calls std lacks,
//! [`browse::tabs`] holds several open folders, [`thumbs`] makes picture previews,
//! [`app`] runs the window loop.

mod app;
mod apps;
mod bookmarks;
mod browse;
mod dir;
mod drives;
mod ops;
mod portal;
mod sys;
mod thumbs;
mod uri;
mod view;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::run()
}
