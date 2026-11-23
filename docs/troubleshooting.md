# Troubleshooting

Find the symptom you see, try the checks in that section, and repeat the action that failed. If the problem stays,
use the logs to see what happened and share the relevant lines in a bug report.

## Start Whaybench when no window opens

On Linux, open a terminal in your Wayland desktop and run Whaybench as your normal user. Starting from a terminal
lets you read the error.

```sh
whaybench
```

- If the shell cannot find `whaybench`, try the installed path below. If that works, add `~/.local/bin` to your `PATH`.
- If the error mentions `XDG_RUNTIME_DIR` or a missing Wayland socket, sign in to a Wayland desktop session and retry
  from a terminal there.
- If the error says `cannot use` and names your data directory, check that your account can write there. Retry with
  the same data directory so you can see your existing conversations.

```sh
~/.local/bin/whaybench
```

Check that the window opens. If it still fails, read the terminal output and the newest `whaybench.log`, if one exists.
For a Windows shortcut that fails, see [Check Windows setup and the work disk](#check-windows-setup-and-the-work-disk).

## Fix black, slow or missing panels

If a panel is missing, reopen it from **Panels**. If the first **View** is gone, restart Whaybench. If panels are in the
wrong places, choose **Panels → Reset layout**. This restores their positions and grouping and closes extra views,
branch maps and imported maps.

If a panel is black or slow to redraw, choose **Whaybench → Restart** and repeat the action that shows the problem.
Check `whaybench.log` for `frame not drawn`. Whaybench skips a failed frame and tries again. On Linux outside WSL,
`apps on the CPU` means Whaybench falls back to software rendering for its panels, which can make redraws slower.

If you see **Panel crashed repeatedly · logged**, Whaybench leaves the panel closed after repeated crashes. Check the
panel log and the terminal you start Whaybench from. If either reports a missing GTK or WebKitGTK library, install the
required packages from [Install](install.md). Reopen the panel and check that it draws and responds when you use it.

## Restore typing in another language

Open **Settings → Preferences…** and check **Input method**. Restart Whaybench after changing it.

| Option | When to use it |
|---|---|
| **Built-in (fcitx5)** | Use this on Windows. On Linux it is the default when `fcitx5` is installed. |
| **The desktop's** | Use this on Linux if you already type with your Wayland desktop's input method. |

With fcitx5, install an engine for your language, such as `fcitx5-mozc` for Japanese. If your language add-on offers
support when you select it under **Settings → Language**, choose **Install**. If the install fails, check the log.
If Whaybench asks you to install the package by hand, use your system package manager.

Open **Settings → Input…**. If you see **fcitx5 not running · Input settings unavailable**, check that `fcitx5` is
installed and restart with **Built-in (fcitx5)** selected. If the engine is installed but missing from **Input
methods**, select it in the list of available engines and click **Add**. Use the shortcut under **Switch keys** and type
a few characters in **Input** to check that they compose as expected.

## Fill an empty conversation map

Open **Panels → Conversation map** and select the conversation you want in **Sessions**. If you use a display style
that filters the map, choose **Display style → Default** to check the tree.

The agent adds nodes when a topic starts or a question is settled. A short exchange may leave the map empty. Ask the
agent to add a test topic with `context_map`, then look for the node after the turn. If the tool reports
**No session is known for this conversation yet: its hooks have not run.**, check the agent connection.

For Claude Code, install or update the `context-map` plugin in a terminal. If the marketplace is already added,
continue with the other commands.

```sh
claude plugin marketplace add Whaybench/whaybench
claude plugin marketplace update whaybench
claude plugin install context-map@whaybench
claude plugin update context-map@whaybench
```

Open a new Claude Code conversation and try the test topic again. For Codex, follow
[Trust the Codex hooks](#trust-the-codex-hooks).

## Find an agent that does not open

If you see **Conversation tab not opened**, read the reason after it. If Whaybench cannot find the agent, check for
the command in the terminal you use to start Whaybench. Run the line for the agent you want.

```sh
command -v claude
command -v codex
```

If nothing is printed, look for `~/.local/bin/claude` or `~/.local/bin/codex`, which Whaybench also tries. Install the
missing CLI or add its directory to your `PATH`. Start Whaybench from the terminal that can find it. If the agent itself
is missing from **Sessions → New conversation**, check that its add-on is installed in your Whaybench data directory. An
unreadable add-on does not appear in the menu. See [Agents and add-ons](guide/agents-and-add-ons.md).

Open a new tab for the agent, sign in if it asks, and send a short prompt. Check that the tab stays open and the
agent replies.

## Send a request that stays unsent

**Request not sent · No conversation tab open** can appear when the session has stopped or is running without a tab.
Click the conversation in **Sessions** to open its tab, then retry the request. If Whaybench says **Working in another
terminal · Input unavailable**, enter `/bg` in that terminal and open the conversation from **Sessions** again.

A normal request waits for the current turn to finish. Answer any question or permission prompt in the tab and check
that your request appears there before sending it again. If you see **Request not sent · Its tab closed before it was
typed in**, reopen the tab and send the request again.

## Trust the Codex hooks

If Codex replies in its tab but **Answer** stays empty or the map tool cannot find the session, check the hooks.
Open **Sessions → New conversation → Codex** and enter `/hooks` in the Codex tab. Review and trust the Whaybench hooks.
Do this again if Codex asks after the Whaybench executable path changes.

Send a short prompt. Check that the reply appears in **Answer**, then ask Codex to add a test topic with `context_map`
and look for that node in **Conversation map**.

## Open a web page or bring back the browser

A web address opens in Whaybench's Firefox. If **View** says **No view** for the selected session, open
**Panels → Browser** to look for the web page. To test the browser, press <kbd>Ctrl</kbd>+<kbd>L</kbd>, enter
`https://example.org`, and press <kbd>Enter</kbd>. Check that the page loads.

If Firefox reports that it is already running after a crash, close Whaybench's Firefox if it is still open and try
**Panels → Browser** again. Whaybench clears a stale profile lock when it finds no live Firefox for that profile.
If the browser still fails to open on Ubuntu and you have Snap Firefox, follow the Firefox package step in
[Install](install.md). Then open the browser and try the same address.

If `whaybench-view` refuses a web address from a Briefsafe conversation, open the address yourself in **Browser**.
The project's `whaybench-view open` command can show local pages or files from that project.

## Retry or roll back an update

With an installed release, open **Whaybench → About** and choose **Update to** when it is offered. Whaybench downloads
and verifies the release, then restarts. If you see **Restart to update to**, the download is ready and you can restart
to apply it.

If the update fails before restart, read the reason in the status line. For a download failure, check that `curl` is
installed and that you can open the [release page](https://github.com/Whaybench/whaybench/releases/latest), then retry.
For a signature or checksum error, retry the download. Whaybench refuses to install files that fail verification.

Automatic download failures go in the run logs. If the install fails after restart, startup continues and the
installer output is in `logs/update.log` under your data directory. Fix the error shown there and retry the update.
Check the installed version in **Whaybench → About** or run:

```sh
whaybench --version
```

To go back, first clear **Download new releases automatically** in **Settings → Preferences…** and quit Whaybench.
Remove `cache/update` from your data directory if it is there, or it installs again. Then run this from the directory
where you saved the release's `install.sh`. On Windows, run it inside the `Whaybench` WSL distribution.

```sh
sh install.sh --rollback
```

Rollback goes back only to a version whose programs are all there and run; newer installs also check that its files are
unchanged. Start Whaybench and check the version, the panels and a conversation tab. If the older version refuses to
read data written by a newer one, update again.

## Open a Briefsafe project that stays locked

Open **Panels → Project**, select the project, and choose **Log in**. Read the reason if it stays locked.
If you see **Made by a newer Whaybench: update, then log in**, update Whaybench and try again.

If you see **Needs its container: check podman and the image**, check Podman from your normal Linux account.

```sh
podman --version
```

Install Podman if the command fails. Isolation also needs an image Whaybench knows for your Linux distribution: Fedora,
Ubuntu, Debian and some derivatives of Ubuntu. If the image download fails, check your connection and try **Log in**
again. Vaults also need `gocryptfs`, `fusermount3`, and a keyring. A project that opens without a container says so in
the project panel. Its agents run as you, outside isolation.

Check that the project opens and shows **Log out**. If you need isolation, also check that the warning about opening
without a container is gone before starting an agent.

## Unlock the Briefsafe keyring

If Whaybench is waiting for the keyring, enter its password in the desktop prompt. On Windows, Whaybench asks for the
**Project access password** itself, or has you set one. For a password you have lost, see [Recover access to a
vault](guide/briefsafe.md#recover-access-to-a-vault). Complete the prompt, then check that the project opens.

If the project's key is missing, choose **Log in**, then **Enter recovery key**. Enter the key you saved when you
made the project. If the key does not fit, check that it belongs to this project and try again.

> [!WARNING]
> **Reset the password** sets a new project access password. You need each encrypted project's recovery key to open it
> once more, and you need to add the Claude Code token again.

If the project opens but Claude Code has no token, choose **Inner login → Claude** in the project and complete sign-in.
Check for **Stored. Start a conversation.**, then open a project conversation and send a short prompt.

## Check Windows setup and the work disk

If Setup stops, read the error in its window or `%LOCALAPPDATA%\Whaybench\install.log`. You need x86-64 Windows 11 or
Windows 10 21H2 or newer, virtualization enabled, and WSL 2.4.4 or newer. If Setup says virtualization is off, enable
it in your firmware settings. If it says WSL is too old, run this in **Command Prompt** as administrator:

```sh
wsl --update
wsl --version
```

After the check, rerun `WhaybenchSetup.exe` and open **Whaybench** from the Start menu. If Setup asks you to restart
Windows, let it continue after you sign back in. See [Windows](windows.md) for the setup checks.

**Work disk not attached · ~/work: Linux system disk** means Whaybench starts with `~/work` on its Linux system disk.
Close Whaybench and start it again from the Start menu to retry the attachment. If it still fails, check
`~/.local/share/whaybench/logs/work-disk.log` inside WSL and `%LOCALAPPDATA%\Whaybench\work-disk.log` on Windows.
Rerun Setup if the disk was never set up. When it attaches, check for your project folders in `~/work`. Look in
`~/work.unmounted-<date>` for files you wrote while the disk was away.

## Find logs and turn on detail

Open the newest dated folder under `logs/run/` in your Whaybench data directory. The usual location is
`~/.local/share/whaybench/logs/run/`. Start with `whaybench.log`, then check `panel-<name>.log` for a panel or
`term-<tab>.log` for a conversation tab. If startup fails before a log exists, use the terminal error.

Detail is off by default. To record more about drawing and sessions, close Whaybench, start it from a terminal as
shown below, and repeat the action that fails.

```sh
WHAYBENCH_TRACE=screen,session whaybench
```

You can also put `{"parts":["screen","session"]}` in `logs/detail.json` under your data directory while Whaybench runs.
Look for `[detail]` in the relevant log after reproducing the problem. Detail stops for a log once it exceeds 5 MiB.
Warnings and errors continue. Lines with `versions` in `whaybench.log` name the components Whaybench found; include them
in a report. To turn detail off, start without `WHAYBENCH_TRACE` and remove the entries you added to `logs/detail.json`.

## Report a bug without sharing private data

Choose **Bug or install problem** in [Issues](https://github.com/Whaybench/whaybench/issues/new/choose). Include the
output of `whaybench --version`, your distribution and desktop, the agent if relevant, and the steps to reproduce
the problem. Say what you expect and what happens. Share only the relevant error and log lines.

Read what you paste. Remove account names, private paths, conversation titles, prompts, answers, sign-in tokens and
recovery keys. Crop or redact screenshots before attaching them. Keep your data directory and browser profile private.

For a security vulnerability, use **Security → Report a vulnerability** to report it privately.
[Whaybench Security](https://github.com/Whaybench/whaybench/security)

