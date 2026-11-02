//! image — our own Wayland image viewer on frame.
//! [`view`] holds zoom and pan (pixels come from our `pix` library), [`folder`]
//! lists the images to step through, [`load`] decodes them (neighbors ahead of time),
//! [`show`] paints the current one and acts on it ([`trash`]), [`shape`] sizes the
//! window once at open, [`app`] runs the
//! window loop.

mod app;
mod folder;
mod load;
mod shape;
mod show;
mod trash;
mod view;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::run()
}
