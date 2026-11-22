# Changelog

All notable changes to Whaybench are written here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions are numbered as [AGENTS.md](AGENTS.md#versions-and-releases) says: a patch unless the maintainer
names another step.

## [Unreleased]

## [0.9.0] - 2026-11-21

### Added
- Project folders (Briefsafe, first part): `whaybench-project new ID NAME --folder DIR [--from SRC]` makes the
  project folder as a btrfs subvolume (a plain folder where btrfs is not there), copies a folder in with reflinks
  where it can, and takes its first snapshot. The project's data (map, documents, a copy of every document
  template) is kept in the folder, in `.whaybench/` laid out like a data folder and left out of git; the data
  folder keeps only a stub with its name and folder, and `whaybench-project list` shows it as `kind: briefsafe`.
  Conversations of such a project find their transcripts in its own Claude folder, and the map filler does not hand
  them to a worker outside the project. The panels list and show a project's sessions (map, answers, states, views,
  permission questions, status messages) from its folder and write what they change there; the answer panel does not
  keep a project's answers in its history file. From a project's folder the compositor takes only requests to bring a
  panel forward, and lets its agents type only into the same project's conversations.
  `whaybench-project open ID` makes an open project's data seen at `cache/open/<id>` of the data folder (the same
  path on the host and in its container) and, where podman and an official image of the host's distribution and
  version are there, starts the project's container: the project folder, the open data place and the host's programs
  (read-only, with the plugin code), nothing of the host's data or agent settings, the resting data hidden. Its
  agents get their own Claude Code folder, made once from the person's settings and plugin lists, so their
  conversations and background sessions stay in the project. `close` stops it and the data is no longer seen.
  A conversation tab whose folder is an open project's runs its agent in the project's container (`podman exec`),
  with the project's data folder and, for Claude Code, the long-lived token kept in the keyring
  (`whaybench token set`, from `claude setup-token`); values go through podman's environment, not its command line.
  Claude Code's own updater is off in the container (the programs are the host's, read-only).
  Whether a project is open is the host's link alone, never a file its agents can change; a project conversation
  (by its record or its folder) runs in the project's folder and only in its container, and gets no tab rather than
  an agent on the host when the project is closed or its container cannot start (without podman or an image it runs
  as before). `open` starts the container before the open place appears and fails when it cannot; `close` keeps the
  project open when the container cannot be removed. Starts are serialised per project, the first open shows the
  image download, container names include the data folder, folders are compared as real paths, a moved or replaced
  project folder is not opened, and a project left open or running by a crash is put back in step at start. Writes
  about a closed project's session go nowhere instead of to the host's data folder.
  Host programs touch an open project's data only beneath it (openat2, no links followed), read only regular files up
  to a limit without blocking on pipes, and write by renaming a new file of an unguessable name into place: request
  folders, session records, answers, views, states and the answer panel's presence. A project's view requests open
  only paths inside the project (by real path); its database is not opened through a link and runs nothing a file
  could carry (defensive mode, untrusted schema); a session record counts only under its own file name; one
  conversation's answers are read from its own root only.
  A tab looks for its session only in the root it runs for (the terminal notes a project tab's project), so a
  project's record cannot take a host tab. A project's agent may type into another conversation only if both the
  asked one and where it lands after moves are the project's own, only plain text, and it is shown as from the
  project; a project's permission question counts only for a session of that project.
  The project container runs with no capabilities and no gaining of privileges, has a writable home of its own (tool
  caches, settings) and in `~/.local/bin` only links to the agents' programs; the person's `~/.local`, plugin lists
  and other folders are not mounted beyond the programs, plugin code and a rustup toolchain, read-only. The project's
  Claude Code folder gets only the settings agents need (plugins, language, model, permissions; not `env`, the key
  helper or hooks), written without following links. `LANG` reaches the agents, Ctrl-P is not taken by podman,
  closing a tab ends its agent in the container, and a missing token or a keyring that does not answer is said in the
  status line.
  Nothing of a project stays with the host: its answers are marked as the project's when they arrive and never saved
  to the host's answer history, and leave the Answer panel when the project closes, as its pages leave the View panel
  and its map the Map panel. Exported, imported and carried maps and the Files panel's AI-edit pages are written in
  the project's data; a project's map does not start a conversation outside the project. The terminal's rewind and
  move records and `whaybench-files` use the conversation's own root.
  More readers take a project conversation's own root: `session wait` reads its answers there, a source link opens
  its map there, a conversation's time span and its earlier numbers are found there (another root's records cannot
  move it), the View panel moves its pages there (not through links), and Notifications lists every open root's
  permission questions. Keep Map from a project's conversation is refused outside it. Opening a project runs the
  old-conversation clean-up for its data (`whaybench tidy`, by the project's own keeping settings) inside its
  container.
  Snapshots: a project's container is paused while one is taken, so it holds one moment of every file. Only snapshots
  of this project count (by the id in their data's marker), so those left beside a reused folder name are never
  restored or pruned. Restoring refuses while the project is open, refuses when the folder cannot be fully checked
  for inner subvolumes, and puts the restored folder in place by a rename that replaces nothing. A first snapshot that
  fails is a warning, not a failed project, and a folder made by a failed creation (or set aside by a restore) is
  removed even without `user_subvol_rm_allowed`.
  Creating: a Briefsafe project takes no `--parent` (said, instead of failing in its own empty store), a plain project
  cannot take a Briefsafe project's id, a folder copied with `--from` does not bring its `.whaybench` (another
  project's data, or a link to one), and `.whaybench` is kept out of git in a repository around the folder too.
  `whaybench-project forget ID` takes a closed Briefsafe project off the list and leaves its folder. `snapshot`, `snapshots` and `restore` take, list and go back to
  read-only snapshots kept beside it (`.<name>.snapshots/`, 30 kept, the oldest go first). Restoring keeps the state
  before as a snapshot and refuses while programs work in the folder or a subvolume is inside it. A folder inside
  another project's folder is refused. Deleting snapshots needs the file system mounted with
  `user_subvol_rm_allowed`; without it the extra snapshots are kept and the answer says so.
- The answer panel has ⟳ (the newest answer) and ↶ (the answer shown before ⟳ was pressed, or the node's answers).
- The answer panel moves to a new answer only while its newest one is shown; while an earlier answer or a node's
  answers are shown, a new one only joins the list. The same conversation under a new number (/resume) is not taken
  for another tab, an answer dropped with ✕ no longer comes back for a moment, and the panel opened from a map node
  keeps that node's answers.
- The computer does not go to sleep while an agent works (Preferences → "Prevent sleep while an agent works", on
  at first): a turn worked or waiting on the person (a command approved in the terminal runs meanwhile),
  or a subagent still running after it. On Linux a `systemd-inhibit` block on sleep and the lid switch, under WSL
  Windows' `SetThreadExecutionState` held by PowerShell. The screen may still turn off and lock. A Claude Code
  session whose process is gone, or a state not written for two hours, no longer holds.

### Changed
- Windows: Whaybench's WSL distribution is Fedora (FedoraLinux-44, the release the development machine runs) instead
  of Ubuntu: WebKitGTK 2.54.2 lets the web panels draw on the GPU, and Firefox is the distribution's own package.
  Installing an input engine from inside Whaybench uses dnf there.
- Windows: ~/work is on a work disk of its own (%LOCALAPPDATA%\Whaybench\work.vhdx, btrfs, or XFS without the
  btrfs module), so projects stay when Whaybench or its Linux system is removed, and working copies can share their
  data. The installer makes it once with administrator rights (the same Windows account), and each start attaches it
  again through the scheduled task Whaybench\MountWork, which only attaches it: mounting it on ~/work runs without
  those rights. Only a blank disk of exactly 1 TiB (the size work.vhdx is made with) is ever formatted. ~/work is the
  disk's folder work: what was in ~/work before is copied there whole (or not at all) and kept as ~/work.before-disk;
  files written while the disk was away are moved to ~/work.unmounted-<date>. Without the disk Whaybench starts on the
  distribution's own disk and says so. Uninstalling keeps the disk, tries to remove the task and says when it cannot,
  and asks about the Linux system by where ~/work really is.
- Windows: the installer needs WSL 2.4.4 or newer, and a Windows account named like a Linux system account (root,
  adm...) gets the Linux account whaybench.

### Fixed
- A background subagent's tool call after the turn ended no longer marks the session working again.
- The input panel draws with GTK's Cairo renderer: under the GL one the top of a just-typed glyph went missing
  until the next full redraw (themes on the default font, Noto Sans CJK). `GSK_RENDERER` set by hand still wins.

## [0.8.11] - 2026-11-20

### Added
- An ask waiting in the answer panel shows in the session list as awaiting the person (a held-back Codex question
  too) and at the top of the notifications panel ("Waiting for a choice"; a press shows the answer panel). The
  answer panel comes forward for a card; the notifications panel never does, so it never covers it.
- The answer panel takes an agent's questions and permission asks: Claude Code's AskUserQuestion and tool
  permissions, and Codex's tool permissions, can be answered there (allow, deny, or the options chosen). Claude
  Code keeps its own terminal dialog open beside the card, and whichever is answered first counts; a card goes
  when its call runs or fails, or when the turn ends. Codex asks in the terminal only after the panel: "Answer in
  terminal", ten minutes without an answer, or a closed answer panel. Codex's own questions are held back, shown
  in the panel, and the answer goes to the session as its next prompt.
- An app can stand behind the panels (wlr-layer-shell, background and bottom layers: swaybg, mpvpaper and the like):
  it shows as the surface. It is laid out on the whole window (under the menu bar and the status line, as a
  wallpaper under a desktop's bars) and again when the window is resized; the newest is on top. A layer above the
  windows, or one that moves there, is closed.
- Where no panel is, the background app takes the pointer: something behind the panels can be touched.

### Changed
- The answer panel names a prompt the person did not type: a finished background task as "Task notice ·
  <its name> · done/failed/stopped", a subagent's report as "Subagent report", pasted text without its wrapping,
  and a request of Whaybench's by its bracketed name. The session list leaves task notices and reports out of the
  last question.

## [0.8.10] - 2026-11-19

### Changed
- A frame is built only when something happened (an app's request, input, a command) or something moves by itself;
  else the picture is looked at 10 times a second, not 60: an idle window spent several percent of a core building
  frames with nothing new. An app's pending events go out after every turn of the loop.
- Drawing makes no GPU fence for each picture it draws when it has one graphics context (Smithay patch
  `patches/smithay-gles-fences.patch`): a driver that flushes at each fence (VMware SVGA) spent more on the fences
  than on the drawing, about 20% of a core with a moving theme.
- `panel_opacity` and `back_opacity` now fade only a panel's frame and fill: its title, tab names and app content
  stay solid, so text no longer fades with the panel. The conversation terminal and the panels' own windows and pages
  leave their background to the panel when it is see-through, the input field too. A panel moved out to the desktop
  keeps its background, and a file the view shows as it is (HTML, PDF) keeps WebKit's white under it. An app that
  paints its own background (a web page) stays opaque.

## [0.8.9] - 2026-11-18

### Added
- `whaybench record SESSION|- task start|alive|done ID [--source NAME] [--name NAME]`: a program that runs
  work for a session where no hook sees it (a review handed to another agent from a shell script) reports it, and
  the session line shows it as it shows a running subagent; `alive` keeps a long one shown past the 30 minutes after
  which a silent one is not. `-` is the session of the agent the command runs under. Claude Code and Codex are told
  of it once at the start of a session, for work they leave running in the background.
  It works inside a Codex tab too: the sandbox may write `cache/subagents` (and no more of `cache`), and the folders
  the sandbox may write under the data folder are made when the tab opens.
- The session line counts what runs besides the session by agent: `Helpers: Codex (2), Claude Code (1)` in place of
  each subagent's type and last tool.

### Changed
- Conversation map → New conversation from this map is now a submenu: Whole map, or As shown, each for an agent.
  As shown reads the main map panel at that moment (the display style, the branches folded or open) and passes on
  the nodes on screen, the screen as text with folded parts as one line, and the command that reads the whole map.
  The separate "Carried to a new conversation" choice is gone.
- Session → New conversation is a submenu of the agents too, in the same shape.
- Taking one branch of an imported map as a reference now tells the session the command that reads the map
  file, so it can look up a node's details (reasons, symptoms), not only its label.

### Added
- Themes can set the title bar's own font, weight and letter spacing (`base.title_font`, `title_weight`,
  `title_spacing`); the title bar was always the desktop's bold sans.
- A session asking the person a question (Claude Code's `AskUserQuestion`, Codex's `request_user_input`) shows as
  awaiting the person, with the desktop notification, until the answer comes back.
- A Codex conversation forked from another (`codex fork`, `/fork`) takes the map it branched from; the other goes on
  with its own. The source comes from the first `session_meta` of the new transcript (`forked_from_id`).
- Codex sessions show their subagents in the session panel too: the Codex add-on now registers `SubagentStart` and
  `SubagentStop`. Codex asks once, in `/hooks`, to trust the two new hooks.

## [0.8.8] - 2026-11-18

What a session scheduled, an older Claude Code under a session, and Claude Code's name for a session now show in
Whaybench; dates and times come from a maintained library.

### Added
- Claude Code's name for a session (in `claude agents`, `/resume`) follows the session's name in Whaybench: the hooks
  give the map's main topic as `sessionTitle` when it differs. A name the person gives in Claude Code (`/rename`,
  `--name`) wins: Whaybench shows it too and no longer sends its own.
- A session whose process still runs an older Claude Code than the one installed (an update leaves sessions that
  are working, waiting or shown in a tab on the old one) has `⟳` before its name in the session panel, and its
  right-click menu restarts it on the new one (`claude respawn`), or says why not now (working, awaiting the person,
  a /loop wake-up pending, already restarting). Only Claude Code's own version files are compared.
- What a session has scheduled for itself (Claude Code's cron prompts, a /loop's next wake-up) shows in the
  notifications panel's Due section among the dated nodes, and as `⏲ n` on the session's line. The hooks follow the
  scheduling tools, as Claude Code keeps the jobs only in the session; the list is kept in `cache/schedules/`. It
  follows what Claude Code does with them: carried by /background, brought back by --resume (not a wake-up).

### Changed
- Dates and times come from jiff instead of chrono, which is no longer maintained (RUSTSEC-2026-0543). Every time
  shown and every time the records keep reads as before: the records' RFC 3339 form, compared as text, is unchanged.

## [0.8.7] - 2026-11-17

A waiting session is told on the desktop, subagents show in the session panel, and a restart no longer splits a
conversation in two.

### Added
- A desktop notification when a session starts waiting on the person (a permission, a question) while Whaybench is
  not the window in front: through the desktop's notification service, under WSL as a Windows notification.
- The session panel shows a session's running subagents under it, each with its type and last tool. The connector
  now also sends Claude Code's `SubagentStart` and `SubagentStop`; the list is kept in `cache/subagents/<session>/`.

### Fixed
- The status line Whaybench gives Claude Code runs `whaybench statusline`, not `whaybench-term statusline`, which
  stopped with "unknown argument": context and plan usage reach the notifications panel again.
- A conversation moved to the background could come up with an empty map: when its new number started before
  Whaybench followed the move, the empty map made at the start kept the old one from being copied. Moves are now
  found only by Claude Code's "continued in" note; the start hook's own note (`cache/forks/pending.json`), which could
  no longer work because the new number has no transcript when it starts, is gone.
- A restart split a conversation in two: its tab came back on the old number while Claude Code carried the
  conversation on in the background under a new one, which answered a background task's end on its own. Claude Code
  now writes the /background command's own rows after its "continued in" note, and Whaybench took them for the
  conversation going on under the old number; they are left out now, so the tab attaches to the new one.
- A session was marked as awaiting the person on any Claude Code notification but the idle one, such as a sign-in
  or an agent finished; now only a permission prompt (a teammate's too), a question (elicitation) or an agent
  needing input does.
- A status line or language the person set in the project's `.claude/settings.json` or `settings.local.json`, or
  in `$CLAUDE_CONFIG_DIR`, is kept: only `~/.claude/settings.json` was looked at, and Whaybench's own covered the others.

## [0.8.6] - 2026-11-17

Menus close when they should and take the item chosen, and small Hangul keeps its strokes.

### Fixed
- Small Hangul in the panels (the menu bar at 10 pt, for one) lost the flat bottom stroke of a final consonant (rieul):
  the panels now fit glyphs to the pixel grid (slight hinting) instead of leaving it to GTK.
- A menu opened again quickly (clicking a submenu such as Theme ▸ closes and reopens it) was not noted as open: a
  press outside left it on the screen, and a press on one of its items closed the menus before the item was
  chosen, so the choice did nothing until tried again.
- `whaybench ctl press|release|pointer` gave every event the same time and could hand a press and its release over
  in the wrong order, so apps saw test clicks as double clicks or a button held down.

## [0.8.5] - 2026-11-16

Easier first steps: the first layout puts the browser in the middle, where the sign-in page opens, and a Windows help
page covers a WSL setup that stops. Codex in Whaybench shares its login with `~/.codex`.

### Added
- A Windows install help page, `docs/windows.md`: turning on virtualization in the BIOS and the WSL features, and
  what the common WSL error codes mean. Both READMEs link to it.

### Changed
- The first layout puts the view panels (the browser with them) in the middle, full height: the map, sessions and
  input on the left, the conversation and answer on the right. A sign-in page now opens where the eye already is.
  Panels -> Reset layout gives it too.

### Fixed
- Codex in Whaybench no longer keeps a login of its own: signed out in Codex elsewhere, it said the token was revoked
  and showed no sign-in. Its login is now a link to `~/.codex/auth.json` (an agent add-on's `home.link`): signing in or
  out there holds here, and signing in here holds there. Signing out here asks to sign in again here and leaves the
  file there alone.
- `whaybench ctl session type` and `session end` refused a conversation that had carried on under a new number (after
  a restart, for one) with "No conversation tab": they follow the move now, as `session show` and `session wait` do.
- A settings window that grew wider ran past the right edge of the screen: it is kept centred and on the screen.

## [0.8.4] - 2026-11-16

The conversation map keeps a truer record with less text: the map tool tells the agent its record slips and like
nodes of other conversations, memory ids get short numbers, and less rides on every prompt.

### Added
- Memory ids have short numbers (`#12`), given once in the order the ids came and never again. The memory list,
  change lines and hints show them, which takes about a fifth off the list a conversation starts with; `memid`, a
  link's `to` and `whaybench memory show` take a number or the full id, and a call that uses a number is told the
  label it stands for, so one wrong digit is seen; ids an older program wrote get their numbers when a conversation starts.
- When an agent adds a node without a memory id and another conversation has a node with a much like label, the map
  tool's reply names it (at most 3 a call), so the agent can give the same memory id if it clearly is the same
  problem. Nothing is linked by the program.
- After Claude Code compacts a conversation, the agent gets this map's open nodes (with what is above them) and
  where it stands once, so it keeps using the ids it gave instead of adding the same nodes again.
- The conversation map tool tells the agent what its own call left against the record rules: a question closed
  without its answer, a conclusion without a reason, a "question → answer" filed as a task, options left open under a
  closed question, a topic or task with nothing open under it. A clean call still answers only "Map updated."; what
  the person closed, dropped or approved is never questioned: only what the call itself closed, named or relabelled. The tool's text also asks to settle the question node
  itself, to read an approval as a decision, and to leave the person's decisions as they are.

### Changed
- Less text rides on every prompt: the full conversation map note goes in when a conversation starts, resumes or is
  compacted, and each prompt gets one line, since the rules are in the tool's text. Memory change lines say who by
  4 characters and only the new label, state or kind (other fields by name), and another map taking this
  conversation's nodes on is one line per session instead of one per node. The tool's input no longer repeats the
  node fields for `nodes` nor lists `inherit`, and the note on asking other conversations is shorter.
- An agent add-on can name a one-line `prompt_context` file for each prompt, its `context` then going only at session
  start; the person confirms both before the add-on runs. Codex now gets its whole note once and one line with each
  prompt.

### Fixed
- Conversation map: a `resolves` link to another conversation's node (by its memory id) now settles that node and what is
  open under it; in the agent's own call a link to a node that exists nowhere is refused instead of dropped without a
  word. Links an agent sends are added to the
  node's (`links: null` clears its links on this map), so adding one no longer erases the others. The map-filling worker can no longer
  settle nodes or empty a node's links. A start or prompt whose hook fails still gives the agent the map note.
  The tool's text now tells every agent, Codex too, to turn a settled question into its conclusion.
- A restart with several Claude tabs no longer hides a conversation. Claude Code could note one session as carried on
  in another conversation's background number; Whaybench now ignores that note when the number already holds another
  conversation, or when the conversation went on under the old number after it.
- Title bar text sat low with CJK fonts (their line box has more room above): it is now centred by a capital's height.

## [0.8.3] - 2026-11-16

Projects get a change proposal template and archived documents, and on Linux one command installs everything.

### Added
- A document template "Change proposal" for projects: one change agreed before the code, with the requirements it
  adds, changes or removes as WHEN/THEN scenarios, the design, the tasks and the checks, merged into the
  specification when done (after OpenSpec's change folder).
- A document template can be added to a project again: the copies after the first get a number ("Change proposal
  2"). Each document records the template version it started from.
- A finished document can be archived (`whaybench-project archive`): it leaves the tabs and opens from
  **+ Document → Archived documents**, shown only when there are some.

### Changed
- On Linux, install.sh finds the distribution and installs the system packages it needs with sudo once you answer
  `y`, so the README starts with the one install command. Without a terminal (the update at start), without sudo or on
  another distribution it shows the command, as before. It knows Fedora (Silverblue and Kinoite through rpm-ostree),
  Ubuntu, Debian, Arch, openSUSE, AlmaLinux and Rocky Linux.
- install.sh tries a download again up to three times, and when `~/.local/bin` is not on PATH it shows the line to add
  for your shell (zsh, fish or bash).
- The README says how to remove the Claude Code plugin too.

### Fixed
- install.sh runs nothing until the whole script has downloaded: a download cut short under `curl | sh` no longer runs
  half of it.
- When adding the Claude Code marketplace failed (no network, or settings that name another source), install.sh
  stopped there and the update at start counted as failed: it now goes on and shows the plugin command.
- On Ubuntu, `apt install firefox` installs the snap, which cannot open in the view panel: install.sh counts a snap
  Firefox as missing and installs Firefox from Mozilla's archive, put before Ubuntu's (as the Windows setup does).
- On Debian the view panel did not find its Firefox, whose process is `firefox-esr`.
- On Debian the system package command asked for `firefox`, which Debian does not have: it is `firefox-esr`. The
  library check found `ldconfig` only when `/usr/sbin` was on PATH, so a normal Debian account was always told
  GTK and WebKitGTK were missing.

## [0.8.2] - 2026-11-15

The Windows setup says why it stopped and finds virtualization turned off before it installs WSL.

### Fixed
- When the Windows setup stopped, its window closed at once and nothing kept the reason: it now waits for Enter, the
  setup writes `%LOCALAPPDATA%\Whaybench\install.log`, and Whaybench Setup's message says where it is.
- The Windows setup checks that virtualization is on before it installs WSL, and says to turn it on in the BIOS:
  turned off, it used to fail later, after a restart, with a message about the internet connection.

## [0.8.1] - 2026-11-15

Glass no longer keeps the window redrawing, see-through themes show the opacity they set, and fixes to the new window
frame and the WSL GPU check.

### Fixed
- Under WSL, WebKit 2.54.0 and 2.54.1 kept the GPU out of their web sandbox too: the panels with web pages draw on
  the GPU from WebKit 2.54.2, which first binds /dev/dxg, not from 2.54.0.
- With glass on, a still window was drawn and sent again and again: the glass keeps what lies under it until that
  changes, and the window's surface is no longer taken off between frames.
- Each floating panel's glass shows what lies under that panel, lower floating panels included.
- A see-through surface laid its opacity twice (50% showed as 75%), and a see-through panel had a second fill under
  its app.
- A press at the left edge of an open menu chose the item instead of starting to resize the window.
- Two presses on the title strip far apart, quickly, no longer maximise the window.
- `--no-session`, which runs without the menu bar, keeps the desktop's frame, so the window can still be moved and
  closed.

## [0.8.0] - 2026-11-15

Themes get light, depth, see-through panels and glass, the window draws its own frame, and under WSL the window
and the apps draw on the GPU.

### Added
- Light and depth for themes, each in percent (`addons/themes/README.md`): a shadow that falls below a panel, and
  higher for the front one (`shadow_drop`, `back_lift`); a border lighter at the top and darker at the bottom
  (`border_light`, `border_shade`); a glossy title bar (`bar_gloss`, `bar_shine`, `bar_shade`); and one light above the
  window's top middle on the surface and, fainter, on the panels' frames (`surface_light`, `surface_vignette`,
  `panel_light`, `panel_shade`). The default theme uses them; at 0 a theme stays flat, so the other themes look as
  before.
- See-through themes (`addons/themes/README.md`): `window_opacity` lets the desktop show through the surface behind the
  panels, `panel_opacity` makes the panels themselves see-through (frame, title, tabs and app), and `back_opacity`
  more so for the panels behind the front one. `glass_blur` gives floating panels and dialogs glass: what lies under
  them shows blurred. The window asks for 8 bits of alpha first. The default theme stays opaque.
- `dock_refraction`: while a panel is docked, the five targets on each panel enlarge what lies under them towards
  their middle, like a drop of water; the default theme at 40.

### Fixed
- The input panel's hint names the keys that switch the input language as fcitx5 has them (it said Shift+Space,
  which the Windows setup no longer sets).
- Under WSL the input method's own texts (its names, an engine's settings) come in Whaybench's language: the setup
  makes the Windows display language's locale, and Whaybench starts fcitx5 with it.
- Codex's sign-in opens in Whaybench's browser, as Claude Code's does: the apps inside get BROWSER set to the same
  way to the panel's browser as xdg-open (under WSL Codex went to the Windows browser). One browser, the one the agent
  sees too.
- `whaybench-view detach` and `attach` wait for the move and say where the browser is, so the agent knows (they
  returned at once without a word).

- Under WSL the window and the apps inside draw on the GPU through Mesa's d3d12 driver (GALLIUM_DRIVER=d3d12, unless
  set otherwise); Mesa took the CPU before. With WebKit older than 2.54, which leaves the GPU out of its web process
  sandbox, the panels that show web pages (answer, view, project, map) keep drawing on the CPU, or their pages stayed
  blank.

### Changed
- The window draws its own frame instead of the desktop's (or, where the desktop has none, as under WSLg and GNOME, a
  plain fallback bar): the menu bar is the title strip, with minimise, maximise and close at its right. Drag the empty
  strip to move the window, double-click it to maximise, and drag the window's edges to resize it.
- Theme settings are ordered by how much of the screen each part changes: base colours, fonts, panels, shadow
  and depth, light, title bar, border, surface, joint lines, docking.
- Input settings list the switch keys one per line under the + that adds one.

## [0.7.0] - 2026-11-15

### Added
- Panels other than the conversations and the input panel (map, sessions, files, answer, notifications, project,
  views) go out to the desktop the same way as the browser: dragged by the dot onto the band at the window's edge,
  each runs again out there with its command line. The input panel's title bar shows "↩ <name>" for each, which
  brings it back where it was. They end with Whaybench.

### Added
- The browser can go out to a window of its own on the desktop and come back, the same Firefox with its tabs and
  sign-ins: drag its panel by the dot onto the band along the window's edge, or ask the agent (`whaybench-view
  detach`). The others take its room; while it is out, the conversations' panel shows "↩ Browser" on its title bar,
  which (like Panels → Browser) brings it back to where it was. The panel Firefox restores its session at each start.
- A panel move can be undone for 10 seconds by the ↩ on the moved panel's title bar: the panels that move changed go
  back as they were, other moves stay.

### Changed
- Docking a panel: the middle of a title bar shows a dot; dragged by it, every other panel shows five soft target
  circles (middle: into its tabs; at each edge: beside it), and the one the dot is in stands out. Dragged anywhere else
  a panel only moves. This replaces dropping on a title bar and the bands at the edges with their wait. The dot and
  the circles take their colour and size from the theme (`frame.dock_dot`, `dock_dot_size`, `dock_ring`,
  `dock_ring_size`). A drop beside a panel takes two circles deep with its circle in the middle (at least a panel's
  minimum, at most half), not the dropped panel's size.
- The colour emoji font is installed with the rest (the panels draw files and folders as emoji): by the Windows setup,
  and named in the Linux install commands.
- The Windows setup installs fcitx5 without its recommends (its setting program, the GTK and Qt input modules,
  im-config): Whaybench's apps type through the compositor and Whaybench has its own input settings.

### Fixed
- The browser no longer says "Firefox is already running, but is not responding": a lock left in its profile by a
  Firefox that is gone (seen under WSL) is taken away before it starts, and a click on Panels → Browser while it is
  still starting does not start a second one.
- The browser's address bar shows all the time; it showed only once focused (Ctrl+L), so it looked missing. The tab
  bar stays hidden.

## [0.6.6] - 2026-11-15

### Changed
- The Windows setup no longer picks an input engine per language: Whaybench offers the language's engine when it
  starts in it. It installs the Noto fonts for every script (CJK included) whatever the language, and leaves fcitx5's
  switch keys at their defaults.
- Ctrl+wheel on a title bar makes all title bars larger or smaller (kept in settings.json `title_zoom`); the bar's
  height follows its text. The default title text is 14 px (was 13).
- Dialogs and settings windows open at their content's size and can be resized; once resized they keep that size.
  A dialog opened from the menu bar (installing language support) now opens as a dialog, not as a big panel.
- The install commands no longer name GTK 4 (`gtk4`, `libgtk-4-1`): WebKitGTK depends on it, so it comes along.
- README: the install section names Windows and Linux side by side. The Windows section no longer says an agent
  cannot become root (programs in WSL can start Windows programs); it quotes the agents' own security guidance.

### Added
- Settings → Input…: the input methods of fcitx5's current group (move up, remove, add), the switch keys, and each
  method's own settings, drawn from the form fcitx5 describes for it.
- Input settings list the engines the distribution has and this system has not installed (asked of apt-cache or
  PackageKit each time); one picked is installed and added to the group.
- A language add-on can say what it types with: `input` in `language.json` gives the keyboard layout and the
  fcitx5 engines with their packages (`addons/languages/README.md`). The Korean add-on declares `hangul`.
- Choosing a language whose input engine is not installed offers to install it (PackageKit on a desktop, `wsl.exe` as
  root under WSL), then adds the engine to fcitx5's input method group so the language types at once.

### Fixed
- Settings → Language ticks the language chosen: it kept the tick on the one in use until the restart, so a choice
  looked lost. In the language, theme and display-style menus the tick moves as soon as one is picked (it waited for
  the pointer to leave the menu bar).
- An add-on's package name is installed only when it is a package name: one starting with `-` was an option to
  apt-get, which can run a command as root.
- The confirmation to install language support says when the language add-on does not come with Whaybench.
- Talking to fcitx5 never starts it: one started by the session bus has no display and breaks composed input.
- Choosing a language no longer holds the menu bar while fcitx5 loads a new engine.
- The package lists are updated before an engine is installed (apt-get under WSL, PackageKit on a desktop: its lists
  went stale after a change made with dnf); a failed install says why in the log.
- The notifications panel keeps its scroll place when its list changes: deleting a node jumped to the top.

## [0.6.5] - 2026-11-14

Under WSL, 0.6.4 made a window flash and keys repeat while reading the Windows clipboard; this fixes it.

### Fixed
- Under WSL a window flashed up and keys repeated: reading the Windows clipboard with wl-paste showed a window of its
  own for a moment (WSLg has no data-control), which took the keyboard and gave it back, and each return read again.
  Under WSL the clipboard is now read from Windows through PowerShell; elsewhere the one-shot read waits 3 seconds
  before it may run again.

## [0.6.4] - 2026-11-14

Agents ask each other across conversations, Codex included, and Korean input and the Windows clipboard work under
WSL.

### Added
- `whaybench ctl session wait ID [SECONDS]`: after `session type`, waits for that session's next answer and prints
  it, so an agent asking another can wait in the background instead of looking again and again. It ends early when
  that session stops to ask the person (a permission or an input), and says so. An answer that came between `type`
  and `wait` is still taken.
- Codex can ask another conversation too: from inside its sandbox, `session type` leaves a request in the data
  folder's `ask/`, and Whaybench, outside the sandbox, checks it (marked as from another conversation, no / command or
  ! line) and types it in. The other tabs' input stays out of Codex's reach.
- Agents are asked to keep an exchange between conversations to a few rounds and to come back to the person when it
  is answered or the way on is unclear.
- Agents are told at the start how to ask another conversation (`session type`) and wait for its answer
  (`session wait`), for when the person asks them to involve it.

### Fixed
- `whaybench memory show` and the other memory commands work inside Codex's sandbox: SQLite could not make the
  files it reads a WAL database with in the read-only data folder ("unable to open database file"); it now reads the
  file as it is on disk, read only.
- Text copied in Windows pastes into the input panel and every other panel, not only the terminal: WSLg cannot be
  watched for clipboard changes (no data-control), so the desktop's text is read when the Whaybench window gets the
  keyboard back.
- Korean and other composed input under WSL: Whaybench asked fcitx5 to attach without first checking that one
  runs, and D-Bus started a bare fcitx5 for the call, without this display's environment, so input stayed dead. It
  now asks only a running fcitx5 and otherwise starts its own.
- Korean and other composed input on Ubuntu 24.04: fcitx5 is started with `--keep`, not `-k`, which fcitx5
  before 5.1.15 lists but refuses, so it did not start at all. Without either, it is started plain.

## [0.6.3] - 2026-11-14

The map panel stops growing with every map change, and Codex is kept from overwriting what the person changed.

### Fixed
- The map panel no longer grows by about 100 MB each time the map changes: a node's menu held its own row, so
  every redraw kept the whole old tree and a copy of the map for each node.
- Codex's file changes (apply_patch) are held back like Claude Code's edits: once when a file it changes was changed
  on disk since (most likely by the person) or is open in vim, also for relative paths and a file a patch moves onto.
  Both agents are told to keep what the person changed and not to undo or "correct" it unless asked.

## [0.6.2] - 2026-11-14

Steadier data and panels, a cleaner start for new conversations (Focus passes only the open part of a map, Codex
takes the map in), Preferences, usage without setup, and basics: pinned CI actions, dependency and clippy checks,
SECURITY.md.

### Changed
- The map tool asks for where (path:line), what and the next step in the detail of a task or problem about files, so
  a later session can carry on without looking again.
- After /rewind the agent records the rewind in the map and marks what no longer holds as changed; it no longer
  removes nodes. The map stays the record of the thinking, which can be taken up again.
- CI and release workflows use their actions by commit, not by a tag that can be moved.
- CI checks the dependencies for known vulnerabilities, licences and sources (cargo-deny, `deny.toml`).
- SECURITY.md: how to report a vulnerability privately and how to check a release's signature.
- CI runs clippy and fails on a warning; the code was cleaned of its warnings (no change in behaviour).
- The notifications panel leaves out empty sections; with all of them empty one line says so.
- `work_dir` in settings.json is not used when it is the home folder, above it, or holds Whaybench's data: an agent
  started there could change its own settings and leave its sandbox. `~/work` is used instead.
- Usage shows without setting anything: a conversation tab starts Claude Code with Whaybench's status line, unless
  the person set one of their own.
- English texts with a count no longer say "1 nodes": the count follows its name ("Nodes: 1").
- "Whole map" was defined twice in the message files, so the lower-case one for a sentence was never used; each has
  its own key now, and a test catches a key defined twice.

### Added
- Conversation map → Carried to a new conversation: the display style a new conversation from the map is given in.
  Focus passes the way to where the conversation is and the open nodes, and that conversation's memory list lists
  only open nodes of others; the other styles pass the whole map. It starts as the main map's style.
- Whaybench → About → User guide opens the README in the view panel's browser.
- Settings → Preferences…: how long conversation records and deleted nodes are kept, automatic updates and the input
  method, saved in settings.json as before.
- Dialogs and settings windows take the size of their content, up to the room there is, instead of one fixed size.

### Fixed
- Windows setup reinstalls Codex CLI when only its `codex` program was copied in, without the
  `codex-code-mode-host` it needs beside it, so Codex tabs start.
- A new Codex conversation from a map takes the map in: Codex makes its session only at its first prompt, so the tab
  starts it with the request to read the map (agent add-on key `prompt`).
- The remaining state files (view lists and history, extra map panels, hidden files, usage, heartbeats, saved and
  exported maps, add-on lists, language choice, terminal settings) are written whole or not at all, so a reader never
  sees half a file.
- A state file that cannot be read is moved aside to `<name>.<time>.unreadable` and logged, instead of being written over
  with an empty list (conversation names, earlier answers, hidden files, extra map panels, view lists and history,
  node files).
- A message from one panel to another sent while the last one was being taken is no longer removed unread.
- An answer is added to its session's answer log before it becomes the latest answer, in one write, so a failed or
  simultaneous write does not leave it out of the earlier answers.
- A panel of Whaybench's own that crashes is started again, up to three times a minute; after that it is left closed
  and the status line says so.
- The list of running Claude Code sessions is asked for once per 3 seconds for all of Whaybench's processes, kept in
  the runtime folder, instead of once per process.
- The end of a turn reads the last 2 MB of a Claude Code transcript, not the whole file; checks for a session opened
  elsewhere reads its last 2 MB, a check for a conversation its first 256 KB.

## [0.6.1] - 2026-11-14

Security: signed updates installed whole with a way back, agent add-ons and cross-conversation text confirmed and
marked, the view panel kept out of an agent's reach. Codex CLI comes with the Windows setup.

### Added

- The Windows setup installs Codex CLI too (OpenAI's standalone installer, no Node), so "New conversation · Codex" works
  without installing anything by hand. Running the installer again adds it to an existing setup.

### Changed

- A conversation that asks another with `whaybench ctl session type` gets the person back when the answer comes,
  unless they moved to another conversation meanwhile.

### Fixed

- A conversation tab whose agent program is missing closed without a word; the reason now stays in the status bar.
- Security: a `whaybench-view read` request named the reply file by path, so an agent that may write `view/` (Codex)
  could make the view panel, outside its sandbox, overwrite any file of the person's. The reply is now a plain name
  in `cache/view-read/`, written as a new file.
- Security: an HTML page in the view panel could read other local files (file URLs read from file URLs).
- Security: the Firefox profile and the view's WebKit data (sign-ins, cookies) moved from `view/` to `browser/`, out of
  an agent's reach; they move once at start, or at a later start while a Firefox still runs on them.
- An update no longer replaces the programs one by one: each version is installed whole in a folder of its own
  (`~/.local/lib/whaybench-versions/<version>`), and `~/.local/lib/whaybench` switches to it in one step. The version
  before is kept, and `install.sh --rollback` goes back to it. A failed install leaves the version in use as it is.
  Hooks, command links and restarts name the stable path, so they follow the version in use.
- Security: releases are signed (minisign; the key is in the README). Whaybench updates itself only with an
  `install.sh` and a package signed for that version, and About → Update no longer runs a script piped from the network:
  it fetches and checks the release like an automatic update, then installs it at the restart. `install.sh` checks the
  signature when `minisign` is installed; the Windows setup installs minisign and checks `install.sh` before it runs.
- Security: the Windows (WSL) account no longer has passwordless sudo, so an agent running as you cannot become root.
  Root work goes through `wsl -d Whaybench -u root`; running the installer again takes the old rule away.
- Security: an agent add-on runs only as Whaybench brings it, or after you confirmed it in its tab: the program it runs,
  the settings it writes and what it tells the agent are shown, and `y` runs it. A changed add-on is asked again.
- Security: text one conversation's agent puts in another (`whaybench ctl session type`) starts with where it came
  from, and a command (`/`) or a shell line (`!`) from another conversation is refused.
- Security: every value put in a conversation tab's shell command is quoted; a map path with a quote could run a
  command.
- A view kept on one session named the number it had when kept, not the one the conversation goes on under.
- `whaybench-view read --firefox` now says the Firefox panel belongs to no session and every session reads the
  same tab.

## [0.6.0] - 2026-11-14

Agents are add-ons and chosen per conversation: Claude Code and Codex side by side, each tab and panel showing
which one it is, and the panels following the conversation you choose at once.

### Changed

- Codex is now an agent add-on (`addons/agents/codex/`): its command, settings folder, hook and MCP templates and
  guidance are files, not code, so other agents with Claude Code-style hooks and MCP can be added as folders.
  See `addons/agents/README.md`.
- `whaybench-view read` starts with whose page the view shows (the session it follows now, the session it stays on,
  or a page pinned for all), and the view's link line names the agent and session. Every session reads what the
  person sees, so an agent can tell another session's page from its own. The read's reply file moved into `view/`,
  where a sandboxed agent may remove it.
- Conversation tab titles and the input panel name the agent and the session's number (`Codex 01a1d60c`), so
  side-by-side conversations are told apart at a glance. Tab names are cut at 44 characters instead of 32.
- The agent is chosen per conversation: the session menu has `New conversation · <agent>` for Claude Code and each
  agent add-on, Claude Code and Codex tabs run side by side, a session opens again with its own agent, and the session
  list shows each session's agent. The `--agent` option is gone.
- New conversations from a map (the current one or an imported one) also offer each agent.

### Fixed

- The answer panel's earlier answers missed an answer when two came at once or came from a session not followed:
  the list now comes from every session's answer log.
- An answer to a request from another session (starting with `[`) was kept under the question before it; the
  answer log now keeps each turn's own question.
- Choosing a conversation panel switches the map, answer, view and usage at once, not up to a second later. A tab
  without a session yet (a Codex tab before its hooks report) shows them empty instead of the session seen before.
- After an update replaced the program while it ran, a new Codex tab registered its hooks and map tool under the old
  file's name ("... (deleted)") and every hook failed with exit code 127.
- A conversation tab that cannot open (its agent add-on deleted, for one) says so in the status bar.
- A file a bundled add-on no longer brings is removed from the data folder at start; files the program or the person
  wrote beside it stay.
- In a Codex tab, `whaybench-view`, `whaybench-project` and `whaybench-files` failed with "Read-only file system": the
  folders those commands write in (`view`, `show`, `projects`, `node-files`) are now Codex's writable roots; its own
  settings, the add-ons and other tabs' input stay out of reach.
- A Codex tab showed Claude Code's usage. Codex's context and plan windows now come from its session file, and each
  agent's sessions show that agent's plan; a session not heard from yet shows no context instead of another's.
- Claude Code in a conversation tab answers in the interface language (its `language` setting, given at start with
  `--settings`), so long tool work no longer drifts into English. A language set in `~/.claude/settings.json` is kept.

## [0.5.6] - 2026-11-14

Updates come by themselves, six more themes, and panel titles that read better.

### Changed

- Updates come by themselves: a newer release found at start or in the six-hourly look is downloaded, checked against
  its checksum and installed at the next start, before the window opens. The status line says when one is ready, and
  Whaybench → About offers to restart for it. `"auto_update": false` in settings.json leaves updating to the person.
- The input panel's hint also says Shift+Space switches the input language.

### Added

- Three more themes come with Whaybench: Catppuccin Mocha and Gruvbox Dark (dark), Solarized Light (light). The
  default theme already has Catppuccin Latte's colours.
- Three themes from Sanzo Wada's *A Dictionary of Color Combinations* (1933-34), bold around calm panels: Wada Dream
  (pastel pink and blue), Wada Jasper (red and green on deep indigo), Wada Carmine (yellow, carmine and green).

### Fixed

- On a desktop whose clipboard cannot be watched (WSLg), the ended `wl-paste` stayed as a zombie process; it is
  waited for now, and the log says desktop copies do not come in.
- Panel titles read better: the title font opens bold (a variable font such as Noto Sans CJK opened at its lightest
  and looked grey), and the title of a panel behind fades only as far as it keeps 4.5:1 contrast on its bar (WCAG AA).

## [0.5.5] - 2026-11-14

Whaybench's files are for this account only, and fixes for Windows and for several sessions at once.

### Security

- What Whaybench writes in its data folder (conversations' records, answers, maps, the memory database) is readable
  by this account only: files 600, folders 700, as Claude Code keeps its conversations. Files from before are changed
  at start. Apps and the agent started inside keep the person's own umask for what they write in their folders.

### Fixed

- Windows: Korean input stopped after Whaybench restarted. The fcitx5 Whaybench starts now keeps running when its
  first display closes, so the restarted Whaybench attaches to a live one.
- Starting Whaybench while it already runs on the same data folder started a second one beside it, whose tabs
  then closed conversations resumed in the new one (one tab per conversation). The new start now takes the place of
  the running one, as a restart does: it is asked to quit, keeping its panels and tabs, and stopped if it does not.
- Two sessions changing a project's map at the same moment could drop one's change from the map (the change log kept
  both). A change to the map now waits for the other to finish.

## [0.5.4] - 2026-11-13

Apps inside draw on the GPU, and Korean shows in the title bars under Windows.

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
