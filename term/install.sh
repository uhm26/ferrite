#!/bin/sh
# Build term and install it for this user: binary, desktop entry, icon.
# The window's app_id is "term", so Hyprland and launchers match it to term.desktop.
set -eu
cd "$(dirname "$0")"
cargo build --release -q
target=$(cargo metadata --format-version 1 --no-deps | sed 's/.*"target_directory":"\([^"]*\)".*/\1/')
data=${XDG_DATA_HOME:-$HOME/.local/share}
install -Dm755 "$target/release/term" "$HOME/.local/bin/term"
install -Dm644 resources/term.desktop "$data/applications/term.desktop"
install -Dm644 resources/term.svg "$data/icons/hicolor/scalable/apps/term.svg"
echo "installed: ~/.local/bin/term, term.desktop, term.svg (Ctrl+Shift+R in a running term switches to it)"
