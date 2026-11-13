#!/bin/sh
# Installs the latest Whaybench release for this user:
#   curl -fsSL https://github.com/Whaybench/whaybench/releases/latest/download/install.sh | sh
# With whaybench-x86_64-linux.tar.gz (and its .sha256) already in the current folder, that file is installed instead.
# Each version goes to a folder of its own in ~/.local/lib/whaybench-versions; ~/.local/lib/whaybench is a link to the
# one in use, switched in one step, and the version before is kept: `sh install.sh --rollback` goes back to it.
# The command is ~/.local/bin/whaybench. When Claude Code is installed, its
# Whaybench plugin is installed too. System packages are only checked: their install command is shown.
set -eu

REPO="${WHAYBENCH_REPO:-https://github.com/Whaybench/whaybench}"
# The key Whaybench releases are signed with (minisign). With minisign installed, the package is checked against it.
KEY=RWTwDOf9rBKzxEepAVlbLTan2g/RQFDuCZeqmU4Q+dNwf+wF+3a2Sfia
NAME=whaybench-x86_64-linux
LIB="$HOME/.local/lib/whaybench"
VERS="$HOME/.local/lib/whaybench-versions"
BIN="$HOME/.local/bin"

say() { printf '%s\n' "$*"; }
fail() { say "whaybench install: $*" >&2; exit 1; }

# Points the link at another folder in one step: a program starting meanwhile finds the old version or the new one.
switch() { ln -sfn "$1" "$LIB.new" && mv -T "$LIB.new" "$LIB"; }

if [ "${1:-}" = --rollback ]; then
    prev=$(readlink "$VERS/previous" 2>/dev/null) || fail "there is no earlier version to go back to."
    [ -x "$prev/whaybench" ] || fail "the earlier version in $prev is incomplete."
    now=$(readlink "$LIB" 2>/dev/null || true)
    switch "$prev"
    [ -n "$now" ] && ln -sfn "$now" "$VERS/previous"
    say "Back to $("$LIB/whaybench" --version 2>/dev/null || echo "$prev"). Restart Whaybench to use it."
    exit 0
fi

[ "$(uname -s)" = Linux ] || fail "Whaybench runs on Linux only."
[ "$(uname -m)" = x86_64 ] || fail "there is a build for x86_64 only; build from source on $(uname -m)."
command -v curl >/dev/null || fail "curl is needed."
command -v tar >/dev/null || fail "tar is needed."

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
if [ -f "$NAME.tar.gz" ] && [ -f "$NAME.tar.gz.sha256" ]; then
    say "Installing $NAME.tar.gz from this folder"
    cp "$NAME.tar.gz" "$NAME.tar.gz.sha256" "$tmp"/
    [ -f "$NAME.tar.gz.minisig" ] && cp "$NAME.tar.gz.minisig" "$tmp"/
else
    say "Downloading $NAME.tar.gz"
    for f in "$NAME.tar.gz" "$NAME.tar.gz.sha256" "$NAME.tar.gz.minisig"; do
        curl -fsSL "$REPO/releases/latest/download/$f" -o "$tmp/$f"
    done
fi
(cd "$tmp" && sha256sum -c --quiet "$NAME.tar.gz.sha256") || fail "the download does not match its checksum."
if command -v minisign >/dev/null; then
    minisign -Vq -m "$tmp/$NAME.tar.gz" -x "$tmp/$NAME.tar.gz.minisig" -P "$KEY" || fail "the package is not signed by Whaybench."
    say "Signature checked"
else
    say "minisign is not installed: only the checksum was checked (install minisign to check the signature too)."
fi
tar xzf "$tmp/$NAME.tar.gz" -C "$tmp"

ver=$("$tmp/$NAME/whaybench" --version 2>/dev/null | awk '{print $2}')
[ -n "$ver" ] || fail "the package's program does not run here."
mkdir -p "$VERS" "$BIN"
# The new version is complete in its folder before anything uses it; a failure here leaves the one in use as it is.
# Alacritty, for the terminal tabs, comes with the release.
dest="$VERS/$ver"
rm -rf "$dest.part"
mkdir -p "$dest.part"
cp "$tmp/$NAME"/whaybench* "$tmp/$NAME"/alacritty "$tmp/$NAME"/NOTICE "$tmp/$NAME"/*LICENSE* "$dest.part"/
rm -rf "$dest"
mv "$dest.part" "$dest"
# Versions before 0.6.1 put the programs straight in $LIB: that folder becomes the version to go back to.
if [ -d "$LIB" ] && [ ! -L "$LIB" ]; then rm -rf "$VERS/before-0.6.1"; mv "$LIB" "$VERS/before-0.6.1"; ln -sfn "$VERS/before-0.6.1" "$VERS/previous"; fi
now=$(readlink "$LIB" 2>/dev/null || true)
# A running Whaybench keeps its own files; it shows that a new version waits for a restart.
switch "$dest"
[ -n "$now" ] && [ "$now" != "$dest" ] && ln -sfn "$now" "$VERS/previous"
# This version and the one before; older ones go.
keep=$(readlink "$VERS/previous" 2>/dev/null || true)
for d in "$VERS"/*; do
    [ "$d" = "$VERS/previous" ] || [ "$d" = "$dest" ] || [ "$d" = "$keep" ] || rm -rf "$d"
done
ln -sf "$LIB/whaybench" "$BIN/whaybench"
say "Installed $("$LIB/whaybench" --version 2>/dev/null || echo whaybench) in $dest"

if command -v claude >/dev/null; then
    # An existing marketplace makes add fail; only a failed install is reported, with the reason.
    added=$(claude plugin marketplace add Whaybench/whaybench 2>&1)
    # A marketplace added before keeps the catalogue of then, and install leaves an installed plugin as it is.
    claude plugin marketplace update whaybench >/dev/null 2>&1 || true
    if claude plugin install context-map@whaybench >/dev/null 2>&1 && claude plugin update context-map@whaybench >/dev/null 2>&1; then
        say "Claude Code plugin context-map is installed and up to date (a new version applies to sessions started after it)"
    else
        say "Claude Code plugin was not installed:"
        printf '%s\n' "$added" | sed 's/^/    /'
        say "Then run:  claude plugin marketplace add Whaybench/whaybench && claude plugin install context-map@whaybench && claude plugin update context-map@whaybench"
    fi
else
    say "Claude Code was not found. With Codex CLI, open a conversation with Session → New conversation · Codex."
fi

# What the programs need from the system.
missing=""
for c in wl-paste firefox fcitx5; do command -v "$c" >/dev/null || missing="$missing $c"; done
[ -x "$LIB/alacritty" ] || command -v alacritty >/dev/null || missing="$missing alacritty"
for l in libgtk-4.so.1 libwebkitgtk-6.0.so.4; do ldconfig -p 2>/dev/null | grep -q "$l" || missing="$missing $l"; done
if [ -n "$missing" ]; then
    say "Missing:$missing. Install them with one of:"
    say "  Fedora:            sudo dnf install gtk4 webkitgtk6.0 wl-clipboard firefox fcitx5"
    say "  Ubuntu, Debian:    sudo apt install libgtk-4-1 libwebkitgtk-6.0-4 wl-clipboard firefox fcitx5"
    say "  Arch:              sudo pacman -S gtk4 webkitgtk-6.0 wl-clipboard firefox fcitx5"
    say "  AlmaLinux, Rocky:  sudo dnf install epel-release && sudo dnf install gtk4 webkitgtk6.0 wl-clipboard firefox && sudo dnf upgrade \"gstreamer1*\""
    say "With fcitx5, add the engine for your language, e.g. fcitx5-hangul, fcitx5-mozc or fcitx5-chinese-addons."
    say "AlmaLinux and Rocky Linux 10 have no fcitx5; Whaybench then takes the desktop's input method."
fi

case ":$PATH:" in *":$BIN:"*) ;; *) say "Add $BIN to your PATH to run whaybench." ;; esac
say "Run: whaybench"
