#!/bin/sh
# ferrite installer: downloads prebuilt ferrite tools from GitHub releases, checks their
# SHA-256 and puts them in ~/.local/bin. No root needed. Re-run it to update.
#
#   curl -fsSL https://raw.githubusercontent.com/uhm26/ferrite/main/install.sh | sh -s -- clipboard
#   sh install.sh [--dir DIR] [--version X.Y.Z] [--uninstall] TOOL...
#
# Everything runs inside main(), called on the last line: if the download is cut off,
# nothing runs.

set -eu

REPO="uhm26/ferrite"
# Tools with prebuilt releases so far; the others: build from source (see README.md).
PREBUILT="clipboard"
ALL_TOOLS="term files image video screenshot wallpaper clipboard"

say() { printf 'ferrite: %s\n' "$*"; }
die() { printf 'ferrite: error: %s\n' "$*" >&2; exit 1; }
has() { command -v "$1" >/dev/null 2>&1; }

usage() {
    cat <<EOF
ferrite installer

usage: install.sh [options] TOOL...

options:
  --dir DIR        install into DIR (default: \$FERRITE_INSTALL_DIR or ~/.local/bin)
  --version X.Y.Z  install this version instead of the latest
  --uninstall      remove the tools instead
  -h, --help       show this

tools with prebuilt binaries: $PREBUILT
all tools: $ALL_TOOLS
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

sha256_of() {
    if has sha256sum; then
        sha256sum "$1" | cut -d' ' -f1
    elif has shasum; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        die "needs sha256sum or shasum to check the download"
    fi
}

# Release asset name for this machine, or fail with what to do instead.
asset_of() {
    case "$(uname -s)" in
        Linux) ;;
        *) die "ferrite is for Linux (with Wayland); this is $(uname -s)" ;;
    esac
    case "$(uname -m)" in
        x86_64 | amd64) ;;
        *) die "no prebuilt $1 for $(uname -m) yet; build it from source: https://github.com/$REPO#build-from-source" ;;
    esac
    printf '%s-x86_64-linux-musl' "$1"
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
    case " $PREBUILT " in
        *" $tool "*) ;;
        *)
            case " $ALL_TOOLS " in
                *" $tool "*) die "$tool has no prebuilt binary yet; build it from source: https://github.com/$REPO#build-from-source" ;;
                *) die "unknown tool '$tool' (tools: $ALL_TOOLS)" ;;
            esac
            ;;
    esac
    asset=$(asset_of "$tool")
    if [ -n "$version" ]; then tag="$tool-v$version"; else tag=$(latest_tag "$tool"); fi
    base="https://github.com/$REPO/releases/download/$tag"

    say "downloading $tag ($asset)"
    fetch "$base/$asset.tar.gz" "$tmp/$asset.tar.gz"
    fetch "$base/$asset.tar.gz.sha256" "$tmp/$asset.tar.gz.sha256"
    want=$(cut -d' ' -f1 "$tmp/$asset.tar.gz.sha256")
    got=$(sha256_of "$tmp/$asset.tar.gz")
    [ "$want" = "$got" ] || die "checksum mismatch for $asset.tar.gz (corrupt or tampered download)"

    tar -xzf "$tmp/$asset.tar.gz" -C "$tmp"
    mkdir -p "$dir"
    # Copy then rename: replacing a running daemon's binary is safe this way.
    cp "$tmp/$asset/$tool" "$dir/.$tool.new"
    chmod 755 "$dir/.$tool.new"
    mv -f "$dir/.$tool.new" "$dir/$tool"
    say "installed $tool ${tag#"$tool"-v} to $dir/$tool"
    notes_for "$tool"
}

notes_for() {
    case "$1" in
        clipboard) say "clipboard is a daemon: start it with your session (e.g. Hyprland: exec-once = clipboard; sway: exec clipboard)" ;;
    esac
}

uninstall_one() {
    if [ -e "$dir/$1" ]; then
        rm -f "$dir/$1"
        say "removed $dir/$1"
    else
        say "$1 is not installed in $dir"
    fi
}

main() {
    dir="${FERRITE_INSTALL_DIR:-$HOME/.local/bin}"
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

    has tar || die "needs tar"
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT INT TERM
    for t in $tools; do install_one "$t"; done

    case ":$PATH:" in
        *":$dir:"*) ;;
        *) say "note: $dir is not in your PATH; add it in your shell's config" ;;
    esac
}

main "$@"
