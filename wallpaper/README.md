# wallpaper

Run `wallpaper pic.jpg` and it is your wallpaper on every screen. Run it again with
another image to switch.

<!-- screenshot: wallpaper -->

## Features

- One image on every output, scaled to cover the whole screen and centered (the sides
  that do not fit are cut off).
- PNG and JPEG. A GIF shows its first frame. JPEG photos are turned upright from their
  EXIF orientation.
- The first call starts the daemon; later calls hand it a new image and exit.
- A bare `wallpaper` shows the last image again (put that in your autostart).
- Covers the full output, also under bars that reserve space.
- Outputs with the same size share one decode and one buffer. Outputs added later get
  the wallpaper too; integer output scales are drawn at full resolution.
- Idle memory is about 3 MB: the decoded image is freed after drawing.
- Needs only libc. It talks to the compositor over the Wayland socket directly; image
  decoding comes from the workspace's [`pix`](../pix) library.

## Install

Prebuilt for Arch Linux and Fedora (x86_64), no root:

```sh
curl -fsSL https://raw.githubusercontent.com/uhm26/ferrite/main/install.sh | sh -s -- wallpaper
```

Build from source (needs Rust stable), from a clone of this repo:

```sh
cargo build --release -p wallpaper
cp target/release/wallpaper ~/.local/bin/
```

Build it on its own with `-p wallpaper`. A workspace-wide build turns on WebP and SVG
support in `pix`, which links libraries `wallpaper` never uses.

Set an image once, then start the daemon with your compositor. A bare `wallpaper`
restores the last image:

```sh
# Hyprland (hyprland.conf)
exec-once = wallpaper

# sway (config)
exec wallpaper

# niri (config.kdl)
spawn-at-startup "wallpaper"
```

You can also give the image directly, e.g. `exec-once = wallpaper ~/Pictures/wall.jpg`.

## Usage

```
wallpaper [IMAGE]
```

| Call | What happens |
|---|---|
| `wallpaper IMAGE`, no daemon running | Becomes the daemon: shows `IMAGE` and stays in the foreground |
| `wallpaper IMAGE`, daemon running | Asks the daemon to switch to `IMAGE`, waits for its answer, exits |
| `wallpaper` | Same as above with the last image shown |

There are no flags. A bare call with no remembered image, more than one argument, or
an argument starting with `-` prints the usage and exits with status 2. Errors (file
missing, not an image) are printed and exit with status 1; the running wallpaper stays.

Examples:

```sh
wallpaper ~/Pictures/wall.jpg &   # start the daemon from a shell
wallpaper other.png               # switch (relative paths are fine)
wallpaper                         # show the last one again
```

The daemon logs one line to stderr per draw: image, size, outputs, decode and scale
times. It exits when the compositor quits.

The surfaces are on the background layer with the namespace `wallpaper`, if you want to
match them in compositor rules.

## Files

| Path | Use |
|---|---|
| `$XDG_STATE_HOME/wallpaper/last` (default `~/.local/state/wallpaper/last`) | The absolute path of the last image shown, written after each change |
| `$XDG_RUNTIME_DIR/wallpaper.sock` | The daemon's socket. A stale one from a killed daemon is taken over. |
| `$XDG_RUNTIME_DIR/wallpaper.lock` | Held briefly at start, so two calls at once cannot both become the daemon |
| `$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY` | The Wayland socket (`wayland-0` if `WAYLAND_DISPLAY` is unset) |

`XDG_RUNTIME_DIR` must be set; there is no fallback to `/tmp`.

Socket protocol: the client writes the absolute image path and a newline (at most
4096 bytes). The daemon answers `ok` or `err <reason>`, each followed by a newline.

## Performance

Time from `wallpaper <image>` until the daemon answers that it is shown (median of 20):

| Image | Time |
|---|---|
| 4733x2662 JPEG | 137 ms |
| 3840x2160 JPEG | 82 ms |
| 1920x1080 JPEG | 16 ms |
| 1920x1080 PNG | 45 ms |

- Memory between switches: about 2.5 MB. Peak while decoding the 4733x2662 JPEG: 55 MB.
- 50 requests sent at once: done in 0.18 s, the last one wins.
- CPU while idle: 0.

Measured on a 6-core desktop CPU from 2019 with NVMe and 1920x1080 screens, under Hyprland,
with a release build for x86-64-v3. The prebuilt binaries target plain x86-64 and can be a
little slower. "Start" is from launch until the window is on screen.

## Requirements

- Arch Linux or Fedora (x86_64).
- No system libraries beyond libc.
- Wayland protocols: `wl_compositor` (version 4), `wl_shm`, `wl_output` (version 2 or
  newer), and `zwlr_layer_shell_v1`.

Hyprland, sway and niri have layer-shell. GNOME does not, so `wallpaper` does not work
on GNOME.

## Limitations

- The same image on every output; no per-output images.
- Cover fit only: no fit-inside, center or tile modes.
- No transitions or animation; a GIF shows only its first frame.
- No WebP or SVG.
- Transparent pixels are drawn on black.
- EXIF orientation is read from JPEG only.
- Fractional scaling is not supported: only the integer output scale is used, and the
  compositor scales the rest.
- Large photos take time to decode: a 4733x2662 JPEG takes about 140 ms.
- The daemon reads the image from disk again when it has to redraw (an output changes
  size or scale, or a new one appears). If you moved or deleted the file, that redraw
  fails and is logged.
- Image paths may not contain a newline.

## License

MIT or Apache-2.0, see the [main README](../README.md#license).
