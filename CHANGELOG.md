# Changelog

All notable changes to Whaybench are written here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- Apps inside draw on the GPU: the compositor offers its GPU through linux-dmabuf and takes their buffers without a
  copy. Where it has no GPU device to offer (WSLg), apps keep drawing on the CPU as before.

### Fixed

- A whole regrouping of a map by the agent took the person's mark off its nodes.
- A choice made in the notifications panel on a node deleted for good a moment before made a new node named by a number.
- Reading the memory took the store's write lock, so programs that only read made each other wait.
- Each prompt read every event of the other sessions to find the new ones.
- The memory failing to be read gave an empty list without a word in the log.
- Korean and other characters the title bar font lacks show in a fallback font that has them, instead of boxes
  (Windows: Ubuntu's "sans" there is Latin only).

## [0.5.3] - 2026-11-13

The memory keeps every map node once, in a database, as it was meant to: a node no longer disappears from the memory
when it leaves a map. Data from before is not converted.

### Changed

- Map nodes are stored once, in `memory/memory.db` (SQLite) in the data folder. A session's map file lists the nodes
  it shows and no longer holds them. A map taken over, and a new conversation started from a saved map, show the same
  nodes instead of copies, so a change made in one conversation shows in the others.
- Deleting a node is the same on the map and in the notifications panel: the node is marked deleted, leaves every
  map, and waits in the panel's Deleted section. Restore puts it back.
- Deleting for good asks once, naming the node and listing its history, and takes that history with it.
- The agent's `remove` only takes a node off its conversation's map. The node stays in the memory.
- Saved and imported maps list their nodes by number. `whaybench map --file MAP.json` reads one with its content.
- With `"keep_days"` set, the record of how nodes changed is dropped after that many days. The nodes stay.

### Added

- History tab: a right-click on a line restores its node to the chosen conversation's map, deletes it, or deletes it
  for good.
- Search in the memory finds a word inside a longer one, in names and details.

### Fixed

- The memory list, search and the notifications panel were built from every session's map file, so a node taken off
  a map was gone from the memory too.
- A conversation resumed after leaving it could show an empty map: its record said it had moved to itself, and its
  tab was paired with the empty session left behind.

## [0.5.2] - 2026-11-13

Whaybench Setup for Windows installs Whaybench inside again; 0.5.1's made only the shortcuts on a new machine.

### Fixed

- WhaybenchSetup.exe set the Linux side up only on a machine where an earlier setup had already done it: Windows
  PowerShell broke the command that starts it, and the setup said it was done. A new machine got shortcuts to a
  distribution without Whaybench.

## [0.5.1] - 2026-11-13

Whaybench installs on Windows.

### Added

- WhaybenchSetup.exe, in each release, installs Whaybench on Windows 11 or Windows 10 21H2 and newer: WSL when it is
  missing, an Ubuntu distribution of its own named Whaybench, Firefox from Mozilla, the input method for the Windows
  display language, Claude Code and Whaybench, then shortcuts in the Start menu and on the desktop. Run again, it goes
  on from where it stopped. Uninstalling asks whether to delete the distribution too.

### Fixed

- A start always looks the latest release up; a check less than six hours old kept a new release from being seen
  after a restart.

## [0.5.0] - 2026-11-13

Whaybench now tells when a new release is out and installs it from its own menu, takes composed input such as Korean
through an input method of its own, and lets the map's look be set.

Update to this version once with the install command, `curl -fsSL
https://github.com/whaybench/whaybench/releases/latest/download/install.sh | sh`: the update offered in the menu starts
with this version. Install fcitx5 and the engine for your language too (see the README).

### Added

- Whaybench → About offers a newer release when there is one: it installs it and Whaybench restarts with the tabs
  kept. Whaybench looks the latest release up on GitHub at start and every six hours, only when it was installed with
  install.sh; nothing is sent. The status line says so until it is installed.
- Whaybench → About → Support Whaybench opens https://ko-fi.com/whaybench in the view panel.
- Conversation map → Display style settings… changes how the map is shown: the indent of Default, the tree (letters
  per level, 4 before; kept as `map_indent` in settings.json), and, for each map style, the settings its frame has
  (what a tree shows, how columns are made and ordered, a time flow's range). They are kept in the style's folder as
  settings.json, so deleting the style takes them along and a new version of the style keeps them.

### Fixed

- Ubuntu's Firefox, a snap, opens in the view panel and answers to Panels → Browser: it names its window
  firefox_firefox, which Whaybench did not take for Firefox, so it opened as a panel of its own.

### Changed

- The input method lives inside Whaybench: fcitx5 attaches to its display (input-method-v2), so composed input such
  as Korean works the same on every desktop, WSL's included, where it did not work before. fcitx5 joins the system
  packages; without it, or with `"input_method": "host"` in settings.json, the desktop's input method reaches the
  apps through the window as before. The input method's candidate list (Hanja, for one) is not drawn yet.
- Whaybench → About shows the version alone; `whaybench --version` and the logs keep the commit.

## [0.4.0] - 2026-11-12

The notifications panel now shows the whole memory too: every change to every map node, in time order.

### Added

- The notifications panel has two tabs: Now, the sections as before, and History, every change to every map node,
  newest first, only to read: created, state and kind as before → after, renamed, marked, links, maps taken over and
  nodes deleted for good, with who did it. It reads the memory again only while it is on show and the memory grew.

### Fixed

- A floating panel, such as the input panel, stays on top after a restart. Each panel that opens is raised, and
  a panel of the layout raised over a floating one was counted as floating too, so it stayed above it.
- Tooltips, such as a map node's detail, take the theme's colours. They kept GTK's dark background with the
  theme's text colour, so only the black theme could read them.
- Running the install command again while Whaybench runs updates it: copying over a running program failed with
  "Text file busy" and stopped the install. It also updates the Claude Code plugin, which install alone left as it
  was.

### Changed

- The title bar text size goes up to 32 in the theme settings (was 18). The title bar keeps its height.
- The alerts panel is called Notifications in English, and `notify` inside (`whaybench-panel notify`). A saved layout
  and panel zoom from before find it by its old name.
- The notifications panel's top button reads Mark changes seen (was Mark seen): it clears only the Changed section.

## [0.3.1] - 2026-11-12

### Fixed

- The answer panel draws the answer as a web page: tables are real tables with lines and wrapping cells, and a
  selection goes across paragraphs, lists, code and tables (a drag past a block, Ctrl+A, Ctrl+C). A link opens
  in the view panel. HTML written in an answer is shown as text.
- A frame that cannot be drawn, or an app that cannot connect, no longer stops Whaybench: the frame is left out
  and tried again, and the failure is written to the log.
- Two programs writing the same file at once no longer take each other's temporary file: session records,
  themes, requests to the compositor, panel zoom and the answer history now name it by process, as the maps did.
- The alerts panel reads each file once a refresh. A session's name read every record and map again for each
  map on the board, so the reading grew with the square of the sessions; it went from 229 file opens a second
  to 43 here.

## [0.3.0] - 2026-11-12

Claude Code and Codex CLI now connect to Whaybench the same way, so a session of one sees what the other
decided and either can carry the work on.

### Added

- Claude Code connects the way Codex does: the `context-map` plugin is now only configuration, standard hooks that
  call `whaybench hook claude` and the MCP server `whaybench mcp` for the map tool, with no code of its own. The
  relay allows the map tool when it starts Claude Code, types queued requests into the tab as a paste once the turn
  is over (for both agents), and notes `/background` and `/rewind` for the hooks. A map style's fill is written
  to the map directly.
- `whaybench hook claude|codex`: one handler for both agents' standard hook events. Codex sessions now get what
  Claude Code sessions get: the memory list at the start and after a compaction, the memory changes and the files
  the person changed on each prompt, the view panel note, and file records for what `apply_patch` changed.
- `whaybench mcp`: the conversation map tool `context_map` over MCP, with the same description and input as
  Claude Code's. Codex now keeps its map with this tool instead of shell commands; Whaybench registers the
  server in Codex's own config and lets the tool run without asking.
- The important mark shows as `!` before a node's name in the alerts panel and the map panel, and the map panel's
  node menu can mark and unmark.
- In the memory list, a decision carries the first sentence of its reason (`why:`), so another session sees why,
  not only what.

### Changed

- The alerts panel shows a node's state in the interface language instead of the stored value.
- `whaybench-view read --firefox` reads the panel's Firefox through AT-SPI in Rust (GIO's D-Bus client) instead of
  a Python script, so python3 and PyGObject are no longer needed.
- The alerts panel's Marked section shows marked nodes that are already done too.
- The map update button's tooltip says what it does: it no longer mentions the view panel, which it does not send.
- The map style fill worker is told all twelve node kinds.

### Removed

- The plugin's TypeScript code, its map pane inside Claude Code outside Whaybench and its `/map` command: the
  Whaybench map panel shows the map. Outside a Whaybench tab the map tool has no session to write to.
- Requests to a session that no tab shows (one in the background): they can only be typed into a tab now.
- The context percentage in the sessions panel: nothing wrote it. Context and plan usage show in Claude Code's
  status line and under the alerts panel.

### Fixed

- The project panel shows every document of a project as a tab, also one made in a format the program no longer
  has (it is Markdown all the same); before, only documents added from a template add-on showed.
- A session the person ended no longer comes back in the session list as not running when its own end hook
  arrives.
- After `/resume` opens a session running in the background, the tab follows it again and the empty session left
  behind is stopped: the relay finds Claude Code's note of the opening in the transcript, as the old plugin did.
- Requests (Tidy, the input panel, project and map requests) go only to the session the person chose: the
  conversation tab in front. Closing that tab makes the next conversation tab the chosen one; with none left, no
  session is chosen and nothing is sent, instead of falling back to the session whose map changed last.
- Opening a project item's source shows that session's map read-only, as an imported map, instead of making that
  session the chosen one.
- The map tool finds its session outside a Whaybench tab too: the hooks note the session by the agent's process,
  which is also the MCP server's parent; the tab is the fallback.

## [0.2.0] - 2026-11-12

### Added

- Memory: every change to a conversation map is appended as an event (CloudEvents 1.0) to
  `memory/events/<year-month>.jsonl` in the data directory: nodes added, changed and removed, links, and maps
  carried from another session.
- Map nodes: kind `idea`; states `dropped` and `postponed`; open or closed now follows the state (closing without
  a state means done). A memory id (`memid`) on every new node, kept through a regrouping. Times and conditions:
  `at`, `due`, and `requires` / `assumes` over node ids with ∧ ∨ ¬, checked when written. A `resolves` link closes
  the problem it settles and what is still open under it. The important mark is the person's only.
- Memory: files a session reads, changes or attaches to a node are recorded as `file.touched` events with the
  node, so every session's nodes about a file can be found by its path (`whaybench record SESSION file …`).
- Memory list: Claude Code sees, with its first prompt and after a compaction, its own open nodes and the related
  and recent nodes of other sessions (one line each, up to 100), and on later prompts only what changed for it
  since. `whaybench memory list | changes | show | search`.
- Alerts panel: Due, Marked, Changed (since the person last looked), To approve (AI suggestions), Focus (the open
  nodes of the conversation on show), On hold and Deleted. Right click: done, drop, hold, reopen, delete, mark important,
  open the session. The person's choices are written to that session's map and recorded as theirs.
- Map nodes: kinds `task`, `appointment`, `problem`, `fact` and `rule`, and a `detail` field for what does not fit
  the short label (shown on hover in the map and alerts panels, searched by `whaybench memory search`). Claude is
  told a label rule per kind.
- Deleted nodes: deleting from the map menu or the alerts panel keeps the nodes in the alerts panel's Deleted
  section instead of a `maps/deleted-…` copy. Restore puts back what that one deletion changed (nodes changed
  since stay as they are, and you are told); delete them for good by hand, or after `trash_days`
  (180 by default) at start. Claude does not see deleted nodes, and a regrouping of its map leaves them alone.
- A session takes the person's changes to its map (alerts panel, map menu) into its own view at the next prompt.
- Map styles: Route and Timeline are drawn from the event log, in the order and turn nodes were made and changed;
  Route shows a step back into an earlier branch as "Back". Kanban has an On hold column and shows dropped nodes
  crossed out under Done. Questions shows ideas, and a decision that resolves a question as its answer. Focus and
  the other styles count dropped nodes as closed.
- To approve: when Claude is not sure a node is finished or still needed, it leaves it open with `suggest`
  (done or dropped, and why); the alerts panel lists it, and the person approves or declines. Claude looks for
  such nodes when it gets the memory list, when another session's change reaches it, and on Tidy. The memory
  list shows other nodes on the same problem (a decision made elsewhere), not only one line per memory id.
- Map marks: ✕ for a dropped node and ‖ for one on hold, in the map panel, its styles and Claude's map text.
- `whaybench statusline`: a Claude Code status line showing context and the 5-hour and weekly plan usage with
  their reset times; the same numbers appear under the alerts panel. The README shows the setting to add.
- An imported map can be handed to the session whole: Refer to the map (whole map), next to Refer to the map
  (this branch).
- `keep_days` and `keep_mb` in the data directory's `settings.json` limit how long and how much conversation
  records (maps, answers) are kept. No limit by default.
- The map of a conversation Claude Code no longer keeps (it deletes old ones, after 30 days by default) is saved
  once to the maps folder as `auto-map-…`, titled with the session's name.
- Related answers from a node of an imported map, when the answers of the session it was saved from are kept
  in this data directory.

### Changed

- Every text only the agent reads (map update and other requests, the map fill worker's prompt, Codex's context,
  the carried-map prompt) is English whatever the interface language, in `config/lang/ai.ftl`; language add-ons
  hold only what people see. The map update request is short: keep every node and id, and put the whole
  conversation, answers included, into the node fields.
- The `blocks` link is gone: what a node depends on is written as `requires`.
- Open or closed comes from a node's `state` alone (`done`, `dropped` and `removed` close it); `isOpen` is no
  longer stored or recorded in events. An agent that still sends it is read as a state, and a stored
  `isOpen: false` of an older map becomes `done` the next time that map is written. Claude reopens a node with an
  empty state.
- A saved map's file name, and that of the copy kept when a map is imported, no longer takes its title (which
  is in the interface language): it is `map-<first change>-<last change>.json`, or `branch-<node id>-…` for a
  branch. The times are when the conversation began and was last active, to the minute (session records now
  keep their start). The imported copy also keeps the session the map came from.

### Removed

- The `visits` field of map nodes: where the conversation went is worked out from when nodes were made and changed.

### Fixed

- Joint lines no longer show over a floating panel whose edge snapped to a wall or a neighbour: a panel lying over
  others floats even when one of its edges touches.
- No more Mesa warnings ("ZINK: failed to choose pdev", "failed to get driver name for fd -1") when apps start
  inside Whaybench: they are told to draw on the CPU, which they did anyway, since Whaybench does not offer them a
  GPU device yet.
- The data directory has its `maps` folder from the start, so Open map shows it before any map was saved.

## [0.1.1] - 2026-11-10

### Changed

- The repository front page and README now describe the related answers of a map node.

### Fixed

- Menus in the panels no longer take the look of the desktop's GTK theme (such as Breeze on KDE), which
  sometimes showed a menu in other colours with a frame around it.

## [0.1.0] - 2026-11-10

### Added

- First public release: a nested Wayland compositor with free-layout panels for
  a conversation with Claude Code or Codex CLI, its conversation map, sessions,
  answers, input, files, views and projects.
- Themes, languages (English, Korean), map styles and project document
  templates as add-ons.
