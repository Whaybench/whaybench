# Making a project document template

A project document template is an add-on: one folder, one document.

```
<data folder>/addons/project-docs/<template id>/
  <document name>.md     the template the AI fills in
  <document name>.docx   the original, if there is one
```

- `<template id>` is the folder name: lowercase letters, digits and `-`.
- One document per folder. The file name is the tab name in the project panel. A language add-on can translate it with the key `doc-<template id>`.
- A folder appears in the project panel's `+ Document` list. Deleting it removes it from the list; documents already added to a project stay.

## In Markdown

Use ordinary Markdown. Headings (`#`, `##`) are the sections; changes are tracked per section.

To tell the AI how to write a part, put a comment there. People do not see it.

```markdown
## 2. Requirements

<!-- guide: Number each one and give how it is checked. -->
```

## From Word or Excel

Put a `.docx` `.doc` `.odt` `.rtf`, `.xlsx` `.xls` or `.ods` file in a folder with no Markdown, and the program converts it to Markdown under the same name. This needs LibreOffice. The original is kept.

In a spreadsheet, each sheet becomes one section with one table.

Conversion carries the structure only. Fonts, colours, column widths, headers and footers are lost, and merged cells can leave rows with a different number of columns.

## After it is in a project

Only the AI edits project documents. People ask through `[Request change]` in the panel. To follow the look of the original, say so; the AI is given where the original is.
