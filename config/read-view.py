#!/usr/bin/env python3
"""Reads the panel Firefox through accessibility, as text: what the person sees, the selected sentences, the
address, and the page of a PDF. Arguments: the Firefox process id, and --all for the whole page."""
import json
import sys

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi  # noqa: E402

pid, whole = int(sys.argv[1]), "--all" in sys.argv
Atspi.init()
desktop = Atspi.get_desktop(0)
app = next((a for a in (desktop.get_child_at_index(i) for i in range(desktop.get_child_count())) if a and a.get_process_id() == pid), None)
if app is None:
    print(json.dumps({"error": "Firefox not on the accessibility bus"}))
    sys.exit(0)


def documents(o, depth=0, out=None):
    out = [] if out is None else out
    if o.get_role() == Atspi.Role.DOCUMENT_WEB:
        out.append(o)
    elif depth < 30:
        for i in range(o.get_child_count()):
            c = o.get_child_at_index(i)
            if c:
                documents(c, depth + 1, out)
    return out


def collect(o, texts, selected, entries):
    ti = o.get_text_iface()
    if ti:
        t = Atspi.Text.get_text(ti, 0, Atspi.Text.get_character_count(ti)).replace("￼", "").strip()
        if t:
            shown = o.get_state_set().contains(Atspi.StateType.SHOWING)
            texts.append((t, shown))
            if o.get_role() == Atspi.Role.SPIN_BUTTON:
                entries.append(t)
        for k in range(Atspi.Text.get_n_selections(ti)):
            r = Atspi.Text.get_selection(ti, k)
            s = Atspi.Text.get_text(ti, r.start_offset, r.end_offset).replace("￼", "").strip()
            if s:
                selected.append(s)
    for i in range(o.get_child_count()):
        c = o.get_child_at_index(i)
        if c:
            collect(c, texts, selected, entries)


found = [d for d in documents(app) if d.get_state_set().contains(Atspi.StateType.SHOWING)]
if not found:
    print(json.dumps({"error": "no page showing"}))
    sys.exit(0)
doc = found[0]
texts, selected, entries = [], [], []
collect(doc, texts, selected, entries)
attrs = doc.get_document_iface().get_document_attributes() or {}
url = attrs.get("DocURL", "")
out = {"url": url, "title": doc.get_name(), "selected": selected,
       "text": [t for t, shown in texts if whole or shown]}
if url.lower().split("#")[0].endswith(".pdf") and entries:
    out["page"] = entries[0]
    out["pages"] = next((t.lstrip("/ ").split()[-1] for t, _ in texts if t.startswith(("/ ", "of "))), "")
print(json.dumps(out, ensure_ascii=False))
