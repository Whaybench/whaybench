# Making an agent

An agent is an add-on: one folder, one coding agent that runs in a conversation tab. Delete the folder and the agent is gone. Claude Code, `claude`, is part of the program; `claude` cannot be an add-on's id.

```
addons/agents/<id>/
  agent.json      required
  <templates>     settings files the agent reads, named in agent.json
  context.md      what the agent is told at the start
  prompt.md       what it is told with each prompt instead (optional)
```

Each agent is a line in the session menu, `New conversation → <name>`. A session opens again with the agent it was
started with, and the session list shows the agent's name.

## What an agent needs

- Command hooks shaped like Claude Code's: the agent runs a command for each event, gives it the event as JSON on stdin (`session_id`, `hook_event_name`, `cwd`, ...) and reads `hookSpecificOutput` back. Whaybench answers them with `whaybench hook <id>`.
- MCP: the conversation map tool is the MCP server `whaybench mcp`.

An agent without these cannot keep the map or the session list.

## agent.json

| Key | Required | Meaning |
|---|---|---|
| `name` | yes | The name people see |
| `command` | yes | The program, found on `PATH` |
| `args` | no | Arguments given every time |
| `resume` | no | Arguments that open a session again; `{session}` is its number |
| `prompt` | no | Arguments that start the agent with a first prompt; `{prompt}` is its text. For an agent that makes its session only at its first prompt (Codex): a new conversation from a map is started with the request to read the map, so its start hook takes the map in |
| `home` | no | A settings folder of the agent's own, `<data>/<id>`, so the agent used elsewhere is not changed. `env`: the variable that names it to the agent. `from`: the person's own folder (`~/` allowed; the variable `env`, when set, wins). `copy`: files copied once from there. `link`: files shared with it by a link, such as the login, so a sign-in on either side holds on both; a sign-out in Whaybench unlinks it until the person's file is newer. `copy_keys`: `{ "file.toml": ["key", ...] }`, keys copied once into a new file |
| `merge` | no | `{ "settings file": "template" }`: the template's keys are written into the settings file in `home`; the person's other keys stay. TOML only |
| `write` | no | `{ "settings file": "template" }`: the template becomes the whole settings file in `home`, each start. JSON or TOML |
| `context` | no | A text file given to the agent at session start and with each prompt. English: only the agent reads it |
| `prompt_context` | no | A shorter text file given with each prompt in place of `context`, which then goes only at session start. Text riding on every prompt stays in the conversation, so keep it to a line |
| `login` | no | Arguments that sign the agent in where it has no browser of its own, such as a code the person enters on a page (Codex's `login --device-auth`). A Briefsafe project's agents keep a login of the project's own: the project panel runs this in its container, in a terminal of the view. Without it the panel offers no sign-in for the agent |
| `login_page` | no | The https page the person opens for that sign-in, shown in the browser tab beside it |
| `trust` | no | The command inside the agent where the person trusts the hooks; Whaybench says so when its tab opens |
| `ready` | no | What the agent writes to its terminal when it waits for input, and not before a sign-in is done; `\u001b` is Escape. For an agent that makes its session only at its first prompt (Codex): once this is seen, the Input panel can send that prompt. Without it the first prompt is typed in the tab |
| `busy` | no | What it writes when it begins to work: from then until `ready` is written again, the Input panel does not send a first prompt |
| `usage` | no | Where the agent's context and plan usage are read, for the notifications panel. `codex-rollout`: the last `token_count` line of Codex's session file, read when a turn ends. Without it the panel shows no usage for the agent's sessions |

In templates, these are filled in text values: `{exe}` the Whaybench program, `{data}` the data folder, `{hook}` the hook command (`'<exe>' hook <id>`). In `context.md` and `prompt.md`: `{session}`.

The settings folder is readable only by the person (700, files 600).

## Example

`codex/` is the Codex CLI.
