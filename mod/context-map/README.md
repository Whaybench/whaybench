# context-map (Claude Code plugin)

The Claude Code connector of Whaybench. It holds no code of its own: it points Claude Code at the `whaybench`
command, the same one Codex CLI uses.

- `.mcp.json`: the conversation map tool `context_map`, served by `whaybench mcp`.
- `hooks/hooks.json`: Claude Code's standard hook events call `whaybench hook claude`, which keeps the session's
  record, state and answers, tells Claude the map guidance, the memory list and changes and the files the person
  changed, records the files it reads and changes, and holds back an edit to a file open in vim or saved by the
  person.

Whaybench types what it queues for a session into the session's tab; the plugin does not submit anything.

## Install

    claude plugin marketplace add whaybench/whaybench
    claude plugin install context-map@whaybench

After updating Whaybench, run `claude plugin update context-map@whaybench`.
