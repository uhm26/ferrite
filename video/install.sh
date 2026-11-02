#!/bin/sh
# Build video and install it for this user: binary, desktop entry (video handler), icon.
# The window's app_id is "video", so Hyprland and the launcher match it to video.desktop.
# Does not make video the default video app (ask the owner; xdg-mime default video.desktop …).
set -eu
cd "$(dirname "$0")"
cargo build --release -q
target=$(cargo metadata --format-version 1 --no-deps | sed 's/.*"target_directory":"\([^"]*\)".*/\1/')
data=${XDG_DATA_HOME:-$HOME/.local/share}
install -Dm755 "$target/release/video" "$HOME/.local/bin/video"
install -Dm644 resources/video.desktop "$data/applications/video.desktop"
install -Dm644 resources/video.svg "$data/icons/hicolor/scalable/apps/video.svg"
echo "installed: ~/.local/bin/video, video.desktop, video.svg"
