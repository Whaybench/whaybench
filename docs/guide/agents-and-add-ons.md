# Agents and add-ons

Choose Claude Code or Codex for each conversation, and connect it to the conversation map. Use add-ons to change how
Whaybench looks and works.

## Choose an agent

Install the CLI you want to use. In Whaybench, choose **Sessions → New conversation → Claude Code** or
**Sessions → New conversation → Codex**. You can use both side by side. When you reopen a session, it uses the
agent it started with.

With Whaybench running, you can also open a Codex tab from a terminal:

```sh
whaybench ctl session new --agent codex
```

If you see **Conversation tab not opened**, read the reason that follows it. For Codex, check that Whaybench can
find the CLI and that the Codex add-on is installed.

## Connect Claude Code

The Linux installer tries to set up the `context-map` plugin when it finds Claude Code. If you need to install the
plugin yourself, run:

```sh
claude plugin marketplace add Whaybench/whaybench
claude plugin install context-map@whaybench
```

The plugin calls `~/.local/bin/whaybench` for its hooks and map tool, so that command must be executable.
To update the plugin, run these commands and start a new Claude Code conversation:

```sh
claude plugin marketplace update whaybench
claude plugin update context-map@whaybench
```

When Whaybench starts Claude Code, it allows the map tool. It supplies a status line if you have none. If your
interface language specifies a language for replies, Whaybench uses it when you have no language set in Claude Code.
A tab that attaches to a background session uses that session's existing options.

## Connect Codex and trust its hooks

Whaybench includes a Codex add-on. In the Codex tab, enter:

```text
/hooks
```

Review and trust the Whaybench hooks there. They let Whaybench follow the session and give Codex its map guidance.
If the path to Whaybench changes, open the hook review again and check the new commands.

For a regular Codex tab, Whaybench uses the `codex` folder in its data directory. It shares your Codex sign-in and takes
your model and reasoning effort settings when it first creates the configuration. To change them for Whaybench's Codex
tabs later, edit `model` or `model_reasoning_effort` in `codex/config.toml` in that data directory and open a new tab.
On each launch it adds the map tool settings and rewrites the hook definitions there. Codex runs with workspace write
access and can write to selected Whaybench folders for features such as the **View** panel. An isolated [Briefsafe
project](briefsafe.md) has its own sign-in.

Use `/fork` in Codex to branch the conversation. With the hooks trusted, the fork starts from the map of the one it came
from.

## Check what each agent records

Both agents use the `context_map` tool to update the [conversation map](conversation-map.md). Their hooks record
session activity and save answers.

| What happens | Claude Code | Codex |
|---|---|---|
| You close a tab | A regular conversation can keep running in the background | The process ends |
| Whaybench saves an answer | The turn's text when available, or the last assistant message | The last assistant message |
| Whaybench records files | Reads and edits through recognized file tools | Edits made through `apply_patch` |
| Whaybench gets usage | From the Claude Code status line | After a turn finishes, when Codex reports usage |

## Find a missing map or usage line

Select the conversation you want. If its map stays empty, ask the agent to add a topic to the map. In Claude Code,
check the plugin and try a new tab. In Codex, check hook trust and send another prompt. The map tool needs a
session it can identify before it can save anything.

Open **Panels → Notifications** for the selected session's context usage and its agent's available 5-hour and 7-day
usage. The 5-hour and 7-day figures are the latest known for that agent, so they can be a little old. Missing fields and
expired usage windows are left out. If there are no numbers, the line is hidden.

For Claude Code, a custom status line can pass the same input it receives to:

```sh
whaybench statusline
```

For Codex, let a turn finish with the hooks trusted. If Codex supplies no usable usage data for that turn, Whaybench
has no new numbers to show.

## Install an add-on

Put each add-on in its own folder under `addons` in your Whaybench data directory. The default directory is
`~/.local/share/whaybench`. If you use a different one, put the add-on there. [Settings and
data](../reference/settings-and-data.md)

| Kind | Folder under `addons` | Where to use it | Authoring guide |
|---|---|---|---|
| Agent | `agents/<id>/` | **Sessions → New conversation** | [Agents](../../addons/agents/README.md) |
| Language | `languages/<id>/` | **Settings → Language** | [Languages](../../addons/languages/README.md) |
| Map style | `map-styles/<id>/` | **Display style** in the map panel | [Map styles](../../addons/map-styles/README.md) |
| Theme | `themes/<id>/` | **Settings → Theme** | [Themes](../../addons/themes/README.md) |
| Project document template | `project-docs/<id>/` | **+ Document** in a plain project; copied into a Briefsafe project when it is made | [Templates](../../addons/project-docs/README.md) |

If an add-on is missing from its picker, restart Whaybench and check the folder and required files against its
guide. After choosing a language, use **Whaybench → Restart** to apply it.

## Approve an add-on

When you open a custom or modified agent add-on, Whaybench shows the command it runs as you, the names of the settings
files it writes, and the guidance it gives the agent. Review these and press <kbd>y</kbd> to run it. Press
<kbd>Enter</kbd> to decline, then press a key to close the tab. Its settings and sign-in are set up only then. An
unchanged bundled agent skips this prompt.

When **Display style** opens **Map style instructions**, read the text the style can send to the agent. Choose
**Use** to approve it or **Cancel** to keep your current style.

## Update or remove an add-on

Install a new version of a third-party add-on in the same folder. When you next open an agent whose setup or
guidance has changed, Whaybench asks for approval again. When you pick a map style with changed instructions,
review the text again.

Whaybench refreshes bundled add-on files at startup and keeps your theme and map style settings beside them.
For changes to a bundled add-on's own files, make a copy under a new folder name.

To remove an add-on, delete its folder. A deleted bundled folder stays deleted. A missing theme or map style
falls back to **Default**. A removed language falls back to **English** after restart. Documents already made
from a removed template stay in their projects. Sessions that use a removed agent need that add-on to reopen.

## Add another agent

Follow the [agent authoring guide](../../addons/agents/README.md) to put `agent.json` and its companion files
in `addons/agents/<id>/`. The ID `claude` is reserved. The agent needs compatible command hooks and MCP for
Whaybench to track the session and keep the map. When Whaybench can load the add-on, its name appears under
**Sessions → New conversation**.

