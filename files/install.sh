#!/bin/sh
# Build files and install it for this user: binary, desktop entry, icon, and the file
# picker portal (D-Bus service + backend file + portals.conf choosing it).
# The window's app_id is "files", so Hyprland and the launcher match it to files.desktop.
set -eu
cd "$(dirname "$0")"
cargo build --release -q
target=$(cargo metadata --format-version 1 --no-deps | sed 's/.*"target_directory":"\([^"]*\)".*/\1/')
data=${XDG_DATA_HOME:-$HOME/.local/share}
config=${XDG_CONFIG_HOME:-$HOME/.config}
install -Dm755 "$target/release/files" "$HOME/.local/bin/files"
install -Dm644 resources/files.desktop "$data/applications/files.desktop"
install -Dm644 resources/files.svg "$data/icons/hicolor/scalable/apps/files.svg"
# The picker portal: D-Bus starts `files --portal` on the first request.
mkdir -p "$data/dbus-1/services"
printf '[D-BUS Service]\nName=org.freedesktop.impl.portal.desktop.files\nExec=%s --portal\n' \
    "$HOME/.local/bin/files" > "$data/dbus-1/services/org.freedesktop.impl.portal.desktop.files.service"
# The running session bus only sees new service files after a reload (else "not activatable").
busctl --user call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig >/dev/null 2>&1 || true
install -Dm644 resources/files.portal "$data/xdg-desktop-portal/portals/files.portal"
install -Dm644 resources/hyprland-portals.conf "$config/xdg-desktop-portal/hyprland-portals.conf"
echo "installed: ~/.local/bin/files, files.desktop, files.svg, file picker portal"
echo "picker: restart the portal once (systemctl --user restart xdg-desktop-portal)"
