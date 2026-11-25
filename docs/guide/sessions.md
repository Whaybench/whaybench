# Sessions

Use **Sessions** to find a conversation, reopen it, or start a new one. Use **Input** to send prompts and **Answer**
to read replies and respond to questions and permission requests.

## Open and select a session

Choose **Panels → Sessions**. Each row shows the agent, its state, and when it was last active. Click a row or a
conversation tab to select it. **Conversation map**, **Answer**, and **Files** follow that selection. Choose **Follow
latest session** to follow the session whose map changed most recently. Select a conversation again before you send
text.

| State | What it means |
|---|---|
| **Not running** | Whaybench sees no running agent for this session. |
| **Working** | The agent is working on a turn. |
| **Awaiting user** | The agent is waiting for a choice, a permission decision, or other input. |
| **Response complete** | The agent has answered and waits for your next prompt. |
| **Running** | The agent is running, and Whaybench does not know whether it works or waits. |

## Start or resume a conversation

Choose **Sessions → New conversation**, then choose **Claude Code**, **Codex**, or another installed agent.
If replies do not appear in Whaybench, check your [agent hooks](agents-and-add-ons.md).

Click a saved conversation in **Sessions** to resume it. If its tab is already open, Whaybench brings it forward.
If Claude Code is running in the background, Whaybench connects to it in a new tab.

If Claude Code is working in another terminal, Whaybench shows its map and answers but cannot send it text, and says
**Working in another terminal · Input unavailable**. To bring it here, enter this command in the other terminal, then
click the session row again.

```text
/bg
```

## Name and find conversations

A conversation's agent names it in the map while you talk, and changes the name when the work changes. Until it does, the
name is the topic of the map's latest work. Without a map it can be Claude Code's session name or the first recorded
question. A name does not depend on your other conversations. To give a Claude Code conversation your own name, use this in its tab.

```text
/rename
```

With the panel open, choose **Sessions → Search sessions**. Each word you type must match somewhere in the name, first
or last recorded question, map labels, or last saved answer. Search ignores case and includes hidden sessions that have
a recorded question. Press <kbd>Enter</kbd> to open the first match. If you see **No matching session**, try fewer
words.

## Close, hide, or end a session

| Action | What happens |
|---|---|
| Click `×` on the tab | Closes the tab. Claude Code can keep running in the background. |
| Right-click a row and choose **Hide** | Moves the row under **Hidden sessions**. The agent can keep working. |
| Right-click a running row and choose **End session** | Asks the agent to stop, closes its tab, and hides the row. Not for one in another terminal, see below. |

Closing a tab ends Codex and conversations in a Briefsafe project, with the turn they are in. Click the row to open the
conversation again. Outside a project, Whaybench tries to move an active Claude Code conversation to the background when
you close its tab. A conversation already running in the background can keep going after you close its tab.

You can also choose **Sessions → End this tab's session**; select the conversation first. If Claude Code is running in
another terminal, stop it there: here Whaybench says **Working in another terminal** and leaves it in the list. To put a
hidden row back in the list, expand **Hidden sessions**, right-click the row, and choose **Restore to list**. You can
also click a saved row there to reopen it.

## Continue from a map

Select the source session and choose **Conversation map → New conversation from this map**. Choose how much to
carry, then choose the agent.

| Option | What the new conversation gets |
|---|---|
| **Whole map** | All nodes in the source map. |
| **As shown** | The nodes shown in the map panel, with the branches and other nodes they need. In **Timeline**, only the branches count as shown. The agent can still read the whole source map. |

The earlier chat stays in the source conversation. If Whaybench says the session has no map, select a session that
has one. To use **As shown**, open the main [conversation map](conversation-map.md) panel first.

## Send text to a conversation

Choose **Panels → Input**. The empty field shows which agent and session receive your text. Press <kbd>Enter</kbd> to
send or <kbd>Shift</kbd>+<kbd>Enter</kbd> for a new line. If you pick another conversation while you write, the text
goes to that one. If an agent brings another tab forward, the text still goes to the one you were writing to. A draft
for a Briefsafe project can only be sent to that project, and locking the project clears it.

Ordinary prompts need a running session with an open tab. **Sent** means the prompt is waiting to go into the tab.
Whaybench submits it when the agent is ready. If it cannot queue the prompt, your draft stays in **Input**.
If you close the tab before a queued prompt goes in, Whaybench discards the prompt and reports it.

A one-line slash command brings its tab forward and can go in during a turn. If the tab is closed, Whaybench
reports that the command did not run. The field clears when you submit the command.

## Read and revisit answers

Choose **Panels → Answer**. Select a conversation to see its latest saved reply when one is available. The panel
shows the time and the start of the question above the reply. Its history holds up to 50 answers across sessions.

| Button | What it does |
|---|---|
| `◀` and `▶` | Move to an earlier or later answer. |
| `⟳` **Newest answer** | Jumps to the newest answer in the history. |
| `↶` **Answer shown before** | Returns to what you were viewing before that jump. |
| `✕` **Remove from history** | Removes the entry from this list. The saved answer stays available. |

With **Answer** open, right-click a map node and choose **Related answers**. Use `◀` and `▶` to browse the saved turns
and `✕` to close that view. If Whaybench finds no saved reply from that point onward, it keeps the current view. Click a
web link in a reply to open it in the browser; the reply stays.

## Answer questions and decide permissions

Keep **Answer** open to get cards. With the agent hooks active, it can show one question or permission card at a time
above the reply. For a question, choose from the options or type in **Other answer…**. Answer every question on the
card, then choose **Send**. For a Codex question, **Send** queues a new prompt. If it cannot queue your reply, the card
stays open. Reopen the conversation tab and try again.

For a permission request, read the tool name and what it is about to use, then choose **Allow**, **Deny**, or
**Answer in terminal**. Requests with **Answer in terminal** also go to the terminal if **Answer** closes or the
request times out. If Whaybench cannot save your choice, it reports the error and keeps the card open.

While the window is in the background, Whaybench can send a desktop notification when a session starts to wait for you.

## Check background work and usage

Look for **Helpers** below a session row to see its reported subagents and tasks. The line shows names and counts.
In **Answer**, a reply can be headed **Task notice** or **Subagent report**.

Open **Panels → Notifications** and look under **Due** on the **Now** tab for prompts scheduled in running
Claude Code sessions. A row shows **Repeats**, **Once**, or **Wake-up**. Click it to open the conversation.

A `⟳` beside a session means it still runs an older version of its agent. For Codex, point at the row to see both
versions; close its tab and open the session again to move it. For Claude Code, right-click a background session and choose
**Restart on new version**. The choice is off while the session works, waits for you or has a loop pending.

At the bottom of **Notifications**, look for context use and the five-hour and seven-day usage percentages for
the selected conversation's agent. Whaybench shows local reset times when the agent reports them and leaves out
missing values. If you hit a limit, check the agent's message in the conversation tab before you try again.

