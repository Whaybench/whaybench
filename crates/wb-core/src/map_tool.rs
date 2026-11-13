//! The conversation map tool every agent calls, `context_map`: its name, description and input, what a call does,
//! and the map as the text a call with `show` returns. `whaybench mcp` serves it over MCP.
use std::path::Path;

use serde_json::{Value, json};

use crate::map::{self, Result};

pub const NAME: &str = "context_map";

/// One node of a change; `nodes` (a whole regrouping) and `upsert` share it.
fn node_schema(required: &[&str]) -> Value {
    let rels = json!(["cause", "leads", "replaces", "related", "resolves"]);
    json!({"type": "object", "required": required, "properties": {
        "id": {"type": "string"},
        "label": {"type": "string"},
        "parent": {"type": "string", "description": "id of the parent node; empty string for a top-level topic"},
        "state": {"type": "string", "enum": ["", "plan", "doing", "done", "changed", "dropped", "postponed"], "description": "open or closed comes from this alone. plan = work put off for later, doing, done = finished or settled (closes), changed, dropped = given up (closes), postponed = put on hold by the person, empty = open again with no state. An open idea or question has no state"},
        "kind": {"type": "string", "enum": ["question", "option", "pro", "con", "conclusion", "task", "appointment", "problem", "idea", "fact", "rule", "note"], "description": "what the node is: question, option (pro and con under it), conclusion (a settled decision), task, appointment (with at), problem (what goes wrong; a decision resolves it), idea, fact, rule (a standing rule or preference), note. A topic has no kind"},
        "detail": {"type": "string", "description": "what does not fit the short label: the full sentence, a decision's reason, a problem's symptoms. Omit when the label says it all"},
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

/// The tool as an MCP client lists it.
pub fn definition() -> Value {
    json!({"name": NAME, "description": crate::i18n::ai("map-tool-description"), "inputSchema": {"type": "object", "properties": {
        "nodes": {"type": "array", "items": node_schema(&["id", "label"])},
        "upsert": {"type": "array", "items": node_schema(&["id"])},
        "remove": {"type": "array", "items": {"type": "string"}},
        "current": {"type": "string", "description": "id of the node the conversation is on now"},
        "show": {"type": "boolean"},
        "inherit": {"type": "string", "description": "a session id whose map this session takes over, replacing its own"}
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
    if !change.is_empty() {
        map::command(data, sid, Some(Value::Object(change))).map_err(|e| format!("Map not updated: {e}. Send the change again, fixed."))?;
    }
    if input.get("show") == Some(&json!(true)) { return Ok(as_text(&map::command(data, sid, None)?)); }
    Ok("Map updated.".into())
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
    fn a_call_changes_the_map_and_shows_it_as_the_plugin_did() {
        let dir = tempfile::tempdir().unwrap();
        let up = json!({"upsert": [{"id": "t", "label": "Topic"}, {"id": "q", "parent": "t", "label": "Which?", "kind": "question"},
            {"id": "x", "parent": "t", "label": "Old", "state": "dropped"}], "current": "q", "show": true});
        assert_eq!(call(dir.path(), "s", &up).unwrap(), "◇ Topic [t]\n├ ● Which? [q]\n└ ✕ Old [x]");
        assert_eq!(call(dir.path(), "s", &json!({"upsert": [{"id": "q", "state": "done"}]})).unwrap(), "Map updated.");
        assert!(call(dir.path(), "s", &json!({"upsert": [{"id": "z", "parent": "nowhere", "label": "z"}]})).unwrap_err().to_string().starts_with("Map not updated"));
        assert!(definition()["description"].as_str().unwrap().starts_with("Updates the conversation map"));
        call(dir.path(), "s2", &json!({"inherit": "s"})).unwrap();
        assert_eq!(map::command(dir.path(), "s2", None).unwrap()["nodes"].as_array().unwrap().len(), 3, "taken over");
    }
}
