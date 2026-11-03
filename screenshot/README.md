# screenshot

Takes a screenshot in about 20 ms. Or freezes the screen so you can pick a region or a
window.

<!-- screenshot: screenshot -->

## Features

- Full-screen capture of every output in one image, placed as they sit in your layout.
  About 20 ms from start to saved file for two 1080p screens (see Performance).
- Capture a single output by name (`-o`).
- Freeze mode (`-s`): the screen stops, the rest dims, and you drag a region or click a
  window or an output. What you select is what was on screen when you started.
- Window pick (Hyprland only): the hovered window is highlighted; a click saves that
  window alone, even if other windows cover it.
- Shots from freeze mode are saved to a file and copied to the clipboard as `image/png`.
- Write to a file or to stdout (`-`) for piping.
- Own PNG encoder: filters and compresses stripes of the image on all CPU cores.
- Needs only libc. It talks to the compositor over the Wayland socket directly, without
  libwayland or any crates.

## Install

Prebuilt for Arch Linux (x86_64), no root:

```sh
curl -fsSL https://raw.githubusercontent.com/uhm26/ferrite/main/install.sh | sh -s -- screenshot
```

Build from source (needs Rust stable), from a clone of this repo:

```sh
cargo build --release -p screenshot
cp target/release/screenshot ~/.local/bin/
```

Bind it to keys in your compositor config. For example:

```sh
# Hyprland (hyprland.conf)
bind = , Print, exec, screenshot -s
bind = SHIFT, Print, exec, screenshot

# sway (config)
bindsym Print exec screenshot -s
bindsym Shift+Print exec screenshot
```

## Usage

```
screenshot [-o OUTPUT | -s] [FILE | -]
```

| Argument | Meaning |
|---|---|
| (none) | Capture all outputs into a new file in your Pictures folder |
| `-o OUTPUT` | Capture only the output with this name (as the compositor names it, e.g. `DP-1`) |
| `-s` | Freeze the screen and pick a region, window or output |
| `FILE` | Write to this file instead (an existing file is overwritten) |
| `-` | Write the PNG to stdout |

`-s` and `-o` cannot be combined: freeze mode always shows every output. Any other
argument prints the usage line and exits with status 1.

Examples:

```sh
screenshot                      # all outputs → ~/Pictures/screenshot-<date>.png
screenshot -o DP-1 shot.png     # one output → shot.png
screenshot - | wl-copy          # full screen to the clipboard (wl-copy is from wl-clipboard)
screenshot -s                   # pick → ~/Pictures/... and the clipboard
screenshot -s crop.png          # pick → crop.png and the clipboard
screenshot -s - > crop.png      # pick → stdout only, no clipboard
```

Only freeze mode copies to the clipboard. After saving, `screenshot -s` keeps running in
the background to serve the image to apps that paste it, and exits as soon as something
else is copied (the same way `wl-copy` works). With `-s -` it writes to stdout and exits
at once.

Cancelling freeze mode saves nothing and exits with status 0.

The freeze overlay is a layer surface with the namespace `screenshot`, if you want to
match it in compositor rules (for example to turn off its open animation).

## Keys and mouse

Freeze mode (`-s`):

| Key / button | Action |
|---|---|
| Left drag | Select a region (it may span several outputs) |
| Left click on a window | Save that window alone (Hyprland only) |
| Left click on the desktop | Save the whole output under the pointer |
| Right click | Cancel |
| Esc | Cancel |

A press that moves less than 4 pixels counts as a click. Outside Hyprland there is no
window list, so any click saves the output under the pointer.

## Files

| Path | Use |
|---|---|
| `<Pictures>/screenshot-YYYY-MM-DD_HH-MM-SS.png` | Default output (local time). The folder is created if missing. A second shot in the same second gets `-2`, `-3`, … and never overwrites. |
| `$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY` | The Wayland socket (`wayland-0` if `WAYLAND_DISPLAY` is unset) |
| `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock` | Hyprland IPC, read in freeze mode for window positions (500 ms timeout; without it, window pick is off) |

The folder is `XDG_PICTURES_DIR` (from the environment, else `~/.config/user-dirs.dirs`),
else `~/Pictures`.

## Performance

| What | Time (median of 100) | Memory |
|---|---|---|
| Two 1920x1080 outputs to a file | 20 ms | 34 MB |
| One output (`-o`) to a file | 13.5 ms | 22 MB |
| Freeze mode, until the overlay is up | 15 ms (median of 50) | |

- Cold start (binary not in the page cache) adds about 1 ms.
- The PNG encoder uses about 80 ms of CPU per shot, spread over all cores.
- 30 out of 30 dragged regions came out at the exact size.

Measured on a 6-core desktop CPU from 2019 with NVMe and 1920x1080 screens, under Hyprland,
with a release build for x86-64-v3. The prebuilt binaries target plain x86-64 and can be a
little slower. "Start" is from launch until the window is on screen.

## Requirements

- Arch Linux (x86_64).
- No system libraries beyond libc.
- Wayland protocols:

| Protocol | Needed for |
|---|---|
| `wl_shm` | Everything |
| `zwlr_screencopy_manager_v1` (version 3 or newer) | Everything |
| `zwlr_layer_shell_v1`, `wp_viewporter`, `wp_single_pixel_buffer_manager_v1`, `wl_subcompositor`, `wl_seat` (version 5) | Freeze mode |
| `ext_data_control_manager_v1` or `zwlr_data_control_manager_v1` | Clipboard copy in freeze mode |
| `wp_cursor_shape_manager_v1` | Crosshair cursor in freeze mode (optional) |
| `hyprland_toplevel_export_manager_v1` + Hyprland IPC | Window pick (Hyprland only) |

Hyprland has all of them. sway and niri have screencopy, layer-shell and data-control,
so full-screen capture and region select work there; window pick does not. GNOME has
no wlr-screencopy, so `screenshot` does not work on GNOME.

## Limitations

- Assumes every output has scale 1 and no rotation. With HiDPI scaling or a rotated
  output, the combined image is wrong (outputs overlap or are cut).
- The mouse cursor is not in the shot.
- Output is 8-bit RGB PNG. Windows are saved without transparency; 10-bit output
  formats are refused.
- The encoder is tuned for speed, not the smallest file: other tools can produce
  smaller PNGs.
- Full-screen mode does not copy to the clipboard; pipe it to a clipboard tool instead.
- Freeze mode cannot be limited to one output.
- A window picked in freeze mode is captured fresh at the click, not from the frozen
  frame, so it may differ slightly from what you saw.

## License

MIT or Apache-2.0, see the [main README](../README.md#license).
