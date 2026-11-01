//! image — our own Wayland image viewer on frame.
//! [`image`] decodes PNG/JPEG, [`scale`] resamples the visible part, [`view`]
//! holds zoom and pan, [`folder`] lists the images to step through, [`show`]
//! paints the current one, [`app`] runs the window loop.

mod app;
mod folder;
mod image;
mod scale;
mod show;
mod view;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::run()
}
