# term

> A small, fast terminal emulator for Wayland, with tabs, a split panel and its own font rendering.

<!-- screenshot: term -->

## Features

- CPU rendering into shared memory. Only changed cells are redrawn, and a flood of output
  costs parsing, not drawing.
- Tabs. The tab bar appears once you have two or more. Drag a tab to move it, right click
  to rename it.
- Text reflows when the window is resized.
- Own font rendering through FreeType and fontconfig, with fallback fonts for characters
  the main font lacks. Box-drawing and block characters are drawn by term so lines meet.
- 256 colors and true color. Apps can query and set the palette (OSC 4, 10, 11, 12).
- Mouse reporting (modes 1000, 1002, 1003, 1006), bracketed paste, focus events,
  synchronized output (mode 2026) and the alternate screen.
- Ctrl+click opens what is under the pointer: a link in the browser, a file or folder in
  its default app. Paths can be absolute, `~/...` or relative to the shell's folder, and a
  `:line` suffix (as in compiler output) is understood.
- A split panel on the right side of a tab. It holds a second shell or a file. Ctrl+click
  on a text file that has no default app opens it there:
  - Markdown files open in a formatted preview; Ctrl+E switches to editing.
  - Other text files open in a small editor with line numbers, syntax
    colors, undo and save.
- Pasting an image (with no text on the clipboard) saves it to a file and pastes the
  file's path.
- Config changes apply at once, without a restart.
- Restart in place (Ctrl+Shift+R): runs the term binary again and keeps every shell, its
  screen and scrollback. Useful after installing a new build.
- No Rust dependencies. System libraries are called through small bindings of its own.

## Install

Prebuilt for Arch Linux (x86_64), no root. The installer also adds a menu entry and prints
one `pacman` line for anything still missing:

```sh
curl -fsSL https://raw.githubusercontent.com/uhm26/ferrite/main/install.sh | sh -s -- term
```

Build from source:

```sh
cargo build --release -p term
cp target/release/term ~/.local/bin/
```

## Usage

```
term [-e command [args...]]
term --version
```

Without `-e`, term starts your shell: `shell` from the config, else your login shell
(`$SHELL`), else `/bin/sh`. With `-e`, the first tab runs the command instead.
New tabs always start the shell, in your home folder.

```sh
term                      # your shell
term -e htop              # run htop in the first tab
term -e nvim notes.md     # open a file in an editor
TERM_STATS=1 term         # print frame, timing and memory stats on exit
```

The shell sees `TERM=xterm-256color`, `COLORTERM=truecolor` and `TERM_PROGRAM=term`.

## Keys and mouse

### Terminal

| Key | Action |
|---|---|
| Ctrl+Shift+T | New tab |
| Ctrl+Shift+W | Close tab |
| Ctrl+Shift+C | Copy selection |
| Ctrl+V | Paste (the app does not see Ctrl+V) |
| Ctrl+Shift+= | Font bigger |
| Ctrl+Shift+- | Font smaller |
| Ctrl+Shift+0 | Font back to the configured size |
| Ctrl+Shift+R | Restart in place, tabs kept |
| Ctrl+Backspace | Sent as Ctrl+W (delete word in most shells) |
| Shift+Enter | Sent as Alt+Enter (ESC CR), a new line in fish |

### Mouse

| Mouse | Action |
|---|---|
| Drag | Select text |
| Double click | Select a word (paths and URLs count as one word) |
| Middle click | Paste the clipboard |
| Wheel | Scroll back, 3 lines per step |
| Ctrl+click | Open the link or path under the pointer |
| Ctrl+click on a selection | Type the selected text into the split panel's shell (not run) |

When an app turns on mouse reporting, clicks and the wheel go to the app. Ctrl+click
still works then.

### Tab bar

| Mouse | Action |
|---|---|
| Click | Switch to the tab |
| Drag | Move the tab |
| Middle click | Close the tab |
| Right click | Rename: type, Enter saves, Esc cancels. An empty name goes back to the shell's title |

### Split panel

| Key or mouse | Action |
|---|---|
| Click a pane | Give it the keys |
| Drag the divider | Resize the panes |
| Click × in the header | Close the panel |
| Ctrl+E, or the Preview/Edit switch | Switch a Markdown file between preview and editing |
| Up, Down, Page Up, Page Down, Home, End | Scroll (preview) |
| Esc or q | Close (preview) |
| Ctrl+S | Save |
| Ctrl+Z / Ctrl+Shift+Z or Ctrl+Y | Undo / redo |
| Ctrl+A | Select all |
| Ctrl+C / Ctrl+X | Copy / cut the selection |
| Shift+arrows, mouse drag | Select text |
| Ctrl+Left / Ctrl+Right | Move by word |
| Ctrl+Backspace / Ctrl+Delete | Delete a word |
| Ctrl+Home / Ctrl+End | Start / end of the file |

Closing a tab, the panel, the window or restarting with unsaved edits is stopped once and
the file's header says so. Doing it again goes ahead.

## Configuration

The config file is `$XDG_CONFIG_HOME/term/config` (`~/.config/term/config` if
`XDG_CONFIG_HOME` is not set). Each line is `key = value`; lines starting with `#` are
comments. Every key is optional. A bad line is reported on stderr and skipped. The file is
watched, so changes apply to every tab while term runs.

| Key | Default | Meaning |
|---|---|---|
| `font` | `Adwaita Mono` | Font family, as a fontconfig name |
| `font_size` | `11` | Size in points (at 96 dpi), 4 to 72 |
| `scrollback` | `10000` | Lines of history per pane, at most 1000000. Applies to new tabs |
| `padding` | `6` | Space around the text, in pixels, at most 200 |
| `opacity` | `0.65` | Background opacity, 0 to 1. See below |
| `shell` | see Usage | Program to start in new tabs (a single program, no arguments) |
| `foreground` | `#d4d4d4` | Text color |
| `background` | `#0d0d0d` | Background color |
| `cursor` | `#d9c8af` | Cursor color |
| `selection` | `#34302a` | Selection background |
| `tab_background` | `#151515` | Tab bar and panel header background |
| `tab_line` | `#2a2a2a` | Hairlines between tabs and panes |
| `tab_inactive` | `#6a6a6a` | Inactive tab text |
| `tab_active` | `#d4d4d4` | Active tab text |
| `color0` ... `color15` | a muted 16-color set | The ANSI palette (0-7 normal, 8-15 bright) |

Colors are `#rrggbb` or `rgb:r/g/b`.

The window is opaque unless the file `$XDG_RUNTIME_DIR/ferrite/glass` contains `1` (a switch
any ferrite tool or script can use, e.g. a bar toggle). Only then
is `opacity` used. That file is watched too, so writing `1` or `0` to it switches
transparency on and off live.

Example:

```
# ~/.config/term/config
font = JetBrains Mono
font_size = 12
padding = 8
scrollback = 50000
background = #101010
color1 = #d06060
```

## Files

| Path | Use |
|---|---|
| `$XDG_CONFIG_HOME/term/config` | Settings (read, watched; the folder is created if missing) |
| `$XDG_RUNTIME_DIR/ferrite/glass` | Transparency switch (read, watched; the folder is created if missing) |
| `$XDG_RUNTIME_DIR/term/paste-<time>.<ext>` | Pasted images; the newest 10 are kept |
| `$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY` | Wayland socket (`wayland-0` if unset) |

Saving a file in the editor writes a temporary file next to it and renames it over the
original, so a crash never leaves half a file.

## Performance

| What | Result |
|---|---|
| Start | 29 ms (median of 100) |
| Memory | 23 MB, about 0.3 MB more per tab |
| Output speed | 76 MB/s of colored text |
| CPU while idle | 0 (no timers) |

Opening and closing tabs five times over leaves memory flat after the first round.

Measured on a 6-core desktop CPU from 2019 with NVMe and 1920x1080 screens, under Hyprland,
with a release build for x86-64-v3. The prebuilt binaries target plain x86-64 and can be a
little slower. "Start" is from launch until the window is on screen.

## Requirements

- Arch Linux (term uses `/proc` and inotify).
- System libraries: fontconfig, freetype, xkbcommon.
- A Wayland compositor with `wl_compositor` version 4, `wl_shm` and `xdg_wm_base`.
  Optional: `wl_seat` (keyboard and mouse), `wl_data_device_manager` (clipboard) and
  `wp_cursor_shape_manager_v1` (pointer shapes).
- `xdg-open` and `xdg-mime` (xdg-utils) for Ctrl+click.

## Limitations

- No HiDPI scaling: the window is drawn at scale 1.
- No window decorations: term draws no title bar and does not request one from the
  compositor.
- Tabs switch by mouse only. There is no key for the next or previous tab.
- Scrollback is by mouse wheel only, and there is no search.
- No primary selection. Middle click pastes the clipboard.
- No way to select text while an app has mouse reporting on.
- No OSC 7 (working folder), OSC 8 (hyperlinks), OSC 52 (clipboard) or OSC 133 (prompts).
- No kitty keyboard protocol, no image protocols (sixel, kitty graphics), no ligatures.
- No `--help`; unknown arguments print the usage line and exit.
- The editor opens files up to 4 MiB. Larger files, non-UTF-8 files and read-only files
  open read-only. Syntax colors come from a word scanner, not a parser.
- Files open in the split panel are not kept across a restart in place; only shells are.

## License

MIT or Apache-2.0, see the [main README](../README.md#license).
