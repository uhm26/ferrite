# clipboard

> A Wayland clipboard keeper: your last copy stays pasteable after the app you copied from closes.

<!-- screenshot: clipboard -->

## Features

- On Wayland the app you copied from serves the paste, so closing it empties the
  clipboard. `clipboard` reads each copy as it happens, and when the clipboard goes
  empty it serves the copy itself.
- Keeps text and images. Text is offered back under all common text types
  (`text/plain;charset=utf-8`, `UTF8_STRING`, `text/plain`, `STRING`, `TEXT`). For
  images it keeps `image/png` if the app offered it, else the first image type.
- While the original app is still open, pastes still go to that app, with every type
  it offers.
- Skips copies that password managers mark as secret (`x-kde-passwordManagerHint`).
- Slow or stuck apps never block it: all transfers are non-blocking, and many pastes
  can be served at once.
- Asleep when idle: no timers, no wakeups. About 2 MB of memory plus the size of the
  kept copy.
- No crates, no system libraries beyond libc. It speaks the Wayland wire protocol itself.

## Install

Prebuilt (x86_64 Linux, static, no root):

```sh
curl -fsSL https://raw.githubusercontent.com/uhm26/ferrite/main/install.sh | sh -s -- clipboard
```

Build from source (needs Rust stable), from a clone of this repo:

```sh
cargo build --release -p clipboard
cp target/release/clipboard ~/.local/bin/
```

Start it with your compositor:

```sh
# Hyprland (hyprland.conf)
exec-once = clipboard

# sway (config)
exec clipboard

# niri (config.kdl)
spawn-at-startup "clipboard"
```

## Usage

```
clipboard
```

It takes no arguments and runs in the foreground until the compositor quits. To try it
from a shell: `clipboard &`, copy some text in a terminal, close that terminal, and
paste somewhere else.

Errors and dropped copies are printed to stderr.

## Files

`clipboard` keeps the copy in memory (an anonymous memory file); the only file it makes is
a small lock file, so a second start in the same session just says "already running".

| Path | Use |
|---|---|
| `$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY` | The Wayland socket (`wayland-0` if `WAYLAND_DISPLAY` is unset) |
| `$XDG_RUNTIME_DIR/clipboard-$WAYLAND_DISPLAY.lock` | One keeper per session (held while it runs, freed when it exits) |

## Requirements

- Linux, glibc. Tested on x86_64.
- No system libraries beyond libc.
- Wayland protocols: `ext_data_control_manager_v1` (preferred) or
  `zwlr_data_control_manager_v1`, and `wl_seat`.

Data-control lets a program see and set the clipboard without having a focused window.
Hyprland, sway and niri have it. GNOME does not, so `clipboard` does not work on GNOME.

## Limitations

- Only the last copy. No history, no picker.
- Nothing is saved to disk: the kept copy is gone when `clipboard` exits.
- Only text and one image type are kept. Other types (HTML, file lists from file
  managers, and so on) are lost once their app closes.
- The primary selection (middle-click paste) is not kept.
- Copies over 64 MiB are not kept.
- An app that sends nothing for 10 seconds while being read is given up on.
- If an app is killed in the middle of sending a copy, the cut-off part is kept as if
  it were complete. The protocol carries no length, so this cannot be detected.

## License

MIT or Apache-2.0, see the [main README](../README.md#license).
