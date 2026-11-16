//! The conversation map tool every agent calls, `context_map`: its name, description and input, what a call does,
//! and the map as the text a call with `show` returns. `whaybench mcp` serves it over MCP.
use std::{collections::HashSet, path::Path};

use serde_json::{Value, json};

use crate::{i18n, map::{self, Result}};

pub const NAME: &str = "context_map";

/// One node of a change, as `upsert` lists it; `nodes` (a whole regrouping) points to it instead of repeating it.
fn node_schema(required: &[&str]) -> Value {
    let rels = json!(["cause", "leads", "replaces", "related", "resolves"]);
    json!({"type": "object", "required": required, "properties": {
        "id": {"type": "string"},
        "label": {"type": "string"},
        "parent": {"type": "string", "description": "id of the parent node; empty string for a top-level topic"},
        "state": {"type": "string", "enum": ["", "plan", "doing", "done", "changed", "dropped", "postponed"], "description": "open or closed comes from this alone. plan = work put off for later, doing, done = finished or settled (closes), changed, dropped = given up (closes), postponed = put on hold by the person, empty = open again with no state. An open idea or question has no state"},
        "kind": {"type": "string", "enum": ["question", "option", "pro", "con", "conclusion", "task", "appointment", "problem", "idea", "fact", "rule", "note"], "description": "what the node is: question, option (pro and con under it), conclusion (a settled decision), task, appointment (with at), problem (what goes wrong; a decision resolves it), idea, fact, rule (a standing rule or preference), note. A topic has no kind"},
        "detail": {"type": "string", "description": "what does not fit the short label: the full sentence, a decision's reason, a problem's symptoms. For a task or problem about files: where (path:line), what, and the next step, so a later session need not look it up again. Omit when the label says it all"},
        "memid": {"type": "string", "description": "the memory id of an existing node in the memory list that is about the same problem; omit for a new problem"},
        "suggest": {"type": "object", "required": ["state", "why"], "properties": {"state": {"type": "string", "enum": ["done", "dropped"]}, "why": {"type": "string"}}, "description": "when you are not sure a node is finished or no longer needed: leave it open and suggest the state with a short reason; the person approves or declines"},
        "at": {"type": "string", "description": "when it happens: YYYY-MM-DD or YYYY-MM-DD HH:MM (an appointment)"},
        "due": {"type": "string", "description": "by when: YYYY-MM-DD or YYYY-MM-DD HH:MM"},
        "requires": {"type": "string", "description": "what must be done first: node ids of this map with ∧ (and), ∨ (or), ¬ (not), parentheses"},
        "assumes": {"type": "string", "description": "what must hold for it to make sense: node ids with ∧ ∨ ¬"},
        "reason": {"type": "string", "description": "one short line: why the conversation moved to this node; give it when a map style asks for it"},
        "links": {"type": "array", "items": {"type": "object", "required": ["to", "rel"], "properties": {"to": {"type": "string"}, "rel": {"type": "string", "enum": rels}}}, "description": "ties to nodes in other branches. resolves: this decision settles that problem (it closes, with what is open under it); replaces: this takes the place of that"},
        "source": {"type": "object", "required": ["file", "node"], "properties": {"file": {"type": "string"}, "node": {"type": "string"}, "title": {"type": "string"}, "path": {"type": "array", "items": {"type": "string"}}}, "description": "for a node taken up from an imported map: that map file and node; give it when Whaybench asks"}
    }})
}

/// The tool as an MCP client lists it. `inherit` is left out: only Whaybench's own callers use it.
pub fn definition() -> Value {
    json!({"name": NAME, "description": crate::i18n::ai("map-tool-description"), "inputSchema": {"type": "object", "properties": {
        "nodes": {"type": "array", "items": {"type": "object", "required": ["id", "label"]}, "description": "the whole map: nodes with the same fields as in upsert"},
        "upsert": {"type": "array", "items": node_schema(&["id"])},
        "remove": {"type": "array", "items": {"type": "string"}},
        "current": {"type": "string", "description": "id of the node the conversation is on now"},
        "show": {"type": "boolean"}
    }}})
}

/// One call: take over another session's map, apply the change, and answer with the map as text or a short line.
/// An Err is a refused call, told back to the agent so it sends the change again, fixed.
pub fn call(data: &Path, sid: &str, input: &Value) -> Result<String> {
    if let Some(from) = input.get("inherit").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        let source = map::view(data, map::safe_id(from)?)?;
        if source.get("nodes").is_none() { return Ok(format!("No map found for {from}.")); }
        map::take_over(data, sid, &source)?;
        // Whaybench follows this to show the old session's file records under the new one too.
        let links = data.join("forks").join("inherited.json");
        let mut known = map::read_json(&links, json!({}))?;
        known[sid] = json!(from);
        map::write_json(&links, &known)?;
    }
    let mut change = serde_json::Map::new();
    for key in ["nodes", "upsert", "remove", "current"] {
        if let Some(v) = input.get(key).filter(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty())) { change.insert(key.into(), v.clone()); }
    }
    let (mut notes, mut similar) = (Vec::new(), Vec::new());
    if !change.is_empty() {
        let before = map::command(data, sid, None)?;
        let sent: HashSet<String> = ["upsert", "nodes"].iter().filter_map(|k| change.get(*k).and_then(Value::as_array)).flatten()
            .filter_map(|n| n["id"].as_str().map(String::from)).collect();
        let sent_memid: HashSet<String> = ["upsert", "nodes"].iter().filter_map(|k| change.get(*k).and_then(Value::as_array)).flatten()
            .filter(|n| n.get("memid").is_some()).filter_map(|n| n["id"].as_str().map(String::from)).collect();
        let after = map::command(data, sid, Some(Value::Object(change))).map_err(|e| format!("Map not updated: {e}. Send the change again, fixed."))?;
        notes = checks(&before, &after, &sent);
        similar = alike(data, sid, &before, &after, &sent_memid);
    }
    if input.get("show") == Some(&json!(true)) { return Ok(as_text(&map::command(data, sid, None)?)); }
    let mut out = if notes.is_empty() { "Map updated.".to_string() } else { format!("{}\n{}", i18n::ai("map-check"), notes.join("\n")) };
    for l in similar { out.push('\n'); out.push_str(&l); }
    Ok(out)
}

/// Nodes of other sessions that may be about the same thing as a node this call added without a memory id: told
/// as a hint, never linked here, since whether two are the same problem is a judgement. Label trigram overlap of
/// 0.4 or more (measured on the store: below that most pairs were only alike in words such as "commit"); records
/// such as conclusions and facts are left out on both sides. At most 3 lines.
fn alike(data: &Path, sid: &str, before: &Value, after: &Value, sent_memid: &HashSet<String>) -> Vec<String> {
    fn grams(s: &str) -> HashSet<String> {
        let c: Vec<char> = s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect();
        if c.len() < 3 { return std::iter::once(c.iter().collect()).collect(); }
        c.windows(3).map(|w| w.iter().collect()).collect()
    }
    const RECORDS: [&str; 4] = ["conclusion", "fact", "note", "rule"];
    let had: HashSet<&str> = before["nodes"].as_array().into_iter().flatten().filter_map(|n| n["id"].as_str()).collect();
    let here: Vec<&Value> = after["nodes"].as_array().into_iter().flatten().collect();
    let on_map: HashSet<&str> = here.iter().filter_map(|n| n["nid"].as_str()).collect();
    let Ok(conn) = crate::db::open(data) else { return Vec::new() };
    let mut out = Vec::new();
    for n in here.iter().filter(|n| n["id"].as_str().is_some_and(|i| !had.contains(i) && !sent_memid.contains(i)) && !RECORDS.contains(&n["kind"].as_str().unwrap_or(""))) {
        let label = n["label"].as_str().unwrap_or("");
        let mine = grams(label);
        let words: Vec<String> = label.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 2).map(String::from).collect();
        // Many recent nodes share common words, so the window is wide: the trigram score picks among them.
        let found = crate::db::search(&conn, &words, 300).unwrap_or_default();
        let best = found.iter()
            .filter(|c| c["origin"].as_str().is_some_and(|o| o != sid) && c["state"] != "removed" && c["memid"] != n["memid"]
                && c["nid"].as_str().is_some_and(|i| !on_map.contains(i)) && !RECORDS.contains(&c["kind"].as_str().unwrap_or("")))
            .map(|c| { let g = grams(c["label"].as_str().unwrap_or("")); (mine.intersection(&g).count() as f64 / mine.union(&g).count().max(1) as f64, c) })
            .filter(|(score, _)| *score >= 0.4).max_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, c)) = best {
            let state = c["state"].as_str().filter(|s| !s.is_empty()).unwrap_or(if map::is_open(c) { "open" } else { "closed" });
            out.push(i18n::ai_f("map-similar", &[("id", n["id"].as_str().unwrap_or("").into()), ("memid", c["memid"].as_str().unwrap_or("").into()),
                ("label", c["label"].as_str().unwrap_or("").into()), ("state", state.into()), ("session", c["origin"].as_str().unwrap_or("").chars().take(4).collect())]));
        }
        if out.len() == 3 { break; }
    }
    out
}

/// What a call left that the record rules say otherwise, told back so the agent can fix it in its next call.
/// Only about the nodes this call sent or closed: what the person closed, dropped or approved is not questioned.
fn checks(before: &Value, after: &Value, sent: &HashSet<String>) -> Vec<String> {
    let nodes = |m: &Value| m["nodes"].as_array().cloned().unwrap_or_default();
    let (old, new) = (nodes(before), nodes(after));
    let was_open = |id: &str| old.iter().find(|n| n["id"] == id).is_none_or(map::is_open);
    let s = |n: &Value, k: &str| n[k].as_str().unwrap_or("").to_string();
    let closed_now: Vec<&Value> = new.iter().filter(|n| !map::is_open(n) && n["state"] != "removed" && was_open(&s(n, "id"))).collect();
    let mut out = Vec::new();
    let tell = |key: &str, id: &str| i18n::ai_f(key, &[("id", id.to_string())]);
    for n in new.iter().filter(|n| sent.contains(&s(n, "id"))) {
        let (id, kind, label) = (s(n, "id"), s(n, "kind"), s(n, "label"));
        if kind == "question" && n["state"] == "done" { out.push(tell("map-check-question", &id)); }
        if kind == "conclusion" && s(n, "detail").chars().count() < 10 { out.push(tell("map-check-reason", &id)); }
        if label.contains('→') && ["task", "question", "option", "problem"].contains(&kind.as_str()) { out.push(tell("map-check-kind", &id)); }
    }
    for n in &closed_now {
        let id = s(n, "id");
        if new.iter().any(|c| c["parent"] == id.as_str() && c["kind"] == "option" && map::is_open(c)) { out.push(tell("map-check-options", &id)); }
    }
    // A task or topic above a node this call closed, with nothing open under it any more.
    let mut parents: HashSet<String> = HashSet::new();
    for n in &closed_now {
        let mut p = s(n, "parent");
        while !p.is_empty() && parents.insert(p.clone()) { p = new.iter().find(|x| x["id"] == p.as_str()).map(|x| s(x, "parent")).unwrap_or_default(); }
    }
    for p in new.iter().filter(|n| parents.contains(&s(n, "id"))) {
        // Everything under it, not only its children: an open option under a closed question keeps the topic open.
        let mut under: Vec<String> = vec![s(p, "id")];
        let mut i = 0;
        while i < under.len() {
            let here = under[i].clone();
            under.extend(new.iter().filter(|c| c["parent"] == here.as_str()).map(|c| s(c, "id")));
            i += 1;
        }
        let open_below = new.iter().any(|c| under[1..].contains(&s(c, "id")) && c["state"] != "removed" && map::is_open(c));
        if map::is_open(p) && p["state"] != "postponed" && ["", "task"].contains(&s(p, "kind").as_str()) && !open_below {
            out.push(tell("map-check-parent", &s(p, "id")));
        }
    }
    out
}

/// The mark before a node: where the conversation is, else its state, else open or settled.
fn glyph(n: &Value, current: &str) -> &'static str {
    if n["id"] == current { return "●"; }
    match n["state"].as_str().unwrap_or("") {
        "plan" => "□", "doing" => "◐", "done" => "✓", "changed" => "✎", "dropped" => "✕", "postponed" => "‖",
        _ => if map::is_open(n) { "◇" } else { "✓" },
    }
}

/// The map as a tree of lines, `<mark> <label> [<id>]`, without the deleted nodes.
pub fn as_text(m: &Value) -> String {
    let nodes: Vec<&Value> = m["nodes"].as_array().into_iter().flatten().filter(|n| n["state"] != "removed").collect();
    let current = m["current"].as_str().unwrap_or("");
    let ids: std::collections::HashSet<&str> = nodes.iter().filter_map(|n| n["id"].as_str()).collect();
    let kids = |id: Option<&str>| -> Vec<&Value> {
        nodes.iter().copied().filter(|n| match (id, n["parent"].as_str()) {
            (None, p) => p.is_none_or(|p| !ids.contains(p)),
            (Some(id), p) => p == Some(id),
        }).collect()
    };
    fn walk<'a>(id: Option<&str>, indent: &str, kids: &dyn Fn(Option<&str>) -> Vec<&'a Value>, current: &str, out: &mut Vec<String>) {
        let list = kids(id);
        for (i, n) in list.iter().enumerate() {
            let last = i + 1 == list.len();
            let branch = if id.is_none() { "" } else if last { "└ " } else { "├ " };
            out.push(format!("{indent}{branch}{} {} [{}]", glyph(n, current), n["label"].as_str().unwrap_or(""), n["id"].as_str().unwrap_or("")));
            let next = if id.is_none() { indent.to_string() } else { format!("{indent}{}", if last { "  " } else { "│ " }) };
            if out.len() < 10_000 { walk(n["id"].as_str(), &next, kids, current, out); }
        }
    }
    let mut out = Vec::new();
    walk(None, "", &kids, current, &mut out);
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_tells_what_the_record_rules_say_otherwise_about_its_own_nodes() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        call(d, "s", &json!({"upsert": [{"id": "t", "label": "Topic"}, {"id": "q", "label": "Which?", "kind": "question", "parent": "t"},
            {"id": "o1", "label": "option: A", "kind": "option", "parent": "q"}, {"id": "o2", "label": "option: B", "kind": "option", "parent": "q"}]})).unwrap();
        assert_eq!(call(d, "s", &json!({"upsert": [{"id": "x", "label": "Side", "parent": "t"}]})).unwrap(), "Map updated.", "a clean call says only that");
        let told = call(d, "s", &json!({"upsert": [{"id": "q", "state": "done"}, {"id": "x", "state": "done"}]})).unwrap();
        for id in ["- q: closed as a question", "- q: closed, but options"] { assert!(told.contains(id), "{told}"); }
        assert!(!told.contains("- t:"), "options still open under q keep t open: {told}");
        let told = call(d, "s", &json!({"upsert": [{"id": "q", "label": "Which? → A", "kind": "conclusion", "detail": "A is the only one that fits"},
            {"id": "o1", "state": "done"}, {"id": "o2", "state": "dropped"}]})).unwrap();
        assert!(told.contains("- t: everything under it") && !told.contains("- q:"), "{told}");
        assert_eq!(call(d, "s", &json!({"upsert": [{"id": "t", "state": "done"}]})).unwrap(), "Map updated.");
        // What the person changes is not questioned.
        let t = map::view(d, "s").unwrap()["nodes"].as_array().unwrap().iter().find(|n| n["id"] == "t").unwrap()["nid"].clone();
        map::person(d, json!([{"id": t, "state": ""}])).unwrap();
        assert!(map::is_open(&map::view(d, "s").unwrap()["nodes"].as_array().unwrap().iter().find(|n| n["id"] == "t").unwrap().clone()));
        let told = call(d, "s", &json!({"upsert": [{"id": "y", "label": "Done? → yes", "kind": "task"}]})).unwrap();
        assert!(told.contains("- y: its label reads") && !told.contains("- t:"), "{told}");
    }

    #[test]
    fn a_new_node_like_one_of_another_session_gets_a_hint_not_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        call(d, "other", &json!({"upsert": [{"id": "p", "label": "The current node drops out of focus", "kind": "problem"}]})).unwrap();
        let memid = map::view(d, "other").unwrap()["nodes"][0]["memid"].as_str().unwrap().to_string();
        let told = call(d, "s", &json!({"upsert": [{"id": "q", "label": "Current node drops out of the focus?", "kind": "question"}]})).unwrap();
        assert!(told.starts_with("Map updated.\n- q may be about the same thing as ") && told.contains(&memid), "{told}");
        assert_ne!(map::view(d, "s").unwrap()["nodes"][0]["memid"].as_str(), Some(memid.as_str()), "not linked by the program");
        for up in [json!({"id": "a", "label": "Current node drops out of focus", "memid": memid}), json!({"id": "b", "label": "Current node drops out of focus → fixed", "kind": "conclusion", "detail": "the focus now follows the current node"}),
            json!({"id": "c", "label": "Pick a theme colour"})] {
            assert_eq!(call(d, "s", &json!({"upsert": [up]})).unwrap(), "Map updated.", "a memid given, a record, or nothing alike: no hint");
        }
    }

    #[test]
    fn a_call_changes_the_map_and_shows_it_as_the_plugin_did() {
        let dir = tempfile::tempdir().unwrap();
        let up = json!({"upsert": [{"id": "t", "label": "Topic"}, {"id": "q", "parent": "t", "label": "Which?", "kind": "question"},
            {"id": "x", "parent": "t", "label": "Old", "state": "dropped"}], "current": "q", "show": true});
        assert_eq!(call(dir.path(), "s", &up).unwrap(), "◇ Topic [t]\n├ ● Which? [q]\n└ ✕ Old [x]");
        assert!(call(dir.path(), "s", &json!({"upsert": [{"id": "q", "state": "done"}]})).unwrap().starts_with("Map updated."));
        assert!(call(dir.path(), "s", &json!({"upsert": [{"id": "z", "parent": "nowhere", "label": "z"}]})).unwrap_err().to_string().starts_with("Map not updated"));
        assert!(definition()["description"].as_str().unwrap().starts_with("Updates the conversation map"));
        call(dir.path(), "s2", &json!({"inherit": "s"})).unwrap();
        assert_eq!(map::command(dir.path(), "s2", None).unwrap()["nodes"].as_array().unwrap().len(), 3, "taken over");
    }
}
