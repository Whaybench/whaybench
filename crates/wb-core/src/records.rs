//! Reading what the map mod records per session: the chain of numbers a conversation
//! went through, every answer kept, how many files each map node has.
use std::{collections::{BTreeMap, HashMap, HashSet}, fs};

use serde_json::Value;

use crate::{paths::Data, sessions};

/// The session and the ids before it: moved on by /background, or whose map it inherited.
pub fn lineage(d: &Data, id: &str) -> Vec<String> {
    let records = sessions::all(d);
    let inherited: Value = fs::read_to_string(d.root.join("forks/inherited.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let (mut ids, mut frontier) = (vec![id.to_string()], vec![id.to_string()]);
    while let Some(here) = frontier.pop() {
        let mut before: Vec<String> = records.iter().filter(|s| s.get("movedTo").and_then(Value::as_str) == Some(&here)).filter_map(|s| s["id"].as_str().map(String::from)).collect();
        if let Some(i) = inherited.get(&here).and_then(Value::as_str) { before.push(i.to_string()); }
        for b in before {
            if !ids.contains(&b) { ids.push(b.clone()); frontier.push(b); }
        }
    }
    ids
}

/// Every kept answer of the conversation, oldest first; a turn carried over under a new id counts once, and a
/// line cut short does not hide the others.
pub fn answers_of(d: &Data, ids: &[String]) -> Vec<Value> {
    let mut rows: BTreeMap<(i64, String), Value> = BTreeMap::new();
    for sid in ids {
        let Ok(text) = fs::read_to_string(d.root.join("answer-log").join(format!("{sid}.jsonl"))) else { continue };
        for line in text.lines() {
            let Ok(row) = serde_json::from_str::<Value>(line) else { continue };
            let Some(at) = row.get("at").and_then(Value::as_i64) else { continue };
            let head: String = row.get("text").and_then(Value::as_str).unwrap_or("").chars().take(80).collect();
            rows.entry((at / 1000, head)).or_insert(row);
        }
    }
    let mut out: Vec<Value> = rows.into_values().collect();
    out.sort_by_key(|r| r["at"].as_i64().unwrap_or(0));
    out
}

/// When a map node came up: opened, else last changed.
pub fn node_time(node: &Value) -> Option<i64> {
    ["opened", "changed"].iter().find_map(|k| node.get(*k)?.get("at")?.as_i64().filter(|a| *a != 0))
}

/// The answer of the turn a node came up in: the first one finished at or after that time.
pub fn related_index(rows: &[Value], at: i64) -> Option<usize> { rows.iter().position(|r| r["at"].as_i64().unwrap_or(0) >= at) }

/// Every recorded read and change of the conversation, and the files attached to its nodes as `attached` touches.
pub fn touches(d: &Data, id: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for sid in lineage(d, id) {
        if let Some(list) = fs::read_to_string(d.root.join("files").join(format!("{sid}.json"))).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v.as_array().cloned()) { out.extend(list); }
        if let Some(table) = fs::read_to_string(d.root.join("node-files").join(format!("{sid}.json"))).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
            for (node, patterns) in table.as_object().into_iter().flatten() {
                for p in patterns.as_array().into_iter().flatten().filter_map(Value::as_str) {
                    for hit in glob::glob(p).into_iter().flatten().flatten().filter(|x| x.is_file()) {
                        let at = fs::metadata(&hit).ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|t| t.as_millis() as u64).unwrap_or(0);
                        out.push(serde_json::json!({"path": hit.to_string_lossy(), "op": "attached", "at": at, "node": node}));
                    }
                }
            }
        }
    }
    out
}

/// The AI's recorded edits of a file in the conversation, oldest first.
pub fn file_edits(d: &Data, id: &str, path: &str) -> Vec<Value> {
    let mut found: Vec<Value> = lineage(d, id).iter().filter_map(|sid| fs::read_to_string(d.root.join("files").join(format!("{sid}.edits.json"))).ok()).filter_map(|t| serde_json::from_str::<Value>(&t).ok()).filter_map(|v| v.as_array().cloned()).flatten().filter(|e| e.get("path").and_then(Value::as_str) == Some(path)).collect();
    found.sort_by_key(|e| e["at"].as_i64().unwrap_or(0));
    found
}

/// Bash run times of the conversation: (start, end) ms.
pub fn bash_runs(d: &Data, id: &str) -> Vec<(i64, i64)> {
    lineage(d, id).iter().filter_map(|sid| fs::read_to_string(d.root.join("files").join(format!("{sid}.bash.json"))).ok()).filter_map(|t| serde_json::from_str::<Value>(&t).ok()).filter_map(|v| v.as_array().cloned()).flatten().filter_map(|r| { let a = r.as_array()?; Some((a.first()?.as_i64()?, a.get(1)?.as_i64()?)) }).collect()
}

/// Map node id to how many different files were read or changed while on it, or attached to it.
pub fn file_counts(d: &Data, id: &str) -> HashMap<String, usize> {
    let mut found: HashMap<String, HashSet<String>> = HashMap::new();
    for sid in lineage(d, id) {
        if let Some(list) = fs::read_to_string(d.root.join("files").join(format!("{sid}.json"))).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v.as_array().cloned()) {
            for t in list {
                if let (Some(n), Some(p)) = (t.get("node").and_then(Value::as_str), t.get("path").and_then(Value::as_str)) {
                    found.entry(n.to_string()).or_default().insert(p.to_string());
                }
            }
        }
        if let Some(table) = fs::read_to_string(d.root.join("node-files").join(format!("{sid}.json"))).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
            for (node, patterns) in table.as_object().into_iter().flatten() {
                for p in patterns.as_array().into_iter().flatten().filter_map(Value::as_str) {
                    for hit in glob::glob(p).into_iter().flatten().flatten().filter(|x| x.is_file()) {
                        found.entry(node.clone()).or_default().insert(hit.to_string_lossy().to_string());
                    }
                }
            }
        }
    }
    found.into_iter().map(|(k, v)| (k, v.len())).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    fn data(name: &str) -> Data {
        let d = std::env::temp_dir().join(format!("wb-rec-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        for s in ["sessions", "answer-log", "files", "forks"] { fs::create_dir_all(d.join(s)).unwrap(); }
        Data::new(d)
    }
    #[test]
    fn both_ids_in_order_each_turn_once() {
        let d = data("both");
        let row = |t: i64| json!({"at": t * 1000, "prompt": "q", "text": format!("a{t}")}).to_string();
        fs::write(d.root.join("answer-log/old.jsonl"), format!("{}\n{}\n", row(1), row(2))).unwrap();
        fs::write(d.root.join("answer-log/new.jsonl"), format!("{}\n{}\n{{\"cut sho", row(2), row(3))).unwrap();
        let got = answers_of(&d, &["new".into(), "old".into()]);
        assert_eq!(got.iter().map(|r| r["text"].as_str().unwrap().to_string()).collect::<Vec<_>>(), ["a1", "a2", "a3"]);
    }
    #[test]
    fn the_answer_finished_after_the_node_came_up() {
        let rows = [json!({"at":100}), json!({"at":200}), json!({"at":300})];
        assert_eq!(related_index(&rows, 150), Some(1));
        assert_eq!(related_index(&rows, 200), Some(1));
        assert_eq!(related_index(&rows, 301), None);
    }
    #[test]
    fn node_times() {
        assert_eq!(node_time(&json!({"opened":{"at":5},"changed":{"at":9}})), Some(5));
        assert_eq!(node_time(&json!({"changed":{"at":9}})), Some(9));
        assert_eq!(node_time(&json!({"label":"x"})), None);
    }
    #[test]
    fn the_chain_of_numbers_and_file_counts() {
        let d = data("chain");
        fs::write(d.root.join("sessions/old.json"), json!({"id":"old","movedTo":"new"}).to_string()).unwrap();
        fs::write(d.root.join("sessions/new.json"), json!({"id":"new"}).to_string()).unwrap();
        assert_eq!(lineage(&d, "new"), ["new", "old"]);
        fs::write(d.root.join("files/old.json"), json!([{"path":"/a","node":"n"},{"path":"/a","node":"n"},{"path":"/b","node":"n"}]).to_string()).unwrap();
        assert_eq!(file_counts(&d, "new")["n"], 2);
    }
}
