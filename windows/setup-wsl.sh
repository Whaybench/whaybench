#!/bin/sh
# Prepares a fresh Fedora WSL distribution for Whaybench. Run as root inside it by the Windows installer:
#   wsl -d Whaybench -u root -- sh setup-wsl.sh USER [LANGUAGE]
# USER is made (no password: the Windows account guards the distribution) and becomes the default user. LANGUAGE (the Windows
# display language: ko, ja, en...) becomes Whaybench's language, which offers its input engine when it starts. Everything comes from its official source: Fedora's repositories
# (Firefox too, a real package), the Whaybench release, Anthropic's Claude Code installer and OpenAI's Codex CLI installer (a standalone program, no Node). Running it again finishes what a failed run left.
set -eu

user="${1:?usage: setup-wsl.sh USER [LANGUAGE [LOCALE]]}"
lang="${2:-en}"
# The Windows display language's locale (ko_KR): made here, so the input method speaks that language.
locale="${3:-}"
say() { printf 'whaybench-setup: %s\n' "$*"; }

# The name comes from the Windows account: one that is a system account here (root, adm, games...) is not taken over,
# or Whaybench and its agents would run as it.
uid=$(id -u "$user" 2>/dev/null || true)
if [ -n "$uid" ] && [ "$uid" -lt 1000 ]; then say "account $user: system account in Linux; account whaybench used"; user=whaybench; fi
say "account $user"
# Made before Fedora's first-start account setup, which then leaves it alone: that one would put the account in wheel,
# with sudo and no password.
id "$user" >/dev/null 2>&1 || useradd -m "$user"
# Out of wheel too, should Fedora's first-start setup have made the account before this ran.
gpasswd -d "$user" wheel >/dev/null 2>&1 || true
# No sudo without a password: an agent running as this user could take root with it. Root work goes through
# `wsl -d Whaybench -u root` from Windows. Earlier versions wrote the rule; a new run takes it away.
rm -f /etc/sudoers.d/whaybench
cat > /etc/wsl.conf <<EOF
[boot]
systemd=true

[user]
default=$user
EOF

say "packages"
# The input engine of the language is not chosen here: Whaybench offers it when it starts in a language that needs one
# (language.json of the add-on). The fonts cover every script, so names in other scripts show too, and the emoji the
# panels draw as icons (a file, a folder); default-fonts-other-sans has the scripts Noto Sans leaves out (Arabic,
# Hebrew, Thai, Indic). dbus-send (dbus-tools) is how Whaybench finds a running fcitx5 to join (start.rs): without it
# every start would start another one. awk is for the Codex and Whaybench installers, git for Claude Code's plugin
# marketplace; Fedora's WSL image has neither.
dnf install -y -q webkitgtk6.0 wl-clipboard firefox google-noto-sans-fonts google-noto-sans-cjk-fonts google-noto-color-emoji-fonts default-fonts-other-sans dbus-tools minisign curl gawk git-core procps-ng btrfs-progs xfsprogs
# fcitx5 without its weak dependencies: its setting program (Whaybench has its own) and the GTK and Qt input modules.
# The apps inside Whaybench type through the compositor (Wayland text-input).
dnf install -y -q --setopt=install_weak_deps=False fcitx5
# The Windows display language's locale, which Whaybench gives fcitx5 for its texts (start.rs). A language Fedora has
# no pack for is skipped; a pack there is that fails to install stops the setup, as any other package does.
case "$locale" in
    [a-z][a-z]_[A-Z][A-Z]|[a-z][a-z][a-z]_[A-Z][A-Z])
        pack="glibc-langpack-${locale%%_*}"
        if dnf repoquery -q "$pack" 2>/dev/null | grep -q .; then say "locale $locale.UTF-8"; dnf install -y -q "$pack"
        else say "locale $locale: no pack; skipped"; fi ;;
esac

# Windows programs (wsl.exe, powershell.exe) run through the WSLInterop registration. systemd-binfmt clears
# binfmt_misc when it starts again; Fedora's image, unlike Ubuntu's, does not register it back.
echo ':WSLInterop:M::MZ::/init:PF' > /etc/binfmt.d/WSLInterop.conf

# Whaybench in the Windows display language: the distribution runs in English, which Whaybench would take as chosen.
# A language Whaybench has no add-on for falls back to English by itself.
data="/home/$user/.local/share/whaybench"
mkdir -p "$data"
[ -f "$data/language.json" ] || printf '{"active":"%s"}\n' "$lang" > "$data/language.json"
chown -R "$user:$user" "/home/$user/.local"

# The shortcut starts Whaybench through wslg.exe, which skips the login: this sets what a login would. The user's
# runtime folder exists from boot on (linger), and WSLg links its display there.
loginctl enable-linger "$user" 2>/dev/null || true
# ~/work lives on the work disk (work.vhdx beside the distribution, made by the Windows installer). Attaching it to WSL
# needs administrator rights, so a start asks the scheduled task the installer registered, which only attaches it;
# whaybench-work-disk, run here as root through wsl.exe (no administrator rights), mounts it on ~/work. Without the
# disk after 20 seconds Whaybench still starts, on the distribution's own disk, and says so. Windows programs by
# name (Windows' PATH): Windows is not always on C:.
cat > /usr/local/bin/whaybench-start <<'EOF'
#!/bin/sh
uid=$(id -u)
export XDG_RUNTIME_DIR="/run/user/$uid" WAYLAND_DISPLAY=wayland-0 DISPLAY=:0
export DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus" PATH="$HOME/.local/bin:$PATH"
[ -e "$XDG_RUNTIME_DIR/wayland-0" ] || ln -sf /mnt/wslg/runtime-dir/wayland-0 "$XDG_RUNTIME_DIR/wayland-0"
if { [ -f /etc/whaybench/work-disk ] || [ -f /etc/whaybench/work-disk-expected ]; } && ! mountpoint -q "$HOME/work"; then
    log="$HOME/.local/share/whaybench/logs/work-disk.log"
    mkdir -p "$(dirname "$log")"
    echo "$(date '+%F %T') attaching" >> "$log"
    schtasks.exe /run /tn 'Whaybench\MountWork' >> "$log" 2>&1 || true
    i=0
    while [ $i -lt 20 ]; do
        # wsl.exe's own messages in UTF-8, as the rest of the log.
        WSL_UTF8=1 WSLENV="WSL_UTF8${WSLENV:+:$WSLENV}" wsl.exe -d "${WSL_DISTRO_NAME:-Whaybench}" -u root -- /usr/local/sbin/whaybench-work-disk mount >> "$log" 2>&1 && break
        sleep 1; i=$((i + 1))
    done
    mountpoint -q "$HOME/work" || { echo "$(date '+%F %T') not attached" >> "$log"; export WHAYBENCH_WORK_DISK=missing; }
fi
cd "$HOME"
exec whaybench "$@"
EOF
chmod 0755 /usr/local/bin/whaybench-start

# Root's part of the work disk, run through wsl.exe by the Windows installer and by whaybench-start:
#   expect  a work.vhdx exists: a start waits for it;
#   format  the work disk is the one blank disk of exactly 1 TiB (the size the installer gives work.vhdx) with no
#           partitions; nothing else is ever formatted, and a disk with the label is left as it is;
#   mount   the work disk on /mnt/whaybench-disk, its folder work bound on the default user's ~/work.
# btrfs for copies that share their data (reflink, for the conversations' working copies), XFS (built into WSL's
# kernel) when the btrfs module cannot be loaded. ~/work is the disk's folder work, not its root: the first copy goes
# to another name there and becomes work in one rename, so a copy that stops never passes for ~/work, and the
# person's files never share a folder with Whaybench's own.
install -d -m 0755 /etc/whaybench
cat > /usr/local/sbin/whaybench-work-disk <<'EOF'
#!/bin/sh
set -eu
label=wb-work-disk
size=1099511627776
disk=/mnt/whaybench-disk
user=$(sed -n 's/^default=//p' /etc/wsl.conf | head -n 1)
home=$(getent passwd "$user" | cut -d: -f6)
work="$home/work"
case "${1:-}" in
expect)
    touch /etc/whaybench/work-disk-expected ;;
format)
    if blkid -L "$label" >/dev/null 2>&1; then touch /etc/whaybench/work-disk; exit 0; fi
    found=""
    for dev in $(lsblk -dnpo NAME,TYPE | awk '$2 == "disk" { print $1 }'); do
        if [ "$(lsblk -dnbo SIZE "$dev")" = "$size" ] && [ -z "$(blkid -o value -s TYPE "$dev" 2>/dev/null)" ] && [ "$(lsblk -nro NAME "$dev" | wc -l)" -eq 1 ]; then found="$found $dev"; fi
    done
    set -- $found
    if [ $# -ne 1 ]; then echo "whaybench-work-disk: work disk not found (blank 1 TiB disks:${found:- none})" >&2; exit 3; fi
    if grep -qw btrfs /proc/filesystems || modprobe btrfs 2>/dev/null; then mkfs.btrfs -q -L "$label" "$1"; else mkfs.xfs -q -L "$label" "$1"; fi
    touch /etc/whaybench/work-disk ;;
mount)
    if mountpoint -q "$work"; then exit 0; fi
    # Never under a running Whaybench: its sessions would go on in the folder this covers or moves.
    if pgrep -u "$user" -x whaybench >/dev/null; then echo "whaybench-work-disk: not mounted (Whaybench running)" >&2; exit 4; fi
    dev=$(blkid -L "$label") || { echo "whaybench-work-disk: work disk not attached" >&2; exit 3; }
    type=$(blkid -o value -s TYPE "$dev")
    opts=noatime
    if [ "$type" = btrfs ]; then modprobe btrfs 2>/dev/null || true; opts="$opts,compress=zstd:1,discard=async"; fi
    install -d -m 0755 "$disk"
    mountpoint -q "$disk" || mount -o "$opts" "$dev" "$disk"
    if [ ! -d "$disk/work" ]; then
        # The first time: ~/work as it is (owner, mode, files) becomes the disk's work; the original stays aside.
        rm -rf "$disk/.work-copying"
        if [ -d "$work" ]; then
            cp -a --reflink=auto "$work" "$disk/.work-copying"
            mv -T "$disk/.work-copying" "$disk/work"
            mv "$work" "$home/work.before-disk"
            echo "whaybench-work-disk: previous ~/work moved to $home/work.before-disk"
        else
            install -d -o "$user" -g "$user" -m 0700 "$disk/work"
        fi
    elif [ -n "$(ls -A "$work" 2>/dev/null)" ]; then
        # Written while the disk was away: set aside, never deleted.
        aside="$home/work.unmounted-$(date +%Y%m%d-%H%M%S)"
        mv "$work" "$aside"
        echo "whaybench-work-disk: previous ~/work moved to $aside"
    fi
    [ -d "$work" ] || install -d -o "$user" -g "$user" -m 0700 "$work"
    mount --bind "$disk/work" "$work"
    # For the uninstaller: ~/work is on the work disk, not inside the Linux system.
    touch /etc/whaybench/work-on-disk ;;
*) echo "usage: whaybench-work-disk expect|format|mount" >&2; exit 2 ;;
esac
EOF
chmod 0700 /usr/local/sbin/whaybench-work-disk

say "Claude Code, Codex CLI and Whaybench"
su - "$user" -c 'command -v claude >/dev/null || curl -fsSL https://claude.ai/install.sh | bash'
# Codex CLI for "New conversation → Codex"; it asks to sign in the first time a Codex tab opens. Codex is a package
# of programs (codex-code-mode-host beside codex): a codex copied alone fails to start, so it is put aside and the
# official installer installs the whole package. Outside any Whaybench folder: CODEX_HOME is the person's own.
su - "$user" -c 'c=$(command -v codex || true); r=${c:+$(readlink -f "$c")}
if [ -z "$r" ] || [ ! -x "$(dirname "$r")/codex-code-mode-host" ]; then
    if [ -f "$HOME/.local/bin/codex" ] && [ ! -L "$HOME/.local/bin/codex" ]; then mv "$HOME/.local/bin/codex" "$HOME/.local/bin/codex.single-$(date +%s)"; fi
    curl -fsSL https://chatgpt.com/codex/install.sh | env -u CODEX_HOME sh
fi'
# Not piped from the network: install.sh is checked against the release key first (minisign, installed above), and
# with minisign it checks the package too.
key=RWTwDOf9rBKzxEepAVlbLTan2g/RQFDuCZeqmU4Q+dNwf+wF+3a2Sfia
dl=https://github.com/Whaybench/whaybench/releases/latest/download
su - "$user" -c "cd \$(mktemp -d) && curl -fsSLO $dl/install.sh && curl -fsSLO $dl/install.sh.minisig && minisign -Vq -m install.sh -P $key && PATH=\"\$HOME/.local/bin:\$PATH\" sh install.sh"

say "done"
