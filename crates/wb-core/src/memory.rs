//! The memory: every change to every conversation map, in time order, appended and never rewritten.
//! A session's map file is what this log says about that session's nodes; the alert panel is another view of it.
//! Events follow CloudEvents 1.0: `specversion`, `id`, `source`, `type`, `time`, `subject`, `data`.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::io::AsRawFd,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::{Map, Value, json};

use crate::map::Result;

/// Fields a node carries that the memory does not follow: the program's stamps and where the conversation stayed.
const NOT_FOLLOWED: [&str; 5] = ["id", "opened", "changed", "closed", "links"];

static SEQ: AtomicU64 = AtomicU64::new(0);

/// The log file of the month `ms` falls in: `memory/events/<YYYY-MM>.jsonl`.
pub fn file_for(data: &Path, ms: i64) -> PathBuf {
    let month = chrono::DateTime::from_timestamp_millis(ms).map(|t| t.format("%Y-%m").to_string()).unwrap_or_else(|| "0000-00".into());
    data.join("memory").join("events").join(format!("{month}.jsonl"))
}

/// One event. `source` is `session/<id>`, `person`, `worker` or `program`; `subject` is `<session>/<node>`.
pub fn event(source: &str, kind: &str, subject: &str, at: i64, data: Value) -> Value {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let time = chrono::DateTime::from_timestamp_millis(at).map(|t| t.to_rfc3339()).unwrap_or_default();
    json!({"specversion": "1.0", "id": format!("{at:x}-{:x}-{n:x}", std::process::id()), "source": source, "type": kind,
        "time": time, "subject": subject, "data": data})
}

/// Appends the events, one line each, under an exclusive lock so lines of two programs never mix.
pub fn record(data: &Path, at: i64, events: &[Value]) -> Result<()> {
    if events.is_empty() { return Ok(()); }
    let path = file_for(data, at);
    fs::create_dir_all(path.parent().ok_or("missing parent directory")?)?;
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    let text: String = events.iter().map(|e| format!("{e}\n")).collect();
    // SAFETY: flock on a file this function owns; released when the file closes.
    unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    file.write_all(text.as_bytes())?;
    Ok(())
}

/// A file the session read, changed or attached to a node: `file.touched`, so every session's nodes that dealt with a
/// file can be found by its path.
pub fn file_touched(data: &Path, sid: &str, op: &str, path: &str, node: Option<&str>) -> Result<()> {
    if !["read", "edit", "attach"].contains(&op) { return Err(format!("file touch must be read, edit or attach: {op}").into()); }
    let at = chrono::Local::now().timestamp_millis();
    let subject = format!("{sid}/{}", node.unwrap_or(""));
    record(data, at, &[event(&format!("session/{sid}"), "file.touched", &subject, at, json!({"path": path, "op": op}))])
}

/// Every session's map as stored: (session id, its nodes, when it was last written).
pub(crate) fn maps(data: &Path) -> Vec<(String, Vec<Value>, i64)> {
    let mut out = Vec::new();
    for f in fs::read_dir(data).into_iter().flatten().flatten() {
        let name = f.file_name().to_string_lossy().to_string();
        let Some(sid) = name.strip_suffix(".json").filter(|s| s.len() == 36 && s.matches('-').count() == 4) else { continue };
        let Some(map) = fs::read_to_string(f.path()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { continue };
        let mut nodes = map["nodes"].as_array().cloned().unwrap_or_default();
        for n in &mut nodes { crate::map::settle_old(n); }
        out.push((sid.to_string(), nodes, map["updatedAt"].as_i64().unwrap_or(0)));
    }
    out.sort_by_key(|m| std::cmp::Reverse(m.2));
    out
}

fn when(n: &Value) -> i64 { ["changed", "opened"].iter().find_map(|k| n.get(*k)?.get("at")?.as_i64()).unwrap_or(0) }
use crate::map::is_open;

/// One node in one line: `memory id · label · state or kind · session · last change`.
pub fn line(sid: &str, n: &Value) -> String {
    let key = n.get("memid").and_then(Value::as_str).map(String::from).unwrap_or_else(|| format!("{}/{}", &sid[..8.min(sid.len())], n["id"].as_str().unwrap_or("")));
    let what = n.get("state").or_else(|| n.get("kind")).and_then(Value::as_str).unwrap_or(if is_open(n) { "open" } else { "closed" });
    let day = chrono::DateTime::from_timestamp_millis(when(n)).map(|t| t.with_timezone(&chrono::Local).format("%m-%d").to_string()).unwrap_or_default();
    let due = n.get("due").or_else(|| n.get("at")).and_then(Value::as_str).map(|d| format!(" · due {d}")).unwrap_or_default();
    // A decision's reason travels with it: a new session misses the why more than the what.
    let why = n.get("detail").and_then(Value::as_str).filter(|_| n["kind"] == "conclusion")
        .map(|d| format!(" · why: {}", d.split(['.', '。', '\n']).next().unwrap_or("").trim().chars().take(60).collect::<String>())).unwrap_or_default();
    format!("{key} · {} · {what} · {} · {day}{due}{why}", n["label"].as_str().unwrap_or(""), &sid[..4.min(sid.len())])
}

/// Every event in the log, oldest first. shortcut: reads all month files each time; keep an index when the log grows large.
pub(crate) fn events(data: &Path) -> Vec<Value> {
    let mut files: Vec<PathBuf> = fs::read_dir(data.join("memory").join("events")).into_iter().flatten().flatten().map(|f| f.path()).collect();
    files.sort();
    files.iter().flat_map(|f| fs::read_to_string(f).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect::<Vec<Value>>()).collect()
}

/// One session's node changes in the order they happened: (turn, time in ms, node id, whether it closed).
pub fn steps(data: &Path, sid: &str) -> Vec<(i64, i64, String, bool)> {
    let prefix = format!("{sid}/");
    events(data).into_iter().filter(|e| e["type"] == "node.added" || e["type"] == "node.changed").filter_map(|e| {
        let id = e["subject"].as_str()?.strip_prefix(&prefix)?.to_string();
        let at = chrono::DateTime::parse_from_rfc3339(e["time"].as_str()?).ok()?.timestamp_millis();
        // A change holds [before, after]; a node made closed holds the value itself.
        let state = if e["type"] == "node.added" { &e["data"]["state"] } else { &e["data"]["state"][1] };
        let closed = !is_open(&json!({"state": state})) && (e["type"] == "node.added" || is_open(&json!({"state": e["data"]["state"][0]})));
        Some((e["turn"].as_i64().unwrap_or(0), at, id, closed))
    }).collect()
}

/// The paths a session's events touched.
fn paths_of(evs: &[Value], sid: &str) -> std::collections::HashSet<String> {
    evs.iter().filter(|e| e["type"] == "file.touched" && e["source"] == format!("session/{sid}")).filter_map(|e| e["data"]["path"].as_str().map(String::from)).collect()
}

/// The memory list a session sees (3.19): A its own open nodes, C related nodes of other sessions (same memory id,
/// same files, due soon), B other sessions' recent nodes; one line each, at most `limit` lines.
pub fn list(data: &Path, sid: &str, limit: usize) -> Vec<String> {
    let all = maps(data);
    let evs = events(data);
    let own: Vec<Value> = all.iter().find(|m| m.0 == sid).map(|m| m.1.clone()).unwrap_or_default();
    let own_mem: std::collections::HashSet<&str> = own.iter().filter_map(|n| n.get("memid")?.as_str()).collect();
    let mine = paths_of(&evs, sid);
    // Nodes of other sessions that touched the same files.
    let by_file: std::collections::HashSet<String> = evs.iter().filter(|e| e["type"] == "file.touched" && e["source"] != format!("session/{sid}"))
        .filter(|e| e["data"]["path"].as_str().is_some_and(|p| mine.contains(p))).filter_map(|e| e["subject"].as_str().map(String::from)).collect();
    let soon = chrono::Local::now().date_naive() + chrono::Duration::days(7);
    let due_soon = |n: &Value| n.get("due").or_else(|| n.get("at")).and_then(Value::as_str).and_then(|d| chrono::NaiveDate::parse_from_str(&d[..10.min(d.len())], "%Y-%m-%d").ok()).is_some_and(|d| d <= soon);
    let (mut out, mut seen) = (Vec::new(), std::collections::HashSet::new());
    let mut push = |s: &str, n: &Value, out: &mut Vec<String>| {
        // A carried map repeats nodes under another session: one line for the same node (memory id and label). Another
        // node about the same problem, such as a decision made elsewhere, is shown.
        let key = n.get("memid").and_then(Value::as_str).map(|m| format!("{m}\u{0}{}", n["label"].as_str().unwrap_or(""))).unwrap_or_else(|| format!("{s}/{}", n["id"]));
        if out.len() < limit && seen.insert(key) { out.push(line(s, n)); }
    };
    for n in own.iter().filter(|n| is_open(n)) { push(sid, n, &mut out); }
    for (s, nodes, _) in all.iter().filter(|m| m.0 != sid) {
        for n in nodes.iter().filter(|n| n["state"] != "removed") {
            let related = n.get("memid").and_then(Value::as_str).is_some_and(|m| own_mem.contains(m))
                || by_file.contains(&format!("{s}/{}", n["id"].as_str().unwrap_or(""))) || (is_open(n) && due_soon(n));
            if related { push(s, n, &mut out); }
        }
    }
    let mut recent: Vec<(&String, &Value)> = all.iter().filter(|m| m.0 != sid).flat_map(|m| m.1.iter().filter(|n| n["state"] != "removed").map(move |n| (&m.0, n))).collect();
    recent.sort_by_key(|(_, n)| std::cmp::Reverse(when(n)));
    for (s, n) in recent { push(s, n, &mut out); }
    out
}

fn seen_file(data: &Path, sid: &str) -> PathBuf { data.join("cache").join("memory-seen").join(sid) }

/// Marks the log as seen by the session up to now.
pub fn mark_seen(data: &Path, sid: &str) {
    let f = seen_file(data, sid);
    let _ = fs::create_dir_all(f.parent().unwrap_or(data));
    let _ = fs::write(f, chrono::Local::now().timestamp_millis().to_string());
}

/// What changed for this session since it last saw the memory: other sessions' and the person's events on its
/// nodes, on its memory ids, or on the files it touched. Marks them seen.
pub fn delta(data: &Path, sid: &str) -> Vec<String> {
    let since: i64 = fs::read_to_string(seen_file(data, sid)).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0);
    let evs = events(data);
    let own: Vec<Value> = maps(data).into_iter().find(|m| m.0 == sid).map(|m| m.1).unwrap_or_default();
    let own_mem: std::collections::HashSet<&str> = own.iter().filter_map(|n| n.get("memid")?.as_str()).collect();
    let mine = paths_of(&evs, sid);
    let out = evs.iter().filter(|e| e["source"] != format!("session/{sid}"))
        .filter(|e| chrono::DateTime::parse_from_rfc3339(e["time"].as_str().unwrap_or("")).is_ok_and(|t| t.timestamp_millis() > since))
        .filter(|e| e["subject"].as_str().is_some_and(|s| s.starts_with(&format!("{sid}/")))
            || e["memid"].as_str().is_some_and(|m| own_mem.contains(m))
            || e["data"]["path"].as_str().is_some_and(|p| mine.contains(p)))
        .map(|e| format!("{} · {} · {} · {}", e["type"].as_str().unwrap_or(""), e["source"].as_str().unwrap_or(""), e["subject"].as_str().unwrap_or(""), e["data"]))
        .collect();
    mark_seen(data, sid);
    out
}

/// A node by memory id or `<session>/<node id>`: the node as stored in every session that has it, and its events.
pub fn show(data: &Path, key: &str) -> Value {
    let mut found = Vec::new();
    for (s, nodes, _) in maps(data) {
        for n in nodes.into_iter().filter(|n| n["memid"] == key || format!("{s}/{}", n["id"].as_str().unwrap_or("")) == key) { found.push(json!({"session": s, "node": n})); }
    }
    let subjects: Vec<String> = found.iter().map(|f| format!("{}/{}", f["session"].as_str().unwrap_or(""), f["node"]["id"].as_str().unwrap_or(""))).collect();
    let evs: Vec<Value> = events(data).into_iter().filter(|e| e["subject"].as_str().is_some_and(|s| subjects.iter().any(|x| x == s))).collect();
    json!({"nodes": found, "events": evs})
}

/// Nodes of every session whose label or detail holds any of the words (any case), most recent first.
pub fn search(data: &Path, words: &[String], limit: usize) -> Vec<String> {
    let words: Vec<String> = words.iter().map(|w| w.to_lowercase()).filter(|w| !w.is_empty()).collect();
    let mut hits: Vec<(String, Value)> = maps(data).into_iter().flat_map(|(s, nodes, _)| nodes.into_iter().map(move |n| (s.clone(), n)))
        .filter(|(_, n)| { let l = format!("{} {}", n["label"].as_str().unwrap_or(""), n["detail"].as_str().unwrap_or("")).to_lowercase(); words.iter().any(|w| l.contains(w)) }).collect();
    hits.sort_by_key(|(_, n)| std::cmp::Reverse(when(n)));
    hits.iter().take(limit).map(|(s, n)| line(s, n)).collect()
}

/// The alert panel's sections (3.19): nodes with a time or due date, the person's marked ones, what changed since the
/// person last looked, held ones and the trash. Each item is `{session, node, line}`; a carried node shows once.
/// One node as the person reads it: `label · state · due · session name` (no ids).
fn person_line(name: &str, n: &Value) -> String {
    let mark = if n.get("mark").is_some() { "! " } else { "" };
    let mut parts = vec![format!("{mark}{}", n["label"].as_str().unwrap_or(""))];
    if let Some(s) = n.get("state").and_then(Value::as_str) { parts.push(crate::i18n::t(&format!("state-{s}")).to_string()); }
    if let Some(d) = n.get("due").or_else(|| n.get("at")).and_then(Value::as_str) { parts.push(d.to_string()); }
    parts.push(name.chars().take(30).collect());
    parts.join(" · ")
}

/// `current` is the conversation on show: its open nodes get a section of their own, so any of them can be marked.
pub fn board(data: &Path, current: Option<&str>) -> Value {
    let all = maps(data);
    let d = crate::paths::Data::new(data.to_path_buf());
    let names: std::collections::HashMap<&str, String> = all.iter().map(|m| (m.0.as_str(), crate::sessions::session_name(&d, &m.0))).collect();
    let (mut due, mut marked, mut check, mut held, mut trash, mut seen) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), std::collections::HashSet::new());
    for (s, nodes, _) in &all {
        for n in nodes {
            // Same rule as the memory list: a carried copy (memory id and label) once; another node on the problem stays.
            let key = n.get("memid").and_then(Value::as_str).map(|m| format!("{m}\u{0}{}", n["label"].as_str().unwrap_or(""))).unwrap_or_else(|| format!("{s}/{}", n["id"]));
            if !seen.insert(key) { continue; }
            let item = json!({"session": s, "node": n, "line": person_line(&names[s.as_str()], n)});
            if n["state"] == "removed" { trash.push(item); continue; }
            if n["state"] == "postponed" { held.push(item.clone()); }
            // The person's mark holds whatever the state: what they marked they want to see again.
            if n.get("mark").is_some() { marked.push(item.clone()); }
            if !is_open(n) { continue; }
            // The agent's suggestion to close it: the person approves or declines.
            if let Some(sg) = n.get("suggest").filter(|v| v.is_object()) {
                let to = crate::i18n::t(if sg["state"] == "dropped" { "alerts-dropped" } else { "alerts-done" });
                let line = format!("{} · {} · {}", n["label"].as_str().unwrap_or(""), crate::i18n::f("alerts-suggested", &[("state", to.to_string()), ("why", sg["why"].as_str().unwrap_or("").to_string())]), names[s.as_str()]);
                check.push(json!({"session": s, "node": n, "line": line}));
            }
            if n.get("due").or_else(|| n.get("at")).is_some() { due.push(item); }
        }
    }
    let date = |i: &Value| i["node"].get("due").or_else(|| i["node"].get("at")).and_then(Value::as_str).unwrap_or("").to_string();
    due.sort_by_key(date);
    let since: i64 = fs::read_to_string(seen_file(data, "person")).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0);
    let node_of = |s: &str, id: &str| all.iter().find(|m| m.0 == s).and_then(|m| m.1.iter().find(|n| n["id"] == id)).cloned();
    let mut changed: Vec<Value> = Vec::new();
    // Settled by a decision since the person last looked: the problem, and the decision that settled it.
    let evs = events(data);
    for e in evs.iter().rev().filter(|e| e["type"] == "link.added" && e["data"]["rel"] == "resolves") {
        if !chrono::DateTime::parse_from_rfc3339(e["time"].as_str().unwrap_or("")).is_ok_and(|t| t.timestamp_millis() > since) { continue; }
        let subject = e["subject"].as_str().unwrap_or("");
        let (s, by) = subject.split_once('/').unwrap_or((subject, ""));
        let (Some(target), Some(decision)) = (node_of(s, e["data"]["to"].as_str().unwrap_or("")), node_of(s, by)) else { continue };
        let line = format!("{} · {} · {}", target["label"].as_str().unwrap_or(""), crate::i18n::f("alerts-resolved", &[("by", decision["label"].as_str().unwrap_or("").to_string())]), names[s]);
        changed.push(json!({"session": s, "node": target, "line": line}));
    }
    // What can be done now: an open node whose condition holds since one of its nodes closed after the person last looked.
    for (s, nodes, _) in &all {
        for n in nodes.iter().filter(|n| is_open(n)) {
            let Some(expr) = n.get("requires").and_then(Value::as_str) else { continue };
            let closed_at = |id: &str| nodes.iter().find(|m| m["id"] == id).filter(|m| !is_open(m)).and_then(|m| m.get("closed")?.get("at")?.as_i64());
            let late = crate::map::condition_ids(expr).unwrap_or_default().iter().any(|id| closed_at(id).is_some_and(|at| at > since));
            if late && crate::map::condition_holds(expr, &|id| closed_at(id).is_some()) {
                changed.push(json!({"session": s, "node": n, "line": format!("{} · {} · {}", n["label"].as_str().unwrap_or(""), crate::i18n::t("alerts-unblocked"), names[s.as_str()])}));
            }
        }
    }
    // The action that put each node in the trash: the last event that set it to removed.
    if !trash.is_empty() {
        let mut by: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
        for e in evs.iter().filter(|e| e["data"]["state"][1] == "removed") {
            if let (Some(sub), Some(a)) = (e["subject"].as_str(), e.get("action")) { by.insert(sub.to_string(), a.clone()); }
        }
        for t in &mut trash { let sub = format!("{}/{}", t["session"].as_str().unwrap_or(""), t["node"]["id"].as_str().unwrap_or("")); t["action"] = by.get(&sub).cloned().unwrap_or(Value::Null); }
    }
    let open: Vec<Value> = all.iter().filter(|m| Some(m.0.as_str()) == current).flat_map(|(s, nodes, _)| nodes.iter().filter(|n| is_open(n)).map(move |n| (s, n)))
        .map(|(s, n)| json!({"session": s, "node": n, "line": person_line(&names[s.as_str()], n)})).collect();
    json!({"due": due, "marked": marked, "changed": changed, "check": check, "open": open, "held": held, "trash": trash})
}

fn links(node: &Value) -> Vec<(String, String)> {
    node.get("links").and_then(Value::as_array).into_iter().flatten()
        .filter_map(|l| Some((l.get("to")?.as_str()?.to_string(), l.get("rel")?.as_str()?.to_string()))).collect()
}

fn followed(node: &Value) -> Map<String, Value> {
    node.as_object().into_iter().flatten().filter(|(k, _)| !NOT_FOLLOWED.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// What changed between two versions of one session's nodes, as events.
pub fn diff(source: &str, sid: &str, at: i64, before: &[Value], after: &[Value]) -> Vec<Value> {
    let find = |nodes: &[Value], id: &str| nodes.iter().find(|n| n["id"] == id).cloned();
    let subject = |id: &str| format!("{sid}/{id}");
    let mut out = Vec::new();
    for node in after {
        let Some(id) = node["id"].as_str() else { continue };
        let now = followed(node);
        match find(before, id) {
            None => out.push(event(source, "node.added", &subject(id), at, Value::Object(now))),
            Some(old) => {
                let was = followed(&old);
                let keys: std::collections::BTreeSet<&String> = was.keys().chain(now.keys()).collect();
                let changed: Map<String, Value> = keys.into_iter().filter(|k| was.get(*k) != now.get(*k))
                    .map(|k| (k.clone(), json!([was.get(k).cloned().unwrap_or(Value::Null), now.get(k).cloned().unwrap_or(Value::Null)]))).collect();
                if !changed.is_empty() { out.push(event(source, "node.changed", &subject(id), at, Value::Object(changed))); }
            }
        }
        let old_links = find(before, id).map(|o| links(&o)).unwrap_or_default();
        let new_links = links(node);
        for (to, rel) in new_links.iter().filter(|l| !old_links.contains(l)) { out.push(event(source, "link.added", &subject(id), at, json!({"to": to, "rel": rel}))); }
        for (to, rel) in old_links.iter().filter(|l| !new_links.contains(l)) { out.push(event(source, "link.removed", &subject(id), at, json!({"to": to, "rel": rel}))); }
    }
    for node in before {
        if let Some(id) = node["id"].as_str().filter(|id| find(after, id).is_none()) {
            out.push(event(source, "node.purged", &subject(id), at, json!({})));
        }
    }
    // The memory id rides on every event as a CloudEvents extension attribute, so a change can be found by it.
    for e in &mut out {
        let id = e["subject"].as_str().and_then(|s| s.rsplit('/').next()).unwrap_or("").to_string();
        if let Some(m) = find(after, &id).or_else(|| find(before, &id)).and_then(|n| n.get("memid").cloned()) { e["memid"] = m; }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_is_told_as_the_events_it_makes() {
        let before = vec![json!({"id":"a","label":"A","opened":{"at":1}}), json!({"id":"b","label":"B"})];
        let after = vec![json!({"id":"a","label":"A2","state":"done","opened":{"at":1},"closed":{"at":2},"links":[{"to":"c","rel":"replaces"}]}), json!({"id":"c","label":"C"})];
        let ev = diff("session/s", "s", 5, &before, &after);
        let kinds: Vec<&str> = ev.iter().map(|e| e["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["node.changed", "link.added", "node.added", "node.purged"]);
        assert_eq!(ev[0]["data"]["label"], json!(["A", "A2"]));
        assert_eq!(ev[0]["data"]["state"], json!([null, "done"]), "closing is a change of state");
        assert!(ev[0]["data"].get("closed").is_none(), "the program's stamps are not followed");
        assert_eq!((ev[0]["subject"].as_str(), ev[0]["specversion"].as_str()), (Some("s/a"), Some("1.0")));
        assert_ne!(ev[0]["id"], ev[1]["id"]);
    }

    #[test]
    fn a_file_touch_is_an_event_by_path() {
        let dir = tempfile::tempdir().unwrap();
        file_touched(dir.path(), "s", "edit", "/w/a.rs", Some("n1")).unwrap();
        assert!(file_touched(dir.path(), "s", "delete", "/w/a.rs", None).is_err());
        let line = fs::read_dir(dir.path().join("memory/events")).unwrap().flatten().map(|f| fs::read_to_string(f.path()).unwrap()).collect::<String>();
        let e: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!((e["type"].as_str(), e["subject"].as_str(), e["data"]["path"].as_str()), (Some("file.touched"), Some("s/n1"), Some("/w/a.rs")));
    }

    #[test]
    fn a_session_sees_its_open_nodes_related_ones_and_what_changed() {
        let dir = tempfile::tempdir().unwrap();
        let (me, other) = ("aaaaaaaa-0000-0000-0000-000000000001", "bbbbbbbb-0000-0000-0000-000000000002");
        let now = chrono::Local::now().timestamp_millis();
        let map = |nodes: Value| json!({"nodes": nodes, "updatedAt": now}).to_string();
        fs::write(dir.path().join(format!("{me}.json")), map(json!([{"id":"q","label":"Pick a store","isOpen":true,"memid":"m1"},{"id":"x","label":"old","isOpen":false}]))).unwrap();
        fs::write(dir.path().join(format!("{other}.json")), map(json!([{"id":"a","label":"Store choice","isOpen":true,"memid":"m1","opened":{"at":1}},{"id":"copy","label":"Pick a store","memid":"m1"},
            {"id":"b","label":"unrelated","detail":"the long sentence about a Warehouse","isOpen":true,"memid":"m9","opened":{"at":now}}]))).unwrap();
        fs::write(dir.path().join("eeeeeeee-0000-0000-0000-000000000005.json"), map(json!([{"id":"d","label":"Store → SQLite","kind":"conclusion","state":"done","detail":"One file is enough. Postgres was too much.","memid":"m1","opened":{"at":now}}]))).unwrap();
        let lines = list(dir.path(), me, 10);
        assert!(lines[0].starts_with("m1 · Pick a store"), "own open node first");
        assert!(lines.iter().any(|l| l.contains("Store choice")), "another node on the same problem is shown");
        assert!(lines.iter().any(|l| l.ends_with("· why: One file is enough")), "a decision carries its first reason: {lines:?}");
        assert_eq!(lines.iter().filter(|l| l.contains("Pick a store")).count(), 1, "a carried copy of the same node: one line");
        assert!(lines.iter().any(|l| l.contains("unrelated")), "recent nodes of other sessions follow");
        assert!(!lines.iter().any(|l| l.contains("old")), "own closed nodes are not listed");
        mark_seen(dir.path(), me);
        std::thread::sleep(std::time::Duration::from_millis(5));
        let at = chrono::Local::now().timestamp_millis();
        let mut on_m1 = event(&format!("session/{other}"), "node.changed", &format!("{other}/a"), at, json!({"state": ["doing", "done"]}));
        on_m1["memid"] = json!("m1");
        record(dir.path(), at, &[on_m1,
            event("person", "node.changed", &format!("{me}/q"), at, json!({"mark": [null, "important"]})),
            event(&format!("session/{other}"), "node.added", &format!("{other}/z"), at, json!({"memid": "m5"}))]).unwrap();
        let d = delta(dir.path(), me);
        assert_eq!(d.len(), 2, "the change on its memory id and the person's mark, not an unrelated node: {d:?}");
        assert!(delta(dir.path(), me).is_empty(), "seen once");
        assert_eq!(search(dir.path(), &["store".into()], 5).len(), 4);
        assert_eq!(search(dir.path(), &["warehouse".into()], 5).len(), 1, "the detail is searched too");
        assert_eq!(show(dir.path(), "m1")["nodes"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn the_board_sorts_nodes_into_its_sections() {
        let dir = tempfile::tempdir().unwrap();
        let s = "cccccccc-0000-0000-0000-000000000003";
        fs::write(dir.path().join(format!("{s}.json")), json!({"updatedAt": 1, "nodes": [
            {"id":"a","label":"meet","isOpen":true,"at":"2026-11-12 15:00","memid":"1"},
            {"id":"b","label":"key","isOpen":true,"mark":"important","memid":"2"},
            {"id":"c","label":"later","isOpen":true,"state":"postponed","memid":"3"},
            {"id":"f","label":"maybe gone","suggest":{"state":"dropped","why":"replaced"},"memid":"6"},
            {"id":"d","label":"gone","isOpen":false,"state":"removed","memid":"4"},
            {"id":"e","label":"done","isOpen":false,"state":"done","mark":"important","memid":"5"}]}).to_string()).unwrap();
        let b = board(dir.path(), None);
        let n = |k: &str| b[k].as_array().unwrap().iter().map(|i| i["node"]["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!((n("due"), n("marked"), n("held"), n("trash")), (vec!["a".into()], vec!["b".into(), "e".into()], vec!["c".into()], vec!["d".into()]), "a marked node shows even when done");
        assert!(n("changed").is_empty(), "nothing settled or freed yet");
        assert_eq!(n("check"), vec!["f".to_string()], "the agent's suggestion waits for the person");
        let line_of = |id: &str| b["marked"].as_array().unwrap().iter().chain(b["held"].as_array().unwrap()).find(|i| i["node"]["id"] == id).unwrap()["line"].as_str().unwrap().to_string();
        assert!(line_of("b").starts_with("! key"), "a marked node shows it");
        assert!(line_of("c").contains("On hold") && !line_of("c").contains("postponed"), "the state in the person's words: {}", line_of("c"));
        // A decision on the same problem in a map written later comes first; the suggestion still shows.
        let later = "dddddddd-0000-0000-0000-000000000004";
        fs::write(dir.path().join(format!("{later}.json")), json!({"updatedAt": 9, "nodes": [{"id":"x","label":"decided elsewhere","state":"done","memid":"6"}]}).to_string()).unwrap();
        assert_eq!(board(dir.path(), None)["check"].as_array().unwrap().len(), 1);
        assert!(n("open").is_empty(), "no conversation on show");
        let mine = board(dir.path(), Some(s));
        assert_eq!(mine["open"].as_array().unwrap().iter().map(|i| i["node"]["id"].as_str().unwrap()).collect::<Vec<_>>(), ["a", "b", "c", "f"], "its open nodes, marked or not");
        fs::write(dir.path().join(format!("{s}.json")), json!({"updatedAt": 2, "nodes": [
            {"id":"p","label":"first","isOpen":false,"state":"done","closed":{"at":5}},
            {"id":"q","label":"then","isOpen":true,"requires":"p"},
            {"id":"r","label":"blocked","isOpen":true,"requires":"p ∧ q"}]}).to_string()).unwrap();
        let b = board(dir.path(), None);
        assert_eq!(b["changed"].as_array().unwrap().iter().map(|i| i["node"]["id"].as_str().unwrap()).collect::<Vec<_>>(), ["q"], "only what its condition now allows");
    }

    #[test]
    fn events_are_appended_to_the_month_file() {
        let dir = tempfile::tempdir().unwrap();
        let at = 1_794_000_000_000_i64;
        record(dir.path(), at, &[event("person", "node.changed", "s/a", at, json!({}))]).unwrap();
        record(dir.path(), at, &[event("person", "node.changed", "s/b", at, json!({}))]).unwrap();
        let text = fs::read_to_string(file_for(dir.path(), at)).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(file_for(dir.path(), at).to_string_lossy().ends_with(".jsonl"));
    }
}
