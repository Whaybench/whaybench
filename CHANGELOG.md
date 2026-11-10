# Changelog

All notable changes to Whaybench are written here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- An imported map can be handed to the session whole: Refer to the map (whole map), next to Refer to the map
  (this branch).
- Related answers from a node of an imported map, when the answers of the session it was saved from are kept
  in this data directory.

### Changed

- A saved map's file name no longer takes its title (which is in the interface language): it is
  `map-<first change>-<last change>.json`, or `branch-<node id>-…` for a branch, to the minute.

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
