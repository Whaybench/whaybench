<h1 align="center">Whaybench</h1>

<p align="center"><b>Keep the thread of long conversations with your coding agent.</b><br>
A Linux desktop workspace for Claude Code (Codex CLI experimental): the conversation, its map, your sessions,<br>
the answers and everything the agent shows, side by side in one window.</p>

<p align="center">
  <img src="docs/images/whaybench.png" alt="Whaybench: a Claude Code conversation in a terminal panel, with its conversation map, sessions, answer, input and view panels around it" width="900">
</p>

> Whaybench is an independent project. It is not affiliated with or endorsed by Anthropic or OpenAI.

## Why

Working with a coding agent in a terminal is fast until the conversation gets long.

- Decisions made an hour ago scroll out of sight, and after the context is compacted the agent loses them too.
- With several sessions open you lose track of which one was doing what, and a session that moved to the
  background or got a new number is hard to find again.
- What the agent shows you, a page, a diagram, a PDF, a file it changed, opens somewhere else on the desktop.
- The last answer is buried between tool calls.

Whaybench puts all of this on one surface and keeps it in step with the conversation.

## What it does

**A conversation map the agent keeps for you.** In every turn the agent updates a tree of the conversation:
topics, open questions, settled decisions written as "question → conclusion (reason)", and the parts of what is
being built with their state. You see where the conversation is and what was decided without scrolling back.
Open a node and its related answers come up: the replies from the turns where that topic was discussed.
When the agent reads a file, the decisions about it come along, so they survive a compacted context.

**One memory across sessions.** Every change to every map is kept as an event, so a session is told what other
sessions decided about the same problem, and the notifications panel gathers what needs you: due dates, the nodes you
marked, problems another session settled, what you put on hold, and what you deleted, which you can restore.

**Sessions that do not get lost.** Every conversation is listed with a name taken from its map, its state
(working, waiting for you, done) and a search over names, questions and answers. Close the window and the
tabs come back as they were; a session moved to the background or to a new number keeps its tab, map and history.

**Everything the agent shows, in one place.** Pages, files, PDFs, pictures and a real Firefox tab open in the view
panel next to the conversation, kept per session. Links clicked in the terminal open there too.

**The answer, the input and the files.** The last answer in a panel of its own with the earlier ones a click away,
an input box that sends to the chosen session, and the files a session read or changed, grouped by map node.

**A free layout.** Panels are moved by their title bar, resized by their borders, dropped on each other as tabs.
Panels that share a border move together. Each panel zooms with Ctrl+wheel and keeps it.

**Projects.** A project map built from conversation nodes, and documents made from templates you provide
(Markdown, Word or a spreadsheet). Only the agent writes them; you ask for changes.

**Yours to change.** Themes, languages, map styles and document templates are plain folders. English and
Korean are included.

**Local and private.** No server, no account, no telemetry. Whaybench uses your own Claude Code or Codex
login and keeps everything in one folder on your computer.

## Install

Whaybench installs on Windows and on Linux, both in a few steps.

- **Windows** 11, or 10 21H2 or newer (64-bit): one installer sets up everything, Claude Code and Codex CLI
  included. See [Windows](#windows).
- **Linux** (x86_64) in a Wayland desktop session: Fedora 39, Ubuntu 24.04, Debian 13 or newer, or another
  distribution with GTK 4.12 or newer (AlmaLinux and Rocky Linux 10 with EPEL). You need Claude Code or Codex CLI,
  installed and signed in. The steps follow.

### 1. System packages

Fedora:

    sudo dnf install webkitgtk6.0 wl-clipboard firefox fcitx5

Ubuntu and Debian:

    sudo apt install libwebkitgtk-6.0-4 wl-clipboard firefox fcitx5

Arch:

    sudo pacman -S webkitgtk-6.0 wl-clipboard firefox fcitx5

AlmaLinux and Rocky Linux 10:

    sudo dnf install epel-release
    sudo dnf install webkitgtk6.0 wl-clipboard firefox
    sudo dnf upgrade "gstreamer1*"   # webkitgtk6.0 from EPEL needs GStreamer 1.26

fcitx5 is the input method inside Whaybench, the same on every desktop. Add the engine for your language, for
example `fcitx5-hangul` (Korean), `fcitx5-mozc` (Japanese), `fcitx5-chinese-addons` (Chinese), `fcitx5-unikey`
(Vietnamese) or `fcitx5-m17n` (many other scripts). AlmaLinux and Rocky Linux 10 do not package fcitx5; there
Whaybench takes the desktop's input method instead.

The release brings its own Alacritty for the terminal tabs. LibreOffice is optional, for Word and spreadsheet
templates.

### 2. Whaybench

    curl -fsSL https://github.com/whaybench/whaybench/releases/latest/download/install.sh | sh

This puts the programs in `~/.local/lib/whaybench` and the `whaybench` command in `~/.local/bin`, checks the system
packages, and, when Claude Code is installed, installs its Whaybench plugin. To do that last step yourself:

    claude plugin marketplace add whaybench/whaybench
    claude plugin install context-map@whaybench

Codex CLI support is experimental: some parts may not work yet. With Codex CLI there is nothing more to install. Open a Codex conversation from the session menu (New conversation · Codex), then open `/hooks` in the Codex tab once
and trust the Whaybench hooks. See [docs/codex.md](docs/codex.md).

### 3. Run

    whaybench

A conversation tab opens with the agent; ask it something and the map starts to grow. Settings → Language
switches the interface language.

Context and plan usage (5-hour and weekly, with when they reset) show in Claude Code's status line and under the
notifications panel: Whaybench's conversation tabs start Claude Code with its status line, without changing your
settings. If you set a status line of your own in `~/.claude/settings.json`, that one is kept; pass it the same
input to keep the panel's numbers: `echo "$input" | ~/.local/bin/whaybench statusline`.

Releases are signed with [minisign](https://jedisct1.github.io/minisign/). Whaybench updates itself only with files
that check against this key, and `install.sh` checks the package with it when `minisign` is installed:

    RWTwDOf9rBKzxEepAVlbLTan2g/RQFDuCZeqmU4Q+dNwf+wF+3a2Sfia

To check a download yourself: `minisign -Vm whaybench-x86_64-linux.tar.gz -P <the key above>`.

To update, run the install command again; it updates the Claude Code plugin too. Whaybench may keep running: the
status line then says a new version is waiting, and Whaybench → Restart starts it. Each version is kept in a folder
of its own and the one before stays: if a new version misbehaves, go back with
`curl -fsSL https://github.com/Whaybench/whaybench/releases/latest/download/install.sh | sh -s -- --rollback` and restart.
To remove Whaybench, delete `~/.local/lib/whaybench`, `~/.local/lib/whaybench-versions` and `~/.local/bin/whaybench`;
your data stays (see below).

### Windows

On a 64-bit PC with Windows 11, or Windows 10 21H2 or newer, download **WhaybenchSetup.exe** from the
[latest release](https://github.com/whaybench/whaybench/releases/latest) and run it. The installer is not signed, so
Windows shows a warning first; it installs only once you confirm it (**More info**, then **Run anyway**).

It sets everything up in WSL, the Linux that comes with Windows: WSL itself (Windows asks for administrator rights,
and may need one restart, after which the setup goes on by itself), an Ubuntu of its own named Whaybench (your other
WSL distributions are not touched), Firefox, the input method for your Windows display language, Claude Code, Codex CLI and
Whaybench. This takes a while. Then start Whaybench from the Start menu or the desktop, and the first time sign in to
Claude in the browser that opens. If the setup stops, run the installer again: it goes on from where it stopped.

Whaybench starts Claude Code and Codex CLI as they are: their own permission prompts and sandbox settings apply.
Programs in WSL can start Windows programs and change your Windows files with your rights, so what an agent runs
there can reach your Windows system too. In the words of [Claude Code's security guide](https://code.claude.com/docs/en/security):

> You're responsible for reviewing proposed code and commands for safety before approval.

> While these protections significantly reduce risk, no system is completely immune to all attacks. Always maintain
> good security practices when working with any AI tool.

For Codex, see [its Windows sandbox guide](https://learn.chatgpt.com/docs/windows/windows-sandbox). To install
something system-wide, open PowerShell and run `wsl -d Whaybench -u root`.

Whaybench updates itself from its menu, as on Linux. To remove it, uninstall Whaybench in Windows Settings → Apps; it
asks whether to delete the Whaybench Linux system too, with everything kept in it.

### Build from source

You need Rust 1.87 or newer ([rustup](https://rustup.rs)), Git and the development packages:
`gtk4-devel webkitgtk6.0-devel` on Fedora, `libgtk-4-dev libwebkitgtk-6.0-dev` on Ubuntu and Debian, and Alacritty.

    git clone https://github.com/whaybench/whaybench
    cd whaybench
    tools/fetch-smithay.sh
    cargo build --release
    target/release/whaybench

## Where your data is

Everything is in one folder, the first of `--data-dir DIR`, `WHAYBENCH_DATA_DIR`, `$XDG_DATA_HOME/whaybench`
and `~/.local/share/whaybench`. Copy it to move your setup to another account. It holds the sign-ins of the view panel's browser, so only your
account can read it and you should not share a copy. Its `cache` folder can be deleted at any time. Conversations are kept without limit; to drop old ones, put `"keep_days"` (days since a conversation was last
active) or `"keep_mb"` (the size all of them may take) in its `settings.json`, for example `{"keep_days": 365}`.
A whole conversation goes at once, at the next start; saved maps in `maps` stay, and so do the map's nodes: they
are kept once for all conversations in `memory/memory.db`, where `"keep_days"` drops only the record of how they changed. Claude Code deletes its own
conversations after 30 days unless its `cleanupPeriodDays` setting says otherwise; Whaybench then saves the map
to `maps` as `auto-map-…`. Claude Code and Codex keep their own conversations where they always do.
Map nodes you delete, on the map or in the notifications panel, wait in that panel's Deleted section, where they can be
restored as they were; after 180 days they go for good at the next start (`"trash_days"` in `settings.json`, `0` to
delete them for good only by hand). Deleting a node for good, there or from a line of the History tab, asks once and
takes the node with its whole history; that is how to remove something you do not want kept.

## Make it yours

| Add-on | Folder in the data directory | Guide |
|---|---|---|
| Theme | `addons/themes/` | [addons/themes/README.md](addons/themes/README.md) |
| Language | `addons/languages/` | [addons/languages/README.md](addons/languages/README.md) |
| Map style | `addons/map-styles/` | [addons/map-styles/README.md](addons/map-styles/README.md) |
| Project document template | `addons/project-docs/` | [addons/project-docs/README.md](addons/project-docs/README.md), with `wbs` as an example |

## How it works

Whaybench is a nested Wayland compositor written in Rust with [Smithay](https://github.com/Smithay/smithay). Each
panel is its own app in its own area: GTK 4 apps for the product panels, Alacritty for the terminals, WebKit and
Firefox for views. The agent runs unchanged in the terminal; a small relay between Alacritty and the agent knows
which session a tab holds. Both agents connect the same way: their standard hooks call `whaybench hook`, and the
conversation map tool is the MCP server `whaybench mcp` (for Claude Code through the `context-map` plugin, which
holds only that configuration). So the files the panels read are the same whichever agent wrote them, and a
session of one agent sees what the other decided.

## FAQ

**macOS or Windows?** Windows, through WSL: see [Windows](#windows). macOS: not now; Whaybench needs a Linux Wayland
desktop.

**Does it call the AI itself?** No. Your agent does the work in its terminal as usual. Whaybench only reads and
shows what the agent records, and sends text to a session when you ask it to.

**Does it change my Claude Code or Codex setup?** For Claude Code it adds one plugin. For Codex it uses a folder of
its own, so Codex outside Whaybench is unchanged.

## Contributing

Bug reports and ideas are welcome in [Issues](https://github.com/whaybench/whaybench/issues). For code, see
[AGENTS.md](AGENTS.md) for building, testing and the rules the code keeps.

## Support

Whaybench is made by one developer in their own time. If it helps you, you can buy me a coffee on
[Ko-fi](https://ko-fi.com/whaybench).

## License

[Apache License 2.0](LICENSE). Smithay's MIT notice is in [NOTICE](NOTICE).
