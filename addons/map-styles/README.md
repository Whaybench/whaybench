# Making a conversation map style

A map style is an add-on: one folder, one style. Delete the folder and the style is gone with its settings. The tree, `Default`, is part of the program.

```
<data folder>/addons/map-styles/<style id>/
  style.json
```

- `<style id>` is the folder name: lowercase letters, digits and `-`.
- A folder appears in the conversation map's `Display style` list. If the style in use is deleted, the panel goes back to `Default`.
- Colours and fonts come from the theme, not from the style.
- `name` is in English. A language add-on can translate it with the key `map-style-<style id>`.

## File format

```json
{
  "format": 1,
  "name": "Kanban",
  "version": "1.0",
  "frame": "columns",
  "group": "state",
  "columns": ["idea", "plan", "doing", "done"],
  "recent_done": 6,
  "needs": ["state"]
}
```

Unknown keys are ignored. A file that is not valid JSON is not listed.

## Common keys

| Key | Value | Meaning |
|---|---|---|
| `format` | number, now 1 | Version of this file format |
| `name` | text | Name in the list; without one, the folder name |
| `version` | text | Version of the style |
| `menu` | `true`, `false` | Whether it is in the `Display style` list. Default `true` |
| `frame` | see below | The frame the program draws. Without one, the AI draws from `instruction` |
| `needs` | see below | Node data this style needs |
| `instruction` | text | Drawing instruction for the AI, when there is no `frame` |
| `node_menu` | see below | Items added to the node's right-click menu |

## `frame`: frames and their settings

| Value | Shows | Settings |
|---|---|---|
| `tree` | Tree | `filter`: `path` (current path and open nodes only) · `show_kind`: `true` (show the kind) |
| `columns` | Columns | `group`: `state`, `kind`, `top` (top branch) · `columns`: their order · `recent_done`: how many recent items in the done column |
| `route` | Start, via, destination | None |
| `time` | Time flow by branch | `range`: `all`, `topic` (from the start of the current topic), `decision` (from the last decision) |
| `questions` | Open questions grouped | None |

With `group: state`, the `columns` values are `idea`, `plan`, `doing`, `done`.

## `needs`: node data

| Value | Content | When empty |
|---|---|---|
| `kind` | question, option, pro, con, conclusion, note | A side worker fills it from the transcript |
| `reason` | Why the conversation moved to this node | A side worker fills it (nodes on the route only) |
| `links` | Relations to nodes of other branches | A side worker fills it |
| `state` | plan, doing, done, changed | The AI writes it during the conversation |
| `opened`, `visits` | Creation time, visits | The mod records them (past nodes stay empty) |

The side worker calls a small model (haiku) once. It does not appear in the conversation.

## `node_menu`: items of the node menu

```json
"node_menu": [
  { "label": "Summarize branch", "do": "request", "arg": "Summarize branch '{branch}' (node {id}) in three lines" },
  { "label": "Show as kanban", "do": "style", "arg": "kanban" },
  { "label": "Outside document", "do": "view", "arg": "https://example.org/{node}" }
]
```

| `do` | Action |
|---|---|
| `request` | Sends `arg` to the selected session |
| `style` | Changes the panel's style to `arg` |
| `view` | Shows `arg` in the view panel |

`{node}` is the node's name, `{branch}` the path from the top down to it, `{id}` its id. The standard menu items cannot be changed.

## Confirming instructions

`instruction` and `request` items go to the AI as written. A style from elsewhere shows them for confirmation before first use, and again when they change.
