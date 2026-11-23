#!/bin/sh
# Checks two parts of install.sh: going back to the earlier version (--rollback), and the system packages: for each
# distribution (a made-up os-release) the command it shows.
# Run from the repository: sh tools/test-install.sh. Nothing is installed: with no terminal, install.sh only shows it.
set -eu

repo=$(cd "$(dirname "$0")/.." && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT

# The part between these lines, as install.sh runs it (set -eu), reading the made-up files.
{
    echo 'set -eu; say() { printf "%s\n" "$*"; }; LIB=/nonexistent; BIN=/nonexistent'
    sed -n '/^# What the programs need/,/^case ":\$PATH:" in$/p' "$repo/install.sh" | sed '$d' |
        sed "s#/etc/os-release#$t/os-release#; s#/usr/lib/os-release#$t/os-release#; s#/run/ostree-booted#$t/ostree#"
} > "$t/part.sh"
grep -q 'distro=' "$t/part.sh" || { echo "test-install: the package part of install.sh was not found" >&2; exit 1; }

# Only the tools the part uses, so wl-paste, fcitx5, firefox and alacritty are missing.
mkdir "$t/bin"
for c in sh sed printf grep id readlink; do ln -s "$(command -v "$c")" "$t/bin/$c"; done

fails=0
# check NAME OS-RELEASE EXPECTED [ostree]: the shown command must contain EXPECTED.
check() {
    printf '%b' "$2" > "$t/os-release"
    rm -f "$t/ostree"; [ "${4:-}" = ostree ] && touch "$t/ostree"
    out=$(PATH="$t/bin" sh "$t/part.sh" </dev/null 2>&1) || { echo "FAIL $1: exit $?"; fails=$((fails + 1)); return 0; }
    case "$out" in
        *"$3"*) echo "ok   $1" ;;
        *) echo "FAIL $1: expected '$3' in:"; printf '%s\n' "$out" | sed 's/^/       /'; fails=$((fails + 1)) ;;
    esac
}

# Fedora, Debian and Arch have no ID_LIKE: install.sh runs with set -u.
check fedora 'ID=fedora\n' 'sudo dnf install -y webkitgtk6.0'
check silverblue 'ID=fedora\nVARIANT_ID=silverblue\n' 'sudo rpm-ostree install' ostree
check debian 'ID=debian\n' 'firefox-esr'
check ubuntu 'ID=ubuntu\nID_LIKE=debian\n' 'packages.mozilla.org'
check mint 'ID=linuxmint\nID_LIKE="ubuntu debian"\n' 'sudo apt-get install'
check arch 'ID=arch\n' 'sudo pacman -S --needed'
check opensuse 'ID="opensuse-tumbleweed"\nID_LIKE="opensuse suse"\n' 'sudo zypper --non-interactive install'
check rocky 'ID="rocky"\nID_LIKE="rhel centos fedora"\n' 'sudo dnf install -y epel-release'
check unknown 'ID=gentoo\n' 'Install them with your package manager'

# A Firefox on PATH that is a snap (a link to /usr/bin/snap) counts as missing: Ubuntu gets Mozilla's.
mkdir -p "$t/usr/bin"
printf '#!/bin/sh\n' > "$t/usr/bin/snap"; chmod +x "$t/usr/bin/snap"
ln -s "$t/usr/bin/snap" "$t/bin/firefox"
check "ubuntu, snap firefox" 'ID=ubuntu\nID_LIKE=debian\n' 'packages.mozilla.org'
# A real Firefox (Linux Mint's) is kept: no Mozilla archive.
rm "$t/bin/firefox"; ln -s "$(command -v sh)" "$t/bin/firefox"
check "mint, own firefox" 'ID=linuxmint\nID_LIKE="ubuntu debian"\n' 'wl-clipboard fcitx5'

# --rollback, in a made-up home: it goes back only to a version that is whole and runs.
home="$t/home"; vers="$home/.local/lib/whaybench-versions"; lib="$home/.local/lib/whaybench"
programs="whaybench whaybench-panel whaybench-term whaybench-view whaybench-project whaybench-files alacritty"
fresh() {
    rm -rf "$home"; mkdir -p "$vers/1" "$vers/2"
    for f in $programs; do printf '#!/bin/sh\necho whaybench 1\n' > "$vers/1/$f"; chmod +x "$vers/1/$f"; done
    [ "${1:-}" = old ] || (cd "$vers/1" && sha256sum -- * > .sha256sums)
    ln -s "$vers/2" "$lib"; ln -s "$vers/1" "$vers/previous"
}
# rolls NAME EXPECTED: the version in use after --rollback.
rolls() {
    HOME="$home" sh "$repo/install.sh" --rollback >/dev/null 2>&1 || true
    now=$(readlink "$lib"); now=${now##*/}
    if [ "$now" = "$2" ]; then echo "ok   $1"; else echo "FAIL $1: version $now in use, expected $2"; fails=$((fails + 1)); fi
}
fresh; rolls "rollback" 1
fresh; rm "$vers/1/whaybench-panel"; rolls "rollback, a program missing" 2
fresh; echo x >> "$vers/1/whaybench-term"; rolls "rollback, a program changed" 2
fresh; chmod -x "$vers/1/whaybench-term"; rolls "rollback, a program that cannot run" 2
fresh old; rolls "rollback, no list of files" 1
fresh old; rm "$vers/1/whaybench-view"; rolls "rollback, no list and a program missing" 2
fresh old; printf '#!/bin/sh\nexit 1\n' > "$vers/1/whaybench"; rolls "rollback, it does not run" 2
# The list is made as install.sh makes it: every file of the version, and not itself.
grep -qF '(cd "$dest.part" && sha256sum -- * > .sha256sums)' "$repo/install.sh" || { echo "FAIL the list of files is not made"; fails=$((fails + 1)); }

[ "$fails" = 0 ] || { echo "$fails failed"; exit 1; }
