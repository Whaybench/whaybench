# Window and appearance

Arrange the panels you use together, change how Whaybench looks, and set up typing in another language. You can also
restart Whaybench with your conversation tabs open or save a picture of the window.

## Move and resize panels

The first layout puts **Conversation map**, **Sessions**, and **Input** on the left, **View** in the middle, and your
conversation tabs above **Answer** on the right. **Files** shares a panel with **Sessions**. **Notifications** and
**Project** share a panel with **View**.

Drag a panel by its title bar. It snaps near the window margins and the edges of other panels. Drop it in free space
to place it on its own. Drag an edge or corner to resize it. Panels that share the edge resize together.

To move the whole window, drag the empty strip beside the menus. Double-click the strip to maximize or restore the
window, or drag the outer edge to resize it.

## Group and separate tabs

Hover over the middle of a single-tab panel's title bar to show the docking dot. Drag the dot onto a target on
another panel.

| Target | Where the panel goes |
|---|---|
| Center | Into the other panel as a tab. The preview says **As a tab of 'Sessions'** if **Sessions** is in front. |
| Edge | Beside the other panel, which makes room on that side. The preview says **Beside 'Sessions'** in this example. |

Click a tab to bring it forward. Drag it along the title bar to change the order of the tabs. To separate it,
drag the tab up or down, away from the title bar, until it becomes a panel of its own. Keep dragging to dock it
somewhere else. After a panel or a pulled-out tab is moved and the layout changes, click **↩** within ten seconds to
undo it while the arrow is available. A changed order of tabs has no **↩**: drag the tab back.

## Close, reopen, and reset panels

Click **×** on a panel's title bar or on the tab you want to close. To reopen a standard panel, choose it from
**Panels**, for example **Panels → Files**. The first **View** stays open. Choose **Panels → View** to bring it forward.

Choose **Panels → Reset layout** to put open panels back in their original places and groups. This also closes extra
views, branch maps, and imported maps. Windows on the desktop stay as they are. Use **Panels** to reopen any standard
panels you want back.

Whaybench restarts its own panels after a crash. If a panel keeps crashing, it stays closed and the status line
shows **Panel crashed repeatedly · logged**.

## Move a panel to the desktop and back

Drag the docking dot of a panel such as **Files** or **Browser** to the highlighted strip at the edge of the panel
area. Release when you see **Into a window of its own**. Separate a tab from its group first. The conversation
terminal and **Input** stay inside Whaybench.

Whaybench relaunches the panel on the desktop, where you can move it to another monitor. To bring it back, click the
return arrow with its name on the **Input** title bar. If it is not there, widen that panel. For **Browser**, click **↩
Browser** on the conversation title bar or choose **Panels → Browser**. Firefox restarts with the same profile. If it
takes too long to close, the move stops.

## Zoom and use shortcuts

| Key or gesture | Where | What it does |
|---|---|---|
| <kbd>Alt</kbd> + drag | Panel title bar | Moves the panel freely, with snapping and docking turned off for the drag. |
| <kbd>Ctrl</kbd> + mouse wheel | Panel title bar | Zooms all panel title bars from 0.7× to 2×. |
| <kbd>Ctrl</kbd> + mouse wheel | Inside a Whaybench panel such as **Sessions** or **View** | Zooms its contents from 0.5× to 3×, including pages shown in **View**. The map and its branch panels share one zoom. |
| <kbd>Enter</kbd> | **Input** | Sends the text to the selected conversation. |
| <kbd>Shift</kbd> + <kbd>Enter</kbd> | **Input** | Inserts a new line. |

Both zoom settings start at 1× and are saved for the next start.

## Restart Whaybench

Choose **Whaybench → Restart**. Whaybench saves the window size, panel positions and sizes, and tab groups. If it
cannot read the saved layout at startup, it uses the first layout.

Standard panels such as **Files** open again, even if you close them before restarting. Extra views you leave
open come back. Detached windows close when Whaybench exits.

Conversation tabs reopen in order. An empty tab reopens with the agent it had. A Claude Code session that carries on in
the background reattaches to its tab. Other agents reopen from the saved conversation. If a project is closed, Whaybench
skips its tabs.

## Choose and edit a theme

Choose a theme under **Settings → Theme**. It applies right away. If your selected theme is missing, Whaybench
uses **Default**.

Open **Settings → Theme settings…** to change fonts, colours, panel shape, and other details. Edits apply as you
make them. Choose **Clear changes (theme defaults)** to restore the selected theme's original settings.

To keep your version as a separate theme, click **Save as new theme…**, fill in **New theme name**, and click
**Save**. Whaybench selects the new theme. If it cannot save, the status line shows **Theme save failed** with the
reason. You can also [add a theme](../../addons/themes/README.md).

## Change the interface language

Choose **Settings → Language** and select **English** or an installed language. Then choose
**Whaybench → Restart**. If you have never chosen a language, Whaybench uses your system language when it is
available. Missing translations appear in English. See how to [add a language](../../addons/languages/README.md).

## Set up typing in another language

You can set up typing separately from the interface language. Whaybench uses **Built-in (fcitx5)** by default when
fcitx5 is available. To use your desktop's input method, choose **The desktop's** under **Input method** in **Settings →
Preferences…**, then restart. Whaybench also uses the desktop input method if fcitx5 is not installed. If fcitx5 is
there but does not start, fix it or choose **The desktop's**.

With fcitx5, open **Settings → Input…**. Under **Input methods**, choose an available method and click **Add**. Use
**Up** to move a method earlier in the list, **Remove** to drop it, and **Settings** where there is one for the engine's
own options. If the window offers the support package you need, select it and click **Install**. The status line reports
installation failures or tells you when to install the package with your package manager.

Under **Switch keys**, click **+** and press the combination you want to use. Press <kbd>Esc</kbd> to cancel or
click **×** beside an existing combination to remove it. The **Input** hint shows the switch keys when available.

If Whaybench cannot reach fcitx5, the settings window shows **fcitx5 not running · Input settings unavailable**.
The built-in input method cannot display fcitx5's candidate popups.

## Take a screenshot

With Whaybench running, save the whole window, including the menus and status line, as a PNG.

```sh
whaybench ctl shot --out whaybench.png
```

Use a fresh filename. A relative path starts in your terminal's current directory, and the command removes any
existing file at that path before capture. It prints the path on success. If it times out, bring the Whaybench
window into view and try again.

> [!NOTE]
> Whaybench removes its screenshots ten minutes after they are taken while a [Briefsafe project](briefsafe.md) is open
> or one of its pages is in **View**, whatever panel they show. Copy one elsewhere to keep it.

