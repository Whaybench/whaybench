//! How long conversations are kept: what goes when a limit is set, and the map saved from one that is gone.
use super::*;

/// A conversation's numbers: this one and every number that moved on into it (/resume, /background).
fn chain(all: &[Value], last: &str) -> Vec<String> {
    let (mut ids, mut i) = (vec![last.to_string()], 0);
    while i < ids.len() {
        let here = ids[i].clone();
        let before = all.iter().filter(|s| s.get("movedTo").and_then(Value::as_str) == Some(&here)).filter_map(|s| s["id"].as_str().map(String::from));
        for b in before.collect::<Vec<_>>() { if !ids.contains(&b) { ids.push(b); } }
        i += 1;
    }
    ids
}

/// How long conversation records are kept: `keep_days` and `keep_mb` in the data directory's settings.json. Missing
/// or 0 means no limit, the default.
pub fn retention(d: &Data) -> (u64, u64) {
    let s = read_json(&d.root.join("settings.json")).unwrap_or(Value::Null);
    (s["keep_days"].as_u64().unwrap_or(0), s["keep_mb"].as_u64().unwrap_or(0))
}

/// What the program keeps of one session number: its map, record, answers, files read and changed, pages shown.
fn session_paths(d: &Data, id: &str) -> Vec<PathBuf> {
    let mut out = vec![d.root.join(format!("{id}.json")), sessions_dir(d).join(format!("{id}.json")), d.root.join("answers").join(format!("{id}.json")),
        d.root.join("answer-log").join(format!("{id}.jsonl")), d.root.join("node-files").join(format!("{id}.json")), d.root.join("show").join(id)];
    out.extend(fs::read_dir(d.root.join("files")).into_iter().flatten().flatten().map(|f| f.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(id))));
    out
}

fn size_of(p: &Path) -> u64 {
    match fs::metadata(p) {
        Ok(m) if m.is_dir() => fs::read_dir(p).into_iter().flatten().flatten().map(|e| size_of(&e.path())).sum(),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

/// Drops whole conversations (every number of one, so its map and answers go together) older than `keep_days`,
/// then the oldest until all of them fit in `keep_mb`. Running ones stay; saved maps (the maps folder) are not
/// conversation records and stay too. The number of conversations dropped.
pub fn prune(d: &Data) -> usize {
    let (days, mb) = retention(d);
    if days == 0 && mb == 0 { return 0; }
    // How the nodes changed is kept as long as the conversations: older events go, the nodes stay as they are.
    // shortcut: `keep_mb` counts conversation records only; count the store's events in it if they grow large.
    if days > 0 && d.root.join("memory").join("memory.db").exists() {
        let before = (chrono::Utc::now() - chrono::Duration::days(days as i64)).to_rfc3339();
        if let Ok(n) = crate::db::open(&d.root).and_then(|c| crate::db::events_drop_before(&c, &before)) {
            if n > 0 { record::decide("memory", "old events dropped", "over the keeping limit", &[("events", n.to_string())]); }
        }
    }
    let all = all(d);
    // Each conversation is its last number with the numbers that moved on into it; most recently active first.
    let mut convs: Vec<(i64, Vec<String>)> = all.iter().filter(|r| r.get("movedTo").and_then(Value::as_str).is_none_or(str::is_empty))
        .filter_map(|r| Some((r.get("lastActive").and_then(Value::as_i64).unwrap_or(0), chain(&all, r.get("id")?.as_str()?)))).collect();
    // A kept conversation that took its map from another reads that one's answers for those nodes: it stays.
    let inherited: Value = read_json(&d.root.join("forks/inherited.json")).unwrap_or(Value::Null);
    let parents: Vec<&str> = inherited.as_object().into_iter().flatten().filter(|(child, _)| record_of(d, child).is_some()).filter_map(|(_, p)| p.as_str()).collect();
    convs.sort_by_key(|c| std::cmp::Reverse(c.0));
    let now = chrono::Local::now().timestamp_millis();
    let sizes: Vec<u64> = convs.iter().map(|c| c.1.iter().flat_map(|id| session_paths(d, id)).map(|p| size_of(&p)).sum()).collect();
    let mut total: u64 = sizes.iter().sum();
    let mut dropped = 0;
    for (i, (at, ids)) in convs.iter().enumerate().rev() {
        let old = days > 0 && now - at > days as i64 * 86_400_000;
        let over = mb > 0 && total > mb * 1024 * 1024;
        if !(old || over) || ids.iter().any(|id| alive(d, id) || parents.contains(&id.as_str())) { continue; }
        for p in ids.iter().flat_map(|id| session_paths(d, id)) { let _ = if p.is_dir() { fs::remove_dir_all(&p) } else { fs::remove_file(&p) }; }
        record::decide("session", "records dropped", if old { "older than the kept days" } else { "over the kept size" }, &[("session", ids[0].clone()), ("numbers", ids.len().to_string())]);
        total = total.saturating_sub(sizes[i]);
        dropped += 1;
    }
    dropped
}

/// When a conversation went on, in ms: from its start to its last activity. Records from before the start was
/// kept fall back to its answers, then to its map's own stamps (which, in a map carried over from another
/// conversation, are that one's), then to now.
pub fn span(d: &Data, id: &str, nodes: &[Value]) -> (i64, i64) {
    let rec = record_of(d, id).unwrap_or(Value::Null);
    let all = all(d);
    // /resume makes a new record; the conversation began with the first of its numbers.
    let started = chain(&all, id).iter().filter_map(|n| all.iter().find(|r| r["id"].as_str() == Some(n.as_str()))?["started"].as_i64()).min();
    let answers: Vec<i64> = if id.is_empty() { Vec::new() } else { crate::records::answers_of(d, &crate::records::lineage(d, id)).iter().filter_map(|r| r["at"].as_i64()).collect() };
    let nodes = crate::map::node_span(nodes);
    let now = chrono::Local::now().timestamp_millis();
    let first = started.or(answers.iter().min().copied()).or(nodes.map(|n| n.0)).unwrap_or(now);
    let last = rec["lastActive"].as_i64().or(answers.iter().max().copied()).or(nodes.map(|n| n.1)).unwrap_or(now);
    (first, last.max(first))
}

/// Claude Code deletes old conversations (after 30 days unless set otherwise); their maps stay but the sessions
/// leave the list. Each such map is saved once to the maps folder, as `auto-map-…` titled with the session's name.
/// Only the last number of a conversation: an earlier one that carried on under another holds part of its map.
pub fn save_lost_maps(d: &Data) -> usize {
    let file = d.root.join("auto-saved.json");
    let mut done: Vec<String> = read_json(&file).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    let all = all(d);
    let mut saved = 0;
    for r in &all {
        let Some(id) = r.get("id").and_then(Value::as_str) else { continue };
        let moved = r.get("movedTo").and_then(Value::as_str).is_some_and(|m| !m.is_empty() && m != id);
        // Without a first prompt nothing was asked: a tab opened on a carried map and closed again holds only a copy.
        let asked = r.get("firstPrompt").and_then(Value::as_str).is_some_and(|p| !p.is_empty());
        if moved || !asked || done.iter().any(|x| x == id) || !ghost(d, &all, id) { continue; }
        let Some(map) = map_of(d, id) else { continue };
        let nodes = map.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
        if nodes.is_empty() { continue; }
        let path = crate::map::free_path(&d.maps(), &crate::map::file_name(span(d, id, &nodes), None, "auto-"));
        let body = serde_json::json!({"format": "whaybench-map", "version": map.get("version"), "title": session_name(d, id),
            "exported": chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(), "from": id, "auto": true, "nodes": crate::map::saved_nodes(&nodes)});
        if fs::create_dir_all(d.maps()).and_then(|_| fs::write(&path, body.to_string())).is_ok() {
            record::decide("session", "map saved", "its conversation is no longer kept", &[("session", id.to_string()), ("file", path.display().to_string())]);
            done.push(id.to_string());
            saved += 1;
        }
    }
    if saved > 0 { let _ = write_atomic(&file, &Value::from(done).to_string()); }
    saved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::tests::{data, rec};

    #[test]
    fn the_map_of_a_conversation_no_longer_kept_is_saved_once_from_its_last_number() {
        let d = data("lost-maps");
        rec(&d, "a", serde_json::json!({"id":"a","movedTo":"b","firstPrompt":"q"}));
        rec(&d, "b", serde_json::json!({"id":"b","firstPrompt":"q"}));
        rec(&d, "tab", serde_json::json!({"id":"tab"}));
        for (sid, label) in [("a", "early"), ("b", "whole"), ("tab", "carried copy")] {
            crate::map::command(&d.root, sid, Some(serde_json::json!({"upsert":[{"id":"n","label":label}]}))).unwrap();
        }
        assert_eq!(save_lost_maps(&d), 1, "only the last number, and not a tab where nothing was asked");
        let files: Vec<_> = fs::read_dir(d.maps()).unwrap().flatten().map(|f| f.path()).collect();
        assert_eq!(files.len(), 1);
        assert!(files[0].file_name().unwrap().to_string_lossy().starts_with("auto-map-"));
        let v = read_json(&files[0]).unwrap();
        assert_eq!((v["from"].as_str(), v["auto"].as_bool(), v["nodes"][0].get("label")), (Some("b"), Some(true), None), "the saved map names the node");
        assert_eq!(crate::map::view_file(&d.root, &files[0]).unwrap()["nodes"][0]["label"], "whole");
        assert_eq!(save_lost_maps(&d), 0, "once");
    }
    #[test]
    fn old_conversations_go_whole_and_only_when_a_limit_is_set() {
        let d = data("prune");
        let day = 86_400_000_i64;
        let now = chrono::Local::now().timestamp_millis();
        rec(&d, "a", serde_json::json!({"id":"a","movedTo":"b"}));
        rec(&d, "b", serde_json::json!({"id":"b","lastActive": now - 40 * day}));
        rec(&d, "c", serde_json::json!({"id":"c","lastActive": now - day}));
        for id in ["a", "b", "c"] {
            fs::write(d.root.join(format!("{id}.json")), "{}").unwrap();
            fs::create_dir_all(d.root.join("answer-log")).unwrap();
            fs::write(d.root.join("answer-log").join(format!("{id}.jsonl")), "x\n").unwrap();
        }
        // How nodes changed is kept as long as the conversations; the nodes themselves stay.
        let conn = crate::db::open(&d.root).unwrap();
        crate::db::node_put(&conn, &serde_json::json!({"nid": "n1", "memid": "m1", "label": "kept"})).unwrap();
        let at = |ms: i64| chrono::DateTime::from_timestamp_millis(ms).unwrap().to_rfc3339();
        for (id, ms) in [("old", now - 40 * day), ("new", now - day)] {
            crate::db::event_add(&conn, &serde_json::json!({"id": id, "source": "person", "type": "node.changed", "time": at(ms), "nid": "n1"})).unwrap();
        }
        let left = || crate::db::events(&conn, "1", &[]).unwrap().iter().map(|e| e["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(prune(&d), 0, "no limit by default");
        assert_eq!(left().len(), 2);
        fs::write(d.root.join("settings.json"), r#"{"keep_days":30}"#).unwrap();
        assert_eq!(prune(&d), 1);
        assert_eq!(left(), ["new"], "the event older than the limit went");
        assert!(crate::db::node_get(&conn, "n1").unwrap().is_some(), "the node stays");
        assert!(!d.root.join("a.json").exists() && !d.root.join("answer-log/b.jsonl").exists(), "every number of it, map and answers");
        assert!(d.root.join("c.json").exists() && record_of(&d, "c").is_some());
    }
    #[test]
    fn over_the_size_the_oldest_go_first_and_a_map_another_took_stays() {
        let d = data("prune-size");
        let now = chrono::Local::now().timestamp_millis();
        fs::create_dir_all(d.root.join("forks")).unwrap();
        for (id, age) in [("p", 4), ("x", 3), ("y", 2), ("child", 1)] {
            rec(&d, id, serde_json::json!({"id": id, "lastActive": now - age * 1000}));
            fs::write(d.root.join(format!("{id}.json")), vec![b' '; 300 * 1024]).unwrap();
        }
        fs::write(d.root.join("forks/inherited.json"), r#"{"child":"p"}"#).unwrap();
        fs::write(d.root.join("settings.json"), r#"{"keep_mb":1}"#).unwrap();
        assert_eq!(prune(&d), 1);
        assert!(d.root.join("p.json").exists(), "child took its map: kept");
        assert!(!d.root.join("x.json").exists(), "the oldest of the others");
        assert!(d.root.join("y.json").exists() && d.root.join("child.json").exists(), "under the size again: stops");
    }
    #[test]
    fn a_conversation_spans_from_its_start_not_from_a_map_it_carried_over() {
        let d = data("span");
        let nodes = vec![serde_json::json!({"id":"n","opened":{"at":100}})];
        rec(&d, "s", serde_json::json!({"id":"s","started":5000,"lastActive":9000}));
        assert_eq!(span(&d, "s", &nodes), (5000, 9000));
        rec(&d, "old", serde_json::json!({"id":"old","lastActive":9000}));
        assert_eq!(span(&d, "old", &nodes), (100, 9000), "no start kept: the map's own stamps");
        rec(&d, "r1", serde_json::json!({"id":"r1","started":2000,"movedTo":"r2"}));
        rec(&d, "r2", serde_json::json!({"id":"r2","started":7000,"lastActive":9500}));
        assert_eq!(span(&d, "r2", &nodes), (2000, 9500), "resumed under a new number: the first number's start");
    }
}
