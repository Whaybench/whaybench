# Get started with Whaybench

In about 15 minutes, you start a conversation, follow it on the map, and find a saved answer. You also open a page
beside the conversation and learn how to come back to your work.

Three words to know:

- The **conversation map** is a tree of topics, questions and decisions that your agent writes while you talk. Each
  entry is a node.
- A **session** is one conversation with an agent. It can be open in a tab, run in the background, or be closed and
  saved.
- A **project** is for longer work. It has a map and documents of its own and keeps them apart from your other
  conversations. You do not need one to start.

## Start Whaybench

1. On Linux, open a terminal in your Wayland desktop session and run:

   ```sh
   whaybench
   ```

   If your shell cannot find the command, add `~/.local/bin` to your `PATH` and try again. On Windows, open
   **Whaybench** from the Start menu or the desktop.

2. Whaybench opens as one window on your desktop, with its panels inside. The menus, **Whaybench**, **Sessions**,
   **Conversation map**, **Panels** and **Settings**, are along the top.

   Look for a **Claude Code** conversation tab on the right. If there is none, choose **Sessions → New conversation →
   Claude Code**. On the first start, the panels are arranged like this. Click a panel's tab to bring it forward.

   | Where | Panel | What you use it for |
   |---|---|---|
   | Left | **Conversation map** | Follow the topics, questions and decisions in the conversation. |
   | Left | **Sessions** | Find and reopen conversations. |
   | Left, next to **Sessions** | **Files** | See the files Whaybench records for the selected conversation. |
   | Left | **Input** | Send a prompt to the selected conversation. |
   | Middle | **View** | Read pages and files beside the conversation. |
   | Middle, next to **View** | **Notifications** | See what is due or waiting for your choice. |
   | Middle, next to **View** | **Project** | Open a project and see its map and documents. |
   | Right, top | Conversation tab | Work directly in your agent's terminal. |
   | Right, below the conversation | **Answer** | Read saved replies. |

## Sign in to your agent

3. Follow the sign-in prompts in the conversation tab. If your agent opens a browser for sign-in, finish there and
   return to the conversation tab. You are ready when the agent accepts a prompt.

   To use Codex, choose **Sessions → New conversation → Codex**. At the Codex prompt, use `/hooks` to review and trust
   the Whaybench hooks. If a tab closes with a launch error, check that the agent is installed, then open a new tab.

## Send a prompt and watch the map

4. Select your conversation tab. Type this in **Input** and press <kbd>Enter</kbd>. While it is empty, **Input** shows
   which conversation gets the text. If it says **None**, or the status line at the bottom says the request was not
   sent, type the prompt in the conversation tab instead.

   > Help me choose between Markdown and plain text for short project notes. Give one reason for each and put the
   > question and options on the conversation map.

   Look for the reply in the conversation tab and the saved reply in **Answer**.

5. Look for the question and its options in **Conversation map**. Click `▸` if you see a folded branch. Send a second
   prompt to settle the question.

   > Let's use Markdown because the notes need headings and lists. Record the decision on the map, with the reason in
   > its detail.

   Look for the decision on the map. Hover over its label to read the reason. If the map stays empty after both replies,
   follow [Fill an empty conversation map](troubleshooting.md#fill-an-empty-conversation-map).

## Find a saved answer

6. If **Answer** is closed, choose **Panels → Answer**. Right-click the decision on the map and choose **Related
   answers**. **Answer** shows the reply from the turn where the node was added. A question that became a decision later
   opens at the question: press `▶` to reach the decision. `◀` and `▶` move through the replies of that conversation.
   Click `✕` to close the related answers. If Whaybench says **No related answers · No answers kept after this**, try a
   node from an earlier turn.

## Open a page beside the conversation

7. Choose **Whaybench → About → User guide**. The Whaybench README opens in the middle of the window. If
   the page fails to load, check your internet connection and try again.

## End the conversation and come back

8. Select your conversation tab and choose **Sessions → End this tab's session**. The tab closes and the conversation
   moves under **Hidden sessions** in the **Sessions** panel. Nothing is lost.

9. Choose **Whaybench → Quit**, then start Whaybench again. In the **Sessions** panel, expand **Hidden sessions** and
   click your conversation. Its tab opens again and you can see its map in **Conversation map**.

## Keep going

- Find earlier conversations and manage their tabs. [Sessions](guide/sessions.md)
- Read branches and reuse a map in a new conversation. [Conversation map](guide/conversation-map.md)
- Open files and work with more than one view. [View and files](guide/view-and-files.md)
- Move, resize and group panels. [Window and appearance](guide/window-and-appearance.md)

