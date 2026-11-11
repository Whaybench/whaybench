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
const NOT_FOLLOWED: [&str; 6] = ["id", "opened", "changed", "closed", "visits", "links"];

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
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_is_told_as_the_events_it_makes() {
        let before = vec![json!({"id":"a","label":"A","isOpen":true,"opened":{"at":1}}), json!({"id":"b","label":"B","isOpen":true})];
        let after = vec![json!({"id":"a","label":"A2","isOpen":false,"opened":{"at":1},"closed":{"at":2},"links":[{"to":"c","rel":"replaces"}]}), json!({"id":"c","label":"C","isOpen":true})];
        let ev = diff("session/s", "s", 5, &before, &after);
        let kinds: Vec<&str> = ev.iter().map(|e| e["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["node.changed", "link.added", "node.added", "node.purged"]);
        assert_eq!(ev[0]["data"]["label"], json!(["A", "A2"]));
        assert_eq!(ev[0]["data"]["isOpen"], json!([true, false]));
        assert!(ev[0]["data"].get("closed").is_none(), "the program's stamps are not followed");
        assert_eq!((ev[0]["subject"].as_str(), ev[0]["specversion"].as_str()), (Some("s/a"), Some("1.0")));
        assert_ne!(ev[0]["id"], ev[1]["id"]);
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
