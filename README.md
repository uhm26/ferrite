# ferrite

Small, fast desktop tools for Wayland, written in Rust. Few dependencies, monochrome look,
built to stay out of your way.

| Tool | What it does | System libraries |
|---|---|---|
| [`term`](term) | Terminal: tabs, reflow, own font rendering | fontconfig, freetype, xkbcommon |
| [`files`](files) | File manager: tabs, split view, thumbnails, search, drives, undo | + cairo, librsvg, libwebp |
| [`image`](image) | Image viewer: PNG, JPEG, GIF, WebP, SVG; zoom, pan, browse a folder | + cairo, librsvg, libwebp |
| [`video`](video) | Video player on libmpv, GPU decode | + libmpv, EGL, gbm |
| [`screenshot`](screenshot) | Full screen in ~25 ms, or freeze + pick a region or window | none |
| [`wallpaper`](wallpaper) | Wallpaper daemon: `wallpaper <image>` | none |
| [`clipboard`](clipboard) | Keeps your last copy (text or image) alive after its app closes | none |

Libraries the tools share: [`frame`](frame) (windows, input, fonts, UI) and
[`pix`](pix) (image decoding and scaling).

## Requirements

- Linux with a Wayland compositor.
- `wallpaper` and the `screenshot` freeze overlay need wlr-layer-shell; `screenshot` needs
  wlr-screencopy; `clipboard` needs ext- or wlr-data-control. Hyprland, sway and niri have
  them; GNOME does not. Picking a window in `screenshot` uses Hyprland.

## Install

A one-line installer is coming. Until then, build from source.

## Build from source

Needs Rust (stable) and the system libraries listed above (with their development files).

```sh
git clone https://github.com/uhm26/ferrite
cd ferrite
cargo build --release -p term      # one tool at a time, see below
cp target/release/term ~/.local/bin/
```

Build tools one at a time (`-p <tool>`). A plain `cargo build --workspace` builds `pix`
once with every feature any tool asks for, so `wallpaper` would link WebP and SVG
libraries it never uses.

## License

MIT or Apache-2.0, at your option: [LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE).
