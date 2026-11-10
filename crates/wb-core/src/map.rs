//! The conversation map file of a session: read it, or merge a change into it. Every agent
//! writes through here, so the rules of a change — merge by id, subtrees removed together, stamps the
//! program records, no broken references — are kept in one place.
use std::{collections::HashSet, fs, io, path::{Path, PathBuf}, time::{SystemTime, UNIX_EPOCH}};

use serde_json::{Value, json};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Fields only the program writes; whatever an agent sends for them is dropped.
const STAMPED: [&str; 4] = ["opened", "changed", "closed", "visits"];

pub(crate) fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

pub(crate) fn read_json(path: &Path, default: Value) -> Result<Value> {
    match fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(default),
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn write_json(path: &Path, value: &Value) -> Result<()> {
    fs::create_dir_all(path.parent().ok_or("missing parent directory")?)?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temp, serde_json::to_vec(value)?)?;
    fs::rename(temp, path)?;
    Ok(())
}

pub(crate) fn safe_id(value: &str) -> Result<&str> {
    if value.is_empty() || value.len() > 128 || !value.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-') {
        return Err("invalid session or node id".into());
    }
    Ok(value)
}

pub(crate) fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value.get(key).and_then(Value::as_str).ok_or_else(|| format!("missing string field: {key}").into())
}

pub fn validate(map: &Value) -> Result<()> {
    let nodes = map.get("nodes").and_then(Value::as_array).ok_or("map requires nodes array")?;
    let mut ids = HashSet::new();
    for node in nodes {
        let id = safe_id(text(node, "id")?)?;
        text(node, "label")?;
        if node.get("isOpen").is_some_and(|v| !v.is_boolean()) { return Err("isOpen must be a boolean".into()); }
        if !ids.insert(id) { return Err("duplicate node id".into()); }
    }
    if let Some(current) = map.get("current").filter(|v| !v.is_null()) {
        if !current.as_str().is_some_and(|id| ids.contains(id)) { return Err("current must refer to a node".into()); }
    }
    for node in nodes {
        let mut cursor = node;
        let mut seen = HashSet::new();
        while let Some(parent) = cursor.get("parent").filter(|v| !v.is_null()) {
            let parent = parent.as_str().ok_or("parent must be a node id")?;
            if !seen.insert(parent) { return Err("map contains a parent cycle".into()); }
            cursor = nodes.iter().find(|n| n["id"] == parent).ok_or("parent must refer to a node")?;
        }
    }
    Ok(())
}

fn upsert(nodes: &mut Vec<Value>, changes: &Value, stamp: &Value) -> Result<()> {
    for raw in changes.as_array().ok_or("node changes must be an array")? {
        let id = safe_id(text(raw, "id")?)?;
        let index = nodes.iter().position(|n| n["id"] == id);
        let before = index.map(|i| nodes[i].clone());
        let mut merged = before.clone().unwrap_or_else(|| json!({"id":id,"label":id,"isOpen":true}));
        for (key, value) in raw.as_object().ok_or("node must be an object")? {
            if !STAMPED.contains(&key.as_str()) { merged[key] = value.clone(); }
        }
        for key in ["parent", "reason"] {
            if merged[key] == "" { merged.as_object_mut().unwrap().shift_remove(key); }
        }
        if let Some(old) = &before {
            let changed = merged.as_object().unwrap().iter()
                .any(|(k, v)| !STAMPED.contains(&k.as_str()) && old.get(k) != Some(v))
                || old.as_object().unwrap().keys().any(|k| !STAMPED.contains(&k.as_str()) && merged.get(k).is_none());
            if changed { merged["changed"] = stamp.clone(); }
        } else {
            merged["opened"] = stamp.clone();
        }
        if before.as_ref().is_none_or(|n| n["isOpen"] != false) && merged["isOpen"] == false {
            merged["closed"] = stamp.clone();
        }
        if before.as_ref().is_some_and(|n| n["isOpen"] == false) && merged["isOpen"] == true {
            merged.as_object_mut().unwrap().shift_remove("closed");
        }
        if let Some(i) = index { nodes[i] = merged; } else { nodes.push(merged); }
    }
    Ok(())
}

/// `whaybench map SESSION [--write]`: without a change, the map as stored; with one (`upsert`, `remove`,
/// `current`, or `nodes` for a whole regrouping), the map after it, saved with `latest.json`. A change
/// that would leave a broken map is refused and the stored map stays as it was.
pub fn command(data: &Path, sid: &str, input: Option<Value>) -> Result<Value> {
    match input {
        None => read_json(&data.join(format!("{}.json", safe_id(sid)?)), json!({})),
        Some(input) => change(data, sid, input, true),
    }
}

/// `whaybench map SESSION --start-from < MAP.json`: a new session starts from an imported map, to know what
/// went before: its nodes start over as opened now, and `latest.json` is left as it is,
/// as the session has not talked yet.
pub fn start_from(data: &Path, sid: &str, source: &Value) -> Result<Value> {
    change(data, sid, json!({"nodes": source.get("nodes").ok_or("map requires nodes array")?}), false)
}

fn change(data: &Path, sid: &str, input: Value, latest: bool) -> Result<Value> {
    let path = data.join(format!("{}.json", safe_id(sid)?));
    if !input.is_object() { return Err("map update must be an object".into()); }
    if !["nodes", "upsert", "remove", "current"].iter().any(|k| input.get(k).is_some()) {
        return Err("map update requires nodes, upsert, remove or current".into());
    }
    let mut map = read_json(&path, json!({"nodes":[],"current":null,"turn":0}))?;
    validate(&map)?;
    let stamp = json!({"at":now_ms(),"turn":map.get("turn").and_then(Value::as_u64).unwrap_or(0)});
    let mut nodes = if let Some(replacement) = input.get("nodes") {
        validate(&json!({"nodes":replacement}))?;
        let mut nodes = Vec::new();
        upsert(&mut nodes, replacement, &stamp)?;
        nodes
    } else { map["nodes"].as_array().unwrap().clone() };
    if let Some(changes) = input.get("upsert") { upsert(&mut nodes, changes, &stamp)?; }
    let mut gone = HashSet::new();
    if let Some(remove) = input.get("remove") {
        for id in remove.as_array().ok_or("remove must be an array")? {
            gone.insert(safe_id(id.as_str().ok_or("remove requires node ids")?)?.to_string());
        }
        loop {
            let before = gone.len();
            for node in &nodes {
                if node.get("parent").and_then(Value::as_str).is_some_and(|p| gone.contains(p)) {
                    gone.insert(text(node, "id")?.to_string());
                }
            }
            if gone.len() == before { break; }
        }
        nodes.retain(|n| !gone.contains(n["id"].as_str().unwrap_or("")));
    }
    let current = input.get("current").cloned().unwrap_or_else(|| {
        let old = map.get("current").cloned().unwrap_or(Value::Null);
        if old.as_str().is_some_and(|id| gone.contains(id)) { Value::Null } else { old }
    });
    if current != map["current"] {
        if let Some(node) = nodes.iter_mut().find(|n| n["id"] == current) {
            let mut visits = node.get("visits").and_then(Value::as_array).cloned().unwrap_or_default();
            if visits.last() != Some(&stamp["turn"]) { visits.push(stamp["turn"].clone()); }
            node["visits"] = json!(visits);
        }
    }
    map["nodes"] = json!(nodes);
    map["current"] = current;
    map["sessionId"] = json!(sid);
    map["version"] = json!(2);
    map["updatedAt"] = stamp["at"].clone();
    validate(&map)?;
    write_json(&path, &map)?;
    if latest { write_json(&data.join("latest.json"), &map)?; }
    Ok(map)
}

/// `whaybench map SESSION --end-turn`: one more finished turn, so the stamps of the next changes say in
/// which turn they were made. `latest.json` is rewritten only while it shows this session.
pub fn end_turn(data: &Path, sid: &str) -> Result<Value> {
    let path = data.join(format!("{}.json", safe_id(sid)?));
    let mut map = read_json(&path, json!({"sessionId":sid,"nodes":[],"current":null,"turn":0}))?;
    validate(&map)?;
    map["turn"] = json!(map.get("turn").and_then(Value::as_u64).unwrap_or(0) + 1);
    map["version"] = json!(2);
    map["updatedAt"] = json!(now_ms());
    write_json(&path, &map)?;
    if read_json(&data.join("latest.json"), Value::Null)?["sessionId"] == sid {
        write_json(&data.join("latest.json"), &map)?;
    }
    Ok(map)
}

/// `whaybench map SESSION --take-over < MAP.json`: this session continues another's conversation map
/// (`/background`, `/resume`, a tab that keeps the map). The nodes keep their stamps, unlike a regrouping
/// with `nodes`; `latest.json` is left as it is.
pub fn take_over(data: &Path, sid: &str, source: &Value) -> Result<Value> {
    let path = data.join(format!("{}.json", safe_id(sid)?));
    validate(source)?;
    let map = json!({"sessionId":sid, "nodes":source["nodes"], "current":source.get("current").cloned().unwrap_or(Value::Null),
        "turn":source.get("turn").and_then(Value::as_u64).unwrap_or(0), "version":2, "updatedAt":now_ms()});
    write_json(&path, &map)?;
    Ok(map)
}

/// The earliest and latest time stamped on the nodes, in ms; None without any.
pub fn node_span(nodes: &[Value]) -> Option<(i64, i64)> {
    let times: Vec<i64> = nodes.iter().flat_map(|n| ["opened", "changed", "closed"].map(|k| n.get(k).and_then(|v| v.get("at")).and_then(Value::as_i64).unwrap_or(0))).filter(|a| *a > 0).collect();
    Some((*times.iter().min()?, *times.iter().max()?))
}

/// A saved map's file name: what it is and the minutes its conversation began and was last active (`span`, ms),
/// never its title (a title is in the person's language; it is kept inside the file).
/// `map-<first>-<last>`, `branch-<node id>-<first>-<last>`.
pub fn file_name(span: (i64, i64), root: Option<&str>, prefix: &str) -> String {
    let minute = |ms: i64| chrono::DateTime::from_timestamp_millis(ms).map(|t| t.with_timezone(&chrono::Local).format("%Y%m%d-%H%M").to_string()).unwrap_or_default();
    let (first, last) = span;
    let kind = match root {
        None => "map".to_string(),
        Some(r) => { let id: String = r.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect(); format!("branch-{}", if id.is_empty() { "node" } else { &id }) }
    };
    format!("{prefix}{kind}-{}-{}", minute(first), minute(last))
}

/// `<name>.json` in the folder, or `<name>-2.json` and on when that is taken: the same map kept twice keeps both.
pub fn free_path(dir: &Path, name: &str) -> PathBuf {
    (1..).map(|i| dir.join(if i == 1 { format!("{name}.json") } else { format!("{name}-{i}.json") })).find(|p| !p.exists()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_map_cannot_replace_a_valid_map() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let initial = json!({"nodes":[{"id":"n1","label":"topic"}],"current":"n1"});
        command(dir.path(), "session1", Some(initial))?;
        let path = dir.path().join("session1.json");
        let before = fs::read(&path)?;
        for map in [
            json!({"nodes":[{"id":"n","label":"a"},{"id":"n","label":"b"}]}),
            json!({"nodes":[],"current":"missing"}),
            json!({"nodes":[{"id":"n","label":"a","parent":"missing"}]}),
            json!({"nodes":[{"id":"a","label":"a","parent":"b"},{"id":"b","label":"b","parent":"a"}]}),
            json!({"nodes":[{"id":"n","label":5}]}),
        ] {
            assert!(command(dir.path(), "session1", Some(map)).is_err());
            assert_eq!(fs::read(&path)?, before);
        }
        assert_eq!(command(dir.path(), "session1", None)?["nodes"][0]["label"], "topic");
        Ok(())
    }

    #[test]
    fn a_finished_turn_stamps_the_next_change() -> Result<()> {
        let dir = tempfile::tempdir()?;
        assert_eq!(end_turn(dir.path(), "s1")?["turn"], 1);
        let m = command(dir.path(), "s1", Some(json!({"upsert":[{"id":"a","label":"topic","isOpen":true}]})))?;
        assert_eq!(m["nodes"][0]["opened"]["turn"], 1);
        command(dir.path(), "other", Some(json!({"nodes":[]})))?;
        end_turn(dir.path(), "s1")?;
        assert_eq!(read_json(&dir.path().join("latest.json"), Value::Null)?["sessionId"], "other");
        Ok(())
    }

    #[test]
    fn a_map_taken_over_keeps_its_stamps() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let old = command(dir.path(), "old", Some(json!({"upsert":[{"id":"a","label":"topic","isOpen":false}],"current":"a"})))?;
        let new = take_over(dir.path(), "new", &old)?;
        assert_eq!(new["nodes"], old["nodes"]);
        assert_eq!(new["sessionId"], "new");
        assert_eq!(read_json(&dir.path().join("latest.json"), Value::Null)?["sessionId"], "old");
        assert!(take_over(dir.path(), "new", &json!({"nodes":[],"current":"gone"})).is_err());
        Ok(())
    }

    #[test]
    fn a_new_session_from_an_imported_map_starts_it_over_without_taking_the_panel() -> Result<()> {
        let dir = tempfile::tempdir()?;
        command(dir.path(), "other", Some(json!({"nodes":[]})))?;
        let m = start_from(dir.path(), "new", &json!({"nodes":[{"id":"a","label":"topic","opened":{"at":1,"turn":9}}]}))?;
        assert_eq!(m["nodes"][0]["opened"]["turn"], 0);
        assert_eq!(read_json(&dir.path().join("latest.json"), Value::Null)?["sessionId"], "other");
        Ok(())
    }

    #[test]
    fn ids_cannot_leave_the_data_directory() {
        for id in ["../original", "/tmp/file", "", "a/b", "a.b"] { assert!(safe_id(id).is_err()); }
    }
}
