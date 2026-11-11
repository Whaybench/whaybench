# Changelog

All notable changes to Whaybench are written here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

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
- Alerts panel: nodes with a time or due date, the person's marked ones, what changed since they last looked, AI
  suggestions, held nodes and the trash. Right click: done, drop, hold, reopen, move to trash, mark important,
  open the session. The person's choices are written to that session's map and recorded as theirs.
- Map nodes: kinds `task`, `appointment`, `problem`, `fact` and `rule`, and a `detail` field for what does not fit
  the short label (shown on hover in the map and alerts panels, searched by `whaybench memory search`). Claude is
  told a label rule per kind.
- Trash: deleting from the map menu, or moving to the trash from the alerts panel, keeps the nodes in the alerts
  panel's trash instead of a `maps/deleted-…` copy. Restore puts back what that one deletion changed (nodes changed
  since stay as they are, and you are told); delete for good or empty the trash by hand, or after `trash_days`
  (180 by default) at start. Claude does not see the trash, and a regrouping of its map leaves it alone.
- A session takes the person's changes to its map (alerts panel, map menu) into its own view at the next prompt.

- An imported map can be handed to the session whole: Refer to the map (whole map), next to Refer to the map
  (this branch).
- `keep_days` and `keep_mb` in the data directory's `settings.json` limit how long and how much conversation
  records (maps, answers) are kept. No limit by default.
- The map of a conversation Claude Code no longer keeps (it deletes old ones, after 30 days by default) is saved
  once to the maps folder as `auto-map-…`, titled with the session's name.
- Related answers from a node of an imported map, when the answers of the session it was saved from are kept
  in this data directory.

### Changed

- The `blocks` link is gone: what a node depends on is written as `requires`.
- A saved map's file name, and that of the copy kept when a map is imported, no longer takes its title (which
  is in the interface language): it is `map-<first change>-<last change>.json`, or `branch-<node id>-…` for a
  branch. The times are when the conversation began and was last active, to the minute (session records now
  keep their start). The imported copy also keeps the session the map came from.

### Fixed

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
