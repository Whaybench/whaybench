# Command reference

Use these commands to start Whaybench, control its window, and work with conversations, views, projects and files.
Replace uppercase placeholders with your own values.

## Start Whaybench

Run `whaybench` from a Wayland desktop session. It reopens remembered tabs or starts a Claude Code conversation.
Starting it again with the same data directory replaces the running instance. Startup errors go to stderr with exit `1`.

| Option | What it does |
|---|---|
| `--data-dir DIR` | Use a different data directory. With a subcommand, put the subcommand first. |
| `--app COMMAND` | Open an app inside Whaybench. Quote a shell command with arguments. Repeat for more apps. |
| `--no-session` | Skip the usual panels and conversation tabs at startup. |
| `-h` or `--help` | Print usage. |
| `--version` | Print the version and build commit. |

Without `--data-dir`, Whaybench uses `WHAYBENCH_DATA_DIR`, then `$XDG_DATA_HOME/whaybench`, then
`~/.local/share/whaybench`. Use `WHAYBENCH_DATA_DIR` with `whaybench-view`, `whaybench-project` and `whaybench-files`.

```sh
whaybench --no-session --app alacritty
```

## Control the window with `whaybench ctl`

Use the running window’s data directory. Most requests return as soon as they are queued. Command errors exit `1`.

| After `whaybench ctl` | What it does |
|---|---|
| `quit` / `restart` | Close or restart Whaybench. Restart uses the options it was started with and reopens remembered tabs, except those of locked projects. |
| `theme NAME` | Select `default` or an installed theme ID. Check the window to see whether it applies. |
| `browser detach` / `browser attach` | Move Whaybench’s Firefox to the desktop or back into its panel. |
| `shot --out FILE.png [--panel N] [--app APP_ID]` | Save a PNG. The default is the whole window. |
| `session new [--agent ID] [--project ID] [MAP]` | Start a tab. `MAP` is a session ID or an absolute map path. |
| `session show ID` / `session show --project ID SESSION` | Focus, attach to, or resume a conversation. |
| `session type ID TEXT` | Submit text to an open tab. Quote `TEXT` as one argument. A text that is only `--help`, `-h` or `--version` cannot be sent. |
| `session wait ID [SECONDS]` | Print the next answer, or the one already given to what the same caller last typed. The default is 900 seconds. |
| `session end ID` | End the conversation, close its tab, and hide it from the session list. One working in another terminal is refused: stop it there. |
| `session list` | List conversations from inside a Briefsafe project. |

An unknown or unreadable theme leaves the current theme in place and is logged by the window. For `shot`, panel numbers
start at `0`. A matching app takes priority and can be in a background tab. The command removes any existing output
first, then prints the new path or reports an error such as **No such panel** or a five-second timeout. Whaybench
removes shots taken while a Briefsafe project is open or a protected view is showing after ten minutes.

`session new` uses `--agent`, else the source session’s agent, else Claude Code. A source session without a map is
refused. Check a saved map with `whaybench map --file` before you pass it: an unreadable one opens a tab without the
map. To show a session that works in another terminal, enter `/bg` there first. Open a Briefsafe project before starting
or showing a tab with `--project`. A project’s map can start a tab only in that project. Inside Briefsafe, `list`,
`type` and `wait` are the available session commands. `type` accepts plain text for running conversations in the same
project. Text sent from another conversation that starts with `/` or `!` is refused. `wait` reports an error if delivery
is refused, the conversation needs your input, or the time runs out.

```sh
whaybench ctl shot --out /tmp/whaybench.png
whaybench ctl session new
```

Integration and test entry points include `hook`, `record`, `mcp`, `statusline`, and `ctl pointer`, `press`, `release`
and `panels`.

## Read or update a map with `whaybench map`

| After `whaybench map` | What it does |
|---|---|
| `SESSION` | Print the session’s map as JSON. A session without a map returns `{}`. |
| `--file MAP.json` | Read a saved map. Nodes referenced by the file are skipped if they are gone from memory. |
| `SESSION --write` | Read a JSON update on stdin and print the resulting map. Invalid changes are refused. |

A malformed or unreadable saved map exits `1`. To start a conversation from a map, use `ctl session new`.

```sh
whaybench map --file ./saved-map.json
```

## Look up memory with `whaybench memory`

| After `whaybench memory` | What it does |
|---|---|
| `list SESSION` | Show up to 100 entries: the session’s open nodes, then related and recent ones of others. Marks its memory as seen. |
| `changes SESSION` | Show what others or you changed since the session last saw memory. Marks the changes as seen. |
| `show NUMBER`, `show MEMID`, or `show SESSION/NODE` | Print matching nodes and events as JSON. |
| `search WORD…` | Match any word in labels or details, ignoring case. Show up to 50, most recently changed first. |

`show` can print an error in JSON and still exit `0`. A read failure can leave a search empty.

```sh
whaybench memory search install update
```

## Manage the Briefsafe Claude token with `whaybench token`

| After `whaybench token` | What it does |
|---|---|
| `set` | Read a token from `claude setup-token` on stdin and store it in your keyring. |
| `check` | Print `stored` or `none`. |
| `clear` | Remove the saved token. |

A token of the wrong form or a keyring failure exits `1`. Whether a token still works is not checked. You can also sign
in from **Inner login → Claude** in the project panel.

```sh
whaybench token check
```

## Open and read views with `whaybench-view`

| After `whaybench-view` | What it does |
|---|---|
| `open [--firefox] PATH [PAGE]` | Open a local file, optionally at a PDF page. `--firefox` sends it to Firefox. |
| `open URL` / `open -` | Open an HTTP or HTTPS address in Firefox, or show HTML read from stdin. |
| `edit FILE` | Open the file in Vim in the view area. |
| `read [--all] [--firefox]` | Read text from the first view or Firefox. `--all` asks for the whole page. A protected page gives a refusal instead. |
| `where [--text]` | Print the current PDF page in Firefox. `--text` uses `pdftotext` on a local PDF. |
| `shot [--out FILE.png] [--firefox]` | Capture the first view, or Firefox while it is in Whaybench’s window. The default is `cache/view-shot.png` in your data. |
| `new [--session] [TARGET]` | Create a view and print its ID. It follows the selected session by default. |
| `new --pin [--title TITLE] TARGET` | Create a view pinned to a file or address, with an optional title. |
| `show NAME TARGET` / `list` / `close NAME` | Show something in a named view (a pinned view is pinned to it), list views and their links, or close an extra view. |
| `detach` / `attach` | Move Firefox to the desktop or back. Wait up to 25 seconds for it to move. |

`new --session` needs a known calling session. `open` reports missing local paths, and a queued request can wait for a
view to follow its session. Inside Briefsafe, use `open` for project files and web addresses, and `read`, `where` and
`shot` with `--firefox` for the project's own browser. Errors and `--help` exit `1`.

```sh
whaybench-view open ./report.pdf 3
```

## Work with projects using `whaybench-project`

`--help` exits `0`. An unknown command exits `2`. Reported operation errors usually exit `1`.
Use `--session ID` after the command to choose the session for a document working copy or work item.

| After `whaybench-project` | What it does |
|---|---|
| `list` / `tree` | Print the project list as JSON, Briefsafe projects included, or show the hierarchy of the older kind. |
| `for-cwd [DIR]` | Print the ID of the older-kind project linked to the closest directory. Default is your current directory. |
| `new ID NAME [--desc TEXT] [--path DIR]…` | Create a Briefsafe project. Use `--path` to link directories. |
| `new` with `--folder DIR`, `--from SRC`, or `--map FILE` | Choose a Briefsafe folder, copy in files, or seed its map. |
| `new` with `--encrypt` or `--no-encrypt` | Require encryption or opt out. By default, Briefsafe encrypts its records where it can, and refuses if this data folder ever had an encrypted project. |
| `new` with `--plain` or `--parent ID` | Create a classic project or a classic subproject. |
| `move ID PARENT` / `move ID -` | Change a classic project’s parent or move it to the top level. |
| `convert ID --folder DIR [--from SRC] [--encrypt]` | Move a standalone classic project to Briefsafe. Add `--encrypt` for encryption. |
| `open ID [--recovery]` / `close ID` | Open a Briefsafe project with its container, or close it. `--recovery` waits for the recovery key on stdin: type it there, not on the command line. |
| `snapshot ID` / `snapshots ID` | Save or list snapshots. Saving needs a Btrfs subvolume. |
| `restore ID SNAPSHOT` | Restore the whole folder and save its current state first. Close the project before restoring. |
| `forget ID` | Remove a closed Briefsafe project from the list. Its folder stays. |
| `map ID` / `map-put ID [FILE]` | Print the project map or update it from JSON. Omit `FILE` or use `-` for stdin. |
| `show ID DOC` / `outline ID DOC` | Print the document or its headings with line numbers. |
| `checkout ID DOC` | Print the working copy’s path. A second checkout replaces that copy. |
| `put ID DOC --what TEXT [--source JSON] [--note TEXT]` | Publish the working copy with a change summary. |
| `archive ID DOC` | Take the document out of the active tabs. It stays readable. |
| `recent ID [DAYS]` | List recent changes, newest first. The default is 30 days. |
| `change ID DOC EVENT` / `change-items ID EVENT` | Show before and after details. Missing details can print an error with exit `0`. |
| `queue ID` / `take ID WORK` / `done ID WORK [--note TEXT]` | List, claim, or finish a work item. |

On the host, the commands from `map` to `done` read the older kind’s store. For a Briefsafe project, run them in one of
its conversations, or set `WHAYBENCH_DATA_DIR` to the open project’s data path that `open ID` prints. `take` prints
`taken` when you may go on, `already` when the item is done or taken elsewhere, and `missing` when there is none.

A new Briefsafe folder defaults to `~/work/ID` when available, otherwise `~/ID`. Creation fails if required encryption
is unavailable. Closing can fail if a program still holds the vault open. Restore from outside the project folder, with
nothing using it. Whaybench keeps the latest 30 snapshots when it can remove older ones. If `put` reports that the
document changed after checkout, save your edits before checking out again, then reapply them to the new copy.

```sh
whaybench-project list
whaybench-project snapshots notes
```

## Attach files with `whaybench-files`

Use `--session ID` to choose a conversation. Otherwise the helper uses the selected conversation, or the one whose map
changed last. Quote globs so the helper receives the pattern. In Briefsafe, wildcards are allowed only in the last path
component.

| After `whaybench-files` | What it does |
|---|---|
| `attach NODE PATTERN…` | Add file paths or globs to a map node. Relative paths start at your working directory. |
| `list [NODE]` | Print stored patterns and match counts. A missing file or invalid glob can show zero. |
| `detach NODE [PATTERN…]` | Remove exact patterns as printed by `list`, or omit them to remove all for the node. |

A missing session or a host request for a Briefsafe conversation exits `1`.

```sh
whaybench-files attach NODE_ID 'src/*.rs'
whaybench-files list NODE_ID
```

