#!/bin/sh
# ferrite installer (Arch Linux or Fedora + Wayland): downloads prebuilt ferrite tools from GitHub
# releases, checks their SHA-256 and puts them in ~/.local/bin. No root needed. Re-run it
# to update.
#
#   curl -fsSL https://raw.githubusercontent.com/uhm26/ferrite/main/install.sh | sh -s -- term
#   sh install.sh [--dir DIR] [--version X.Y.Z] [--uninstall] TOOL...
#
# Everything runs inside main(), called on the last line: if the download is cut off,
# nothing runs.

set -eu

REPO="uhm26/ferrite"
TOOLS="term files image video screenshot wallpaper clipboard auth"

say() { printf 'ferrite: %s\n' "$*"; }
die() { printf 'ferrite: error: %s\n' "$*" >&2; exit 1; }
has() { command -v "$1" >/dev/null 2>&1; }

usage() {
    cat <<EOF
ferrite installer (Arch Linux or Fedora + Wayland)

usage: install.sh [options] TOOL...

options:
  --dir DIR        install into DIR (default: \$FERRITE_INSTALL_DIR or ~/.local/bin)
  --version X.Y.Z  install this version instead of the latest
  --uninstall      remove the tools instead
  -h, --help       show this

tools: $TOOLS
EOF
}

# Download $1 to file $2. FERRITE_MIRROR (a local folder laid out like the release
# downloads: <tag>/<file>) is for testing without the network.
fetch() {
    if [ -n "${FERRITE_MIRROR:-}" ]; then
        cp "$FERRITE_MIRROR/${1#*/releases/download/}" "$2" 2>/dev/null \
            || die "not in mirror: ${1#*/releases/download/}"
    elif has curl; then
        curl --proto '=https' --tlsv1.2 -fsSL "$1" -o "$2" || die "download failed: $1"
    elif has wget; then
        wget -q --https-only "$1" -O "$2" || die "download failed: $1"
    else
        die "needs curl or wget"
    fi
}

# ferrite supports Arch Linux, Fedora and distros based on them (ID or ID_LIKE), x86_64.
# Sets $distro (arch or fedora): it picks the package names and the install command.
check_system() {
    ids=$(grep -E '^(ID|ID_LIKE)=' /etc/os-release 2>/dev/null | cut -d= -f2 | tr -d '"' | tr '\n' ' ')
    case " $ids " in
        *" arch "*) distro=arch ;;
        *" fedora "*) distro=fedora ;;
        *) die "ferrite supports Arch Linux and Fedora (and distros based on them) only" ;;
    esac
    case "$(uname -m)" in
        x86_64) ;;
        *) die "no prebuilt tools for $(uname -m); build from source: https://github.com/$REPO#build-from-source" ;;
    esac
}

# Newest release tag of tool $1 (tags are <tool>-v<version>, newest first in the API).
latest_tag() {
    [ -n "${FERRITE_MIRROR:-}" ] && die "with FERRITE_MIRROR, pass --version"
    list="$tmp/releases.json"
    fetch "https://api.github.com/repos/$REPO/releases?per_page=100" "$list"
    tag=$(grep -o "\"tag_name\": *\"$1-v[^\"]*\"" "$list" | head -n 1 | sed 's/.*"\([^"]*\)"$/\1/')
    [ -n "$tag" ] || die "no release of $1 found"
    printf '%s' "$tag"
}

install_one() {
    tool=$1
    case " $TOOLS " in
        *" $tool "*) ;;
        *) die "unknown tool '$tool' (tools: $TOOLS)" ;;
    esac
    asset="$tool-x86_64-linux"
    if [ -n "$version" ]; then tag="$tool-v$version"; else tag=$(latest_tag "$tool"); fi
    base="https://github.com/$REPO/releases/download/$tag"

    say "downloading $tag"
    fetch "$base/$asset.tar.gz" "$tmp/$asset.tar.gz"
    fetch "$base/$asset.tar.gz.sha256" "$tmp/$asset.tar.gz.sha256"
    want=$(cut -d' ' -f1 "$tmp/$asset.tar.gz.sha256")
    got=$(sha256sum "$tmp/$asset.tar.gz" | cut -d' ' -f1)
    [ "$want" = "$got" ] || die "checksum mismatch for $asset.tar.gz (corrupt or tampered download)"

    tar -xzf "$tmp/$asset.tar.gz" -C "$tmp"
    mkdir -p "$dir"
    # Copy then rename: replacing a running daemon's binary is safe this way.
    cp "$tmp/$asset/$tool" "$dir/.$tool.new"
    chmod 755 "$dir/.$tool.new"
    mv -f "$dir/.$tool.new" "$dir/$tool"
    # The version, for the updater to compare with the newest release.
    mkdir -p "$state" && printf '%s\n' "${tag#"$tool"-v}" >"$state/$tool"
    say "installed $tool ${tag#"$tool"-v} to $dir/$tool"
    install_menu_entry "$tool" "$tmp/$asset"
    missing_packages "$tool"
    notes_for "$tool"
}

# GUI tools ship a desktop entry + icon: app launchers and "open with" find them.
# Exec gets the full path, since a session's PATH often lacks ~/.local/bin.
install_menu_entry() {
    [ -f "$2/$1.desktop" ] || return 0
    data="${XDG_DATA_HOME:-$HOME/.local/share}"
    mkdir -p "$data/applications" "$data/icons/hicolor/scalable/apps"
    sed "s|^Exec=$1\$|Exec=$dir/$1|; s|^Exec=$1 |Exec=$dir/$1 |" "$2/$1.desktop" > "$data/applications/$1.desktop"
    if [ -f "$2/$1.svg" ]; then cp "$2/$1.svg" "$data/icons/hicolor/scalable/apps/$1.svg"; fi
    say "added a menu entry: $data/applications/$1.desktop"
}

# Package of a library or program the tools use, for $distro.
pkg_of() {
    if [ "$distro" = fedora ]; then
        case "$1" in
            libfreetype.so.6) echo freetype ;;
            librsvg-2.so.2) echo librsvg2 ;;
            libglib-2.0.so.0 | libgobject-2.0.so.0) echo glib2 ;;
            libwebpdemux.so.2) echo libwebp ;;
            libmpv.so.2) echo mpv-libs ;;
            libEGL.so.1) echo libglvnd-egl ;;
            libgbm.so.1) echo mesa-libgbm ;;
            adwaita-fonts) echo adwaita-sans-fonts adwaita-mono-fonts ;;
            bsdtar) echo bsdtar ;;
            *) pkg_of_arch "$1" ;;
        esac
    else
        pkg_of_arch "$1"
    fi
}

pkg_of_arch() {
    case "$1" in
        libfontconfig.so.1) echo fontconfig ;;
        libfreetype.so.6) echo freetype2 ;;
        libxkbcommon.so.0) echo libxkbcommon ;;
        librsvg-2.so.2) echo librsvg ;;
        libcairo.so.2) echo cairo ;;
        libglib-2.0.so.0 | libgobject-2.0.so.0) echo glib2 ;;
        libwebpdemux.so.2) echo libwebp ;;
        libmpv.so.2) echo mpv ;;
        libEGL.so.1) echo libglvnd ;;
        libgbm.so.1) echo mesa ;;
        xdg-open) echo xdg-utils ;;
        bsdtar) echo libarchive ;;
        udisksctl) echo udisks2 ;;
        pkexec) echo polkit ;;
        *) echo "$1" ;;
    esac
}

# One pacman/dnf line for everything tool $1 still needs: libraries it links (ldd) and
# programs it runs (xdg-open to open files, bsdtar for archives, udisks2 for drives).
missing_packages() {
    case "$1" in
        term) progs="xdg-open" ;;
        files) progs="xdg-open bsdtar udisksctl" ;; # udisksctl: the udisks2 drive service is there
        auth) progs="pkexec" ;;
        *) progs="" ;;
    esac
    need=$(ldd "$dir/$1" 2>/dev/null | awk '/not found/ {print $1}')
    for p in $progs; do has "$p" || need="$need $p"; done
    # The GUI tools draw with Adwaita Sans/Mono; without it they fall back to another font.
    case "$1" in
        term | files | image | video | auth)
            if ! fc-list : family 2>/dev/null | grep -q "Adwaita"; then need="$need adwaita-fonts"; fi ;;
    esac
    pkgs=""
    for n in $need; do
        for p in $(pkg_of "$n"); do
            case " $pkgs " in *" $p "*) ;; *) pkgs="$pkgs $p" ;; esac
        done
    done
    [ -n "$pkgs" ] || return 0
    if [ "$distro" = fedora ]; then cmd="sudo dnf install"; else cmd="sudo pacman -S --needed"; fi
    say "$1 needs a few more packages: $cmd$pkgs"
}

notes_for() {
    case "$1" in
        auth)
            say "auth is a daemon: start it with your session (e.g. Hyprland: exec-once = $dir/auth); only one polkit agent can run"
            [ -S /run/polkit/agent-helper.socket ] || say "auth needs polkit 126+ with its helper socket: sudo systemctl enable --now polkit-agent-helper.socket" ;;
        clipboard) say "clipboard is a daemon: start it with your session (e.g. Hyprland: exec-once = $dir/clipboard; sway: exec $dir/clipboard)" ;;
        wallpaper) say "wallpaper: run wallpaper IMAGE once; start a bare wallpaper with your session to restore it" ;;
        screenshot) say "screenshot: bind a key to screenshot -s (freeze + pick); plain screenshot saves to your Pictures folder" ;;
    esac
}

uninstall_one() {
    rm -f "$state/$1"
    if [ -e "$dir/$1" ]; then
        rm -f "$dir/$1"
        say "removed $dir/$1"
    else
        say "$1 is not installed in $dir"
    fi
    # Only the menu entry this installer wrote (it runs $dir/$1), never someone else's.
    data="${XDG_DATA_HOME:-$HOME/.local/share}"
    if grep -q "^Exec=$dir/$1\( \|\$\)" "$data/applications/$1.desktop" 2>/dev/null; then
        rm -f "$data/applications/$1.desktop" "$data/icons/hicolor/scalable/apps/$1.svg"
        say "removed its menu entry"
    fi
}

main() {
    dir="${FERRITE_INSTALL_DIR:-$HOME/.local/bin}"
    state="${XDG_STATE_HOME:-$HOME/.local/state}/ferrite"
    version=""
    remove=0
    tools=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --dir) [ $# -ge 2 ] || die "--dir needs a folder"; dir=$2; shift ;;
            --version) [ $# -ge 2 ] || die "--version needs a version"; version=${2#v}; shift ;;
            --uninstall) remove=1 ;;
            -h | --help) usage; return 0 ;;
            -*) die "unknown option $1 (see --help)" ;;
            *) tools="$tools $1" ;;
        esac
        shift
    done
    [ -n "$tools" ] || { usage; die "name at least one tool"; }

    if [ $remove = 1 ]; then
        for t in $tools; do uninstall_one "$t"; done
        return 0
    fi

    check_system
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT INT TERM
    for t in $tools; do install_one "$t"; done
    setup_path
}

# Make the tools runnable by name: put ~/.local/bin on PATH in each installed shell's
# config, once (a marked line). Many distros (Arch among them) leave it out. A custom --dir
# is the caller's to manage.
setup_path() {
    case ":$PATH:" in *":$dir:"*) return 0 ;; esac
    if [ "$dir" != "$HOME/.local/bin" ]; then
        say "note: $dir is not in your PATH; add it in your shell's config"
        return 0
    fi
    mark="# added by the ferrite installer"
    # shellcheck disable=SC2016 # $HOME and $PATH must expand when the shell starts, not now
    line='export PATH="$HOME/.local/bin:$PATH"'
    if has bash; then add_line "$HOME/.bashrc" "$line $mark"; fi
    if has zsh; then add_line "${ZDOTDIR:-$HOME}/.zshrc" "$line $mark"; fi
    f="${XDG_CONFIG_HOME:-$HOME/.config}/fish/conf.d/ferrite-path.fish"
    if has fish && [ ! -e "$f" ]; then
        mkdir -p "${f%/*}"
        printf 'fish_add_path -g ~/.local/bin %s\n' "$mark" > "$f"
        say "added ~/.local/bin to your PATH in $f"
    fi
    say "open a new terminal to run the tools by name (or, in this one: export PATH=\"$dir:\$PATH\")"
}

# Append line $2 to file $1 unless it is there already.
add_line() {
    grep -qxF "$2" "$1" 2>/dev/null && return 0
    printf '\n%s\n' "$2" >> "$1"
    say "added ~/.local/bin to your PATH in $1"
}

main "$@"
