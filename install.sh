#!/bin/sh
# Installs the latest Whaybench release for this user:
#   curl -fsSL https://github.com/Whaybench/whaybench/releases/latest/download/install.sh | sh
# With whaybench-x86_64-linux.tar.gz (and its .sha256) already in the current folder, that file is installed instead.
# Each version goes to a folder of its own in ~/.local/lib/whaybench-versions; ~/.local/lib/whaybench is a link to the
# one in use, switched in one step, and the version before is kept: `sh install.sh --rollback` goes back to it.
# The command is ~/.local/bin/whaybench. When Claude Code is installed, its
# Whaybench plugin is installed too. Missing system packages are installed with sudo when the person says yes.
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

# Everything runs from main, called on the last line: a download cut short under curl | sh runs nothing.
main() {

if [ "${1:-}" = --rollback ]; then
    prev=$(readlink "$VERS/previous" 2>/dev/null) || fail "there is no earlier version to go back to."
    # Checked before the switch: every file as it was installed; a version from before that was kept, its programs.
    if [ -f "$prev/.sha256sums" ]; then
        (cd "$prev" && sha256sum -c --quiet .sha256sums >/dev/null 2>&1) || fail "the earlier version in $prev is incomplete or changed."
        for f in "$prev"/whaybench* "$prev/alacritty"; do [ -x "$f" ] || fail "the earlier version in $prev is incomplete (${f##*/})."; done
    else
        for f in whaybench whaybench-panel whaybench-term whaybench-view whaybench-project whaybench-files alacritty; do [ -x "$prev/$f" ] || fail "the earlier version in $prev is incomplete ($f)."; done
    fi
    "$prev/whaybench" --version >/dev/null 2>&1 || fail "the earlier version in $prev does not run here."
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
        curl -fsSL --retry 3 "$REPO/releases/latest/download/$f" -o "$tmp/$f"
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
# What --rollback checks this version against, once it is the earlier one (a dot file: not among what * names).
(cd "$dest.part" && sha256sum -- * > .sha256sums)
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
    added=$(claude plugin marketplace add Whaybench/whaybench 2>&1) || true
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
    say "Claude Code was not found. With Codex CLI, open a conversation with Session → New conversation → Codex."
fi

# What the programs need from the system: missing packages are installed with sudo once the person says yes.
osr=/etc/os-release; [ -r "$osr" ] || osr=/usr/lib/os-release
distro=$( (. "$osr" && echo " ${ID:-} ${ID_LIKE:-} ") 2>/dev/null || true)
# Fedora Silverblue and Kinoite say ID=fedora but take packages through rpm-ostree.
[ -e /run/ostree-booted ] && distro=" ostree$distro"
missing=""
for c in wl-paste fcitx5; do command -v "$c" >/dev/null || missing="$missing $c"; done
# A snap Firefox cannot open in Whaybench's view panel: Ubuntu's firefox package is only a way to that snap, and a
# snap's command (/snap/bin/firefox) is a link to /usr/bin/snap.
ff=$(command -v firefox || true)
case "${ff:-/}|$(readlink -f "${ff:-/}")" in "/|"*|/snap/*|*/usr/bin/snap) missing="$missing firefox" ;; esac
case " $missing " in *" firefox "*) ;; *) dpkg-query -W -f='${Version}' firefox 2>/dev/null | grep -q snap && missing="$missing firefox" ;; esac
# AlmaLinux and Rocky Linux 10 have no fcitx5; Whaybench then takes the desktop's input method.
case "$distro" in *" rhel "*|*" almalinux "*|*" rocky "*) missing=$(printf '%s' "$missing" | sed 's/ fcitx5//') ;; esac
[ -x "$LIB/alacritty" ] || command -v alacritty >/dev/null || missing="$missing alacritty"
libs=$(PATH="$PATH:/usr/sbin:/sbin" ldconfig -p 2>/dev/null || true)
if [ -n "$libs" ]; then
    for l in libgtk-4.so.1 libwebkitgtk-6.0.so.4; do printf '%s' "$libs" | grep -q "$l" || missing="$missing $l"; done
fi
if [ -n "$missing" ]; then
    S="sudo "; [ "$(id -u)" = 0 ] && S=""
    after=""
    case "$distro" in
        *" rhel "*|*" almalinux "*|*" rocky "*) cmd="${S}dnf install -y epel-release && ${S}dnf install -y webkitgtk6.0 wl-clipboard firefox google-noto-color-emoji-fonts && ${S}dnf upgrade -y \"gstreamer1*\"" ;;
        *" ostree "*) cmd="${S}rpm-ostree install -y --idempotent webkitgtk6.0 wl-clipboard firefox fcitx5 google-noto-color-emoji-fonts"
            after="Restart the computer to use them (rpm-ostree applies packages at the next boot)." ;;
        *" fedora "*) cmd="${S}dnf install -y webkitgtk6.0 wl-clipboard firefox fcitx5 google-noto-color-emoji-fonts" ;;
        *" arch "*) cmd="${S}pacman -S --needed --noconfirm webkitgtk-6.0 wl-clipboard firefox fcitx5 noto-fonts-emoji" ;;
        # Firefox from Mozilla's archive, put before Ubuntu's snap one; only when it is missing,
        # so a system with a Firefox of its own (Linux Mint) keeps it.
        # Ubuntu's snap package has the epoch 1:, so apt counts Mozilla's as a downgrade.
        *" ubuntu "*) case " $missing " in
            *" firefox "*) cmd="${S}install -d -m 0755 /etc/apt/keyrings && curl -fsSL --retry 3 https://packages.mozilla.org/apt/repo-signing-key.gpg | ${S}tee /etc/apt/keyrings/packages.mozilla.org.asc >/dev/null && echo 'deb [signed-by=/etc/apt/keyrings/packages.mozilla.org.asc] https://packages.mozilla.org/apt mozilla main' | ${S}tee /etc/apt/sources.list.d/mozilla.list >/dev/null && printf 'Package: *\\nPin: origin packages.mozilla.org\\nPin-Priority: 1000\\n' | ${S}tee /etc/apt/preferences.d/mozilla >/dev/null && " ff="--allow-downgrades firefox" ;;
            *) cmd="" ff="" ;;
            esac
            cmd="$cmd${S}apt-get update && ${S}apt-get install -y libwebkitgtk-6.0-4 wl-clipboard ${ff:+$ff }fcitx5 fonts-noto-color-emoji" ;;
        # Debian has Firefox as firefox-esr (it gives /usr/bin/firefox too).
        *" debian "*) cmd="${S}apt-get update && ${S}apt-get install -y libwebkitgtk-6.0-4 wl-clipboard firefox-esr fcitx5 fonts-noto-color-emoji" ;;
        *" suse "*|*" opensuse "*) cmd="${S}zypper --non-interactive install libwebkitgtk-6_0-4 wl-clipboard MozillaFirefox fcitx5 google-noto-coloremoji-fonts" ;;
        *) cmd="" ;;
    esac
    say "Missing:$missing."
    ans=n
    # Asked on the terminal: under curl | sh the script itself is stdin. Not asked when there is no terminal (the
    # update at start writes to a log), when sudo is missing, or on a distribution this script does not know.
    if [ -n "$cmd" ] && [ -t 1 ] && { [ -z "$S" ] || command -v sudo >/dev/null; } && (: </dev/tty) 2>/dev/null; then
        printf 'Install them now with:\n  %s\nProceed? [y/N] ' "$cmd"
        read -r ans </dev/tty || ans=n
    fi
    case "$ans" in
        y|Y|yes|YES) if sh -c "$cmd"; then say "System packages installed.${after:+ $after}"; else say "Installing the system packages failed. Run:  $cmd"; fi ;;
        *) if [ -n "$cmd" ]; then say "Install them with:  $cmd"; else
            say "Install them with your package manager: WebKitGTK 6.0, wl-clipboard, Firefox, fcitx5 and a colour emoji font."; fi ;;
    esac
    say "With fcitx5, add the engine for your language, e.g. fcitx5-hangul, fcitx5-mozc or fcitx5-chinese-addons."
fi

case ":$PATH:" in
    *":$BIN:"*) ;;
    *) say "$BIN is not on your PATH. Add it with:"
        case "${SHELL:-}" in
            *zsh) say "  echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.zshrc && . ~/.zshrc" ;;
            *fish) say "  fish_add_path -U ~/.local/bin" ;;
            *) say "  echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.bashrc && . ~/.bashrc" ;;
        esac
        say "Or run it as $BIN/whaybench." ;;
esac
say "Run: whaybench"
}

main "$@"
