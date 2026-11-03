# pix

The image library ferrite's tools share: decode PNG, JPEG, GIF, WebP and SVG, then scale the result into ARGB buffers ready to show.

## What's inside

| Module | What it does |
|---|---|
| `decode` | `open` and `load`: sniff the format from the first bytes and decode to packed 8-bit RGB (`Rgb`), blending transparency onto a background color you pass. `open` returns every frame of an animation (`Image::Frames`) or an SVG document (`Image::Svg`); `load` returns just the first frame (the rest of an animation is never decoded); `open_each` hands frames to a callback one at a time, so a viewer can show the first while the rest decode. PNG and JPEG use the zune decoders. |
| `gif` | Still and animated GIF through the `gif` crate, composited into whole frames. |
| `webp` | Still and animated WebP through the system libwebp (feature `webp`). |
| `svg` | `Svg`: keeps the document and renders it at any size through the system librsvg (feature `svg`). |
| `orient` | `orient` turns and flips an image. JPEG EXIF orientation is applied at load; `CW` and `CCW` turn a quarter. |
| `mip` | `Mip`: a chain of half-size copies built once, so zooming out stays sharp and costs as much as the window, not the photo. |
| `scale` | Write opaque ARGB8888: `resample` (filtered zoom and crop), `nearest` (crisp pixels), `cover` (fill and crop, wallpaper fit), `transform` (any affine map, bilinear). Work is split across all cores. |
| `encode` | `png`: an `Rgb` as PNG file bytes, for the clipboard. |

## Used by

- [`files`](../files) (thumbnails)
- [`image`](../image)
- [`wallpaper`](../wallpaper) (without WebP and SVG)

## Example

Load an image and fill a wallpaper-sized buffer with it, then draw it at 30% zoom
through the mip chain.

```rust
use std::path::Path;

fn main() -> Result<(), pix::Error> {
    let (w, h) = (1920, 1080);
    let mut argb = vec![0u32; w * h];

    // First frame; transparent pixels land on #111111.
    let img = pix::load(Path::new("photo.jpg"), [0x11; 3])?;
    pix::cover(&img, w, h, &mut argb);

    // Zoomed out: draw from the mip level just above the zoom.
    let zoom = 0.3;
    let mip = pix::Mip::new(img);
    let (level, k) = mip.pick(zoom);
    pix::resample(level, zoom * k, (0.0, 0.0), (w, h), (&mut argb, w));

    // Turn a quarter clockwise and save as PNG.
    let up = pix::orient(mip.full(), pix::CW);
    std::fs::write("turned.png", pix::png(&up)?).map_err(pix::Error::Read)?;
    Ok(())
}
```

For a viewer, `open` gives every frame with its delay, or the SVG itself:

```rust
match pix::open(path, bg)? {
    pix::Image::Frames(frames) => { /* frames[i].img, shown frames[i].ms (0 = still) */ }
    pix::Image::Svg(svg) => { let img = svg.render((w, h), bg)?; }
}
```

## Features

| Feature | Default | What it adds |
|---|---|---|
| `webp` | on | WebP decoding, linked to the system libwebp. |
| `svg` | on | SVG documents (`Svg`, `Image::Svg`), linked to librsvg. |

Both pull in large system libraries. librsvg alone brings cairo, pango and glib, about
45 shared libraries in total. A tool that only needs PNG, JPEG and GIF turns them off:

```toml
pix = { path = "../pix", default-features = false }
```

With a feature off, files in that format fail with `Error::Format`, the same as any
unknown format.

## Requirements

- Always: nothing. PNG, JPEG and GIF are pure Rust (zune-png, zune-jpeg, gif).
- `webp`: `libwebpdemux` (part of libwebp).
- `svg`: `librsvg-2`, `cairo`, `gobject-2.0` and `glib-2.0`.

pix binds these itself (`#[link(name = …)]`); install them with their development files.

## Status

pix is an internal library of ferrite. It is not published on crates.io, and its API
changes whenever the tools need it to. To use it anyway, depend on it by path or git:

```toml
pix = { git = "https://github.com/uhm26/ferrite" }
```

## License

MIT or Apache-2.0, see the [main README](../README.md#license).
