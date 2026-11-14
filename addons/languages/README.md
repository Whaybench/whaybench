# Making a language

A language is an add-on: one folder, one language. Delete the folder and the language is gone.

```
<data folder>/addons/languages/<language code>/
  messages.ftl    the texts (Fluent format, https://projectfluent.org)
  language.json   { "format": 1, "name": "Deutsch", "english": "German", "version": "1.0" }
```

- The folder name is the language code: `de`, `ja`, `pt-BR`.
- `english` is the language's name in English. Claude Code started in a conversation tab is told to answer in it
  (its `language` setting), unless the person set one in `~/.claude/settings.json`.
- It appears under `Settings → Language`. A change applies after a restart.
- With no choice made, the program uses the system language if there is a folder for it, otherwise English.

## Keyboard and input method: `input`

Whaybench types through fcitx5. `input` in `language.json` says what the language needs from it:

```
"input": { "layout": "de" }
"input": { "layout": "jp", "engines": [{ "name": "mozc", "package": "fcitx5-mozc" }] }
"input": { "layout": "us", "engines": [{ "name": "pinyin", "package": "fcitx5-chinese-addons" },
                                       { "name": "rime", "package": "fcitx5-rime" }] }
```

- `layout` is the keyboard layout, an XKB name (`us`, `de`, `fr`, `jp`). Without `input` it is `us`.
- `engines` are the fcitx5 input methods the language types with, if it needs one; the first is the default.
  `name` is the input method's fcitx5 name, `package` the system package that brings it.
- When the language is chosen in `Settings → Language` and none of its engines is installed, Whaybench offers to
  install the first one's package: through PackageKit on a desktop (the system asks for the password), as root
  through `wsl.exe` under WSL. Then the engine is added to fcitx5's current input method group, so the language types
  at once; nothing already in the group is changed.

## Writing `messages.ftl`

Copy `en.ftl` from this folder and translate the right side of each line:

```
session-off = Session { $a1 } is off
```

- Keep the key on the left and every `{ $name }` as it is. Their order in the sentence may change.
- `{"\u000A"}` is a new line. `{" "}` keeps a space at the start or end of a text.
- HTML tags in a text stay as they are.
- Add-ons are in English. To translate their names add `map-style-<folder> = ...`, `theme-<folder> = ...` or `doc-<folder> = ...`; without one the English name shows.
- Only what people see is translated. What Whaybench sends to the agent is in `config/lang/ai.ftl` and stays English in every language.
- A key you leave out shows in English, so a new version of the program never breaks a language. Compare your file with the new `en.ftl` to find what to add.
