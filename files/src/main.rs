//! files — our own Wayland file explorer on frame.
//! [`dir`] reads folders, [`browse`] moves between them, [`view`] draws them,
//! [`app`] runs the window loop.

mod app;
mod browse;
mod dir;
mod view;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::run()
}
