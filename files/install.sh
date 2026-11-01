#!/bin/sh
# Build files and install it for this user: binary, desktop entry, icon.
# The window's app_id is "files", so Hyprland and the launcher match it to files.desktop.
set -eu
cd "$(dirname "$0")"
cargo build --release -q
target=$(cargo metadata --format-version 1 --no-deps | sed 's/.*"target_directory":"\([^"]*\)".*/\1/')
data=${XDG_DATA_HOME:-$HOME/.local/share}
install -Dm755 "$target/release/files" "$HOME/.local/bin/files"
install -Dm644 resources/files.desktop "$data/applications/files.desktop"
install -Dm644 resources/files.svg "$data/icons/hicolor/scalable/apps/files.svg"
echo "installed: ~/.local/bin/files, files.desktop, files.svg"
