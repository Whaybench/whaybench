#!/bin/sh
# Prepares a fresh Ubuntu WSL distribution for Whaybench. Run as root inside it by the Windows installer:
#   wsl -d Whaybench -u root -- sh setup-wsl.sh USER [LANGUAGE]
# USER is made (no password: the Windows account guards the distribution) and becomes the default user. LANGUAGE (ko,
# ja, zh, vi or en) picks the input method engine. Everything comes from its official source: Ubuntu's archive,
# Mozilla's archive for Firefox (Ubuntu's own is a snap, which cannot reach Whaybench's display in WSL), the
# Whaybench release, Anthropic's Claude Code installer and OpenAI's Codex CLI installer (a standalone program, no Node). Running it again finishes what a failed run left.
set -eu

user="${1:?usage: setup-wsl.sh USER [LANGUAGE]}"
lang="${2:-en}"
say() { printf 'whaybench-setup: %s\n' "$*"; }
export DEBIAN_FRONTEND=noninteractive

say "account $user"
id "$user" >/dev/null 2>&1 || adduser --disabled-password --gecos "" "$user"
# No sudo without a password: an agent running as this user could take root with it. Root work goes through
# `wsl -d Whaybench -u root` from Windows. Earlier versions wrote the rule; a new run takes it away.
rm -f /etc/sudoers.d/whaybench
cat > /etc/wsl.conf <<EOF
[boot]
systemd=true

[user]
default=$user
EOF

say "Firefox from Mozilla"
apt-get update -q
apt-get install -y -q ca-certificates curl gpg
install -d -m 0755 /etc/apt/keyrings
curl -fsSL https://packages.mozilla.org/apt/repo-signing-key.gpg -o /etc/apt/keyrings/packages.mozilla.org.asc
echo "deb [signed-by=/etc/apt/keyrings/packages.mozilla.org.asc] https://packages.mozilla.org/apt mozilla main" > /etc/apt/sources.list.d/mozilla.list
# Mozilla's Firefox over Ubuntu's snap package of the same name.
printf 'Package: *\nPin: origin packages.mozilla.org\nPin-Priority: 1000\n' > /etc/apt/preferences.d/mozilla

case "$lang" in
    ko) engine=fcitx5-hangul; im=hangul; fonts=fonts-noto-cjk ;;
    ja) engine=fcitx5-mozc; im=mozc; fonts=fonts-noto-cjk ;;
    zh) engine=fcitx5-chinese-addons; im=pinyin; fonts=fonts-noto-cjk ;;
    vi) engine=fcitx5-unikey; im=unikey; fonts=fonts-noto-core ;;
    *) engine=""; im=""; fonts=fonts-noto-core ;;
esac
say "packages"
apt-get update -q
# shellcheck disable=SC2086 # engine is one word or none
apt-get install -y -q libgtk-4-1 libwebkitgtk-6.0-4 wl-clipboard firefox fcitx5 $engine "$fonts" dbus-user-session

# The input method inside Whaybench: the engine of the language, after the plain keyboard.
if [ -n "$im" ]; then
    conf="/home/$user/.config/fcitx5"
    mkdir -p "$conf"
    printf '[Groups/0]\nName=Default\nDefault Layout=us\nDefaultIM=%s\n\n[Groups/0/Items/0]\nName=keyboard-us\nLayout=\n\n[Groups/0/Items/1]\nName=%s\nLayout=\n\n[GroupOrder]\n0=Default\n' "$im" "$im" > "$conf/profile"
    printf '[Hotkey/TriggerKeys]\n0=Hangul\n1=Control+space\n2=Shift+space\n' > "$conf/config"
    chown -R "$user:$user" "/home/$user/.config"
fi

# Whaybench in the Windows display language: WSL's Ubuntu runs in C.UTF-8, which Whaybench would take for English.
# A language Whaybench has no add-on for falls back to English by itself.
data="/home/$user/.local/share/whaybench"
mkdir -p "$data"
[ -f "$data/language.json" ] || printf '{"active":"%s"}\n' "$lang" > "$data/language.json"
chown -R "$user:$user" "/home/$user/.local"

# The shortcut starts Whaybench through wslg.exe, which skips the login: this sets what a login would. The user's
# runtime folder exists from boot on (linger), and WSLg links its display there.
loginctl enable-linger "$user" 2>/dev/null || true
cat > /usr/local/bin/whaybench-start <<'EOF'
#!/bin/sh
uid=$(id -u)
export XDG_RUNTIME_DIR="/run/user/$uid" WAYLAND_DISPLAY=wayland-0 DISPLAY=:0
export DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus" PATH="$HOME/.local/bin:$PATH"
[ -e "$XDG_RUNTIME_DIR/wayland-0" ] || ln -sf /mnt/wslg/runtime-dir/wayland-0 "$XDG_RUNTIME_DIR/wayland-0"
cd "$HOME"
exec whaybench "$@"
EOF
chmod 0755 /usr/local/bin/whaybench-start

say "Claude Code, Codex CLI and Whaybench"
su - "$user" -c 'command -v claude >/dev/null || curl -fsSL https://claude.ai/install.sh | bash'
# Codex CLI for "New conversation · Codex"; it asks to sign in the first time a Codex tab opens. Codex is a package
# of programs (codex-code-mode-host beside codex): a codex copied alone fails to start, so it is put aside and the
# official installer installs the whole package. Outside any Whaybench folder: CODEX_HOME is the person's own.
su - "$user" -c 'c=$(command -v codex || true); r=${c:+$(readlink -f "$c")}
if [ -z "$r" ] || [ ! -x "$(dirname "$r")/codex-code-mode-host" ]; then
    if [ -f "$HOME/.local/bin/codex" ] && [ ! -L "$HOME/.local/bin/codex" ]; then mv "$HOME/.local/bin/codex" "$HOME/.local/bin/codex.single-$(date +%s)"; fi
    curl -fsSL https://chatgpt.com/codex/install.sh | env -u CODEX_HOME sh
fi'
# Not piped from the network: install.sh is checked against the release key first, and with minisign installed it
# checks the package too.
apt-get install -y -q minisign
key=RWTwDOf9rBKzxEepAVlbLTan2g/RQFDuCZeqmU4Q+dNwf+wF+3a2Sfia
dl=https://github.com/Whaybench/whaybench/releases/latest/download
su - "$user" -c "cd \$(mktemp -d) && curl -fsSLO $dl/install.sh && curl -fsSLO $dl/install.sh.minisig && minisign -Vq -m install.sh -P $key && PATH=\"\$HOME/.local/bin:\$PATH\" sh install.sh"

say "done"
