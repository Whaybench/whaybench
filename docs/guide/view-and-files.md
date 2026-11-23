# View and files

Use **View** to open files and see what your agent shows you. Use **Files** to find what the agent read or changed
during a conversation.

## Open the View panel

Choose **Panels → View**. It shows PDFs, images, MP4 and WebM videos, HTML, Markdown and plain text. A very long text is
cut. Whaybench opens logs, archives, man pages and detected binary files in a terminal in the same area. Terminal
viewers bypass **View** history. Hold <kbd>Ctrl</kbd> and scroll to zoom the preview.

Choose **Panels → New view panel** to keep another view open. Use the **Link** menu in each view to choose what it
shows.

| Option | What it does |
|---|---|
| **Linked to session tab** | Follows the session you select. This is the default |
| **Current session:** followed by a name | Keeps showing the session you see now |
| **Other session** | Lets you choose another session |
| **Pinned view:** followed by a title | Holds the current item, when available, and pauses new items from the session |

## Reopen earlier previews

Click **◀** or **▶** to move through the history. Each view keeps up to 50 items per session and returns to the newest
item when you switch back. Missing local files drop out when the history loads. Click **✕** to remove the current
entry. Its tooltip is **Remove from history (the file stays)**. This control is unavailable while the item is pinned.

When your agent shows a figure that updates automatically, **View** comes forward and keeps a copy as **Image**. You can
go back to an earlier figure even after the agent updates it. Pictures the figure links to come from where they are now.

## Open and read files from a terminal

With Whaybench running, use `whaybench-view` to open a file yourself. Pass a number after a PDF path to request
that page. Use `-` to open HTML from standard input, or `edit` to open a file in Vim in the View area.

```sh
whaybench-view open ./notes.md
whaybench-view open ./report.pdf 3
whaybench-view open - < ./figure.html
whaybench-view edit ./notes.md
```

A missing file path gives an error. A file that a conversation opens in **View** waits for a view that follows that
session. Opened from a terminal outside Whaybench's conversations, it goes to the first **View**.

```sh
whaybench-view read
whaybench-view read --all
whaybench-view shot --out ./view.png
```

`read` prints visible text from the first **View** and includes any selection it can read. It withholds a Briefsafe
project's pages. `--all` requests the whole page. For a local PDF in **View**, either form uses `pdftotext` to extract
text from the whole file. `shot` saves a PNG; give a new file name. While a Briefsafe project is open, Whaybench removes
it after ten minutes.

From inside a [Briefsafe project](briefsafe.md) conversation, the helper only opens project files, with `open`. Logs and
archives open in a terminal, whose program runs in the project's container where it has one. Run the other commands from
a shell outside that conversation.

## Browse and sign in

Choose **Panels → Browser** to open Firefox. A web address passed to `open` goes there too. It starts Firefox if needed
or opens a new tab in the running browser. Press <kbd>Ctrl</kbd>+<kbd>L</kbd> to go to the address bar. The tab strip is
hidden: switch tabs with <kbd>Ctrl</kbd>+<kbd>Page Up</kbd> and <kbd>Ctrl</kbd>+<kbd>Page Down</kbd>, and close one with
<kbd>Ctrl</kbd>+<kbd>W</kbd>.

Sign in on the site in this Firefox. It has its own profile, shared across your Whaybench conversations. Firefox uses
that profile to restore ordinary tabs across restarts and moves. Sign-ins are kept where the site allows it.

```sh
whaybench-view open https://example.org
whaybench-view open --firefox ./report.pdf
whaybench-view read --firefox
whaybench-view where --text
```

Use `where` to get the current PDF page number when Firefox makes it available. `--text` also extracts that page's text
from a local PDF. If extracting fails, the command says why. If no PDF page is detected, the command reports an error.

## Move Firefox to the desktop

Drag the dot in the browser panel's title bar to the edge of the Whaybench window. If the browser shares a panel with
other tabs, drag its tab out of the title bar first. Release when **Into a window of its own** appears.

To bring it back, click **↩ Browser** on the conversation terminal's title bar or choose **Panels → Browser**.
Firefox restarts with the same profile. If Firefox fails to close, the move stops and Whaybench logs why.

```sh
whaybench-view detach
whaybench-view attach
```

## Find a conversation's files

Choose **Panels → Files**. Rows are grouped by map node, newest first. Files with no node go under **No map node**.
Hover over a row to see its full path and read and edit counts. Shell commands bypass file recording.

| Mark | Meaning |
|---|---|
| `✎` | The agent edited the file |
| `👁` | The agent read the file and has no recorded edit in this group |
| `📌` | The file is attached to the node |
| `✏` | The file may be open in Vim |
| `✋` | The file appears to have changed on disk after the agent last worked with it |

Click a node on the [conversation map](conversation-map.md) to show only its files and those of its descendants.
Click the node again to clear the filter. The numbered **Related files** icon also brings an open **Files** panel
forward.

## Open, hide, or review a file

Click a row to open the file as you would in **View**. Right-click for **Open in view panel**, **Open in vim**, or an
available desktop app. A dim row with **No read permission** means Whaybench cannot read the file. Check whether it
still exists and whether you can read it.

Choose **Remove from list** to hide that path across your conversations. Each Briefsafe project has its own list of
hidden files. You can still open the file from its original path.

Choose **AI edits** to see additions and removals in **View**, newest first. The menu item appears when Whaybench
has a saved diff for the file. Whaybench saves diffs for Claude Code's `Edit` and `Write` tools. Long changes can
be cut short, and older diffs can drop out as new edits are recorded.

## Avoid overwriting each other's edits

Before your next prompt goes in, Whaybench looks for files that changed after the agent last read or edited them. When
it finds a newer save, it can warn the agent at your next prompt or block its next edit and ask it to reread the file.
If it detects that Vim has the file open, it can block the edit and tell the agent to ask you before trying again.

Save and close the file before asking the agent to edit it. The warnings use clues from the file and Vim. A later
attempt can go through after a warning, and shell commands bypass these checks.

