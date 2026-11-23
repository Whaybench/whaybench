<h1 align="center">Whaybench</h1>

<p align="center"><b>Keep the thread of long conversations with your coding agent.</b><br>
Claude Code (Codex CLI experimental) with its conversation map, sessions, answers and everything it shows,<br>
side by side in one window.</p>

<p align="center">
  <img src="docs/images/whaybench.png" alt="Whaybench: the view panel in the middle showing what the agent made, with the conversation map, input, conversation and answer panels around it" width="900">
</p>

> Whaybench is an independent project. It is not affiliated with or endorsed by Anthropic or OpenAI.

## Install

### Windows

1. Download **[WhaybenchSetup.exe](https://github.com/Whaybench/whaybench/releases/latest/download/WhaybenchSetup.exe)**.
2. Run it. Windows warns about an unknown app: click **More info**, then **Run anyway**.
3. Wait while it installs everything. Windows may restart once; the setup goes on by itself.
4. Start **Whaybench** from the Start menu and sign in to Claude in the browser that opens in the middle of its window.

Needs Windows 11, or Windows 10 21H2 or newer (64-bit). The setup puts a Linux of its own in WSL with Claude Code and
Codex CLI; your other WSL systems are not touched. If it stops, run it again and it goes on where it stopped. If it
stops with an error, [docs/windows.md](docs/windows.md) shows how to fix it step by step.

Programs in WSL can reach your Windows files with your rights, so check what an agent wants to run before you allow
it.

### Linux

1. Install Whaybench:

   ```sh
   curl -fsSL https://github.com/Whaybench/whaybench/releases/latest/download/install.sh | sh
   ```

   On Fedora (Silverblue and Kinoite too), Ubuntu, Debian, Arch, openSUSE, AlmaLinux and Rocky Linux it shows the command for the system packages it needs
   (WebKitGTK, wl-clipboard, Firefox, fcitx5, an emoji font) and runs it with `sudo` once you answer `y`.

2. Start it:

   ```sh
   whaybench
   ```

Needs an x86_64 PC with a Wayland desktop, and [Claude Code](https://code.claude.com/docs) installed and signed in.
To type in Korean, Japanese, Chinese or another language, also install its fcitx5 engine (for Korean,
`fcitx5-hangul`).

### Update and remove

Whaybench updates itself and says so in its status line; **Whaybench → Restart** starts the new version.

- Windows: remove it in **Settings → Apps**. Your projects stay in `%LOCALAPPDATA%\Whaybench\work.vhdx`; see
  [docs/windows.md](docs/windows.md#removing-everything) to remove them too.
- Linux: delete `~/.local/lib/whaybench`, `~/.local/lib/whaybench-versions` and `~/.local/bin/whaybench`. Remove the
  Claude Code plugin with `claude plugin uninstall context-map@whaybench`.

Your data stays in its folder (see below).

## What it does

- **Conversation map.** The agent keeps a tree of the conversation: topics, open questions, and decisions as
  "question → conclusion (reason)". Open a node to see the answers where it was discussed.
- **One memory across sessions.** A session is told what other sessions decided about the same problem. The
  notifications panel gathers what needs you.
- **Sessions.** Every conversation is listed with a name, its state and a search. Tabs come back after a restart.
- **View panel.** Pages, files, PDFs, pictures and a real Firefox tab open beside the conversation.
- **Answer, input and files.** The last answer in its own panel, an input box for the chosen session, and the files
  each session touched.
- **Layout and look.** Move, resize and stack panels as tabs. Themes set colours, light and depth, see-through panels
  and glass.
- **Projects.** A project map built from conversation nodes, and documents from your templates.
- **Briefsafe projects.** A project in a folder of its own, its agents in a container that sees only that folder, its
  data encrypted while you are logged out, and snapshots you can go back to (see below).
- **Local.** No server, no account, no telemetry.

## Briefsafe projects

A Briefsafe project keeps one piece of work apart from everything else on your computer. Every new project is one:
press **New project** in the **Projects** panel (or ask your agent). It goes in `~/work/<name>`. Once, before its
first conversation, press **Claude token for projects** there: a terminal in the view signs you in and keeps the token
its conversations use. Then **Log in** and start a conversation.

- **Its own folder.** The project lives in one folder, a btrfs subvolume where the disk allows it. A snapshot is
  taken when it is made, and again whenever you press **Snapshot**. The 30 latest are kept, and **Restore** puts the
  whole folder back as it was.
- **Its own agents.** Its conversations run in a rootless podman container. The container sees the project folder,
  your system's programs (read only) and nothing else of your home folder. Claude Code and Codex there have their own
  settings and history.
- **Encrypted while logged out** (`--encrypt`). The project's documents, map, history, memory and conversations are
  kept with gocryptfs, under a key held in your keyring. When it is made, the view panel shows a recovery key once:
  keep it somewhere safe. Without it, a lost keyring means a lost project.
- **Kept apart in Whaybench.** A project's text, map and pages stay in its own panels and conversations. Your other
  conversations can give a project things, but nothing of the project comes back out to them.

What it does not protect: while a project is logged in, other programs running as your user (an agent outside the
project included) can still read its files. Log out when you are done. Pictures of the screen taken while a project
is open are removed after ten minutes.

**What it needs on Linux.** The Windows setup brings all of this. On Linux, install `podman` for the container and
`gocryptfs` (with FUSE) for encryption. Without podman a project still gets its own folder and snapshots, but its
agents run as you, outside a container.

**Removing snapshots.** Whaybench deletes snapshots past the 30 kept as you, not as root. That needs the btrfs mount
option `user_subvol_rm_allowed` on the disk that holds your projects. The Windows setup sets it. On Linux, add it once
to that file system's line in `/etc/fstab`, then remount:

```sh
# Example: the line for /home, with the option added at the end of its options
UUID=...  /home  btrfs  subvol=home,compress=zstd:1,user_subvol_rm_allowed  0 0
sudo mount -o remount /home
findmnt -no OPTIONS /home   # user_subvol_rm_allowed should be listed
```

Not sure which line it is? Ask your agent to do it with you. Without the option, Whaybench tells you which snapshots
it could not remove.

**A project from before.** A project made before Briefsafe keeps working as it did. To move it into a Briefsafe
project, ask your agent (it runs `whaybench-project convert ID --folder DIR`). Its records move into the new folder.
The conversations that worked on it stay where they were.

## Your data

Everything is in `~/.local/share/whaybench`. Copy that folder to move your setup. It holds the sign-ins of the view
panel's browser, so keep it to yourself. Its `cache` folder can be deleted at any time.

Conversations are kept until you choose otherwise: `{"keep_days": 365}` in its `settings.json` drops those not used
for a year.

## Make it yours

Themes, languages, map styles and document templates are folders. English and Korean are included.

| Add-on | Folder in your data | Guide |
|---|---|---|
| Theme | `addons/themes/` | [addons/themes/README.md](addons/themes/README.md) |
| Language | `addons/languages/` | [addons/languages/README.md](addons/languages/README.md) |
| Map style | `addons/map-styles/` | [addons/map-styles/README.md](addons/map-styles/README.md) |
| Document template | `addons/project-docs/` | [addons/project-docs/README.md](addons/project-docs/README.md) |

## Codex CLI

Experimental. Open a Codex conversation from the session menu (**New conversation → Codex**). See
[docs/codex.md](docs/codex.md).

## Build from source

You need Rust ([rustup](https://rustup.rs)), Git, Alacritty and the GTK 4 and WebKitGTK 6 development packages
(`gtk4-devel webkitgtk6.0-devel` on Fedora, `libgtk-4-dev libwebkitgtk-6.0-dev` on Ubuntu and Debian).

    git clone https://github.com/Whaybench/whaybench
    cd whaybench
    tools/fetch-smithay.sh
    cargo build --release
    target/release/whaybench

## How it works

Whaybench is a Wayland compositor written in Rust with [Smithay](https://github.com/Smithay/smithay). Each panel is
an app of its own: GTK 4 panels, Alacritty terminals, WebKit and Firefox views. The agent runs unchanged in the
terminal; its standard hooks call `whaybench hook`, and the map tool is the MCP server `whaybench mcp`.

Releases are signed with [minisign](https://jedisct1.github.io/minisign/), and Whaybench updates only with files that
check against this key: `RWTwDOf9rBKzxEepAVlbLTan2g/RQFDuCZeqmU4Q+dNwf+wF+3a2Sfia`

## FAQ

**macOS?** Not now: Whaybench needs Linux, on its own or in WSL on Windows.

**Does it call the AI itself?** No. Your agent works in its terminal as usual; Whaybench shows what it records.

**Does it change my Claude Code or Codex setup?** It adds one plugin to Claude Code. Codex gets a folder of its own.

## Contributing

Bug reports and ideas are welcome in [Issues](https://github.com/Whaybench/whaybench/issues). For code, see
[AGENTS.md](AGENTS.md).

## Support

Whaybench is made by one developer in their own time. If it helps you, you can buy me a coffee on
[Ko-fi](https://ko-fi.com/whaybench).

## License

[Apache License 2.0](LICENSE). Smithay's MIT notice is in [NOTICE](NOTICE).
