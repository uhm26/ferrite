#!/bin/sh
# Build image and install it for this user: binary, desktop entry (handler for PNG, JPEG, GIF, WebP, SVG), icon.
# The window's app_id is "image", so Hyprland and the launcher match it to image.desktop.
set -eu
cd "$(dirname "$0")"
cargo build --release -q
target=$(cargo metadata --format-version 1 --no-deps | sed 's/.*"target_directory":"\([^"]*\)".*/\1/')
data=${XDG_DATA_HOME:-$HOME/.local/share}
install -Dm755 "$target/release/image" "$HOME/.local/bin/image"
install -Dm644 resources/image.desktop "$data/applications/image.desktop"
install -Dm644 resources/image.svg "$data/icons/hicolor/scalable/apps/image.svg"
echo "installed: ~/.local/bin/image, image.desktop, image.svg"
