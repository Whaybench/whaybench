# Codex CLI

> Codex CLI support is experimental. Some parts may not work yet; reports are welcome in [Issues](https://github.com/Whaybench/whaybench/issues).

Run `whaybench --agent codex` and Codex CLI is the agent of that run. One run does not mix Claude Code and Codex.
Without `--agent` the agent is Claude Code, and nothing on that path changes.

## Parts

| Part | What it does | Where |
|---|---|---|
| `whaybench --agent codex` | Prepares `<data>/codex` as Codex's folder (`CODEX_HOME`) and passes `WHAYBENCH_AGENT=codex` to everything the run starts | `crates/whaybench/src/agent.rs`, `prepare` in `crates/wb-core/src/codex.rs` |
| Terminal relay | Starts `codex --no-daemon --sandbox workspace-write` instead of Claude Code; a session comes back with `codex resume ID` | `Relay::start` in `crates/whaybench-term/src/main.rs`, `codex::args` |
| `whaybench hook codex` | Receives Codex's eight hooks (SessionStart, UserPromptSubmit, PreToolUse, PermissionRequest, PostToolUse, Stop, Interrupt, SessionEnd) and writes the shared files | `hook` in `crates/wb-core/src/codex.rs` |
| `whaybench record SESSION touch\|state\|alive\|off\|answer\|moved\|carry-from` | Session record, state, heartbeat and answers. Only these functions write those files, whatever the agent | `crates/wb-core/src/session_record.rs` |
| `whaybench map SESSION [--write]` | Reads or changes a conversation map. A change comes on stdin as `upsert`, `remove`, `current` (`nodes` for a whole replacement) | `command` in `crates/wb-core/src/map.rs` |
| Agent guidance | At SessionStart and every UserPromptSubmit, the rules of keeping the map and the commands go into Codex's context | `codex-context`, `codex-map-context` in `config/lang/ai.ftl` (English in every language) |

## How it differs from Claude Code

| | Claude Code | Codex |
|---|---|---|
| Who writes the map | The plugin calls `whaybench map` | The agent calls `whaybench map`; `whaybench hook codex` counts turns |
| Who writes session, state, heartbeat, answers | The plugin calls `whaybench record` | `whaybench hook codex`, through the same functions |
| Running sessions | `claude agents --json` | Hook records and the relay's heartbeat (every 2 seconds) |
| Closing a tab | A session with a conversation goes to the background (`/background`) | It ends and its heartbeat goes to 0; reopening uses `codex resume` |
| Sending a request | The plugin reads the session's inbox | The relay pastes the tab's queue into the terminal, only while the state is `done` |
| Running hooks | Installing the plugin is enough | Codex runs a hook only after the person trusts it in `/hooks`; trust again when the program's path changes |
| Answer | The turn's answer | The last assistant message of Stop |

## Data

Codex sessions use the same files in the same data directory as Claude Code: `sessions/<id>.json` (`agent: codex`,
`tab`, `lastCompletedTurn`), `cache/state/<id>.json`, `cache/alive/<id>`, `answers/<id>.json`,
`answer-log/<id>.jsonl`, `<id>.json` and `latest.json` (the map), `cache/term/<tab>.in/*.json` (the tab's queue).

`<data>/codex` holds the login (`auth.json`, mode 0600) and model choice copied once from `~/.codex`, and
`hooks.json`, written at every start. The hooks are not written to `~/.codex`, so Codex used outside Whaybench is
not changed.

## Writing the map

Only `whaybench map` writes map files, whatever the agent:

- `--write`: the plugin's map tool, a side worker's fill, imported branches, deletions from the menu.
- `--take-over`: taking over another session's map (`/background`, `/resume`, a tab that keeps the map). Node
  times are kept and `latest.json` is left as it is.
- `--start-from`: a new session from an imported map. Node times start over and `latest.json` is left as it is.
- `--end-turn`: the end of a turn.

The plugin calls `~/.local/bin/whaybench`, which Whaybench links at start.

`whaybench map` merges by id, removes the nodes under a removed node, and records `opened`, `changed` and `closed`
itself, dropping what the agent sent for them. A duplicate id, a missing parent or current node, a
parent loop or a bad id is refused before saving, and the stored map stays. Removing the current node clears
`current`.

## Not yet

File read and change records, usage and limits, requests to a Codex session without a tab, the whole turn's
answer, taking over another session's map, filling a map style's fields, handing work between agents.

## Tests

`cargo test --workspace`: the `codex` and `map` tests in `wb-core`, and `crates/whaybench/tests/codex.rs`, which
runs the real `whaybench` binary through a first prompt, two turns and a resume, and checks the guidance in
English and Korean.
