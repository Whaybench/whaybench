# Install Whaybench

Install Whaybench on Linux or Windows and check the files you download. You can also update it, go back one version
or remove it.

## Install on Linux

You need an x86-64 computer, a Wayland desktop session, `curl`, `tar` and `sha256sum`. If you use Claude Code, install
it and sign in first. If you use Codex, install Codex CLI separately and sign in when it asks.

Run the installer as your normal user.

```sh
curl -fsSL https://github.com/Whaybench/whaybench/releases/latest/download/install.sh | sh
```

The release includes Whaybench and Alacritty for its terminal tabs. If `claude` is on your `PATH`, the installer adds
or updates the `context-map` plugin. If that step fails, it prints commands to try again.

The installer checks for GTK 4, WebKitGTK 6.0, `wl-clipboard`, Firefox and `fcitx5`. The Snap version of Firefox
cannot open inside Whaybench. If packages are missing, the installer offers a package command on Fedora, Ubuntu,
Debian, Arch, openSUSE, AlmaLinux and Rocky Linux.

On Ubuntu, if Firefox is missing or a Snap, the command also adds Mozilla's package source and prefers its Firefox. If
no question appears, run the printed command yourself.

At **Proceed? [y/N]**, type <kbd>y</kbd> and press <kbd>Enter</kbd> to run the displayed command with `sudo`.
Press <kbd>Enter</kbd> alone to skip it. If that step fails, Whaybench is already installed. Run the printed command
or install the listed packages with your package manager. On Fedora Silverblue and Kinoite, reboot after installing
the system packages.

Start Whaybench from your Wayland desktop.

```sh
whaybench
```

If your shell cannot find it, follow the installer's instructions to add `~/.local/bin` to your `PATH`, or start it
using the full path `~/.local/bin/whaybench`.

## Set up input on Linux

To type with fcitx5, install the input engine for your language. For example, `fcitx5-mozc` for Japanese: `sudo apt
install fcitx5-mozc` on Ubuntu and Debian, `sudo dnf install fcitx5-mozc` on Fedora. If Whaybench offers to add language
support, choose **Install**. If it cannot install the engine, use your package manager. If fcitx5 is unavailable,
Whaybench uses your desktop's input method.

## Install on Windows

You need an x86-64 computer with virtualization enabled and Windows 11 or Windows 10 version 21H2 or newer. Setup
installs or updates WSL. It needs WSL 2 with WSLg and a WSL version of 2.4.4 or newer.

1. Download and run
   [WhaybenchSetup.exe](https://github.com/Whaybench/whaybench/releases/latest/download/WhaybenchSetup.exe).
2. If setup asks you to restart Windows after installing WSL, restart and sign in again; setup goes on by itself. It
   installs Whaybench, Claude Code and Codex CLI in a Fedora distribution named `Whaybench`.
3. Open **Whaybench** from Start or the desktop and follow Claude Code's sign-in. Its page opens in the browser inside
   Whaybench.

If setup stops, check `%LOCALAPPDATA%\Whaybench\install.log`. Fix the problem and run the installer again.
See [Windows setup help](windows.md).

## Check a release signature

With `minisign` installed, download the file and its matching `.minisig` from the same release. Keep them in the
same folder. Use the Whaybench public key to check the file before you run it. For example, to check `install.sh`:

```sh
minisign -Vm install.sh \
  -P RWTwDOf9rBKzxEepAVlbLTan2g/RQFDuCZeqmU4Q+dNwf+wF+3a2Sfia
```

Use the same command with `WhaybenchSetup.exe` or `whaybench-x86_64-linux.tar.gz` in place of `install.sh`.
If verification fails, download both files again and check them before continuing.
See the [release signing instructions](../SECURITY.md#checking-a-release).

The Linux one-line install runs the script directly without checking its signature. To check it first, download
and verify `install.sh` as shown above, then run it from that folder.

```sh
sh install.sh
```

The script installs the latest release, unless the package and its checksum file lie in the folder you run it from. It
always checks the Linux package's checksum. It checks the package's signature if `minisign` is installed and stops if a
required check fails. The built-in updater always checks both the script and package signatures before it puts an update
aside for the next start.

## Update Whaybench

On Linux and Windows, an installed release checks for new releases and downloads them by default. A build from source
does not. A downloaded update is installed the next time Whaybench starts.

| What you want to do | Where to go |
|---|---|
| Apply a downloaded update | **Whaybench → Restart** |
| Download an available update now | Open **Whaybench → About** and choose the item that starts with **Update to** |
| Download updates only when you choose | Open **Settings → Preferences…** and clear **Download new releases automatically** |

A manual update verifies the download and restarts Whaybench. If the download or verification fails, the status line
shows the reason. If installation fails at startup, Whaybench continues starting. See `logs/update.log` in your data
folder.

## Go back one version

The installer keeps one previous version.

1. Clear **Settings → Preferences… → Download new releases automatically**, or the newer version comes back by itself.
2. Download `install.sh` of the latest release and check it as shown above.
3. Quit Whaybench. Remove `cache/update` from your Whaybench data folder if it is there: it is an update waiting for the
   next start.
4. Run this from the folder where you saved the script. On Windows, first open a shell in Whaybench's Linux system with
   `wsl -d Whaybench`.

   ```sh
   sh install.sh --rollback
   ```

5. Start Whaybench and check that its panels and terminal open.

If there is no earlier version, or it is not complete, the command says so and changes nothing. Running it
once more goes back to the version you left. An older version may refuse data a newer one has written; update again
then.

## Uninstall on Linux

Log out of your Briefsafe projects first. Their folders, snapshots and keyring entries, and podman's images, stay; see
[Remove a project](guide/briefsafe.md#remove-a-project).

Close Whaybench and remove its installed files.

```sh
rm -f ~/.local/bin/whaybench ~/.local/bin/whaybench-view ~/.local/bin/whaybench-project ~/.local/bin/whaybench-files
rm -rf ~/.local/lib/whaybench ~/.local/lib/whaybench-versions
```

If you installed the Claude Code plugin, remove it too.

```sh
claude plugin uninstall context-map@whaybench
```

Your system packages stay installed. Your conversations and settings stay in your Whaybench data folder, usually
`~/.local/share/whaybench`.

## Uninstall on Windows

Choose **Settings → Apps → Whaybench → Uninstall**. This removes the setup files and the shortcuts. When asked whether
to delete the Whaybench Linux system, choose **No** to keep it with its programs, conversations, settings and files.
Deleting that system also deletes any work stored on its own disk, and the keyring: after that, encrypted projects on
the work disk open only with their recovery keys.

If setup made the work disk, `%LOCALAPPDATA%\Whaybench\work.vhdx` stays with the projects stored on it. Projects made
while the disk was missing, or without one, are on the Linux system, so check where each one is before you delete that.
To delete the work disk too, see [Removing everything](windows.md#removing-everything).

