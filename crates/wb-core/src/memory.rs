//! The memory: every node of every conversation, stored once (`db`), and how each came to be what it is.
//! This is what a session and the person are shown of it: the memory list, what changed, a node with its events,
//! search, and the notifications panel's sections.
use std::{collections::{HashMap, HashSet}, fs, path::{Path, PathBuf}};

use serde_json::{Value, json};

use crate::{db, map::{self, Result, is_open}};

/// Open nodes, as a condition over the node columns.
const OPEN: &str = "coalesce(state, '') NOT IN ('done', 'dropped', 'removed')";
/// Most recently changed first.
const RECENT: &str = "coalesce(json_extract(changed, '$.at'), json_extract(opened, '$.at')) DESC";

/// A file the session read, changed or attached to a node: `file.touched`, so every session's nodes that dealt with a
/// file can be found by its path. `node` is the session's id for the node.
pub fn file_touched(data: &Path, sid: &str, op: &str, path: &str, node: Option<&str>) -> Result<()> {
    if !["read", "edit", "attach"].contains(&op) { return Err(format!("file touch must be read, edit or attach: {op}").into()); }
    let entries = map::session_entries(data, sid);
    let nid = node.and_then(|id| entries.iter().find(|e| e.0 == id)).map(|e| e.1.as_str());
    let mut e = map::event(&format!("session/{sid}"), "file.touched", nid, Some(sid), chrono::Local::now().timestamp_millis(), json!({"path": path, "op": op}));
    e["path"] = json!(path);
    db::event_add(&db::open(data)?, &e)
}

fn when(n: &Value) -> i64 { ["changed", "opened"].iter().find_map(|k| n.get(*k)?.get("at")?.as_i64()).unwrap_or(0) }

/// One node in one line: `memory id · label · state or kind · the session it came from · last change`.
pub fn line(n: &Value) -> String {
    let from = n["origin"].as_str().unwrap_or("");
    let key = n.get("memid").or_else(|| n.get("nid")).and_then(Value::as_str).unwrap_or("");
    let what = n.get("state").or_else(|| n.get("kind")).and_then(Value::as_str).unwrap_or(if is_open(n) { "open" } else { "closed" });
    let day = chrono::DateTime::from_timestamp_millis(when(n)).map(|t| t.with_timezone(&chrono::Local).format("%m-%d").to_string()).unwrap_or_default();
    let due = n.get("due").or_else(|| n.get("at")).and_then(Value::as_str).map(|d| format!(" · due {d}")).unwrap_or_default();
    // A decision's reason travels with it: a new session misses the why more than the what.
    let why = n.get("detail").and_then(Value::as_str).filter(|_| n["kind"] == "conclusion")
        .map(|d| format!(" · why: {}", d.split(['.', '。', '\n']).next().unwrap_or("").trim().chars().take(60).collect::<String>())).unwrap_or_default();
    format!("{key} · {} · {what} · {} · {day}{due}{why}", n["label"].as_str().unwrap_or(""), &from[..4.min(from.len())])
}

/// Every event in the store, oldest first. shortcut: reads all of them each time; page it when the History tab grows slow.
pub fn events(data: &Path) -> Vec<Value> {
    db::open(data).and_then(|c| db::events(&c, "1", &[])).unwrap_or_default()
}

/// Changes whenever the events did: a panel reads them again only then.
pub fn events_mark(data: &Path) -> String { db::open(data).and_then(|c| db::events_mark(&c)).unwrap_or_default() }

/// One node's name as it is now; None for a node that is gone.
pub fn label_of(data: &Path, nid: &str) -> Option<String> {
    db::node_get(&db::open(data).ok()?, nid).ok()??.get("label")?.as_str().map(String::from)
}

/// One session's node changes in the order they happened: (turn, time in ms, the session's id for the node, whether it closed).
pub fn steps(data: &Path, sid: &str) -> Vec<(i64, i64, String, bool)> {
    let entries = map::session_entries(data, sid);
    let evs = db::open(data).and_then(|c| db::events(&c, "session = ? AND type IN ('node.added', 'node.changed')", &[sid])).unwrap_or_default();
    evs.into_iter().filter_map(|e| {
        let id = entries.iter().find(|x| Some(x.1.as_str()) == e["nid"].as_str())?.0.clone();
        let at = chrono::DateTime::parse_from_rfc3339(e["time"].as_str()?).ok()?.timestamp_millis();
        // A change holds [before, after]; a node made closed holds the value itself.
        let state = if e["type"] == "node.added" { &e["data"]["state"] } else { &e["data"]["state"][1] };
        let closed = !is_open(&json!({"state": state})) && (e["type"] == "node.added" || is_open(&json!({"state": e["data"]["state"][0]})));
        Some((e["turn"].as_i64().unwrap_or(0), at, id, closed))
    }).collect()
}

/// The paths a session touched, and the nodes other sessions had in hand when they touched the same ones.
/// shortcut: reads every file event; index by path in SQL when there are many.
fn by_file(conn: &rusqlite::Connection, sid: &str) -> Result<(HashSet<String>, HashSet<String>)> {
    let mine: HashSet<String> = db::events(conn, "type = 'file.touched' AND session = ?", &[sid])?.iter().filter_map(|e| e["path"].as_str().map(String::from)).collect();
    let nids = db::events(conn, "type = 'file.touched' AND session != ? AND nid IS NOT NULL", &[sid])?.iter()
        .filter(|e| e["path"].as_str().is_some_and(|p| mine.contains(p))).filter_map(|e| e["nid"].as_str().map(String::from)).collect();
    Ok((mine, nids))
}

/// The memory list a session sees (3.19): A its own open nodes, C related nodes not on its map (same memory id,
/// same files, due soon), B recent nodes not on its map; one line each, at most `limit` lines.
pub fn list(data: &Path, sid: &str, limit: usize) -> Vec<String> {
    let inner = || -> Result<Vec<String>> {
        let conn = db::open(data)?;
        let own = map::view(data, sid)?.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut seen: HashSet<String> = own.iter().filter(|n| !is_open(n)).filter_map(|n| n["nid"].as_str().map(String::from)).collect();
        let mut out = Vec::new();
        let cap = limit + own.len();
        let mut push = |n: &Value, out: &mut Vec<String>| if out.len() < limit && n["state"] != "removed" && seen.insert(n["nid"].as_str().unwrap_or("").to_string()) { out.push(line(n)); };
        for n in own.iter().filter(|n| is_open(n)) { push(n, &mut out); }
        let own_mem: HashSet<&str> = own.iter().filter_map(|n| n.get("memid")?.as_str()).collect();
        for m in &own_mem { for n in db::nodes(&conn, "memid = ?", &[m])? { push(&n, &mut out); } }
        for nid in by_file(&conn, sid)?.1 { if let Some(n) = db::node_get(&conn, &nid)? { push(&n, &mut out); } }
        let soon = (chrono::Local::now().date_naive() + chrono::Duration::days(7)).format("%Y-%m-%d 99").to_string();
        for n in db::nodes(&conn, &format!("{OPEN} AND coalesce(due, at) <= ?"), &[&soon])? { push(&n, &mut out); }
        // A conversation given only part of a map (a display style such as Focus) gets only open nodes of others too.
        let which = if part_carried(data, sid).exists() { OPEN } else { "1" };
        for n in db::nodes(&conn, &format!("{which} ORDER BY {RECENT} LIMIT {cap}"), &[])? { push(&n, &mut out); }
        Ok(out)
    };
    inner().unwrap_or_else(|e| unread("list", e))
}

/// The memory could not be read: said in the log, so an empty list is not taken for "nothing there".
fn unread(what: &str, e: Box<dyn std::error::Error>) -> Vec<String> {
    crate::log::warn("memory", "memory not read", &e.to_string(), &[("for", what.to_string())]);
    Vec::new()
}

/// Marks a session that started from part of a map; holds the display style it was passed in.
pub fn part_carried(data: &Path, sid: &str) -> PathBuf { data.join("cache").join("carried-part").join(sid) }

fn seen_file(data: &Path, sid: &str) -> PathBuf { data.join("cache").join("memory-seen").join(sid) }

/// Marks the memory as seen by the session up to now.
pub fn mark_seen(data: &Path, sid: &str) {
    let f = seen_file(data, sid);
    let _ = fs::create_dir_all(f.parent().unwrap_or(data));
    let _ = crate::paths::write_atomic(&f, chrono::Local::now().timestamp_millis().to_string());
}

fn seen_at(data: &Path, sid: &str) -> i64 { fs::read_to_string(seen_file(data, sid)).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0) }

fn after(e: &Value, since: i64) -> bool { chrono::DateTime::parse_from_rfc3339(e["time"].as_str().unwrap_or("")).is_ok_and(|t| t.timestamp_millis() > since) }

/// What changed for this session since it last saw the memory: other sessions' and the person's events on its
/// nodes, on its memory ids, or on the files it touched. Marks them seen.
pub fn delta(data: &Path, sid: &str) -> Vec<String> {
    let since = seen_at(data, sid);
    let inner = || -> Result<Vec<String>> {
        let conn = db::open(data)?;
        let own = map::view(data, sid)?.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
        let ids: HashMap<&str, &str> = own.iter().filter_map(|n| Some((n["nid"].as_str()?, n["id"].as_str()?))).collect();
        let own_mem: HashSet<&str> = own.iter().filter_map(|n| n.get("memid")?.as_str()).collect();
        let mine = by_file(&conn, sid)?.0;
        // Only what came after the session last looked is read (the store keeps times as RFC 3339 in UTC, which sort as text).
        let from = chrono::DateTime::from_timestamp_millis(since).map(|t| t.to_rfc3339()).unwrap_or_default();
        Ok(db::events(&conn, "source != ? AND time >= ?", &[&format!("session/{sid}"), &from])?.iter().filter(|e| after(e, since))
            .filter(|e| e["nid"].as_str().is_some_and(|n| ids.contains_key(n)) || e["memid"].as_str().is_some_and(|m| own_mem.contains(m)) || e["path"].as_str().is_some_and(|p| mine.contains(p)))
            // The node as this session knows it: its id on this map, else its memory id.
            .map(|e| { let node = e["nid"].as_str().and_then(|n| ids.get(n).copied()).or(e["memid"].as_str()).unwrap_or("");
                format!("{} · {} · {node} · {}", e["type"].as_str().unwrap_or(""), e["source"].as_str().unwrap_or(""), e["data"]) })
            .collect())
    };
    let out = inner().unwrap_or_else(|e| unread("changes", e));
    mark_seen(data, sid);
    out
}

/// A node by memory id, node number or `<session>/<the session's id for it>`: every node it names, and their events.
pub fn show(data: &Path, key: &str) -> Value {
    let inner = || -> Result<Value> {
        let conn = db::open(data)?;
        let on_map = key.split_once('/').and_then(|(s, id)| map::session_entries(data, s).into_iter().find(|e| e.0 == id)).map(|e| e.1);
        let key = on_map.as_deref().unwrap_or(key);
        let (mut nodes, mut evs) = (Vec::new(), Vec::new());
        for n in db::nodes(&conn, "memid = ?1 OR nid = ?1", &[key])? {
            let nid = n["nid"].as_str().unwrap_or("");
            evs.extend(db::events(&conn, "nid = ?", &[nid])?);
            nodes.extend(db::node_get(&conn, nid)?);
        }
        Ok(json!({"nodes": nodes, "events": evs}))
    };
    inner().unwrap_or_else(|e| json!({"error": e.to_string()}))
}

/// Nodes whose label or detail holds any of the words (any case), most recent first.
pub fn search(data: &Path, words: &[String], limit: usize) -> Vec<String> {
    db::open(data).and_then(|c| db::search(&c, words, limit)).unwrap_or_default().iter().map(line).collect()
}

/// One node as the person reads it: `label · state · due · session name` (no ids).
fn person_line(name: &str, n: &Value) -> String {
    let mark = if n.get("mark").is_some() { "! " } else { "" };
    let mut parts = vec![format!("{mark}{}", n["label"].as_str().unwrap_or(""))];
    if let Some(s) = n.get("state").and_then(Value::as_str) { parts.push(crate::i18n::t(&format!("state-{s}")).to_string()); }
    if let Some(d) = n.get("due").or_else(|| n.get("at")).and_then(Value::as_str) { parts.push(d.to_string()); }
    parts.push(name.chars().take(30).collect());
    parts.join(" · ")
}

/// The notifications panel's sections (3.19): nodes with a time or due date, the person's marked ones, what changed since the
/// person last looked, held ones and the deleted ones. Each item is `{session, node, line}`: `session` is where the
/// node came from, and the node's `id` is its node number, the one `map::person`, `trash` and `purge` take.
/// `current` is the conversation on show: its open nodes get a section of their own, so any of them can be marked.
/// shortcut: reads every node each time; ask the store per section when there are many.
pub fn board(data: &Path, current: Option<&str>) -> Value {
    let Ok(conn) = db::open(data) else { return json!({}) };
    let mut all = db::nodes(&conn, &format!("1 ORDER BY {RECENT}"), &[]).unwrap_or_default();
    for n in &mut all { n["id"] = n["nid"].clone(); }
    let d = crate::paths::Data::new(data.to_path_buf());
    let mut names: HashMap<String, String> = HashMap::new();
    let mut name = |n: &Value| { let s = n["origin"].as_str().unwrap_or(""); names.entry(s.to_string()).or_insert_with(|| crate::sessions::session_name(&d, s)).clone() };
    let mut item = |n: &Value, line: Option<String>| { let who = name(n); json!({"session": n["origin"], "node": n, "line": line.map(|l| format!("{l} · {who}")).unwrap_or_else(|| person_line(&who, n))}) };
    let (mut due, mut marked, mut check, mut held, mut trash) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for n in &all {
        if n["state"] == "removed" { trash.push(item(n, None)); continue; }
        if n["state"] == "postponed" { held.push(item(n, None)); }
        // The person's mark holds whatever the state: what they marked they want to see again.
        if n.get("mark").is_some() { marked.push(item(n, None)); }
        if !is_open(n) { continue; }
        // The agent's suggestion to close it: the person approves or declines.
        if let Some(sg) = n.get("suggest").filter(|v| v.is_object()) {
            let to = crate::i18n::t(if sg["state"] == "dropped" { "notify-dropped" } else { "notify-done" });
            check.push(item(n, Some(format!("{} · {}", n["label"].as_str().unwrap_or(""), crate::i18n::f("notify-suggested", &[("state", to.to_string()), ("why", sg["why"].as_str().unwrap_or("").to_string())])))));
        }
        if n.get("due").or_else(|| n.get("at")).is_some() { due.push(item(n, None)); }
    }
    let date = |i: &Value| i["node"].get("due").or_else(|| i["node"].get("at")).and_then(Value::as_str).unwrap_or("").to_string();
    due.sort_by_key(date);
    let since = seen_at(data, "person");
    let node_of = |nid: &str| all.iter().find(|n| n["nid"] == nid);
    let mut changed: Vec<Value> = Vec::new();
    // Settled by a decision since the person last looked: the problem, and the decision that settled it.
    for e in db::events(&conn, "type = 'link.added'", &[]).unwrap_or_default().iter().rev().filter(|e| e["data"]["rel"] == "resolves" && after(e, since)) {
        let (Some(target), Some(decision)) = (node_of(e["data"]["to"].as_str().unwrap_or("")), node_of(e["nid"].as_str().unwrap_or(""))) else { continue };
        changed.push(item(target, Some(format!("{} · {}", target["label"].as_str().unwrap_or(""), crate::i18n::f("notify-resolved", &[("by", decision["label"].as_str().unwrap_or("").to_string())])))));
    }
    // What can be done now: an open node whose condition holds since one of its nodes closed after the person last looked.
    for n in all.iter().filter(|n| is_open(n)) {
        let Some(expr) = n.get("requires").and_then(Value::as_str) else { continue };
        let closed_at = |nid: &str| node_of(nid).filter(|m| !is_open(m)).and_then(|m| m.get("closed")?.get("at")?.as_i64());
        let late = map::condition_ids(expr).unwrap_or_default().iter().any(|id| closed_at(id).is_some_and(|at| at > since));
        if late && map::condition_holds(expr, &|id| closed_at(id).is_some()) {
            changed.push(item(n, Some(format!("{} · {}", n["label"].as_str().unwrap_or(""), crate::i18n::t("notify-unblocked")))));
        }
    }
    // The action that deleted each node: the last event that set it to removed.
    if !trash.is_empty() {
        let mut by: HashMap<String, Value> = HashMap::new();
        for e in db::events(&conn, "action IS NOT NULL AND type = 'node.changed'", &[]).unwrap_or_default().iter().filter(|e| e["data"]["state"][1] == "removed") {
            if let Some(nid) = e["nid"].as_str() { by.insert(nid.to_string(), e["action"].clone()); }
        }
        for t in &mut trash { t["action"] = by.get(t["node"]["nid"].as_str().unwrap_or("")).cloned().unwrap_or(Value::Null); }
    }
    // The conversation on show, in its map's order.
    let open: Vec<Value> = current.map(|s| map::session_entries(data, s)).unwrap_or_default().iter()
        .filter_map(|(_, nid)| node_of(nid)).filter(|n| is_open(n)).map(|n| item(n, None)).collect();
    json!({"due": due, "marked": marked, "changed": changed, "check": check, "open": open, "held": held, "trash": trash})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(dir: &Path, sid: &str, nodes: Value) -> Value { map::command(dir, sid, Some(json!({"upsert": nodes}))).unwrap() }
    fn nid(map: &Value, id: &str) -> String { map["nodes"].as_array().unwrap().iter().find(|n| n["id"] == id).unwrap()["nid"].as_str().unwrap().to_string() }

    #[test]
    fn a_file_touch_is_an_event_by_path() {
        let dir = tempfile::tempdir().unwrap();
        let m = put(dir.path(), "s", json!([{"id": "n1", "label": "N"}]));
        file_touched(dir.path(), "s", "edit", "/w/a.rs", Some("n1")).unwrap();
        assert!(file_touched(dir.path(), "s", "delete", "/w/a.rs", None).is_err());
        let evs = events(dir.path());
        let e = evs.iter().find(|e| e["type"] == "file.touched").unwrap();
        assert_eq!((e["nid"].as_str(), e["session"].as_str(), e["path"].as_str(), e["data"]["op"].as_str()), (Some(nid(&m, "n1").as_str()), Some("s"), Some("/w/a.rs"), Some("edit")));
    }

    #[test]
    fn a_session_sees_its_open_nodes_related_ones_and_what_changed() {
        let dir = tempfile::tempdir().unwrap();
        let (me, other, third) = ("aaaaaaaa-0000-0000-0000-000000000001", "bbbbbbbb-0000-0000-0000-000000000002", "eeeeeeee-0000-0000-0000-000000000005");
        let mine = put(dir.path(), me, json!([{"id":"q","label":"Pick a store","memid":"m1"},{"id":"x","label":"old","state":"done"}]));
        // The other session shows the same node (taken over), and has its own on the same problem.
        map::take_over(dir.path(), other, &json!({"nodes": [mine["nodes"][0]]})).unwrap();
        put(dir.path(), other, json!([{"id":"a","label":"Store choice","memid":"m1"},{"id":"b","label":"unrelated","detail":"the long sentence about a Warehouse","memid":"m9"}]));
        put(dir.path(), third, json!([{"id":"d","label":"Store → SQLite","kind":"conclusion","state":"done","detail":"One file is enough. Postgres was too much.","memid":"m1"}]));
        let lines = list(dir.path(), me, 10);
        assert!(lines[0].starts_with("m1 · Pick a store"), "own open node first: {lines:?}");
        assert!(lines.iter().any(|l| l.contains("Store choice")), "another node on the same problem is shown");
        assert!(lines.iter().any(|l| l.ends_with("· why: One file is enough")), "a decision carries its first reason: {lines:?}");
        assert_eq!(lines.iter().filter(|l| l.contains("Pick a store")).count(), 1, "a node two sessions show: one line");
        assert!(lines.iter().any(|l| l.contains("unrelated")), "recent nodes of other sessions follow");
        assert!(!lines.iter().any(|l| l.contains("old")), "own closed nodes are not listed");
        put(dir.path(), "third", json!([{"id":"c","label":"closed elsewhere","state":"done"}]));
        assert!(list(dir.path(), me, 20).iter().any(|l| l.contains("closed elsewhere")), "recent closed nodes of others are listed");
        crate::paths::write_atomic(&part_carried(dir.path(), me), "focus").unwrap();
        let part = list(dir.path(), me, 20);
        assert!(!part.iter().any(|l| l.contains("closed elsewhere")) && part.iter().any(|l| l.contains("unrelated")), "part of a map carried: only open ones: {part:?}");
        std::fs::remove_file(part_carried(dir.path(), me)).unwrap();
        mark_seen(dir.path(), me);
        std::thread::sleep(std::time::Duration::from_millis(5));
        // The other session settles its node on the problem, the person marks mine, and something unrelated is added.
        put(dir.path(), other, json!([{"id":"a","state":"done"}, {"id":"z","label":"elsewhere","memid":"m5"}]));
        map::person(dir.path(), json!([{"id": nid(&mine, "q"), "mark": "important"}])).unwrap();
        let d = delta(dir.path(), me);
        assert_eq!(d.len(), 2, "the change on its memory id and the person's mark, not an unrelated node: {d:?}");
        assert!(d.iter().any(|l| l.contains("person · q ·")), "its own node by the id it knows: {d:?}");
        assert!(delta(dir.path(), me).is_empty(), "seen once");
        assert_eq!(search(dir.path(), &["store".into()], 5).len(), 3);
        assert_eq!(search(dir.path(), &["warehouse".into()], 5).len(), 1, "the detail is searched too");
        assert_eq!(show(dir.path(), "m1")["nodes"].as_array().unwrap().len(), 3);
        assert_eq!(show(dir.path(), &format!("{other}/q"))["nodes"][0]["label"], "Pick a store");
    }

    #[test]
    fn a_node_taken_off_a_map_stays_in_the_memory() {
        let dir = tempfile::tempdir().unwrap();
        let m = put(dir.path(), "s", json!([{"id":"a","label":"Kept thought"},{"id":"b","label":"Other"}]));
        let after = map::command(dir.path(), "s", Some(json!({"remove": ["a"]}))).unwrap();
        assert_eq!(after["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(search(dir.path(), &["kept".into()], 5).len(), 1, "still found");
        assert_eq!(show(dir.path(), &nid(&m, "a"))["events"].as_array().unwrap().last().unwrap()["type"], "session.unlinked");
        assert!(list(dir.path(), "s", 10).iter().any(|l| l.contains("Kept thought")), "and listed again as a node not on this map");
    }

    #[test]
    fn the_board_sorts_nodes_into_its_sections() {
        let dir = tempfile::tempdir().unwrap();
        let s = "cccccccc-0000-0000-0000-000000000003";
        let m = put(dir.path(), s, json!([
            {"id":"a","label":"meet","at":"2026-11-12 15:00"},
            {"id":"b","label":"key"},
            {"id":"c","label":"later","state":"postponed"},
            {"id":"f","label":"maybe gone","suggest":{"state":"dropped","why":"replaced"}},
            {"id":"d","label":"gone"},
            {"id":"e","label":"done","state":"done"}]));
        map::person(dir.path(), json!([{"id": nid(&m, "b"), "mark": "important"}, {"id": nid(&m, "e"), "mark": "important"}])).unwrap();
        let action = map::trash(dir.path(), &nid(&m, "d"), false).unwrap();
        let b = board(dir.path(), None);
        let n = |b: &Value, k: &str| { let mut v: Vec<String> = b[k].as_array().unwrap().iter().map(|i| i["node"]["label"].as_str().unwrap().to_string()).collect(); v.sort(); v };
        assert_eq!((n(&b, "due"), n(&b, "marked"), n(&b, "held"), n(&b, "trash")), (vec!["meet".to_string()], vec!["done".to_string(), "key".into()], vec!["later".to_string()], vec!["gone".to_string()]), "a marked node shows even when done");
        assert_eq!(b["trash"][0]["action"], json!(action), "the deletion can be undone by its action");
        assert_eq!(b["trash"][0]["node"]["id"], json!(nid(&m, "d")), "a node is named by its number");
        assert!(n(&b, "changed").is_empty(), "nothing settled or freed yet");
        assert_eq!(n(&b, "check"), vec!["maybe gone".to_string()], "the agent's suggestion waits for the person");
        let line_of = |label: &str| b["marked"].as_array().unwrap().iter().chain(b["held"].as_array().unwrap()).find(|i| i["node"]["label"] == label).unwrap()["line"].as_str().unwrap().to_string();
        assert!(line_of("key").starts_with("! key"), "a marked node shows it");
        assert!(line_of("later").contains("On hold") && !line_of("later").contains("postponed"), "the state in the person's words: {}", line_of("later"));
        assert!(n(&b, "open").is_empty(), "no conversation on show");
        let mine = board(dir.path(), Some(s));
        assert_eq!(mine["open"].as_array().unwrap().iter().map(|i| i["node"]["label"].as_str().unwrap()).collect::<Vec<_>>(), ["meet", "key", "later", "maybe gone"], "its open nodes in the map's order, marked or not");
        // A condition that now holds, and a problem a decision settled.
        put(dir.path(), s, json!([{"id":"p","label":"first"}, {"id":"q","label":"then","requires":"p"}, {"id":"r","label":"blocked","requires":"p ∧ q"}]));
        put(dir.path(), s, json!([{"id":"p","state":"done"}, {"id":"w","label":"decided","state":"done","links":[{"to":"b","rel":"resolves"}]}]));
        let b = board(dir.path(), None);
        assert_eq!(n(&b, "changed"), ["key", "then"], "what a decision settled, and only what its condition now allows");
    }

    #[test]
    fn steps_follow_one_session_in_order() {
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), "s", json!([{"id":"t","label":"top"},{"id":"a","label":"a","state":"done"}]));
        put(dir.path(), "s", json!([{"id":"t","state":"done"}]));
        let st = steps(dir.path(), "s");
        assert_eq!(st.iter().map(|x| (x.2.as_str(), x.3)).collect::<Vec<_>>(), [("t", false), ("a", true), ("t", true)]);
    }
}
