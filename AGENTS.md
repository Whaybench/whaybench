# AGENTS.md

## Build and test

    tools/fetch-smithay.sh
    cargo test --workspace
    cargo run --release -p whaybench -- --app alacritty

After changing `mod/context-map`, run `claude plugin validate mod/context-map`. It does not catch every syntax
error, so also run one short session with a temporary data folder and check that it logs:

    d=$(mktemp -d); WHAYBENCH_DATA_DIR=$d claude -p "Reply ok." && ls $d/logs/sessions

## Rules

- Whaybench runs as a normal user. Do not require root or sudo.
- User data lives in one directory: `~/.local/share/whaybench` by default,
  `--data-dir` to change it. No fixed paths in code. Paths stored in files
  are relative to that directory.
- Only `whaybench map` writes map files. Only `whaybench record` and
  `whaybench hook codex` write session records, states and answers.
  `mod/context-map` is the Claude Code connector and calls these commands.
- Internal values (setting keys, stored values, states, log messages) are in
  English.
- Text shown on screen or sent to the agent goes in Fluent files:
  `config/lang/en.ftl`, and `addons/languages/<lang>/messages.ftl` with the
  same keys in the same order.
- Code holds only the default theme and map style. Others are add-on folders
  under `addons/`.
- Apps inside panels cannot move or resize their panel: Whaybench ignores
  those requests and draws the title bar itself. A panel moves by its title
  bar and is resized by its borders.
- Do not copy code from GPL projects.

## Versions and releases

Versions follow Semantic Versioning:

- Major (2.0.0): a change in how the program is built, such as a new way of
  connecting agents.
- Minor (0.2.0): new features, or a change users have to follow (a data
  format, a command).
- Patch (0.1.1): fixes, and following a change in an external component.

Every change worth telling goes into `CHANGELOG.md` under `[Unreleased]`.

A release:

1. Raise the version in `Cargo.toml` and `mod/context-map/.claude-plugin/plugin.json` together.
2. Turn `[Unreleased]` into the new version with its date.
3. Write `releases/<version>.toml` with the component versions it was tested with.
4. Commit and push the tag `v<version>`. The Release workflow builds the programs and publishes them, with
   `install.sh` and that changelog section, as a GitHub release.

## Done

- `cargo test --workspace` passes.
- A change on screen is checked by running Whaybench and taking a screenshot
  (`whaybench ctl shot --out /tmp/a.png`, outside the repository: a screenshot holds the conversation on screen).
- Test results are reported as they are.
