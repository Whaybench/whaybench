#!/bin/sh
# Installs the latest Whaybench release for this user:
#   curl -fsSL https://github.com/Whaybench/whaybench/releases/latest/download/install.sh | sh
# With whaybench-x86_64-linux.tar.gz (and its .sha256) already in the current folder, that file is installed instead.
# The programs go to ~/.local/lib/whaybench, the command to ~/.local/bin/whaybench. When Claude Code is installed, its
# Whaybench plugin is installed too. System packages are only checked: their install command is shown.
set -eu

REPO="${WHAYBENCH_REPO:-https://github.com/Whaybench/whaybench}"
NAME=whaybench-x86_64-linux
LIB="$HOME/.local/lib/whaybench"
BIN="$HOME/.local/bin"

say() { printf '%s\n' "$*"; }
fail() { say "whaybench install: $*" >&2; exit 1; }

[ "$(uname -s)" = Linux ] || fail "Whaybench runs on Linux only."
[ "$(uname -m)" = x86_64 ] || fail "there is a build for x86_64 only; build from source on $(uname -m)."
command -v curl >/dev/null || fail "curl is needed."
command -v tar >/dev/null || fail "tar is needed."

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
if [ -f "$NAME.tar.gz" ] && [ -f "$NAME.tar.gz.sha256" ]; then
    say "Installing $NAME.tar.gz from this folder"
    cp "$NAME.tar.gz" "$NAME.tar.gz.sha256" "$tmp"/
else
    say "Downloading $NAME.tar.gz"
    curl -fsSL "$REPO/releases/latest/download/$NAME.tar.gz" -o "$tmp/$NAME.tar.gz"
    curl -fsSL "$REPO/releases/latest/download/$NAME.tar.gz.sha256" -o "$tmp/$NAME.tar.gz.sha256"
fi
(cd "$tmp" && sha256sum -c --quiet "$NAME.tar.gz.sha256") || fail "the download does not match its checksum."
tar xzf "$tmp/$NAME.tar.gz" -C "$tmp"

mkdir -p "$LIB" "$BIN"
cp "$tmp/$NAME"/whaybench* "$LIB"/
cp "$tmp/$NAME"/NOTICE "$tmp/$NAME"/*LICENSE* "$LIB"/
# Alacritty, for the terminal tabs, comes with the release.
cp "$tmp/$NAME"/alacritty "$LIB"/
ln -sf "$LIB/whaybench" "$BIN/whaybench"
say "Installed $("$LIB/whaybench" --version 2>/dev/null || echo whaybench) in $LIB"

if command -v claude >/dev/null; then
    # An existing marketplace makes add fail; only a failed install is reported, with the reason.
    added=$(claude plugin marketplace add Whaybench/whaybench 2>&1)
    if claude plugin install context-map@whaybench >/dev/null 2>&1; then say "Claude Code plugin context-map installed"
    else
        say "Claude Code plugin was not installed:"
        printf '%s\n' "$added" | sed 's/^/    /'
        say "Then run:  claude plugin marketplace add Whaybench/whaybench && claude plugin install context-map@whaybench"
    fi
else
    say "Claude Code was not found. With Codex CLI start Whaybench with --agent codex."
fi

# What the programs need from the system.
missing=""
for c in wl-paste firefox; do command -v "$c" >/dev/null || missing="$missing $c"; done
[ -x "$LIB/alacritty" ] || command -v alacritty >/dev/null || missing="$missing alacritty"
for l in libgtk-4.so.1 libwebkitgtk-6.0.so.4; do ldconfig -p 2>/dev/null | grep -q "$l" || missing="$missing $l"; done
if [ -n "$missing" ]; then
    say "Missing:$missing. Install them with one of:"
    say "  Fedora:            sudo dnf install gtk4 webkitgtk6.0 wl-clipboard firefox"
    say "  Ubuntu, Debian:    sudo apt install libgtk-4-1 libwebkitgtk-6.0-4 wl-clipboard firefox"
    say "  Arch:              sudo pacman -S gtk4 webkitgtk-6.0 wl-clipboard firefox"
    say "  AlmaLinux, Rocky:  sudo dnf install epel-release && sudo dnf install gtk4 webkitgtk6.0 wl-clipboard firefox && sudo dnf upgrade \"gstreamer1*\""
fi

case ":$PATH:" in *":$BIN:"*) ;; *) say "Add $BIN to your PATH to run whaybench." ;; esac
say "Run: whaybench"
