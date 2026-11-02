# video

> A small video player for Wayland, built on libmpv.

<!-- screenshot: video -->

## Features

- Plays any file libmpv can open: MP4, MKV, WebM, MOV, AVI, MPEG, Ogg and more.
- Hardware decoding through VA-API when the GPU supports it. mpv draws each frame
  straight into a GPU buffer that goes to the compositor as is (a dmabuf), with no copy
  through the CPU.
- A control bar floating at the bottom: play/pause, current time, seek track, total
  time, mute, volume slider and fullscreen.
- Hovering the seek track shows the time under the pointer. Dragging seeks fast (to
  keyframes) while you move and lands exactly where you let go.
- The bar and the cursor hide 2 seconds after the pointer stops while playing. They
  stay while paused, at the end, or while the pointer is on the bar.
- Click the picture to pause, double-click it for fullscreen.
- Stays on the last frame at the end. Play again starts from the beginning.
- The window title shows the media title.
- If a file can't be played, the window says so and shows why.
- Volume from 0 to 100, by keys, wheel or slider.

## Install

One-line installer: coming soon (see the [main README](../README.md)).

Build from source (needs Rust stable and the libraries under [Requirements](#requirements)):

```sh
git clone https://github.com/uhm26/ferrite
cd ferrite
cargo build --release -p video
cp target/release/video ~/.local/bin/
```

Or run `video/install.sh`. It builds a release binary and installs, for your user only:

- the binary to `~/.local/bin/video`
- a desktop entry to `$XDG_DATA_HOME/applications/video.desktop`, listing MP4, MKV,
  WebM, QuickTime, AVI, MPEG and Ogg video, so `video` shows up under "Open with"
- an icon to `$XDG_DATA_HOME/icons/hicolor/scalable/apps/video.svg`

`$XDG_DATA_HOME` defaults to `~/.local/share`. The script does not make `video` your
default player. To do that yourself, for example:

```sh
xdg-mime default video.desktop video/mp4 video/x-matroska video/webm
```

## Usage

```
video FILE
```

The file is passed to mpv as given.

```sh
video ~/Videos/clip.mkv
```

For testing, `video --png OUT [--size WxH] [--pointer X,Y]` renders the control bar over
a gray stand-in frame to a PNG file, without a compositor (default size 1280x720).
`--pointer` places the pointer, for example over the seek track to show its hover.

## Keys and mouse

Playback:

| Key | Action |
|---|---|
| Space, `k` | Play / pause (at the end: play from the start) |
| Left / Right | Back / forward 5 seconds |
| `j` / `l` | Back / forward 10 seconds |
| Home | Jump to the start |

Volume and window:

| Key | Action |
|---|---|
| Up / Down | Volume up / down by 5 |
| `m` | Mute / unmute |
| `f` | Fullscreen on / off |
| Escape | Leave fullscreen; quit when not fullscreen |
| `q` | Quit |

Holding a key repeats only seeking and volume. Letter keys work the same with Shift or
Caps Lock.

Mouse:

| Input | Action |
|---|---|
| Click on the picture | Play / pause |
| Double-click on the picture | Fullscreen on / off |
| Mouse wheel | Volume up / down by 5 per step |
| Click or drag on the seek track | Seek |
| Click or drag on the volume slider | Set volume |
| Bar buttons | Play / pause, mute, fullscreen |

## Files

- Reads only the file you give it.
- Does not read mpv's own configuration: no `mpv.conf`, `input.conf` or scripts, and
  mpv's on-screen controller is off. `video` has its own keys and bar.
- Opens the GPU render node `/dev/dri/renderD128`.
- Writes nothing. There is no config file.

## Requirements

- Linux with a Wayland compositor that offers `wl_subcompositor` and
  `zwp_linux_dmabuf_v1` (version 3 or newer), besides `wl_compositor` v4, `wl_shm` and
  xdg-shell. `video` exits with an error if one is missing.
- A GPU and driver with EGL on GBM and OpenGL 3.3 or newer (Mesa).
- System libraries, with their development files to build: libmpv, EGL, gbm,
  fontconfig, freetype and xkbcommon.
- The bar text uses the Adwaita Sans font if it is installed; fontconfig picks a
  fallback otherwise.

## Limitations

- One file per run. There is no playlist and no way to open another file from the
  window.
- No keys or menus for subtitle or audio tracks, playback speed, frame stepping or
  screenshots. Tracks follow mpv's defaults.
- Hardware decoding is set up for VA-API only (AMD and Intel). On other GPUs mpv is
  likely to fall back to software decoding.
- Always uses the first render node, `/dev/dri/renderD128`. On a machine with more than
  one GPU this may be the wrong one.
- Relies on the driver's implicit sync for GPU buffers. Drivers without it may show
  half-drawn frames.
- Volume stops at 100; there is no boost above it.
- No HiDPI support: on a scaled output the compositor stretches the window and the
  video.
- The window draws no title bar of its own. On compositors that expect apps to draw
  one (such as GNOME), it has none.

## License

MIT or Apache-2.0, see the [main README](../README.md#license).
