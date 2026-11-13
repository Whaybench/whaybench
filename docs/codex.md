# Codex CLI

> Codex CLI support is experimental. Some parts may not work yet; reports are welcome in [Issues](https://github.com/Whaybench/whaybench/issues).

Choose `New conversation · Codex` in the session menu and that tab runs Codex CLI. Claude Code and Codex tabs work side
by side; a session opens again with the agent it was started with.
Codex is an agent add-on, `addons/agents/codex/`: its command, arguments, settings folder, hook and MCP templates and
guidance are files there, not code. How to make one is in `addons/agents/README.md`.
Other tabs are Claude Code, and nothing on that path changes.

## Parts

| Part | What it does | Where |
|---|---|---|
| Opening a Codex tab | Prepares `<data>/codex` as Codex's folder (`CODEX_HOME`) as `agent.json` says and passes `WHAYBENCH_AGENT=codex` to everything the tab starts | `agent_env` in `crates/whaybench/src/start.rs`, `prepare` in `crates/wb-core/src/agents.rs`, `addons/agents/codex/` |
| Terminal relay | Starts the add-on's `command` and `args` (`codex --no-daemon --sandbox workspace-write`) instead of Claude Code; a session comes back with its `resume` (`codex resume ID`) | `Relay::start` in `crates/whaybench-term/src/main.rs`, `args` in `crates/wb-core/src/agents.rs` |
| `whaybench hook codex` | Receives Codex's eight hooks (SessionStart, UserPromptSubmit, PreToolUse, PermissionRequest, PostToolUse, Stop, Interrupt, SessionEnd). The same handler serves Claude Code's events (`whaybench hook claude`): the session record, state and heartbeat; at the start (and after a compaction) the map guidance, the view panel note and the whole memory list; on each prompt the memory changes and the files the person changed; after `apply_patch` the files it changed, as file records and memory events; at the end of a turn its count and answer | `crates/wb-core/src/hook.rs` |
| `whaybench record SESSION touch\|state\|alive\|off\|answer\|moved\|carry-from` | Session record, state, heartbeat and answers. Only these functions write those files, whatever the agent | `crates/wb-core/src/session_record.rs` |
| `whaybench mcp` | The conversation map tool `context_map` over MCP. Whaybench registers it in Codex's `config.toml` as the server `whaybench`, passes it `WHAYBENCH_TAB` (Codex passes an MCP server only the variables it lists) and lets the tool run without asking. The tab tells the server which session it serves | `crates/whaybench/src/mcp.rs`, `crates/wb-core/src/map_tool.rs` |
| `whaybench map SESSION [--write]` | Reads or changes a conversation map. A change comes on stdin as `upsert`, `remove`, `current` (`nodes` for a whole replacement) | `command` in `crates/wb-core/src/map.rs` |
| Agent guidance | At SessionStart and every UserPromptSubmit, the rules of keeping the map and the commands go into Codex's context | `addons/agents/codex/context.md` (English) |

## How it differs from Claude Code

| | Claude Code | Codex |
|---|---|---|
| Who writes the map | The agent calls the MCP tool `context_map` (`mcp__plugin_context-map_whaybench__context_map`, allowed when the relay starts Claude Code); `whaybench hook claude` counts turns | The agent calls the MCP tool `context_map`; `whaybench hook codex` counts turns |
| Who writes session, state, heartbeat, answers | `whaybench hook claude` | `whaybench hook codex`, through the same functions |
| Running sessions | `claude agents --json` | Hook records and the relay's heartbeat (every 2 seconds) |
| Closing a tab | A session with a conversation goes to the background (`/background`) | It ends and its heartbeat goes to 0; reopening uses `codex resume` |
| Sending a request | The relay pastes the tab's queue into the terminal, only while the state is `done` | The same |
| Running hooks | Installing the plugin is enough | Codex runs a hook only after the person trusts it in `/hooks`; trust again when the program's path changes |
| Answer | The turn's whole text, from the transcript | The last assistant message of Stop |

## Data

Codex sessions use the same files in the same data directory as Claude Code: `sessions/<id>.json` (`agent: codex`,
`tab`, `lastCompletedTurn`), `cache/state/<id>.json`, `cache/alive/<id>`, `answers/<id>.json`,
`answer-log/<id>.jsonl`, `<id>.json` and `latest.json` (the map: the nodes the session shows, by number),
`memory/memory.db` (the nodes themselves and how they changed), `cache/term/<tab>.in/*.json` (the tab's queue).

`<data>/codex` holds the login (`auth.json`, mode 0600) and model choice copied once from `~/.codex`, and
`hooks.json`, written at every start. The hooks are not written to `~/.codex`, so Codex used outside Whaybench is
not changed.

## Writing the map

Only `whaybench map` writes map files, whatever the agent. A node is stored once, in the memory; a map file lists
the nodes its session shows:

- `--write`: the plugin's map tool, a side worker's fill, imported branches. The MCP tool and the person's choices
  (notifications panel, map menu) write through the same code in-process.
- `--take-over`: taking over another session's map (`/background`, `/resume`, a tab that keeps the map). The new
  session shows the same nodes, not copies, and `latest.json` is left as it is.
- `--start-from`: a new session from a saved map. Nodes the memory has are shown as they are now; a node only the
  file holds is made new. `latest.json` is left as it is.
- `--file MAP.json`: reads a saved map with its nodes' content, which the file itself does not hold.
- `--end-turn`: the end of a turn.

The plugin calls `~/.local/bin/whaybench`, which Whaybench links at start.

`whaybench map` merges by id, removes the nodes under a removed node, and records `opened`, `changed` and `closed`
itself, dropping what the agent sent for them. A duplicate id, a missing parent or current node, a
parent loop or a bad id is refused before saving, and the stored map stays. Removing the current node clears
`current`.

## Sandbox and usage

Codex runs with `--sandbox workspace-write`. The data folder is one of its writable roots
(`sandbox_workspace_write.writable_roots`), so `whaybench-view`, `whaybench-project` and `whaybench-files` can leave their
requests there. Codex has no status line: when a turn ends, the hook reads the last `token_count` line of Codex's
session file for the context and the 5-hour and weekly windows. The notifications panel shows Codex's windows for Codex
sessions and Claude Code's for Claude Code sessions.

## Not yet

Holding back a patch to a file the person changed or has open in vim (Claude Code's edits are), files read
through shell commands, requests to a Codex session without a tab, the whole turn's answer,
taking over another session's map, filling a map style's fields.

## Tests

`cargo test --workspace`: the `agents` and `map` tests in `wb-core`, and `crates/whaybench/tests/codex.rs`, which
runs the real `whaybench` binary through a first prompt, two turns and a resume, and checks the guidance in
English and Korean.
