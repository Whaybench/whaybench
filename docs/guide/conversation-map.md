# Conversation map

The conversation map is a tree of what you and your agent have discussed: topics, questions, tasks and decisions. Use
it to see what is settled, find the answer behind a decision, and carry the work into another conversation.

## Open the map

Choose **Panels → Conversation map**. The map follows the session you select. With **Follow latest session** in
**Sessions**, it shows the map that changed last; select a conversation before you send anything from the map.

Your agent writes the map while you talk. It adds a node when a topic or task starts, and turns a settled question into
a conclusion with its reason. Asking for something small may change nothing. If the map stays empty after several
turns, check your [agent setup](agents-and-add-ons.md).

## Read the map

Click `▸` or `▾` to open or fold a branch. Hover over a label to read its detail. Below the top level, a branch where
everything is closed starts folded, unless the conversation is in it.

The symbol before a label is its state. The current node shows `●` instead, and `!` stands beside the state:

| Symbol | State |
|---|---|
| `●` | Where the conversation is now |
| `◇` | Open |
| `□` | Planned |
| `◐` | In progress |
| `✎` | Changed |
| `‖` | On hold |
| `✓` | Done |
| `✕` | Dropped |
| `!` | Marked important by you |

A node also has a kind. Display styles that show kinds use these names:

| Kind | What it holds |
|---|---|
| **Question**, **Option** | A question and a possible answer, with `+` and `−` for what speaks for and against it |
| **Conclusion** | A decision, with its reason |
| **Task**, **Appointment** | Work to do, or something with a date |
| **Problem**, **Idea** | Something to solve or to consider |
| **Fact**, **Rule**, **Note** | Information, a standing rule, or a remark |

A topic has no kind.

## Find the answer behind a node

Keep **Panels → Answer** open, right-click a node and choose **Related answers**. The **Answer** panel comes forward
with the first answer kept from the time the node was made. Use `▶` if the decision came in a later turn. Move with `◀`
and `▶`, and close with `✕`.

Nodes added later by **Tidy** carry the time of the tidying, so look through earlier answers for those.

## Change the map

You do not edit the map by hand: you ask the agent to. Type what you want in **Additional request (optional)** above the
map and press <kbd>Enter</kbd>, for example "move the install questions under Release". **Tidy** with the field empty
asks the agent to read back through the conversation and put the map in order. Both send a request to the selected
conversation, which needs an open tab, and use tokens. In a branch panel, the request is about that branch only.

Right-click a node for the rest:

| Action | What it does |
|---|---|
| **Open in new panel** | Shows the node and what is below it in a panel of its own. Not for a Briefsafe project's map |
| **Mark important** / **Remove mark** | Puts `!` beside the node, or takes it off. Only you can set it |
| **Reflect in project** | Asks an agent to write the branch into a [project](projects.md) |
| **Delete → Delete node** | Hides the node only; what is below it takes its place |
| **Delete → Delete branch** | Hides the node and everything below it |

> [!NOTE]
> A node can be on the maps of several sessions. Deleting it hides it from all of them. Recover it under
> [**Deleted** in Notifications](#read-the-notifications). The counts are
> of the whole memory, so they can include nodes that only another session's map shows.

If you rewind the conversation in its terminal, the map does not go back with it. At your next prompt, Whaybench asks
the agent to bring the map in line.

## See a node's files

Click a node to show only the files of that node and what is below it in the [Files panel](view-and-files.md). Click it
again to show all files. Files that Claude Code reads or edits with its file tools, and files Codex edits with
`apply_patch`, are recorded at the current node. Files touched only through shell commands are not.

You can also attach files to a node. Ask the agent for the session's ID and the node's, then use them in a terminal.
Quote patterns so that Whaybench, not your shell, reads them. For a Briefsafe conversation, have its agent run the
command:

```sh
whaybench-files --session SESSION attach review 'docs/*.md'
whaybench-files --session SESSION list review
whaybench-files --session SESSION detach review
```

The last command takes every attachment off the node. To take off one, give its pattern as `list` prints it.

## Save and reuse a map

With the map panel open:

- **Save:** choose **Conversation map → Save map**, or right-click a node and choose **Save branch**. The status line
  shows where it was saved.
- **Open:** choose **Conversation map → Open map**. It opens read only in its own panel.
- **Continue:** choose **Conversation map → New conversation from this map**, then how much to carry and the agent.
  **Whole map** carries every node. **As shown** starts from what the panel shows, with what it depends on, and the
  agent can still read the rest of the source map. In an opened map, right-click and choose **New conversation from this
  map**. The earlier chat is not carried.
- **Use as a reference:** right-click in an opened map and choose **Refer to the map (whole map)** or, on a node,
  **Refer to the map (this branch)**. The selected conversation, which needs an open tab, is asked to add a reference
  node. **Open source** on that node leads back.

A saved map points at nodes in your Whaybench data. Opening it shows those nodes as they are now and leaves out the ones
that are gone. To use it on another computer, [move your
data](../reference/settings-and-data.md#back-up-and-move-on-linux) too.

## Change how it is drawn

Click **Display style** in the map panel. **Default** is the tree. The other styles draw the same map differently:

| Style | What it shows |
|---|---|
| **Focus** | The path to the current node and what is open |
| **Questions** | Questions with their options and conclusions |
| **Kanban** | Columns, by state unless you choose otherwise; **Done** shows the six latest |
| **Route** | How the conversation moved from node to node |
| **Timeline** | What changed, by branch and turn, from the start of the current topic unless you choose otherwise |

Each panel keeps its own choice. A style may ask the agent to draw it, or have a separate small Claude model fill in
what the map lacks from the map and the recent conversation. Both use tokens; a Briefsafe conversation never uses the
separate model. **Conversation map → Display style settings…** changes a style's settings for all panels that use it. A
style from elsewhere shows its instructions for you to confirm before they reach the agent. To add one, see [Agents and
add-ons](agents-and-add-ons.md#install-an-add-on).

## Use the memory across sessions

Whaybench keeps the nodes of your ordinary sessions in one memory, so a session knows what others decided. Each
Briefsafe project has a memory of its own. Each node has a number there, such as `#12`, and an agent can give nodes
about the same issue the same number. Ask an agent for a node's number. When several sessions share a node, a change to
it appears on each map.

With the agent's hooks active, a session that starts or resumes gets a summary of up to 100 nodes. A Briefsafe
conversation gets only its project's. Its own open nodes come first, followed by related and recent nodes from other
sessions. A decision can include a short reason. On later prompts, the agent gets relevant changes you or other sessions
make since it last sees the memory.

Use `show` to see the nodes and their history for a memory number. Pass the number without the `#`. Use `search`
to find nodes by words in their labels or details.

```sh
whaybench memory show 12
whaybench memory search install update
```

Search matches any of the words and shows up to 50 results, most recently changed first. An unknown number gives an
empty result.

## Read the notifications

Choose **Panels → Notifications**. **Now** shows the groups that have items. If every group is empty, it shows
**None**. Hover over a node to read its detail when available.

| Group | What it shows |
|---|---|
| **Waiting for a choice** | Pending questions and permission requests. Click a row to bring **Answer** forward. |
| **Due** | Open nodes with dates and scheduled items from running sessions. Click a scheduled item to go to its session. |
| **Marked** | Nodes you mark as important, including closed nodes. |
| **Changed** | Nodes settled by a decision and nodes whose prerequisites are now met. |
| **To approve** | Open nodes the agent suggests closing, with any reason it gives. |
| **Focus** | Open nodes for the session on show, in map order. |
| **On hold** | Nodes put on hold. |
| **Deleted** | Deleted nodes. Click the heading to expand the group. |

Click **Mark changes seen** to clear **Changed**. In **To approve**, right-click a suggestion and choose **Approve**
to apply the proposed state or **Decline** to clear the suggestion.

Right-click a node under **Now** to set it to **Done** or **Drop** it yourself, or choose **Open session** to go to the
conversation it came from.

Right-click a node and choose **Hold** to put it aside. In **On hold**, right-click it and choose **Reopen**.
If a choice fails to save, the status line shows **Notifications choice not written** with the reason.

In **Deleted**, right-click a node and choose **Restore** when available. This undoes the deletion, including a branch
deletion. Whaybench skips nodes that have changed since and reports how many it skips.

What you set on a node holds until the agent has been told of it, and a node you deleted is not brought back by an
agent.

**Delete for good** asks for confirmation and removes the node and its history. By default, Whaybench also removes
nodes that have been in **Deleted** for more than 180 days when it starts.

**History** shows node changes, newest first. To put an existing node on a session’s map, select the session, then
right-click the node in **History** and choose **Restore**. The status line reports if the node is already on that map.
**Due**, **Focus** and **History** cover ordinary conversations, not those of Briefsafe projects.

At the bottom, **Context** shows the session’s context use. **5h** and **7d** show plan use and reset times when
available. The footer disappears when there are no numbers to show.
