//! vid — our own Wayland video player. libmpv is the engine ([`player`]): it
//! decodes on the GPU and draws each frame into a GPU buffer we own ([`gpu`]),
//! which goes to the compositor as is, on a subsurface under the window
//! ([`video`]). frame draws the [`controls`] on the window above it. [`app`] runs
//! the loop; [`shot`] renders the controls headless; [`sys`] holds the C bindings.

mod app;
mod controls;
mod gpu;
mod player;
mod shot;
mod sys;
mod video;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::run()
}
