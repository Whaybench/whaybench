# Making a theme

A theme is an add-on: one folder, one theme. Delete the folder and the theme and its settings are gone.

```
<data folder>/addons/themes/<theme id>/
  theme.json      the theme (required)
  settings.json   values changed on this computer (optional, written by the program)
```

- The default data folder is `~/.local/share/whaybench`.
- `<theme id>` is the folder name: lowercase letters, digits and `-`. `default` is the name of the theme in the program and cannot be used.
- A folder appears under `Settings → Theme`. Changes to the file show within 2 seconds.
- If the theme in use is deleted, the program goes back to the default.
- `name` is in English. A language add-on can translate it with the key `theme-<theme id>`.

The quickest way is to copy `black/theme.json` and change it.

## File format

```json
{
  "format": 1,
  "name": "Black",
  "version": "1.0",
  "palette": { "night": "#1a1b26" },
  "base":  { "background": "night" },
  "frame": { "gap": 12, "surface_top": "night" }
}
```

A missing key takes the default in the tables below. Unknown keys are ignored. A bad value takes the default for that key only. A file that is not valid JSON is not listed.

## Top level

| Key | Value | Meaning |
|---|---|---|
| `format` | number, now 1 | Version of this file format |
| `name` | text | Name in the menu; without one, the folder name |
| `version` | text | Version of the theme |
| `palette` | name: colour | Colours to call by name below |
| `base` | see below | The four base colours |
| `frame` | see below | Colours and sizes of the frame |

A **colour** is a `palette` name, `#rrggbb`, or `#rrggbbaa` with transparency.

## `base`: base colours

Panel apps take their text and background from these.

| Key | Meaning | Default |
|---|---|---|
| `background` | Background | `#eff1f5` |
| `surface` | Quieter surfaces | `#e6e9ef` |
| `text` | Text | `#4c4f69` |
| `accent` | Emphasis, links, chosen rows | `#1e66f5` |

## `base`: fonts

| Key | Meaning | Default |
|---|---|---|
| `font` | Font of the panels | `sans-serif` |
| `font_size` | Its size in points, 6 to 24 | `10` |
| `title_size` | Size of the title bar text in pixels, 8 to 32 | `13` |
| `terminal_font` | Font of the conversation terminals | `monospace` |
| `terminal_size` | Its size in points, 6 to 32 | `12` |

The terminals also take the panel colour (`frame.panel`) as their background and `text` as their text.

## `frame`: sizes

In screen pixels, from 0 to 64.

| Key | Meaning | Default |
|---|---|---|
| `outer` | Between the window's edge and a panel | 0 |
| `gap` | Between two panels side by side | 0 |
| `pad` | Around the app inside a panel (left, right, below) | 0 |
| `radius` | Corner radius of panels | 0 |
| `border_width` | Panel border width | 0 |
| `shadow_size` | Panel shadow size | 0 |
| `joint_width` | Width of joint lines | 2 |

## `frame`: when joint lines show

| Key | Value | Meaning | Default |
|---|---|---|---|
| `joint_show` | `always` | Always | `always` |
| | `active` | While dragging a panel or border, or with the pointer on a border | |
| | `never` | Never | |

## `frame`: colours

| Key | Meaning | Default |
|---|---|---|
| `surface_top` | Surface behind the panels, top | `base.background` |
| `surface_bottom` | Surface behind the panels, bottom (a gradient if different) | `surface_top` |
| `panel` | Inside of a panel | `base.background` |
| `bar_top` | Title bar, top | `base.surface` |
| `bar_bottom` | Title bar, bottom | `bar_top` |
| `front_top` | Title bar of the panel in front, top | `bar_top` |
| `front_bottom` | Title bar of the panel in front, bottom | `front_top` |
| `border` | Panel border | `base.surface` |
| `front_border` | Border of the panel in front | `base.accent` |
| `tab_front` | The tab in front | `base.background` |
| `shadow` | Panel shadow | `#00000040` |
| `shared` | Line where panels join | `#fe640b` |
| `wall` | Line along the window's edge | `#04a5e5` |
| `edge` | The border about to be dragged | `#d20f39` |

## `settings.json`

Same shape, only some keys; it lies over `theme.json`. The program writes what the person changes in `Settings → Theme settings…` here; `Save as new theme…` makes a new folder from `theme.json` with these values in it. Theme makers do not include it.
