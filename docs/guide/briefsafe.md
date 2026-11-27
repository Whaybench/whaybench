# Briefsafe projects

Use a Briefsafe project for work that is between you and your AI. It has its own folder and conversations. On a
supported system, its agents run in a container, its records are encrypted while you are logged out, and you can save
snapshots and restore them.

## Check what Briefsafe protects

The project folder holds your work files, as plain files: they are not encrypted, logged out or not. Use disk encryption
for those. Its hidden `.whaybench` folder holds the project's documents, map, memory and conversation history. It is
encrypted with gocryptfs where possible; the encrypted store is called the vault here. The key that opens it is kept in
your keyring, the desktop's store for passwords. The work files, the project's name and ID, its folder path and whether
it has a vault stay readable.

| Project state | Whaybench data with a vault | Whaybench data without a vault |
|---|---|---|
| Logged out (**locked**) | Encrypted on disk | Readable on disk |
| Logged in (**open**) | Readable through the open vault | Readable on disk |

Snapshots and copies of the closed vault keep that data encrypted. The **Sessions** panel hides conversations
belonging to locked projects.

An isolated project shares one rootless Podman container across its conversations.

| Container access | What it gets |
|---|---|
| Read and write | The project folder, open Whaybench data and a home folder for the project's settings and caches |
| Read only | Directories of host programs, toolchains, fonts and certificates, agent add-ons and Claude Code plugin folders. The container can read everything in them |
| Display | Windows of its own in Whaybench. What runs there cannot type in other panels. The clipboard is shared with the window you are in |
| Outside by default | The rest of your home folder, other project folders and host Whaybench data |
| Outside by default | Host agent sign-in files and the host keyring |

Whaybench passes the shared token from your keyring to Claude Code. Codex keeps its sign-in inside the isolated project.

A project is **locked** while you are logged out and open while you are logged in. Quitting Whaybench does not lock it,
and neither does suspending the computer. Use **Log out** and check that it shows **locked**.

To see whether a project is encrypted, look in its folder. This file is there when it has a vault:

```sh
ls ~/work/PROJECT/.whaybench/gocryptfs.conf
```

## Set up Briefsafe on Linux

The Windows setup brings all of this. If restoring a snapshot says that `fuser` is missing, an earlier setup did not
bring it: run `wsl -d Whaybench -u root -- dnf install psmisc` in PowerShell. The same with `dbus-daemon` when the
project's browser says `dbus-daemon: No such file or directory`. On a work disk that is not btrfs there are
no snapshots. On Fedora and on Ubuntu or Debian:

```sh
sudo dnf install podman gocryptfs fuse3 btrfs-progs psmisc dbus-daemon     # Fedora
sudo apt install podman gocryptfs fuse3 btrfs-progs psmisc     # Ubuntu, Debian
podman run --rm docker.io/library/hello-world                  # checks podman
```


- For isolation, install `podman` and run it as your user. Whaybench selects an image for Fedora, Ubuntu, Debian and
  some Ubuntu derivatives. It downloads the image at first login if needed. On other systems there is no container, so a
  project has no isolation and no encryption there.
- For encryption, install `gocryptfs` with `gocryptfs-xray` and FUSE with `fusermount3`. You also need Podman with a
  supported image, as above, and a Secret Service keyring (KDE and GNOME bring one).
- For snapshots, install `btrfs-progs` before you create the project and make it on a btrfs disk: Whaybench then makes
  the folder a btrfs subvolume, which **Save state** needs. Restoring a snapshot also needs `fuser`.

## Create a project

Open **Panels → Project** and click **New project**. Enter a **Project name**. Under **Project map**, choose
**Empty map** or a map from the list. Click **Create**. The project starts **locked**.

Whaybench usually names the folder from the project name. It goes in `~/work` when that exists, or in your home folder
otherwise. On Windows it also goes in the home folder while the work disk is missing. For an encrypted project, **View**
shows the recovery key once. If the panel cannot show it, the message names a file: copy the key and delete the file. If
it says the key could not be shown, put nothing in the project and make another. Whaybench cannot show a recovery key
again.

When Whaybench finds a requirement for encryption missing, it tells you why and makes the project without it. If making
the vault fails after that check, it makes nothing. If this data folder has ever had an encrypted project, it makes
nothing instead. To insist on encryption, make the project in a terminal with `whaybench-project new ID NAME --encrypt`.

## Log in to the project

Select the project and click **Log in**. Whaybench unlocks the vault if present and starts the container if available.
If login fails, the project page shows the reason. A project with a vault needs its container to open. Without Podman or
a matching image, a project without a vault can open with a warning, and its agents run as your user on the host. If the
image is known but cannot be downloaded, login fails.

## Sign in to your agent

- **Claude Code.** Click **Claude** beside **Inner login**. Follow the sign-in prompts in the **View** terminal.
  Whaybench saves one token in your keyring for all Briefsafe Claude Code conversations. If the terminal asks you
  to paste the token, use <kbd>Ctrl+Shift+V</kbd>, then press <kbd>Enter</kbd> and <kbd>Ctrl+D</kbd>.
- **Codex.** Enable device code login in your ChatGPT security settings (in a workspace, the admin does). Click
  **Codex** beside **Inner login** in the open, isolated project. Enter the code from the **View** terminal on the page
  that opens in Firefox, or follow the terminal's link. Repeat this sign-in for each project that uses Codex.

On the project's **Sessions** tab, click **Claude** or **Codex** to start a conversation.

After you install or update a Claude Code plugin on the host, log out of the project and log in again to bring it in.

## Browse in the project

An isolated project has its own Firefox, which runs in the project's container. Choose **Panels → Browser** in one of
its conversations, or let its agent open a web address. The browser panel follows the conversation you are in, and its
title starts with the project's name.

- Sign in on a site once for the project. Sign-ins, history and tabs are kept in the project's data, encrypted with it
  where it has a vault, and come back the next time. Your other browser's sign-ins and extensions are not there.
- The project's agents read what you have in front with `whaybench-view read --firefox`. They can reach this browser's
  sign-ins, as ordinary agents can those of the other browser. Other conversations cannot read it.
- It opens and saves the project's files, not the rest of your home folder. It has no sound. The Firefox of a snap does
  not run there.
- Copy and paste work between it and your other windows: clear the clipboard after copying a secret.
- A project opened by an earlier Whaybench says that its container has no display: log out and log in again. Without a
  container there is no browser.
- A Codex conversation works in a sandbox that can refuse it the browser.

## Log out of the project

Click **Log out** and confirm if conversation tabs are open. Whaybench closes those tabs, stops the container and
locks the vault. Check that the project shows **locked** before you quit. If the vault stays open, close the programs
using it and click **Log out** again.

## Recover access to a vault

Your keyring holds a separate unlock key for each vault. If your Linux desktop asks for the keyring password, enter it
there. On Windows, Whaybench asks for the **Project access password**: one for all encrypted projects. You set it at the
first one, and Whaybench asks for it again when the keyring is locked, for example after the computer starts.

If you lose that password on Windows, submit a wrong password. Choose **Reset the password**, confirm, enter the new
password twice and click **Set**. Then open each encrypted project once with its recovery key, and use **Inner login**
to add the Claude Code token again.

If you lose it on Linux, make a new keyring with your desktop's keyring app. Then recover each vault.

Select the locked project and click **Log in**. When **Enter recovery key** appears, select it, paste the key and
click **Send**. Use one line of 64 hexadecimal digits. Dashes are accepted. Whaybench reports a rejected key and lets
you try again. With a working keyring, recovery stores a new unlock key. Keep the same recovery key.

> [!WARNING]
> If you lose both keys, the vault stays locked. Keep a copy of each recovery key apart from this computer. Anyone who
> has it and a copy of the project folder can read the vault.

A vault can be read without Whaybench, on any Linux with gocryptfs. Enter the recovery key when the command waits:

```sh
mkdir /tmp/vault
gocryptfs -ro -masterkey=stdin ~/work/PROJECT/.whaybench /tmp/vault
fusermount3 -u /tmp/vault
```

The last command closes it again.

## Save a snapshot

Whaybench tries to save a first snapshot when it creates a project on a btrfs subvolume. While the project is open,
click **Save state** to take another. The snapshot includes your work files and Whaybench data and lives beside the
project folder. If Whaybench cannot pause the running container, the save fails.

Whaybench keeps the latest 30 snapshots and deletes older ones as you save. It needs no root for this. Where the
file system is mounted with the btrfs option `user_subvol_rm_allowed` (the Windows setup sets it), a snapshot goes at
once; without it, Whaybench empties it file by file, which takes longer.

## Restore a snapshot

Click **Log out** and stop any programs using the project folder. From a host terminal outside that folder, find
the project ID, list its snapshots and restore the one you choose. Replace `ID` and `SNAPSHOT` with the listed values.

```sh
whaybench-project list
whaybench-project snapshots ID
whaybench-project restore ID SNAPSHOT
```

Whaybench saves the current folder as a snapshot before replacing it. Restore rolls back your files, conversations and
the sign-ins saved inside the project, such as Codex's. The current vault key and the shared Claude Code token stay in
effect. If Codex asks you to sign in again, use **Inner login**.

## Keep your project when you change computers

Whaybench looks for the project at the absolute path in its project listing. Copy both the host Whaybench data directory
and the project folder, including `.whaybench`, and keep both paths, or links back to source maps stop working. If the
folder moves, update `folder` in the copied `projects/<id>.json` while Whaybench is closed. An ordinary copy of the
sibling `.<folder-name>.snapshots` folder keeps the files, but the copies are no longer snapshots you can restore. To
keep restoring, make the project folder a btrfs subvolume and bring the snapshots over as read-only subvolumes, for
example with `btrfs send` and `btrfs receive`. Keep the recovery key for an encrypted project if the new computer has a
different keyring.

## Remove a project

Log out of it, then ask your AI to delete the project. It first says what goes: the project folder with everything
in it, its snapshots and, for an encrypted project, the vault's key in your keyring. Deleting cannot be undone, so look
in the folder for anything to keep and copy it elsewhere before you say yes.

```sh
whaybench-project delete ID
whaybench-project delete ID --confirm CODE
```

The first command gives `CODE`. If a delete stops part way, run the second one again to finish it.

To only take it off the list and keep its folder, use `whaybench-project forget ID`.

## Convert an older project

From a host terminal, convert a top-level project with no subprojects. Use a new folder for `DIR`. Converting does not
encrypt unless you add `--encrypt`. The project records move into the folder. Existing conversations stay with your host
data. Old backups, and deleted records on the disk, can still hold the earlier plain text.

```sh
whaybench-project convert ID --folder DIR
```

## Know the limits

Programs running as you can read a project while it is logged in. Its agents use their model and the network.

You can send a branch of an ordinary map into an open project with **Reflect in project**, while one of its
conversations runs in an open tab. The request waits in memory until that conversation takes it.

Whaybench does not give a project's pages in **View** or its browser to other conversations that ask to read them. A
screenshot can still show them: Whaybench removes its own ten minutes after they were taken while a project was open. A
copy you make stays.

In a Git repository, worktrees and submodules included, Whaybench adds `.whaybench` to the repository's local exclusions
when it makes the project and when you log in, so it stays out of commits.

