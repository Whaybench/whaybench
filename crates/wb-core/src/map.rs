//! The conversation map file of a session: read it, or merge a change into it. Every agent
//! writes through here, so the rules of a change — merge by id, subtrees removed together, stamps the
//! program records, no broken references — are kept in one place.
use std::{collections::{BTreeMap, HashSet}, fs, io, path::{Path, PathBuf}, time::{SystemTime, UNIX_EPOCH}};

use serde_json::{Value, json};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Fields only the program writes; whatever an agent sends for them is dropped.
const STAMPED: [&str; 4] = ["opened", "changed", "closed", "visits"];
/// Fields only the person sets (through the alert panel); an agent's value is dropped.
const PERSON_ONLY: [&str; 1] = ["mark"];
/// States that close a node; every other state, and none, leaves it open. `removed` is the trash.
const CLOSING: [&str; 3] = ["done", "dropped", "removed"];

/// A new id no other program or call shares: `<prefix><time>-<process>-<count>`.
fn fresh(prefix: &str) -> String {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{prefix}{:x}-{:x}-{n:x}", now_ms(), std::process::id())
}

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

/// A condition: node ids joined by ∧ (and), ∨ (or) and ¬ (not), with parentheses. The node ids it names.
pub fn condition_ids(expr: &str) -> Result<Vec<String>> {
    let spaced = expr.replace('(', " ( ").replace(')', " ) ").replace('∧', " ∧ ").replace('∨', " ∨ ").replace('¬', " ¬ ");
    let (mut ids, mut depth, mut want_term) = (Vec::new(), 0i32, true);
    for tok in spaced.split_whitespace() {
        match tok {
            "(" | "¬" if want_term => { if tok == "(" { depth += 1; } }
            ")" if !want_term && depth > 0 => depth -= 1,
            "∧" | "∨" if !want_term => want_term = true,
            id if want_term && safe_id(id).is_ok() => { ids.push(id.to_string()); want_term = false; }
            _ => return Err(format!("condition is not well formed: {expr}").into()),
        }
    }
    if want_term || depth != 0 { return Err(format!("condition is not well formed: {expr}").into()); }
    Ok(ids)
}

/// Whether a condition holds, a node being true when `done` says so. ¬ binds tighter than ∧, ∧ tighter than ∨.
pub fn condition_holds(expr: &str, done: &dyn Fn(&str) -> bool) -> bool {
    let spaced = expr.replace('(', " ( ").replace(')', " ) ").replace('∧', " ∧ ").replace('∨', " ∨ ").replace('¬', " ¬ ");
    let toks: Vec<&str> = spaced.split_whitespace().collect();
    fn or(t: &[&str], i: &mut usize, d: &dyn Fn(&str) -> bool) -> bool {
        let mut v = and(t, i, d);
        while t.get(*i) == Some(&"∨") { *i += 1; let r = and(t, i, d); v = v || r; }
        v
    }
    fn and(t: &[&str], i: &mut usize, d: &dyn Fn(&str) -> bool) -> bool {
        let mut v = not(t, i, d);
        while t.get(*i) == Some(&"∧") { *i += 1; let r = not(t, i, d); v = v && r; }
        v
    }
    fn not(t: &[&str], i: &mut usize, d: &dyn Fn(&str) -> bool) -> bool {
        match t.get(*i).copied() {
            Some("¬") => { *i += 1; !not(t, i, d) }
            Some("(") => { *i += 1; let v = or(t, i, d); *i += 1; v }
            Some(id) => { *i += 1; d(id) }
            None => false,
        }
    }
    condition_ids(expr).is_ok() && or(&toks, &mut 0, done)
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
    for node in nodes {
        for key in ["requires", "assumes"] {
            if let Some(expr) = node.get(key).and_then(Value::as_str) {
                if let Some(missing) = condition_ids(expr)?.into_iter().find(|i| !ids.contains(i.as_str())) {
                    return Err(format!("{key} names a node that is not in the map: {missing}").into());
                }
            }
        }
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

fn upsert(nodes: &mut Vec<Value>, changes: &Value, stamp: &Value, person: bool) -> Result<()> {
    for raw in changes.as_array().ok_or("node changes must be an array")? {
        let id = safe_id(text(raw, "id")?)?;
        let index = nodes.iter().position(|n| n["id"] == id);
        let before = index.map(|i| nodes[i].clone());
        let mut merged = before.clone().unwrap_or_else(|| json!({"id":id,"label":id,"isOpen":true}));
        for (key, value) in raw.as_object().ok_or("node must be an object")? {
            if STAMPED.contains(&key.as_str()) || (!person && PERSON_ONLY.contains(&key.as_str())) { continue; }
            if value.is_null() && (person || PERSON_ONLY.contains(&key.as_str())) { merged.as_object_mut().unwrap().shift_remove(key); } else { merged[key] = value.clone(); }
        }
        // Open or closed follows the state: a closing state closes, closing without one means done, reopening
        // drops a closing state.
        let state = merged.get("state").and_then(Value::as_str).unwrap_or("").to_string();
        if raw.get("state").is_some() {
            merged["isOpen"] = json!(!CLOSING.contains(&state.as_str()));
        } else if raw.get("isOpen") == Some(&json!(false)) && !CLOSING.contains(&state.as_str()) {
            merged["state"] = json!("done");
        } else if raw.get("isOpen") == Some(&json!(true)) && CLOSING.contains(&state.as_str()) {
            merged.as_object_mut().unwrap().shift_remove("state");
        }
        // A new node without a memory id the agent chose starts its own.
        if before.is_none() && merged.get("memid").and_then(Value::as_str).is_none_or(str::is_empty) {
            merged["memid"] = json!(fresh("m"));
        }
        for key in ["parent", "reason", "detail"] {
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

/// A `resolves` link sent in this change settles its target: the target and what is still open under it become done.
fn resolve(nodes: &mut [Value], changes: &Value, stamp: &Value) {
    let targets: Vec<String> = changes.as_array().into_iter().flatten()
        .flat_map(|c| c.get("links").and_then(Value::as_array).cloned().unwrap_or_default())
        .filter(|l| l["rel"] == "resolves").filter_map(|l| l["to"].as_str().map(String::from)).collect();
    let mut settle: HashSet<String> = targets.into_iter().collect();
    loop {
        let before = settle.len();
        for n in nodes.iter() {
            if n.get("parent").and_then(Value::as_str).is_some_and(|p| settle.contains(p)) { settle.insert(n["id"].as_str().unwrap_or("").to_string()); }
        }
        if settle.len() == before { break; }
    }
    for n in nodes.iter_mut().filter(|n| settle.contains(n["id"].as_str().unwrap_or("")) && n["isOpen"] != false) {
        n["state"] = json!("done");
        n["isOpen"] = json!(false);
        n["changed"] = stamp.clone();
        n["closed"] = stamp.clone();
    }
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
    let map = change(data, sid, json!({"nodes": source.get("nodes").ok_or("map requires nodes array")?}), false)?;
    carried(data, sid, source);
    Ok(map)
}

/// The memory hears that this session's map came from another session or a saved map.
fn carried(data: &Path, sid: &str, source: &Value) {
    let from = source.get("sessionId").or_else(|| source.get("from")).and_then(Value::as_str).unwrap_or("");
    let ids: Vec<Value> = source.get("nodes").and_then(Value::as_array).into_iter().flatten().filter_map(|n| n.get("id").cloned()).collect();
    let at = now_ms() as i64;
    let ev = crate::memory::event(&format!("session/{sid}"), "map.carried", &format!("{sid}/"), at, json!({"from": from, "nodes": ids}));
    if let Err(e) = crate::memory::record(data, at, &[ev]) { crate::log::warn("map", "memory not written", &e.to_string(), &[]); }
}

fn change(data: &Path, sid: &str, input: Value, latest: bool) -> Result<Value> { change_by(data, sid, input, latest, false, None) }

/// The person's change from the alert panel: it may set the person's fields (the mark), and is recorded as the person's.
pub fn person(data: &Path, sid: &str, upsert: Value) -> Result<Value> { change_by(data, sid, json!({"upsert": upsert}), false, true, None) }

/// `action` numbers the events of one person's action (a move to the trash), so it can be undone as one.
fn change_by(data: &Path, sid: &str, input: Value, latest: bool, person: bool, action: Option<&str>) -> Result<Value> {
    let path = data.join(format!("{}.json", safe_id(sid)?));
    if !input.is_object() { return Err("map update must be an object".into()); }
    if !["nodes", "upsert", "remove", "current"].iter().any(|k| input.get(k).is_some()) {
        return Err("map update requires nodes, upsert, remove or current".into());
    }
    let mut map = read_json(&path, json!({"nodes":[],"current":null,"turn":0}))?;
    validate(&map)?;
    let before = map["nodes"].as_array().cloned().unwrap_or_default();
    let stamp = json!({"at":now_ms(),"turn":map.get("turn").and_then(Value::as_u64).unwrap_or(0)});
    let mut nodes = if let Some(replacement) = input.get("nodes") {
        validate(&json!({"nodes":replacement}))?;
        // A regrouping keeps each node's memory id: the agent sends the nodes without it.
        let mut replacement = replacement.clone();
        for n in replacement.as_array_mut().into_iter().flatten() {
            if n.get("memid").is_none() {
                if let Some(m) = before.iter().find(|o| o["id"] == n["id"]).and_then(|o| o.get("memid")).cloned() { n["memid"] = m; }
            }
        }
        let mut nodes = Vec::new();
        upsert(&mut nodes, &replacement, &stamp, false)?;
        // The trash is the person's: a regrouping that leaves its nodes out does not empty it.
        let kept: Vec<Value> = before.iter().filter(|o| o["state"] == "removed" && !nodes.iter().any(|n| n["id"] == o["id"])).cloned().collect();
        nodes.extend(kept);
        nodes
    } else { map["nodes"].as_array().unwrap().clone() };
    if let Some(changes) = input.get("upsert") {
        upsert(&mut nodes, changes, &stamp, person)?;
        resolve(&mut nodes, changes, &stamp);
    }
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
    // shortcut: every write counts as the session's own; a side worker's fill is not told apart yet.
    let at = stamp["at"].as_i64().unwrap_or(0);
    let source = if person { "person".to_string() } else { format!("session/{sid}") };
    let mut events = crate::memory::diff(&source, sid, at, &before, map["nodes"].as_array().map(Vec::as_slice).unwrap_or(&[]));
    for e in &mut events {
        e["turn"] = stamp["turn"].clone();
        if let Some(a) = action { e["action"] = json!(a); }
    }
    if let Err(e) = crate::memory::record(data, at, &events) { crate::log::warn("map", "memory not written", &e.to_string(), &[]); }
    Ok(map)
}

fn map_nodes(data: &Path, sid: &str) -> Result<Vec<Value>> {
    Ok(read_json(&data.join(format!("{}.json", safe_id(sid)?)), json!({"nodes":[]}))?["nodes"].as_array().cloned().unwrap_or_default())
}

/// The person moves a node to the trash: with its whole branch, or alone, its children moving up to its parent.
/// The events share one action number, the one `restore` takes.
pub fn trash(data: &Path, sid: &str, id: &str, branch: bool) -> Result<String> {
    let nodes = map_nodes(data, sid)?;
    let node = nodes.iter().find(|n| n["id"] == id).ok_or("no such node")?;
    let mut ups = vec![json!({"id": id, "state": "removed"})];
    if branch {
        let mut under: HashSet<&str> = HashSet::from([id]);
        loop {
            let before = under.len();
            for n in &nodes { if n["parent"].as_str().is_some_and(|p| under.contains(p)) { under.insert(n["id"].as_str().unwrap_or("")); } }
            if under.len() == before { break; }
        }
        // What an earlier action put in the trash stays with that action.
        ups.extend(nodes.iter().filter(|n| n["id"] != id && under.contains(n["id"].as_str().unwrap_or("")) && n["state"] != "removed").map(|n| json!({"id": n["id"], "state": "removed"})));
    } else {
        let up = node.get("parent").cloned().unwrap_or(json!(""));
        ups.extend(nodes.iter().filter(|n| n["parent"] == id).map(|n| json!({"id": n["id"], "parent": up})));
    }
    let action = fresh("a");
    change_by(data, sid, json!({"upsert": ups}), false, true, Some(&action))?;
    Ok(action)
}

/// Undoes one trash action: every node it changed gets its fields back, unless the node changed since; those are
/// left as they are and returned (`session/node id`).
pub fn restore(data: &Path, action: &str) -> Result<Vec<String>> {
    let evs = crate::memory::events(data);
    let mut by_session: BTreeMap<String, Vec<(String, Value)>> = BTreeMap::new();
    let mut since: HashSet<String> = HashSet::new();
    for (i, e) in evs.iter().enumerate().filter(|(_, e)| e["action"] == action && e["type"] == "node.changed") {
        let Some((sid, id)) = e["subject"].as_str().and_then(|s| s.split_once('/')) else { continue };
        if evs[i + 1..].iter().any(|l| l["subject"] == e["subject"] && l["action"] != action) { since.insert(format!("{sid}/{id}")); }
        by_session.entry(sid.to_string()).or_default().push((id.to_string(), e["data"].clone()));
    }
    let mut kept = Vec::new();
    for (sid, items) in by_session {
        let nodes = map_nodes(data, &sid)?;
        let mut ups = Vec::new();
        for (id, fields) in items {
            let now = nodes.iter().find(|n| n["id"] == id.as_str());
            let fields = fields.as_object().cloned().unwrap_or_default();
            let as_left = !since.contains(&format!("{sid}/{id}")) && now.is_some_and(|n| fields.iter().all(|(k, v)| n.get(k).unwrap_or(&Value::Null) == &v[1]));
            if !as_left { kept.push(format!("{sid}/{id}")); continue; }
            let mut up = json!({"id": id});
            for (k, v) in fields { up[k] = v[0].clone(); }
            ups.push(up);
        }
        if !ups.is_empty() { change_by(data, &sid, json!({"upsert": ups}), false, true, Some(&fresh("a")))?; }
    }
    Ok(kept)
}

/// Empties the trash for good: the given nodes of one session, if they are in the trash. A node outside the trash
/// under one of them moves up to the top level instead of going with it.
pub fn purge(data: &Path, sid: &str, ids: &[String]) -> Result<usize> {
    let nodes = map_nodes(data, sid)?;
    let gone: HashSet<&str> = nodes.iter().filter(|n| n["state"] == "removed" && ids.iter().any(|i| n["id"] == i.as_str())).filter_map(|n| n["id"].as_str()).collect();
    if gone.is_empty() { return Ok(0); }
    let lift: Vec<Value> = nodes.iter().filter(|n| n["state"] != "removed" && n["parent"].as_str().is_some_and(|p| gone.contains(p))).map(|n| json!({"id": n["id"], "parent": ""})).collect();
    let mut input = json!({"remove": gone.iter().collect::<Vec<_>>()});
    if !lift.is_empty() { input["upsert"] = json!(lift); }
    change_by(data, sid, input, false, true, None)?;
    Ok(gone.len())
}

/// At start: nodes in the trash longer than `trash_days` in `settings.json` (180 when unset, 0 = only by hand) go for good.
pub fn empty_old_trash(data: &Path) -> usize {
    let days = read_json(&data.join("settings.json"), Value::Null).ok().and_then(|s| s["trash_days"].as_u64()).unwrap_or(180);
    if days == 0 { return 0; }
    let before = now_ms().saturating_sub(days * 86_400_000);
    let mut n = 0;
    for (sid, nodes, _) in crate::memory::maps(data) {
        let old: Vec<String> = nodes.iter().filter(|x| x["state"] == "removed" && x["closed"]["at"].as_u64().is_some_and(|t| t < before)).filter_map(|x| x["id"].as_str().map(String::from)).collect();
        if !old.is_empty() { n += purge(data, &sid, &old).unwrap_or(0); }
    }
    n
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
    carried(data, sid, source);
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
    fn the_trash_takes_one_action_and_gives_it_back() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let sid = "aaaaaaaa-0000-0000-0000-000000000001";
        command(dir.path(), sid, Some(json!({"upsert": [{"id":"t","label":"top","isOpen":true},{"id":"a","parent":"t","label":"a","isOpen":true},
            {"id":"b","parent":"a","label":"b","isOpen":false},{"id":"c","parent":"a","label":"c","isOpen":true}]})))?;
        let get = |id: &str| map_nodes(dir.path(), sid).unwrap().into_iter().find(|n| n["id"] == id);
        // Alone: the children move up, and come back under it.
        let steps = crate::memory::steps(dir.path(), sid);
        assert_eq!((steps.len(), steps[0].2.as_str(), steps[2].3), (4, "t", true), "the event log gives the order, and which closed");
        let act = trash(dir.path(), sid, "a", false)?;
        assert_eq!((get("a").unwrap()["state"].as_str(), get("b").unwrap()["parent"].as_str()), (Some("removed"), Some("t")));
        assert!(restore(dir.path(), &act)?.is_empty());
        assert_eq!((get("a").unwrap().get("state"), get("a").unwrap()["isOpen"].as_bool(), get("b").unwrap()["parent"].as_str()), (None, Some(true), Some("a")));
        assert_eq!(get("b").unwrap()["state"], "done", "a closed child keeps its own state");
        // The branch: all of it, and a node changed since stays as it is.
        let act = trash(dir.path(), sid, "a", true)?;
        assert!(["a", "b", "c"].iter().all(|i| get(i).unwrap()["state"] == "removed"));
        let regrouped = command(dir.path(), sid, Some(json!({"nodes": [{"id":"t","label":"top","isOpen":true}]})))?;
        assert_eq!(regrouped["nodes"].as_array().unwrap().len(), 4, "a regrouping leaves the trash alone");
        person(dir.path(), sid, json!([{"id":"c","label":"renamed"}]))?;
        assert_eq!(restore(dir.path(), &act)?, vec![format!("{sid}/c")]);
        assert_eq!((get("a").unwrap()["isOpen"].as_bool(), get("c").unwrap()["state"].as_str()), (Some(true), Some("removed")));
        // Emptying: what is outside the trash under a purged node moves to the top level.
        trash(dir.path(), sid, "a", false)?;
        assert_eq!(purge(dir.path(), sid, &["a".into(), "c".into(), "t".into()])?, 2, "only nodes in the trash go");
        assert!(get("a").is_none() && get("c").is_none() && get("t").is_some());
        assert_eq!(get("b").unwrap()["parent"], "t");
        // Old trash goes at start; 0 days keeps it.
        trash(dir.path(), sid, "b", false)?;
        let path = dir.path().join(format!("{sid}.json"));
        let mut m: Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
        for n in m["nodes"].as_array_mut().unwrap() { if n["id"] == "b" { n["closed"]["at"] = json!(1); } }
        fs::write(&path, m.to_string())?;
        fs::write(dir.path().join("settings.json"), r#"{"trash_days":0}"#)?;
        assert_eq!(empty_old_trash(dir.path()), 0);
        fs::remove_file(dir.path().join("settings.json"))?;
        assert_eq!(empty_old_trash(dir.path()), 1);
        assert!(get("b").is_none());
        Ok(())
    }

    #[test]
    fn open_follows_the_state_and_the_mark_is_the_persons() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let up = |c: Value| command(dir.path(), "s", Some(json!({"upsert": [c]})));
        let map = up(json!({"id":"q","label":"question","isOpen":true,"mark":"important"}))?;
        assert!(map["nodes"][0].get("mark").is_none(), "an agent cannot set the mark");
        assert!(map["nodes"][0]["memid"].as_str().is_some_and(|m| !m.is_empty()), "a new node starts its own memory id");
        let map = up(json!({"id":"q","isOpen":false}))?;
        assert_eq!((map["nodes"][0]["state"].as_str(), map["nodes"][0]["isOpen"].as_bool()), (Some("done"), Some(false)));
        let map = up(json!({"id":"q","isOpen":true}))?;
        assert!(map["nodes"][0].get("state").is_none(), "reopening drops the closing state");
        let map = up(json!({"id":"q","state":"dropped"}))?;
        assert_eq!(map["nodes"][0]["isOpen"], json!(false));
        let map = up(json!({"id":"q","state":"postponed"}))?;
        assert_eq!(map["nodes"][0]["isOpen"], json!(true));
        let map = command(dir.path(), "s", Some(json!({"upsert": [{"id":"o","label":"option","isOpen":true,"parent":"q"},
            {"id":"d","label":"decided","isOpen":false,"links":[{"to":"q","rel":"resolves"}]}]})))?;
        let get = |id: &str| map["nodes"].as_array().unwrap().iter().find(|n| n["id"] == id).cloned().unwrap();
        assert_eq!((get("q")["state"].as_str(), get("o")["isOpen"].as_bool()), (Some("done"), Some(false)), "resolves settles the problem and what is open under it");
        let marked = person(dir.path(), "s", json!([{"id":"o","mark":"important"}]))?;
        assert_eq!(marked["nodes"].as_array().unwrap().iter().find(|n| n["id"] == "o").unwrap()["mark"], json!("important"), "the person may mark");
        let unmarked = person(dir.path(), "s", json!([{"id":"o","mark":null}]))?;
        assert!(unmarked["nodes"].as_array().unwrap().iter().find(|n| n["id"] == "o").unwrap().get("mark").is_none(), "and take it off");
        let memid = get("q")["memid"].clone();
        let map = command(dir.path(), "s", Some(json!({"nodes": [{"id":"q","label":"regrouped","isOpen":true}]})))?;
        let map = json!({"nodes": [map["nodes"].as_array().unwrap().iter().find(|n| n["id"] == "q").unwrap()]});
        assert_eq!(map["nodes"][0]["memid"], memid, "a regrouping keeps the memory id");
        Ok(())
    }

    #[test]
    fn conditions_name_nodes_with_and_or_not() -> Result<()> {
        assert_eq!(condition_ids("a ∧ (b ∨ ¬c)")?, ["a", "b", "c"]);
        let d = |id: &str| id == "a" || id == "b";
        assert!(condition_holds("a ∧ (b ∨ ¬c)", &d) && !condition_holds("a ∧ c", &d) && condition_holds("¬c ∨ c", &d) && !condition_holds("¬(a ∨ c)", &d));
        for bad in ["a ∧", "(a", "a b", "∧ a", "a)"] { assert!(condition_ids(bad).is_err(), "{bad}"); }
        let dir = tempfile::tempdir()?;
        command(dir.path(), "s", Some(json!({"upsert": [{"id":"a","label":"A","isOpen":true}]})))?;
        assert!(command(dir.path(), "s", Some(json!({"upsert": [{"id":"b","label":"B","isOpen":true,"requires":"a ∧ x"}]}))).is_err(), "x is not in the map");
        command(dir.path(), "s", Some(json!({"upsert": [{"id":"b","label":"B","isOpen":true,"requires":"a","due":"2026-12-01"}]})))?;
        Ok(())
    }

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
