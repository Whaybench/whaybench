# AGENTS.md

## Build and test

    tools/fetch-smithay.sh
    cargo test --workspace
    cargo run --release -p whaybench -- --app alacritty

After changing `mod/context-map` or the hook and MCP code (`crates/wb-core/src/hook.rs`, `map_tool.rs`,
`crates/whaybench/src/mcp.rs`), run `claude plugin validate mod/context-map`, build the release binary (the plugin
calls `~/.local/bin/whaybench`), and run one short session with a temporary data folder that uses the map tool:

    d=$(mktemp -d); WHAYBENCH_DATA_DIR=$d WHAYBENCH_TAB=t1 claude -p "Add a topic 'test' with the context_map tool." \
      --allowedTools=mcp__plugin_context-map_whaybench__context_map && ls $d/sessions && cat $d/*-*.json

## Rules

- Whaybench runs as a normal user. Do not require root or sudo.
- User data lives in one directory: `~/.local/share/whaybench` by default,
  `--data-dir` to change it. No fixed paths in code. Paths stored in files
  are relative to that directory.
- Only `whaybench map` writes map files. Only `whaybench record` and
  `whaybench hook <agent>` write session records, states and answers.
  `mod/context-map` is the Claude Code connector and calls these commands.
- Internal values (setting keys, stored values, states, log messages) are in
  English.
- Text shown on screen goes in Fluent files: `config/lang/en.ftl`, and
  `addons/languages/<lang>/messages.ftl` with the same keys in the same order
  (`i18n::t`, `i18n::f`).
- Text only the agent reads (requests, worker prompts, Codex context) is
  English in every language: `config/lang/ai.ftl` (`i18n::ai`, `i18n::ai_f`).
  It is never translated.
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
- Minor (0.2.0): a new way of working that users notice as new, or a change
  users have to follow (a data format, a command). The bar rises as the
  version does: without users, the version is not raised for small features.
- Patch (0.1.1): fixes, small features and refinements, and following a change
  in an external component.

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
