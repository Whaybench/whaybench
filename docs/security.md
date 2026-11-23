# Security

What Whaybench trusts, what it keeps on your computer, what goes over the network, and how a Briefsafe project is kept
apart. To report a vulnerability, see the [security policy](../SECURITY.md).

## What Whaybench trusts

Whaybench runs on your computer within the boundary of your user account, and so do the agents in its tabs. Each agent
follows its own permission settings; Whaybench starts Codex in its `workspace-write` sandbox. Check what an agent wants
to run before you allow it.

Whaybench keeps its data readable by your account only. It marks text that one conversation sends to another, and does
not deliver such text when it starts with `/` or `!`.

Pages are shown by WebKitGTK and Firefox. Whaybench does not check pages, files or links for harmful content.

## What goes over the network

Whaybench has no server, no account and no telemetry.

| What | When |
|---|---|
| Release check | An installed release asks GitHub for the latest one. Nothing of yours is in the request |
| Release download | From GitHub, by itself unless you clear **Download new releases automatically** in **Settings → Preferences…** |
| Your agent | As when you use it by itself: its model, and the tools you allow |
| A map style that needs more than the map holds | A small Claude model fills it in once, from the map and the recent conversation. Not for Briefsafe projects |
| Pages you open | The site in question, and what a page shown in **View** loads from the web |
| A project's first login | Podman downloads the container image if it is not there |

## What is kept on your computer

Your data folder, usually `~/.local/share/whaybench`, holds conversations' records, answers and maps, which files the
agents touched, the browser's profile with its sign-ins, and logs, which can name folders and conversations. Your files
stay where they are, and a Briefsafe project keeps its data in its own folder. The full list is in [Settings and
data](reference/settings-and-data.md#find-your-data). Keep the folder and its copies to yourself.

Agents keep their own transcripts in their own folders, such as `~/.claude`.

To keep less, set **Conversation records kept (days)** or **(MB)** in **Settings → Preferences…**. Map nodes, the
**Answer** panel's history and saved maps are kept.

## How a Briefsafe project is kept apart

The aim is that ideas, personal matters and secrets of a project do not come out of it by the way.

| What | How |
|---|---|
| Its agents | Run in a rootless podman container with the project folder. Programs, toolchains and the agents' plugins are there read only; the rest of your home folder, your other projects and your keyring are not |
| Its conversations, map, memory and documents | Stay in the project. Other conversations do not get them, and **View** does not read its pages out to them |
| What goes in | From an ordinary map, a branch you send with **Reflect in project**. It waits in memory, where the system allows it, until the project's tab takes it |
| On disk | Encrypted with gocryptfs while logged out, where the project has a vault |
| Screenshots by Whaybench | Removed ten minutes after they were taken while a project was open. One that cannot be removed is told on the status line and tried again, also after a restart |

| State | Records, map, documents | Your files in the folder |
|---|---|---|
| Logged out, with a vault | Encrypted | Plain files |
| Logged in | Readable by programs running as you | Plain files |
| Without a vault | Plain on disk; hidden in Whaybench while logged out | Plain files |

Its limits:

- The agents use their model and the network. Whaybench leaves the container's network to Podman.
- Use disk encryption for the files you work on. The project's name, folder and whether it has a vault are readable too.
- Quitting Whaybench does not lock a project. Log out, and check that it shows **locked**.
- Without podman or an image for your system, a project's agents run as you, and the project panel says so. A project
  with a vault does not open there.

The guide is [Briefsafe projects](guide/briefsafe.md).

## Keys and sign-ins

| What | Where | If it is lost |
|---|---|---|
| A vault's key | Your keyring | Open the project with its recovery key |
| Recovery key | Where you put it; shown once | With the key in the keyring too: the vault stays locked |
| Claude Code token for projects | Your keyring, one for all projects | Add it again with **Inner login → Claude** |
| Codex sign-in of a project with a container | In the project's own data | Sign in again with **Inner login → Codex** |

Anyone with a recovery key and a copy of the project folder can read the vault. From a terminal, `whaybench token check`
says whether a Claude Code token is kept, and `whaybench token clear` removes it; tabs already running keep theirs until
they are closed. Revoke a token on claude.ai.

Ordinary Codex tabs share the sign-in of your own Codex.

## Add-ons

A bundled agent add-on runs as it came. For any other, or one that was changed, the tab first shows the command, the
settings files it merges or writes and the instructions it gives the agent. Nothing of it is set up until you press
<kbd>y</kbd>. The approval holds for exactly those files.

A map style's instructions are shown before they go to the agent, and again when they change. Codex asks you to trust
Whaybench's hooks itself, with `/hooks`.

## Releases

`install.sh`, `whaybench-x86_64-linux.tar.gz`, `setup-wsl.sh` and `WhaybenchSetup.exe` are signed with minisign. With
the file and its `.minisig` side by side:

```sh
minisign -Vm install.sh -P RWTwDOf9rBKzxEepAVlbLTan2g/RQFDuCZeqmU4Q+dNwf+wF+3a2Sfia
```

| Who checks | What |
|---|---|
| Built-in updater | The signatures of `install.sh` and the package, their version and the checksum, before an update is set aside |
| `install.sh` | The package's checksum always; its signature when `minisign` is installed |
| `curl … \| sh` | `install.sh` itself is not checked. [Check it first](install.md#check-a-release-signature) if you want that |
| Windows setup | `install.sh`, inside WSL, before it runs it |
