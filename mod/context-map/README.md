# context-map (Claude Code plugin)

The Claude Code connector of Whaybench. It gives Claude the conversation map tool and keeps the session record
Whaybench's panels read: the session's record, state, heartbeat, answers and the files it read or changed. It
also submits what Whaybench queues for a session and applies the map changes Whaybench writes (style fields,
imported branches, deletions). Every write to a map or a session record goes through the `whaybench` command.

Outside Whaybench it draws the conversation map in a pane beside the chat.

## Install

    claude plugin marketplace add whaybench/whaybench
    claude plugin install context-map@whaybench

After updating Whaybench, run `claude plugin update context-map@whaybench`.

## Structure (`hooks/`)

`register.tsx` only assembles: it registers each module's hooks in order, and keeps the session start and the one
prompt hook the engine accepts.

| Module | What it holds |
|---|---|
| `paths.ts` | Every path shared with Whaybench |
| `map-data.ts` | The map's values (the only reader and writer), applying changes, taking over another session's map, the summary |
| `map-text.ts` | The map as text rows: marks, tree lines, folding |
| `map-tool.ts` | The map tool Claude calls, and the `/map` command |
| `map-patches.ts` | Map changes from files Whaybench writes |
| `session-record.ts` | Session record and state, `/rewind`, `/background` and `/resume` moves |
| `inbox.ts` | Submitting what Whaybench queued for the session |
| `answers.ts` | Counting turns and keeping each turn's whole answer |
| `files.ts` | Bash run times, so Claude's own changes are told from the person's |
| `decisions.ts` | The map's decisions that come with a file Claude reads |
| `log.ts` | Debug log: one file per session in `logs/sessions/`, in Whaybench's line format |
| `safety.ts` | Holding back a change once for a file open in vim or saved by the person |
| `pane.tsx` | The map pane inside Claude Code |

## Rules the engine's checker enforces

- A state value (`atom`) is read and written only in the file that defines it (`map-data.ts`).
- A function that takes `$` and is called from another file is a `function` declaration, not an arrow function.
- A hook's matchers (tool names, pane id) are literals in that hook's own file.
- There is one `prompt.submit` hook.

## Check

    claude plugin validate .

Run it after every change. Sessions already open keep the old code; new sessions load the new one.
