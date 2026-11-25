# Settings and data

Use this page to change Whaybench’s defaults, find what it keeps, and back up or move your setup to another computer.

## Choose a data folder

Whaybench takes the first of these that is set, and stops if it cannot use it:

| Order | Location |
|---|---|
| 1 | The folder you give with `--data-dir` |
| 2 | A nonempty `WHAYBENCH_DATA_DIR` |
| 3 | `$XDG_DATA_HOME/whaybench` if `XDG_DATA_HOME` is set and nonempty |
| 4 | `~/.local/share/whaybench` |

For example, start Whaybench with a separate data folder and use the same folder when you ask it to quit:

```sh
whaybench --data-dir "$HOME/whaybench-other"
whaybench ctl --data-dir "$HOME/whaybench-other" quit
```

Run the second command from another terminal. A second copy still shares your keyring, `~/.claude` with the plugin, and
the commands in `~/.local/bin`. Starting again with the same data folder replaces the running instance. Whaybench stops
with an error if it cannot find a data folder or create its `cache/` folder.

## Set preferences

Open **Settings → Preferences…**. Changes save as you make them in `settings.json` in the data folder.

| Control | Key | Default and values |
|---|---|---|
| **Conversation records kept (days)** | `keep_days` | `0`, no limit. The control accepts 0 to 3650. |
| **Conversation records kept (MB)** | `keep_mb` | `0`, no limit. The control accepts 0 to 100,000. |
| **Deleted nodes kept (days)** | `trash_days` | `180`. The control accepts 0 to 3650. Use `0` for manual removal only. |
| **Download new releases automatically** | `auto_update` | `true` for installed releases. Clear the checkbox to turn it off. |
| **Prevent sleep while an agent works** | `keep_awake` | `true`. Clear the checkbox to turn it off. |
| **Input method** | `input_method` | Unset uses **Built-in (fcitx5)** when available. `"host"` uses **The desktop's**. |
| **Conversation map → Display style settings…** | `map_indent` | `4`, from 1 to 8: **Indent per level (letters)** of **Default**. |
| <kbd>Ctrl</kbd> + wheel on a title bar | `title_zoom` | `1.0`, from `0.7` to `2.0`, in steps of `0.1`. |
| Edit `settings.json` | `work_dir` | `~/work`, the starting folder for new regular conversations. |

Sleep prevention depends on the system. The screen can still turn off or lock. Input method changes apply after a
restart. Without fcitx5, Whaybench uses the desktop input method. On WSL, use **Built-in (fcitx5)** for composed
input. Open **Settings → Input…** to add an engine or change **Switch keys**. If Whaybench shows
**fcitx5 not running · Input settings unavailable**, check that fcitx5 is installed and running.

To start new conversations in `~/code`, close Whaybench and add `"work_dir": "~/code"` to `settings.json`. Keep the
other keys in the file. Use a path starting with `~/` or an absolute path. A path with `..`, the home or data folder
itself, or a parent of either falls back to `~/work`. A resumed conversation uses its old folder if it still exists.

If **Preferences** finds malformed settings, it loads defaults and tries to set the old file aside with an
`.unreadable` suffix. Look for that file if you need to recover your choices.

## Change the language, zoom, and layout

- **Settings → Language** saves the language ID as `active` in `language.json`. Restart to apply it. With no saved
  choice, Whaybench uses an available language matching your locale, or English. A missing saved language uses English.
- Hold <kbd>Ctrl</kbd> and scroll inside a supported panel to zoom it. `panel-zoom.json` keeps the zoom per panel. The
  default is `1.0`. The range is `0.5` to `3.0`, in steps of `0.1`. Do the same on a title bar to change `title_zoom`.
- **Panels → Reset layout** puts panels back in their starting positions and groups, and closes extra views, branch maps
  and imported maps. Whaybench uses the initial layout if it cannot read `layout.json`.

## Change the theme

Choose **Settings → Theme**. The selection is `active` in `theme.json`. The default ID is `default`. If a selected
theme is unavailable, Whaybench uses **Default**.

Open **Settings → Theme settings…** to edit the selected theme. For example, set **Fonts → Terminal font size** to
`14`. **Clear changes (theme defaults)** restores the theme’s own values. To keep a variation, choose
**Save as new theme…**, give it a name, and press **Save**.

Changes to **Default** go in `addons/themes-default-settings.json`. Changes to other themes go in
`addons/themes/<id>/settings.json`. Each theme brings its own values. The window has these groups:

| Group | What it sets |
|---|---|
| **Base colours** | Background, surface, accent and text |
| **Fonts** | Font and size of the interface, the terminal and the title bars |
| **Panel shape** | Corner radius, the gap between panels, the outer margin and the inner padding |
| **Opacity** | How see-through the window surface, the panels and the panels behind are, and the glass blur |
| **Shadow and depth**, **Light** | Shadow, the height of the panels behind, and the light on the surface and the panels |
| **Title bar**, **Border**, **Surface** | Their colours, gloss and edges |
| **Joint lines**, **Docking** | The lines between joined panels, and the dot and targets used for docking |

The keys in the file are in the [theme guide](../../addons/themes/README.md).

## Set up map display styles

Choose **Display style** in a map panel. Each panel keeps its choice in `map-style-current.json` and uses **Default**
if its chosen style is missing. To edit a style, open **Conversation map → Display style settings…** and select it.

| Style | Control | Key | Default and values |
|---|---|---|---|
| **Default** | **Indent per level (letters)** | `map_indent` | `4`, from 1 to 8 |
| **Focus** | **Show** | `filter` | **The current path and open nodes** (`path`) or **Every node** (`""`) |
| **Focus** | **Show each node's kind** | `show_kind` | `false`. Applies when you show every node. |
| **Kanban** | **Make columns by** | `group` | **State** (`state`), **Kind** (`kind`), or **Top branch** (`top`) |
| **Kanban** | **Column order** | `columns` | `idea, plan, doing, held, done`. Enter names separated by commas: states, or the shown kinds or branch titles. Columns you leave out come last. |
| **Kanban** | **Recent items in the done column** | `recent_done` | `6`, from 0 to 50. In **State**, `0` shows all done items. |
| **Timeline** | **Range** | `range` | **From the start of the current topic** (`topic`) by default |
| **Timeline** | **Range** | `range` | Also **The whole map** (`all`) or **From the last decision** (`decision`) |

For example, set **Recent items in the done column** to `3` to show fewer finished cards. A style’s changes go in
`addons/map-styles/<id>/settings.json`. Invalid values fall back to the style’s own values. Use
**Back to the style's own settings** to clear the changes. For **Default**, set the indent back to `4` by hand.

## Set environment variables

Set these in the shell you launch from. For example, start in English with extra session and request details in the log:

```sh
WHAYBENCH_LANG=en WHAYBENCH_TRACE=session,request whaybench
```

| Variable | What it changes |
|---|---|
| `WHAYBENCH_DATA_DIR` | Sets the data folder. Use it for command helpers that take their data location from the environment. |
| `XDG_DATA_HOME`, `HOME` | Supply the fallback data locations listed above. |
| `WHAYBENCH_LANG` | Overrides your saved interface language for this launch. Use a language ID such as `en`. |
| `LC_ALL`, `LANG` | Supply the default locale in that order when there is no saved or forced language. |
| `WHAYBENCH_TRACE` | Adds comma-separated log categories. Extra detail is off by default. |
| `CLAUDE_CONFIG_DIR` | Points to your regular Claude Code settings and transcripts. It defaults to `~/.claude`. |
| `CODEX_HOME` | Points to the regular Codex folder, default `~/.codex`, that the bundled add-on takes settings from. |
| `WHAYBENCH_SESSION_ID` | Targets a session in the view and project helpers. They also try `CLAUDE_CODE_SESSION_ID`, then `CLAUDE_SESSION_ID`. |
| `XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY` | Locate the desktop Wayland socket. Launch from your desktop session if Whaybench cannot find it. |
| `XDG_DATA_DIRS` | Locates fcitx5 engine information. It defaults to `/usr/local/share:/usr/share`. |
| `GALLIUM_DRIVER` | Keeps your Mesa driver choice. When unset and `/dev/dxg` exists, Whaybench uses `d3d12`. |
| `GSK_RENDERER` | Keeps your GTK renderer choice. When unset, the **Input** panel uses `cairo`. |
| `WHAYBENCH_REPO` | Changes the Linux installer’s download source. It defaults to the Whaybench repository. |

For a lasting log setting, set `parts` in `logs/detail.json`, for example `{"parts":["session","request"]}`.
Whaybench combines it with `WHAYBENCH_TRACE`. Each log stops adding detail once it passes 5 MiB. Warnings and errors
still go into the log.

## Find your data

These paths are relative to the data folder. Keep the folder private. It can hold prompts, answers, and browser
sign-ins.

| Path | What it holds |
|---|---|
| `settings.json`, `theme.json`, `language.json`, `panel-zoom.json`, `layout.json` | Preferences and panel layout |
| `sessions/`, `names.json`, `focus.json` | Session records, names, and the selected session |
| `<session-id>.json`, `latest.json`, `forks/`, `memory/` | Conversation maps, their shared node content, and change history |
| `answers/`, `answer-log/`, `answer-history.json` | Latest answers, prompts and answers by turn, and recent answers |
| `files/`, `node-files/`, `files-hidden.json` | File activity and edits, map attachments, and files you hide |
| `maps/`, `imported/`, `map-panels.json` | Saved and imported maps, and map panels to reopen |
| `addons/`, `addon-trust.json`, `map-style-current.json` | Add-ons, your choices about trust, and map styles |
| `show/`, `view/` | Pages shown by agents, view bindings and history, and pending view requests |
| `browser/firefox/`, `browser/webkit/` | Whaybench’s browser profiles and sign-ins |
| `projects/<id>/`, `projects/<id>.json` | Classic project records and documents, and pointers to Briefsafe folders |
| `codex/` | The bundled Codex add-on’s settings and history inside Whaybench |
| `logs/`, `cache/`, `ask/` | Logs, temporary state, and queued requests between conversations |

Outside the data folder, keep track of your working folders, your regular Claude Code folder with its plugin and
transcripts, and your regular Codex folder. The bundled Codex add-on can link its `auth.json` to the regular Codex
folder. fcitx5 keeps its own input settings. Installed binaries and links live under `~/.local/lib/` and
`~/.local/bin/`.

Each Briefsafe project keeps its own data and agent history in `.whaybench` inside the project folder. Its snapshots
are beside that folder in `.<project-folder-name>.snapshots/`. The system keyring holds vault keys and the Briefsafe
Claude token.

## Clear temporary data and set retention

Close Whaybench and log out of Briefsafe projects before clearing `cache/`. Whaybench recreates it. Clearing it discards
cached tab reattachment, pending tab requests, staged updates, the prompts listed under **Due** and the list of
screenshots to delete. You can remove old files from `logs/run/` and `logs/sessions/` once you no longer need them.
Whaybench keeps the latest 10 run logs and 14 days of session logs.

To keep a year of records, set **Conversation records kept (days)** to `365` and restart. Whaybench removes entire
conversation records by last activity, oldest first. It also trims older map change history and keeps the nodes. The
size setting uses MiB and counts conversation records only. Running conversations and conversations still needed by
another conversation that inherited their map can keep the total above the limit. Answer history, map nodes and the
agents' own transcripts are not counted. These settings do not apply to the records of Briefsafe projects.

Save any map you want to keep before you set a limit. Saved maps stay in `maps/`. Keep `memory/` with them so Whaybench
can fill in the nodes they refer to. If Claude Code has already dropped a transcript, look for an `auto-map-` file in
**Conversation map → Open map**. An eligible map can still be opened from that copy. Resuming the chat requires an agent
transcript.

In **Notifications → Deleted**, right-click a deleted item and choose **Restore** or **Delete for good**. You can
also right-click the section heading and choose **Delete all for good**. At startup, Whaybench permanently clears
nodes past **Deleted nodes kept (days)**. A saved map skips any referenced node that is gone from memory.

## Back up and move on Linux

Finish active work, stop background sessions that can still write, log out of Briefsafe projects, and close Whaybench
and its browser. Copy the whole data folder to a new location on your backup drive. For example:

```sh
cp -a "$HOME/.local/share/whaybench" /path/to/backup/whaybench
```

Use your actual data and backup paths. Preserve permissions and links. Copy regular agent transcripts and working
folders separately. For Briefsafe, copy the project folder with its hidden `.whaybench` folder, and keep the host entry
in the data backup. An ordinary copy of the snapshot folder keeps the files but not the snapshots: use `btrfs send` and
`btrfs receive` to keep them restorable.

> [!WARNING]
> Keep each encrypted project’s recovery key separately. The vault key in your system keyring is outside the data
> backup. If you lose both keys, you cannot open the encrypted project.

On the new computer, [install Whaybench](../install.md) and your agents. Restore the data and working folders before
starting your conversations. Keep the same absolute working paths where you can. Claude Code looks for transcripts by
working folder. File attachments and local view history can still point to the old paths. Missing files drop out of
view history. If a Briefsafe folder moves, update `folder` in its copied `projects/<id>.json` while Whaybench is closed.

Use **Log in → Enter recovery key** if the destination keyring lacks the vault key. The recovery saves a fresh key
there. Set up the Briefsafe Claude token again through **Inner login → Claude**. Reinstall the agent integration and
sign in where needed. Browser profiles come with the data copy. Live agent processes and desktop settings stay on
the old computer.

## Move the Windows work disk

Back up the data folder and regular agent folders from inside Whaybench’s WSL distribution. Back up
`%LOCALAPPDATA%\Whaybench\work.vhdx` separately after shutting down WSL. When attached, that disk supplies `~/work`.
With the default paths, the distribution holds the data folder and agent settings.

On a fresh destination, put a copy of `work.vhdx` at the same location under your Windows account before running
Whaybench Setup. Setup uses the existing work disk. Restore the Linux data and agent folders in the new distribution
and check that your project paths still point to the right folders before opening them.

If you see **Work disk not attached · ~/work: Linux system disk**, check
`~/.local/share/whaybench/logs/work-disk.log` inside WSL. Check `~/work.before-disk` and `~/work.unmounted-*` on the
old system too. Copy any files you still need from those folders. See [Windows setup](../windows.md#the-work-disk).

