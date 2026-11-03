# frame

The Wayland base that ferrite's windowed tools share: a window, input, fonts and a flat UI toolkit, with no crate dependencies.

## What's inside

| Module | What it does |
|---|---|
| `wl` | A minimal Wayland client: the wire format over the compositor's socket, opcodes, typed requests. No libwayland. |
| `window` | One xdg toplevel drawn from shared-memory buffers (a pool of three, reused across resizes). Turns protocol events into a few `Event`s: resize, close, keys, pointer, focus, drag and drop. Also clipboard copy and paste. |
| `xkb` | Bindings to libxkbcommon: keymap, modifiers, key repeat, keysyms and text. |
| `font` | FreeType faces in four styles (regular, bold, italic, bold italic), with fontconfig fallback for missing characters. |
| `ui` | The shared flat look and the widgets: theme tokens, antialiased shapes, cached text, line icons, springs, an immediate-mode context (`State`, `Ui`), buttons, switches, rows, a text field, context menus and the app shell (sidebar, toolbar, status band). |
| `sys` | The few libc calls std does not wrap: `poll`, memfd and mmap, fd passing, inotify folder watching (`sys::watch::Watch`). |
| `png` | A tiny PNG writer (uncompressed) for headless screenshots. |

Run `cargo run --example window` for the smallest app, or
`cargo run --release --example gallery` to see every widget.

## Used by

- [`files`](../files)
- [`image`](../image)
- [`video`](../video)

## Example

A window with one button. The loop feeds pointer events to the UI, draws when
something changed, and sleeps in `poll` otherwise.

```rust
use frame::sys::{self, POLLIN, PollFd};
use frame::ui::{Rect, State, Text, theme};
use frame::window::{self, Event, Pointer, Window};
use frame::wl::proto::pointer::BTN_LEFT;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut win: Window = Window::new("hello", "hello", true)?;
    win.set_size(320, 160);
    let mut ui = State::new(Text::new(theme::FAMILY)?);
    let mut dirty = true;
    loop {
        while let Some(ev) = win.next_event() {
            match ev {
                Event::Close => return Ok(()),
                Event::Resize(w, h) if w > 0 && h > 0 => win.set_size(w, h),
                Event::Pointer(Pointer::Motion(x, y)) => ui.input.pos = (x as f32, y as f32),
                Event::Pointer(Pointer::Button { button, pressed }) if button == BTN_LEFT => {
                    ui.input.button(pressed)
                }
                _ => continue,
            }
            dirty = true;
        }
        if (dirty || ui.animating()) && !win.frame_pending() {
            if let Some(i) = win.acquire()? {
                let (c, ()) = win.canvas(i);
                let all = window::Rect { x: 0, y: 0, w: c.w, h: c.h };
                let mut f = ui.begin(c, 1.0 / 60.0);
                f.panel(Rect { x: 0.0, y: 0.0, w: all.w as f32, h: all.h as f32 });
                if f.button("ok", Rect { x: 24.0, y: 24.0, w: 96.0, h: 32.0 }, "Click me", true) {
                    println!("clicked");
                }
                f.end();
                win.present(i, &[all])?;
                dirty = false;
            }
        }
        win.flush()?;
        let timeout = win.next_deadline().map_or(-1, |at| {
            at.saturating_duration_since(Instant::now()).as_millis() as i32
        });
        sys::wait(&mut [PollFd::new(win.fd(), POLLIN)], timeout)?;
        win.dispatch()?;
        win.tick(Instant::now());
    }
}
```

There are two `Rect` types: `window::Rect` (whole pixels, for damage) and `ui::Rect`
(logical `f32` pixels, for layout).

## Features

None. Everything is always built.

## Requirements

- A Wayland compositor with xdg-shell.
- System libraries, with their development files: `fontconfig`, `freetype` and
  `xkbcommon`. frame binds them itself (`#[link(name = …)]`), so no `-sys` crates are
  involved.
- The UI's default font is Adwaita Sans (`theme::FAMILY`); fontconfig picks a fallback
  if it is missing.

## Status

frame is an internal library of ferrite. It is not published on crates.io, and its API
changes whenever the tools need it to. To use it anyway, depend on it by path or git:

```toml
frame = { git = "https://github.com/uhm26/ferrite" }
```

## License

MIT or Apache-2.0, see the [main README](../README.md#license).
