#!/bin/sh
# Build vid and install it for this user: binary, desktop entry (video handler), icon.
# The window's app_id is "vid", so Hyprland and the launcher match it to vid.desktop.
# Does not make vid the default video app (ask the owner; xdg-mime default vid.desktop …).
set -eu
cd "$(dirname "$0")"
cargo build --release -q
target=$(cargo metadata --format-version 1 --no-deps | sed 's/.*"target_directory":"\([^"]*\)".*/\1/')
data=${XDG_DATA_HOME:-$HOME/.local/share}
install -Dm755 "$target/release/vid" "$HOME/.local/bin/vid"
install -Dm644 resources/vid.desktop "$data/applications/vid.desktop"
install -Dm644 resources/vid.svg "$data/icons/hicolor/scalable/apps/vid.svg"
echo "installed: ~/.local/bin/vid, vid.desktop, vid.svg"
