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
| `title_size` | Size of the title bar text in pixels, 8 to 32; the bar's height follows it | `14` |
| `title_font` | Font of the title bar and tab text: one family, or several separated by commas as fontconfig takes them | `sans` |
| `title_weight` | Its weight, 100 (thin) to 900 (black), as CSS `font-weight`; a variable font is set to it | `700` |
| `title_spacing` | Pixels added between the letters of the title bar text, 0 to 10: wide spacing reads lighter and more formal | `0` |
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
| `dock_dot_size` | Diameter of the dot in the middle of a title bar that docks a panel | 10 |
| `dock_ring_size` | Diameter of the five targets on each panel while one is docked | 48 |

## `frame`: light and depth

In percent, from 0 to 100. At 0 (the default, but for `back_lift`) a theme stays flat.

| Key | Meaning | Default |
|---|---|---|
| `shadow_drop` | How far a panel's shadow falls below it, as a share of `shadow_size`; 0 keeps an even shadow all round | 0 |
| `back_lift` | How high the panels behind stand against the front one: their shadow falls this share as far | 55 |
| `border_light` | How much lighter a panel's border gets at the top | 0 |
| `border_shade` | How much darker a panel's border gets at the bottom | 0 |
| `bar_gloss` | How much lighter the upper half of a title bar is | 0 |
| `bar_shine` | A bright line along the top of a title bar, inside the border | 0 |
| `bar_shade` | A shade at the foot of a title bar, parting it from the panel | 0 |
| `surface_light` | A light above the window's top middle, on the surface behind the panels | 0 |
| `surface_vignette` | The surface darker towards the window's edges | 0 |
| `panel_light` | The same light on the panels' frames | 0 |
| `panel_shade` | The same darkening on the panels' frames towards the edges | 0 |

## `frame`: opacity

In percent, from 0 to 100; 100 (the default) is opaque.

| Key | Meaning | Default |
|---|---|---|
| `window_opacity` | The surface behind the panels; below 100 the desktop shows through, where the desktop allows it | 100 |
| `panel_opacity` | Every panel: its frame, title bar, tabs and app | 100 |
| `back_opacity` | The panels other than the front one, on top of `panel_opacity` | 100 |

`glass_blur`, in pixels from 0 to 64 (default 0, no glass): a floating panel or dialog shows what lies under it blurred this far, where the panel is see-through.

`dock_refraction`, in percent (default 0): the five docking targets enlarge what lies under them towards their middle, like a drop of water.

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
| `dock_dot` | The docking dot | `#e64553` |
| `dock_ring` | The docking targets: soft circles, see-through toward their edge | `base.accent` at half |

## `settings.json`

Same shape, only some keys; it lies over `theme.json`. The program writes what the person changes in `Settings → Theme settings…` here; `Save as new theme…` makes a new folder from `theme.json` with these values in it. Theme makers do not include it.
