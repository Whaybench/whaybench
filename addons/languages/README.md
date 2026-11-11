# Making a language

A language is an add-on: one folder, one language. Delete the folder and the language is gone.

```
<data folder>/addons/languages/<language code>/
  messages.ftl    the texts (Fluent format, https://projectfluent.org)
  language.json   { "format": 1, "name": "Deutsch", "version": "1.0" }
```

- The folder name is the language code: `de`, `ja`, `pt-BR`.
- It appears under `Settings → Language`. A change applies after a restart.
- With no choice made, the program uses the system language if there is a folder for it, otherwise English.

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
