# Projects

Use a project when work spans several conversations and you want one place for decisions, open work and documents. A
project keeps its own map and change history. New projects are [Briefsafe projects](briefsafe.md): they have a folder of
their own, kept apart and, where possible, encrypted. Projects made before that are called plain projects here; you do
not need one. [How they differ](#use-or-convert-a-plain-project).

## Create a project

Choose **Panels → Project** and click **New project**. Enter a **Project name**, choose a **Project map**, then
click **Create** or press <kbd>Enter</kbd> in the name field.

| Starting map | What you get |
| --- | --- |
| **Empty map** | A project you can fill in as you work |
| A saved map | A copy of the saved map as the first area in **Ideas** |
| Another conversation | Its whole map as the first area in **Ideas** |

The picker shows up to 15 saved maps and lists other conversations outside Briefsafe that have a map. To use the map of
an ordinary conversation you have selected, choose **Conversation map → Save map** first. If creation fails, read the
reason in the status line.

Whaybench makes the project folder under `~/work` when available, or in your home folder. To start with files from an
existing folder, run this from a terminal outside Briefsafe:

```sh
whaybench-project new redesign "Redesign" --folder "$HOME/work/redesign" --from "$HOME/old-site"
```

Replace the example paths with your own. Whaybench copies the source into a new folder and starts fresh records there; a
`.whaybench` folder in the source is not taken over. The destination must be outside the source. An existing destination
or an unreadable source stops creation.

## Open a project and start a conversation

Select the project in the panel's dropdown. A new Briefsafe project starts **locked**. Click **Log in** to see its
tabs. See [Briefsafe](briefsafe.md) for encryption and containers.

| Tab | What you can do |
| --- | --- |
| **Sessions** | Start or reopen a Briefsafe project conversation |
| **Ideas** | Read the project map. Its top-level entries are areas, with their items below. Use **Open items** to find work still open |
| A document's name | Read that document and ask for changes |
| **Recent changes** | See changes to the map and documents |

In **Sessions**, click an agent's name above the list to start a conversation. Click an existing conversation to open
it. To sign in to Claude Code for Briefsafe, click **Claude** beside **Inner login**. **Log out** asks for confirmation
when any of the project's conversation tabs are open. If you confirm, those tabs close.

## Reflect a map branch

Right-click a node in the [conversation map](conversation-map.md) or an opened map. Choose **Reflect in project**
and select an existing project. The request includes the branch and context from the nodes above it. It asks the
agent to compare the branch with **Ideas** and add only new information.

For a Briefsafe project, log in and keep a running conversation open in a tab. It goes to the conversation the branch
came from if that is in the project, else to the project's most recently active one. If Whaybench reports that no
conversation is running or no tab is open, open that one and send the branch again. A map from a Briefsafe project can
reflect only into that project. For a plain project, use a map and a selected conversation from outside Briefsafe.

A sent message means the request is queued. Check **Ideas** after the agent replies. When an item has a source link,
click it to open the map at that node. If the source map is missing, Whaybench reports that it cannot open it.

## Check the work queue

The work queue is the list of branches you sent to a project and how far the agent is with them.

Each reflection request creates a work item. To see its state, ask your agent to run the command below. Use a
conversation in the project for Briefsafe, and replace `ID` with the project ID.

```sh
whaybench-project queue ID
```

| State | Meaning |
| --- | --- |
| `waiting` | The agent has yet to claim the request |
| `taken` | A conversation has claimed the request |
| `done` | The agent has marked the request finished |

Another conversation can claim the same item after 30 minutes. Sending the branch again creates a new item.
Document change and restore requests go to the selected conversation instead, and go in when its turn ends.

## Add a document from a template

In a plain project, click **+ Document** and choose a template such as **WBS** or **Change proposal**. Whaybench
opens a copy in its own tab. Choose the same template again to make a numbered copy. Briefsafe projects copy the
installed templates when you create them. **+ Document** is available only in plain projects.

To install your own template, put a Markdown file in a folder under `addons/project-docs` in your Whaybench data folder.
The filename supplies the label in English. See the [template guide](../../addons/project-docs/README.md). If the folder
has only a supported office document or spreadsheet, Whaybench uses LibreOffice to make a Markdown copy, while the
project panel shows a plain project or none. If conversion fails, fix the reported cause and remove the `.error.txt`
file beside the original to retry.

## Ask for a document change

Select a running conversation with an open tab. For Briefsafe, choose one from the same project. Open the document
tab and use **Contents** to find the section you want.

Click **[Request change]** by a heading below the title, or **[Request change to this document]** at the top. Enter
**Request details** and click **Send**. If Whaybench reports **Request not sent · No conversation tab open**, open
the conversation and send the request again.

## Review or restore a change

Open **Recent changes**. It starts at **7 days** and shows the newest changes first. Choose **Today** for the last 24
hours, **30 days** for a longer view, or **All** for the history still available, up to 500 records. Click a section's
name to open the document there.

Where it appears, click **[Details]** to compare a map item or document section before and after the change. **[Restore
section]** and **[Restore item]** send a request to the selected conversation right away. Use **[Request rework from the
earlier version]** to tell the agent what to change from that version.

History keeps copies for up to 30 days and records for up to 365 days. The 50 MiB limit per project can shorten both.
You may see a record after its copy is gone.

## Archive a finished document

Ask your agent to archive the document. Its tab goes away, and you can still read the document and its history.
In a plain project, choose **+ Document → Archived documents** and select its name to read it while it is archived.

## Use or convert a plain project

Older projects are plain projects. They keep their records in your regular Whaybench data folder and use your regular
conversations. You can nest them under other plain projects or link them to working folders.

To convert one to Briefsafe, use a terminal outside Briefsafe. The project must have no parent or child projects.
Find its ID with the first command, then substitute it for `ID` and give the project a new folder:

```sh
whaybench-project list
whaybench-project convert ID --folder "$HOME/work/converted-project"
```

Add `--encrypt` to encrypt the project's records. Work files, earlier conversations and old backups stay outside the
encryption. Conversion moves the project map, documents, history and queue into the new folder. Find your earlier
conversations in your regular session list. If Whaybench cannot remove the old project records after conversion, it
reports a warning.

