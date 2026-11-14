//! The conversation map file of a session: read it, or merge a change into it. Every agent
//! writes through here, so the rules of a change — merge by id, subtrees removed together, stamps the
//! program records, no broken references — are kept in one place.
use std::{collections::{HashMap, HashSet}, fs, io, path::{Path, PathBuf}, time::{SystemTime, UNIX_EPOCH}};

use serde_json::{Value, json};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Fields only the program writes; whatever an agent sends for them is dropped.
const STAMPED: [&str; 3] = ["opened", "changed", "closed"];
/// What the program gives a node: its number in the store and the session it came from.
const PROGRAM: [&str; 2] = ["nid", "origin"];
/// Fields only the person sets (through the notifications panel); an agent's value is dropped.
const PERSON_ONLY: [&str; 1] = ["mark"];
/// States that close a node; every other state, and none, leaves it open. `removed` is the trash.
const CLOSING: [&str; 3] = ["done", "dropped", "removed"];

/// A new id no other program or call shares: `<prefix><time>-<process>-<count>`.
fn fresh(prefix: &str) -> String {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{prefix}{:x}-{:x}-{n:x}", now_ms(), std::process::id())
}

/// Open or closed comes from the state alone: done, dropped and removed (the trash) are closed; plan, doing,
/// changed, postponed and no state are open. Nothing else is stored for it.
pub fn is_open(n: &Value) -> bool { !CLOSING.contains(&n["state"].as_str().unwrap_or("")) }

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
    Ok(crate::paths::write_atomic(path, serde_json::to_vec(value)?)?)
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
        if let Some(sg) = node.get("suggest").filter(|v| !v.is_null())
            && !["done", "dropped"].contains(&sg["state"].as_str().unwrap_or("")) { return Err("suggest.state must be done or dropped".into()); }
        if !ids.insert(id) { return Err("duplicate node id".into()); }
    }
    for node in nodes {
        for key in ["requires", "assumes"] {
            if let Some(expr) = node.get(key).and_then(Value::as_str)
                && let Some(missing) = condition_ids(expr)?.into_iter().find(|i| !ids.contains(i.as_str())) {
                    return Err(format!("{key} names a node that is not in the map: {missing}").into());
                }
        }
    }
    if let Some(current) = map.get("current").filter(|v| !v.is_null())
        && !current.as_str().is_some_and(|id| ids.contains(id)) { return Err("current must refer to a node".into()); }
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
        let mut merged = before.clone().unwrap_or_else(|| json!({"id":id,"label":id}));
        for (key, value) in raw.as_object().ok_or("node must be an object")? {
            if STAMPED.contains(&key.as_str()) || PROGRAM.contains(&key.as_str()) || (!person && PERSON_ONLY.contains(&key.as_str())) { continue; }
            if value.is_null() && (person || PERSON_ONLY.contains(&key.as_str())) { merged.as_object_mut().unwrap().shift_remove(key); } else { merged[key] = value.clone(); }
        }
        // Open or closed is the state's. An agent that still says isOpen (an older adapter) is read as a state:
        // closing without one means done, reopening drops a closing state; isOpen itself is not kept.
        let state = merged.get("state").and_then(Value::as_str).unwrap_or("").to_string();
        if raw.get("state").is_none() && raw.get("isOpen") == Some(&json!(false)) && !CLOSING.contains(&state.as_str()) {
            merged["state"] = json!("done");
        } else if raw.get("state").is_none() && raw.get("isOpen") == Some(&json!(true)) && CLOSING.contains(&state.as_str()) {
            merged.as_object_mut().unwrap().shift_remove("state");
        }
        merged.as_object_mut().unwrap().shift_remove("isOpen");
        // A suggestion is about an open node: once it closes, by the person or the agent, there is nothing to approve.
        if !is_open(&merged) { merged.as_object_mut().unwrap().shift_remove("suggest"); }
        // A new node without a memory id the agent chose starts its own.
        if before.is_none() && merged.get("memid").and_then(Value::as_str).is_none_or(str::is_empty) {
            merged["memid"] = json!(fresh("m"));
        }
        for key in ["parent", "reason", "detail", "state"] {
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
        if before.as_ref().is_none_or(is_open) && !is_open(&merged) {
            merged["closed"] = stamp.clone();
        }
        if before.as_ref().is_some_and(|n| !is_open(n)) && is_open(&merged) {
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
    for n in nodes.iter_mut().filter(|n| settle.contains(n["id"].as_str().unwrap_or("")) && is_open(n)) {
        n["state"] = json!("done");
        n["changed"] = stamp.clone();
        n["closed"] = stamp.clone();
    }
}

/// Where a change applies: a session's map (ids are the session's own short ids), or nodes named by their node number
/// (the person's change from the notifications panel, which has no session).
enum Scope<'a> { Session(&'a str), Nodes(Vec<String>) }

fn session_path(data: &Path, sid: &str) -> Result<PathBuf> { Ok(data.join(format!("{}.json", safe_id(sid)?))) }

/// A session's map file: the nodes it shows (`nodes: [{id, nid}]`, in the order shown), where the conversation is
/// (`current`) and its finished turns (`turn`). No node content: that is in the store.
fn session_read(data: &Path, sid: &str) -> Result<Value> {
    read_json(&session_path(data, sid)?, json!({"sessionId": sid, "nodes": [], "current": null, "turn": 0}))
}

fn entries_of(session: &Value) -> Vec<(String, String)> {
    session["nodes"].as_array().into_iter().flatten()
        .filter_map(|e| Some((e.get("id")?.as_str()?.to_string(), e.get("nid")?.as_str()?.to_string()))).collect()
}

fn session_write(data: &Path, sid: &str, entries: &[(String, String)], current: &Value, turn: u64, latest: bool) -> Result<Value> {
    let file = json!({"sessionId": sid, "nodes": entries.iter().map(|(id, nid)| json!({"id": id, "nid": nid})).collect::<Vec<_>>(),
        "current": current, "turn": turn, "version": 3, "updatedAt": now_ms()});
    write_json(&session_path(data, sid)?, &file)?;
    if latest { write_json(&data.join("latest.json"), &file)?; }
    Ok(file)
}

/// The nodes a session shows: `(its id for the node, the node's number)`.
pub(crate) fn session_entries(data: &Path, sid: &str) -> Vec<(String, String)> { session_read(data, sid).map(|s| entries_of(&s)).unwrap_or_default() }

/// The session's map file, made empty if it has none yet.
pub fn ensure(data: &Path, sid: &str) -> Result<Value> {
    let path = session_path(data, sid)?;
    if !path.exists() { session_write(data, sid, &[], &Value::Null, 0, false)?; }
    session_read(data, sid)
}

/// A condition with every node id put through `f`; None when `f` does not know one of them.
fn rename_ids(expr: &str, f: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let spaced = expr.replace('(', " ( ").replace(')', " ) ").replace('∧', " ∧ ").replace('∨', " ∨ ").replace('¬', " ¬ ");
    let toks: Option<Vec<String>> = spaced.split_whitespace().map(|t| if ["(", ")", "∧", "∨", "¬"].contains(&t) { Some(t.to_string()) } else { f(t) }).collect();
    Some(toks?.join(" "))
}

/// The nodes of the entries as a map shows them: `id` is the entry's id, and what points at another node (parent,
/// links, conditions) names that node's id here. What points at a node outside the entries is left out of the view;
/// the store keeps it. A node no longer in the store (deleted for good) is skipped.
fn view_nodes(conn: &rusqlite::Connection, entries: &[(String, String)]) -> Result<Vec<Value>> {
    let local: HashMap<&str, &str> = entries.iter().map(|(id, nid)| (nid.as_str(), id.as_str())).collect();
    let here = |nid: &str| local.get(nid).map(|i| i.to_string());
    let mut out = Vec::new();
    for (id, nid) in entries {
        let Some(mut n) = crate::db::node_get(conn, nid)? else { continue };
        let o = n.as_object_mut().ok_or("a node is an object")?;
        o.insert("id".into(), json!(id));
        match o.get("parent").and_then(Value::as_str).and_then(here) { Some(p) => { o.insert("parent".into(), json!(p)); } None => { o.shift_remove("parent"); } }
        for key in ["requires", "assumes"] {
            match o.get(key).and_then(Value::as_str).and_then(|e| rename_ids(e, &here)) { Some(e) => { o.insert(key.into(), json!(e)); } None => { o.shift_remove(key); } }
        }
        let links: Vec<Value> = o.get("links").and_then(Value::as_array).into_iter().flatten()
            .filter_map(|l| Some(json!({"to": here(l["to"].as_str()?)?, "rel": l["rel"]}))).collect();
        if links.is_empty() { o.shift_remove("links"); } else { o.insert("links".into(), json!(links)); }
        out.push(n);
    }
    Ok(out)
}

fn view_of(conn: &rusqlite::Connection, session: &Value, sid: &str) -> Result<Value> {
    let nodes = view_nodes(conn, &entries_of(session))?;
    // A node deleted for good may have been where the conversation was.
    let current = session.get("current").filter(|c| nodes.iter().any(|n| &&n["id"] == c)).cloned().unwrap_or(Value::Null);
    Ok(json!({"sessionId": sid, "nodes": nodes, "current": current, "turn": session.get("turn").and_then(Value::as_u64).unwrap_or(0),
        "version": 3, "updatedAt": session.get("updatedAt").cloned().unwrap_or(Value::Null)}))
}

/// A session's map with its nodes' content, read from the store: what the map panel draws and the agent is shown.
/// An empty object for a session without a map file.
pub fn view(data: &Path, sid: &str) -> Result<Value> {
    if !session_path(data, sid)?.exists() { return Ok(json!({})); }
    view_of(&crate::db::open(data)?, &session_read(data, sid)?, sid)
}

/// What a saved map keeps of these nodes: which they are (`{id, nid}`), not their content. A node that is not in the
/// memory (a map file from elsewhere) is kept whole.
pub fn saved_nodes(nodes: &[Value]) -> Vec<Value> {
    nodes.iter().map(|n| match n.get("nid") { Some(nid) => json!({"id": n["id"], "nid": nid}), None => n.clone() }).collect()
}

/// A saved map file (`whaybench map --file PATH`) with its nodes' content read from the memory, as `view` gives a
/// session's. A node the file holds whole (a file from elsewhere, or one saved before nodes had numbers) is shown
/// as the file has it; one named by a number the memory no longer has is skipped.
pub fn view_file(data: &Path, path: &Path) -> Result<Value> {
    let mut file: Value = serde_json::from_slice(&fs::read(path)?)?;
    let listed = file.get("nodes").and_then(Value::as_array).cloned().ok_or("map requires nodes array")?;
    let conn = crate::db::open(data)?;
    let entries: Vec<(String, String)> = listed.iter().filter(|n| n.get("label").is_none())
        .filter_map(|n| Some((n.get("id")?.as_str()?.to_string(), n.get("nid")?.as_str()?.to_string()))).collect();
    let mut shown = view_nodes(&conn, &entries)?.into_iter();
    let mut next = shown.next();
    // In the file's order: a listed node from the memory, a whole one as it is.
    let mut nodes = Vec::new();
    for n in &listed {
        if n.get("label").is_some() { nodes.push(n.clone()); }
        else if next.as_ref().is_some_and(|v| v["id"] == n["id"]) { nodes.extend(next.take()); next = shown.next(); }
    }
    file["nodes"] = json!(nodes);
    Ok(file)
}

/// `whaybench map SESSION [--write]`: without a change, the map (`view`); with one (`upsert`, `remove`,
/// `current`, or `nodes` for a whole regrouping), the map after it, with `latest.json` pointing at it. A change
/// that would leave a broken map is refused and nothing is stored. `remove` only takes a node off this map:
/// the node stays in the memory.
pub fn command(data: &Path, sid: &str, input: Option<Value>) -> Result<Value> {
    match input {
        None => view(data, sid),
        Some(input) => change(data, sid, input, true),
    }
}

/// `whaybench map SESSION --start-from < MAP.json`: a new session starts from another map, to know what went
/// before. `latest.json` is left as it is, as the session has not talked yet.
pub fn start_from(data: &Path, sid: &str, source: &Value) -> Result<Value> { adopt(data, sid, source, false) }

/// `whaybench map SESSION --take-over < MAP.json`: this session continues another's conversation map
/// (`/background`, `/resume`, a tab that keeps the map), with where it was and its turns; `latest.json` is left as it is.
pub fn take_over(data: &Path, sid: &str, source: &Value) -> Result<Value> { adopt(data, sid, source, true) }

/// The session's map becomes the source's: the same nodes, linked, not copied. A source node that is not in the
/// store (a map file from elsewhere) is made new.
fn adopt(data: &Path, sid: &str, source: &Value, position: bool) -> Result<Value> {
    validate(source)?;
    let nodes = source.get("nodes").and_then(Value::as_array).ok_or("map requires nodes array")?;
    let conn = crate::db::open(data)?;
    let (mut entries, mut fresh_nodes) = (Vec::new(), Vec::new());
    for n in nodes {
        match n.get("nid").and_then(Value::as_str).filter(|nid| crate::db::node_get(&conn, nid).is_ok_and(|f| f.is_some())) {
            Some(nid) => entries.push((text(n, "id")?.to_string(), nid.to_string())),
            None => fresh_nodes.push(n.clone()),
        }
    }
    let at = now_ms() as i64;
    for (_, nid) in &entries {
        crate::db::event_add(&conn, &event(&format!("session/{sid}"), "session.linked", Some(nid), Some(sid), at, json!({})))?;
    }
    drop(conn);
    let (current, turn) = if position { (source.get("current").cloned().unwrap_or(Value::Null), source.get("turn").and_then(Value::as_u64).unwrap_or(0)) } else { (Value::Null, 0) };
    // Where the conversation was may be one of the nodes still to be made.
    let known = entries.iter().any(|(id, _)| Some(id.as_str()) == current.as_str());
    session_write(data, sid, &entries, if known { &current } else { &Value::Null }, turn, false)?;
    if !fresh_nodes.is_empty() {
        let mut input = json!({"upsert": fresh_nodes});
        if !known && !current.is_null() { input["current"] = current; }
        change(data, sid, input, false)?;
    }
    view(data, sid)
}

/// One event: `source` is `session/<id>`, `person`, `worker` or `program`.
pub(crate) fn event(source: &str, kind: &str, nid: Option<&str>, session: Option<&str>, at: i64, data: Value) -> Value {
    let time = chrono::DateTime::from_timestamp_millis(at).map(|t| t.to_rfc3339()).unwrap_or_default();
    let mut e = json!({"specversion": "1.0", "id": fresh("e"), "source": source, "type": kind, "time": time, "data": data});
    if let Some(n) = nid { e["nid"] = json!(n); }
    if let Some(s) = session { e["session"] = json!(s); }
    e
}

fn change(data: &Path, sid: &str, input: Value, latest: bool) -> Result<Value> { change_by(data, Scope::Session(sid), input, latest, false, None) }

/// The person's change from a panel: `upsert` names nodes by their node number (`id` is the `nid`). It may set the
/// person's fields (the mark), and is recorded as the person's.
pub fn person(data: &Path, upsert: Value) -> Result<Value> {
    let nids = upsert.as_array().into_iter().flatten().filter_map(|u| u["id"].as_str().map(String::from)).collect();
    change_by(data, Scope::Nodes(nids), json!({"upsert": upsert}), false, true, None)
}

/// The session and the person write the same map file: one at a time, or a change is lost. Held until dropped.
fn lock(data: &Path, sid: &str) -> Result<fs::File> {
    let locks = data.join("cache").join("map-locks");
    fs::create_dir_all(&locks)?;
    let lock = fs::OpenOptions::new().create(true).truncate(false).write(true).open(locks.join(format!("{}.lock", safe_id(sid)?)))?;
    // SAFETY: flock on a file this function owns; released when the file closes.
    unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&lock), libc::LOCK_EX) };
    Ok(lock)
}

/// The person puts a node of the memory on a session's map. The id it has there.
pub fn link(data: &Path, sid: &str, nid: &str) -> Result<String> {
    let _lock = lock(data, sid)?;
    let session = session_read(data, sid)?;
    let mut entries = entries_of(&session);
    if entries.iter().any(|e| e.1 == nid) { return Err("the node is already on this map".into()); }
    let conn = crate::db::open(data)?;
    if crate::db::node_get(&conn, nid)?.is_none() { return Err("no such node".into()); }
    let id = (1..).map(|i| format!("k{i}")).find(|k| !entries.iter().any(|e| &e.0 == k)).unwrap_or_default();
    entries.push((id.clone(), nid.to_string()));
    crate::db::event_add(&conn, &event("person", "session.linked", Some(nid), Some(sid), now_ms() as i64, json!({})))?;
    session_write(data, sid, &entries, &session["current"], session.get("turn").and_then(Value::as_u64).unwrap_or(0), false)?;
    Ok(id)
}

/// How many nodes are under a node, in the whole memory.
pub fn below(data: &Path, nid: &str) -> usize {
    let Ok(conn) = crate::db::open(data) else { return 0 };
    let mut all = vec![nid.to_string()];
    let mut i = 0;
    while i < all.len() { for k in crate::db::children(&conn, &all[i]).unwrap_or_default() { if !all.contains(&k) { all.push(k); } } i += 1; }
    all.len() - 1
}

/// `action` numbers the events of one person's action (a deletion), so it can be undone as one.
fn change_by(data: &Path, scope: Scope, input: Value, latest: bool, person: bool, action: Option<&str>) -> Result<Value> {
    if !input.is_object() { return Err("map update must be an object".into()); }
    if !["nodes", "upsert", "remove", "current"].iter().any(|k| input.get(k).is_some()) {
        return Err("map update requires nodes, upsert, remove or current".into());
    }
    let _lock = match &scope { Scope::Session(sid) => Some(lock(data, sid)?), Scope::Nodes(_) => None };
    let mut conn = crate::db::open(data)?;
    // Reading the nodes and writing them back is one step in the store: another program's change cannot fall between.
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let (sid, session) = match &scope {
        Scope::Session(sid) => (Some(*sid), session_read(data, sid)?),
        Scope::Nodes(nids) => (None, json!({"nodes": nids.iter().map(|n| json!({"id": n, "nid": n})).collect::<Vec<_>>(), "current": null, "turn": 0})),
    };
    let mut map = view_of(&tx, &session, sid.unwrap_or(""))?;
    validate(&map)?;
    let before = map["nodes"].as_array().cloned().unwrap_or_default();
    // A change by node number is to nodes that exist: one that is gone (deleted for good meanwhile) is not made anew.
    if let Scope::Nodes(nids) = &scope
        && let Some(gone) = nids.iter().find(|n| !before.iter().any(|b| &b["id"] == *n)) { return Err(format!("no such node: {gone}").into()); }
    let stamp = json!({"at":now_ms(),"turn":map.get("turn").and_then(Value::as_u64).unwrap_or(0)});
    let mut nodes = if let Some(replacement) = input.get("nodes") {
        validate(&json!({"nodes":replacement}))?;
        let mut nodes = Vec::new();
        upsert(&mut nodes, replacement, &stamp, false)?;
        // A regrouping names the same nodes again: each keeps what the program gave it (its number, its memory id
        // unless the agent sent one, the session it came from) and the person's mark, which only the person takes off.
        for n in &mut nodes {
            let Some(old) = before.iter().find(|o| o["id"] == n["id"]) else { continue };
            for key in ["nid", "origin", "mark"] { if let Some(v) = old.get(key) { n[key] = v.clone(); } }
            if replacement.as_array().into_iter().flatten().find(|r| r["id"] == n["id"]).is_some_and(|r| r.get("memid").is_none())
                && let Some(m) = old.get("memid") { n["memid"] = m.clone(); }
        }
        // A deleted node is the person's: a regrouping that leaves it out does not take it off the map.
        let kept: Vec<Value> = before.iter().filter(|o| o["state"] == "removed" && !nodes.iter().any(|n| n["id"] == o["id"])).cloned().collect();
        nodes.extend(kept);
        nodes
    } else { before.clone() };
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
    map["nodes"] = json!(nodes);
    map["current"] = current.clone();
    validate(&map)?;
    // shortcut: every write counts as the session's own; a side worker's fill is not told apart yet.
    let source = if person { "person".to_string() } else { format!("session/{}", sid.unwrap_or("")) };
    let entries = store(&tx, &source, sid, &stamp, action, &before, &nodes)?;
    tx.commit()?;
    let Some(sid) = sid else { return Ok(json!({"nodes": view_nodes(&conn, &entries)?})) };
    let file = session_write(data, sid, &entries, &current, stamp["turn"].as_u64().unwrap_or(0), latest)?;
    view_of(&conn, &file, sid)
}

/// Writes what changed between two views of the same nodes into the store, with the events of it, and gives the
/// entries the map has afterwards. Only a field that differs between the views is written, so what a view leaves out
/// (a parent outside this map) stays as it is. A node gone from the view is only taken off this map.
fn store(conn: &rusqlite::Connection, source: &str, sid: Option<&str>, stamp: &Value, action: Option<&str>, before: &[Value], after: &[Value]) -> Result<Vec<(String, String)>> {
    use crate::db;
    let at = stamp["at"].as_i64().unwrap_or(0);
    // Each node's number: the one it has, or a new one.
    let nid_of: HashMap<String, String> = after.iter().map(|n| (n["id"].as_str().unwrap_or("").to_string(), n.get("nid").and_then(Value::as_str).map(String::from).unwrap_or_else(|| fresh("n")))).collect();
    let there = |id: &str| nid_of.get(id).cloned();
    let stored = |key: &str, v: &Value| -> Value {
        match (key, v.as_str()) {
            ("parent", Some(p)) => there(p).map(Value::String).unwrap_or(Value::Null),
            ("requires" | "assumes", Some(e)) => rename_ids(e, &there).map(Value::String).unwrap_or(Value::Null),
            _ => v.clone(),
        }
    };
    let links_in = |n: &Value| -> Vec<(String, String)> {
        n.get("links").and_then(Value::as_array).into_iter().flatten().filter_map(|l| Some((there(l.get("to")?.as_str()?)?, l.get("rel")?.as_str()?.to_string()))).collect()
    };
    let mut events = Vec::new();
    let mut tell = |kind: &str, nid: &str, memid: &Value, data: Value| {
        let mut e = event(source, kind, Some(nid), sid, at, data);
        e["memid"] = memid.clone();
        e["turn"] = stamp["turn"].clone();
        if let Some(a) = action { e["action"] = json!(a); }
        events.push(e);
    };
    const UNTOLD: [&str; 6] = ["id", "nid", "origin", "opened", "changed", "closed"];
    for n in after {
        let id = n["id"].as_str().unwrap_or("");
        let nid = &nid_of[id];
        let old = before.iter().find(|o| o["id"] == id);
        let keys: std::collections::BTreeSet<&String> = n.as_object().into_iter().flatten().map(|(k, _)| k)
            .chain(old.and_then(Value::as_object).into_iter().flatten().map(|(k, _)| k)).filter(|k| !["id", "nid", "links"].contains(&k.as_str())).collect();
        let was = match old { Some(_) => db::node_get(conn, nid)?.ok_or("a node of the map is not in the store")?, None => json!({"nid": nid, "origin": sid.unwrap_or("")}) };
        let mut row = was.clone();
        row.as_object_mut().unwrap().shift_remove("links");
        let mut told = serde_json::Map::new();
        for key in keys {
            if old.is_some_and(|o| o.get(key) == n.get(key)) { continue; }
            let new = n.get(key).map(|v| stored(key, v)).unwrap_or(Value::Null);
            let prior = was.get(key).cloned().unwrap_or(Value::Null);
            if new.is_null() { row.as_object_mut().unwrap().shift_remove(key); } else { row[key] = new.clone(); }
            if !UNTOLD.contains(&key.as_str()) && prior != new { told.insert(key.clone(), if old.is_some() { json!([prior, new]) } else { new }); }
        }
        if row != was || old.is_none() { db::node_put(conn, &row)?; }
        let memid = row.get("memid").cloned().unwrap_or(Value::Null);
        if old.is_none() { tell("node.added", nid, &memid, Value::Object(told)); } else if !told.is_empty() { tell("node.changed", nid, &memid, Value::Object(told)); }
        if old.is_none_or(|o| o.get("links") != n.get("links")) {
            let had = db::links_of(conn, nid)?;
            // A link to a node outside this map is not in the view: it stays.
            let shown: HashSet<&String> = nid_of.values().collect();
            let mut now: Vec<(String, String)> = had.iter().filter(|(to, _)| !shown.contains(to)).cloned().collect();
            for l in links_in(n) { if !now.contains(&l) { now.push(l); } }
            if now != had {
                db::links_set(conn, nid, &now)?;
                for (to, rel) in now.iter().filter(|l| !had.contains(l)) { tell("link.added", nid, &memid, json!({"to": to, "rel": rel})); }
                for (to, rel) in had.iter().filter(|l| !now.contains(l)) { tell("link.removed", nid, &memid, json!({"to": to, "rel": rel})); }
            }
        }
    }
    for o in before.iter().filter(|o| !after.iter().any(|n| n["id"] == o["id"])) {
        if let Some(nid) = o.get("nid").and_then(Value::as_str) { tell("session.unlinked", nid, o.get("memid").unwrap_or(&Value::Null), json!({})); }
    }
    for e in &events { db::event_add(conn, e)?; }
    Ok(after.iter().map(|n| { let id = n["id"].as_str().unwrap_or("").to_string(); let nid = nid_of[&id].clone(); (id, nid) }).collect())
}

fn map_nodes(data: &Path, sid: &str) -> Result<Vec<Value>> {
    Ok(view(data, sid)?.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default())
}

/// A side worker's fill for a map style: only kind, reason and links of nodes that exist, with known values.
/// shortcut: recorded as the session's own change, like before; a worker source is not told apart yet.
pub fn fill(data: &Path, sid: &str, items: &[Value]) -> Result<usize> {
    const KINDS: [&str; 12] = ["question", "option", "pro", "con", "conclusion", "task", "appointment", "problem", "idea", "fact", "rule", "note"];
    const RELS: [&str; 5] = ["cause", "leads", "replaces", "related", "resolves"];
    let nodes = map_nodes(data, sid)?;
    let ids: HashSet<&str> = nodes.iter().filter_map(|n| n["id"].as_str()).collect();
    let ups: Vec<Value> = items.iter().filter(|p| p["id"].as_str().is_some_and(|i| ids.contains(i))).map(|p| {
        let mut c = json!({"id": p["id"]});
        if let Some(k) = p["kind"].as_str().filter(|k| KINDS.contains(k)) { c["kind"] = json!(k); }
        if let Some(r) = p["reason"].as_str().map(str::trim).filter(|r| !r.is_empty()) { c["reason"] = json!(r.chars().take(80).collect::<String>()); }
        if let Some(links) = p["links"].as_array() {
            c["links"] = json!(links.iter().filter(|l| l["to"].as_str().is_some_and(|t| ids.contains(t)) && l["rel"].as_str().is_some_and(|r| RELS.contains(&r))).cloned().collect::<Vec<_>>());
        }
        c
    }).collect();
    if ups.is_empty() { return Ok(0); }
    change(data, sid, json!({"upsert": ups}), false)?;
    Ok(ups.len())
}

/// The person deletes a node (state `removed`, shown only in the notifications panel's deleted section): with its
/// whole branch, or alone, its children moving up to its parent. The events share one action number, the one
/// `restore` takes.
pub fn trash(data: &Path, nid: &str, branch: bool) -> Result<String> {
    let conn = crate::db::open(data)?;
    let node = crate::db::node_get(&conn, nid)?.ok_or("no such node")?;
    let mut scope = vec![nid.to_string()];
    let mut ups = vec![json!({"id": nid, "state": "removed"})];
    if branch {
        let mut i = 0;
        while i < scope.len() { for k in crate::db::children(&conn, &scope[i])? { if !scope.contains(&k) { scope.push(k); } } i += 1; }
        // What an earlier action deleted stays with that action.
        for k in &scope[1..] {
            if crate::db::node_get(&conn, k)?.is_some_and(|n| n["state"] != "removed") { ups.push(json!({"id": k, "state": "removed"})); }
        }
    } else {
        let up = node.get("parent").and_then(Value::as_str).unwrap_or("").to_string();
        for k in crate::db::children(&conn, nid)? { ups.push(json!({"id": k, "parent": up})); scope.push(k); }
        if !up.is_empty() { scope.push(up); }
    }
    drop(conn);
    let action = fresh("a");
    change_by(data, Scope::Nodes(scope), json!({"upsert": ups}), false, true, Some(&action))?;
    Ok(action)
}

/// Undoes one deletion: every node it changed gets its fields back, unless the node changed since; those are
/// left as they are and returned (node numbers).
pub fn restore(data: &Path, action: &str) -> Result<Vec<String>> {
    let conn = crate::db::open(data)?;
    let (mut ups, mut scope, mut kept) = (Vec::new(), Vec::new(), Vec::new());
    for e in crate::db::events(&conn, "action = ? AND type = 'node.changed'", &[action])? {
        let Some(nid) = e["nid"].as_str() else { continue };
        let Some(now) = crate::db::node_get(&conn, nid)? else { continue };
        let later = crate::db::events(&conn, "nid = ?", &[nid])?;
        let since = later.iter().skip_while(|l| l["id"] != e["id"]).any(|l| l.get("action").and_then(Value::as_str) != Some(action));
        let fields = e["data"].as_object().cloned().unwrap_or_default();
        if since || !fields.iter().all(|(k, v)| now.get(k).unwrap_or(&Value::Null) == &v[1]) { kept.push(nid.to_string()); continue; }
        let mut up = json!({"id": nid});
        for (k, v) in fields {
            if k == "parent" && let Some(p) = v[0].as_str() { scope.push(p.to_string()); }
            up[k] = if v[0].is_null() { json!("") } else { v[0].clone() };
        }
        scope.push(nid.to_string());
        ups.push(up);
    }
    drop(conn);
    scope.sort();
    scope.dedup();
    if !ups.is_empty() { change_by(data, Scope::Nodes(scope), json!({"upsert": ups}), false, true, Some(&fresh("a")))?; }
    Ok(kept)
}

/// Deletes nodes for good, the ones that are deleted (`removed`): the node, its links and its events. A node under
/// one of them that is not deleted moves to the top level. No event is left of it.
pub fn purge(data: &Path, nids: &[String]) -> Result<usize> {
    let mut conn = crate::db::open(data)?;
    let mut gone = Vec::new();
    for nid in nids { if crate::db::node_get(&conn, nid)?.is_some_and(|n| n["state"] == "removed") { gone.push(nid.clone()); } }
    crate::db::purge(&mut conn, &gone)
}

/// At start: nodes deleted longer ago than `trash_days` (counted from the deletion, its last change) in `settings.json` (180 when unset, 0 = only by hand) go for good.
pub fn empty_old_trash(data: &Path) -> usize {
    let days = read_json(&data.join("settings.json"), Value::Null).ok().and_then(|s| s["trash_days"].as_u64()).unwrap_or(180);
    if days == 0 || !data.join("memory").join("memory.db").exists() { return 0; }
    let before = now_ms().saturating_sub(days * 86_400_000).to_string();
    let old: Vec<String> = crate::db::open(data).and_then(|c| crate::db::nodes(&c, "state = 'removed' AND json_extract(changed, '$.at') < CAST(? AS INTEGER)", &[&before])).unwrap_or_default()
        .iter().filter_map(|n| n["nid"].as_str().map(String::from)).collect();
    purge(data, &old).unwrap_or(0)
}

/// `whaybench map SESSION --end-turn`: one more finished turn, so the stamps of the next changes say in
/// which turn they were made. `latest.json` is rewritten only while it shows this session.
pub fn end_turn(data: &Path, sid: &str) -> Result<Value> {
    let session = session_read(data, sid)?;
    let latest = read_json(&data.join("latest.json"), Value::Null)?["sessionId"] == sid;
    session_write(data, sid, &entries_of(&session), &session["current"], session.get("turn").and_then(Value::as_u64).unwrap_or(0) + 1, latest)
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

    fn get(dir: &Path, sid: &str, id: &str) -> Option<Value> { map_nodes(dir, sid).unwrap().into_iter().find(|n| n["id"] == id) }
    fn nid(dir: &Path, sid: &str, id: &str) -> String { get(dir, sid, id).unwrap()["nid"].as_str().unwrap().to_string() }

    #[test]
    fn a_fill_takes_only_known_fields_of_existing_nodes() {
        let dir = tempfile::tempdir().unwrap();
        command(dir.path(), "s", Some(json!({"upsert": [{"id": "a", "label": "A"}, {"id": "b", "label": "B"}]}))).unwrap();
        let n = fill(dir.path(), "s", &[json!({"id": "a", "kind": "task", "label": "renamed", "reason": "r", "links": [{"to": "b", "rel": "leads"}, {"to": "zz", "rel": "leads"}, {"to": "b", "rel": "blocks"}]}),
            json!({"id": "gone", "kind": "task"}), json!({"id": "b", "kind": "nonsense"})]).unwrap();
        let a = get(dir.path(), "s", "a").unwrap();
        assert_eq!((n, a["kind"].as_str(), a["label"].as_str(), a["links"].as_array().unwrap().len()), (2, Some("task"), Some("A"), 1));
    }

    #[test]
    fn writers_at_the_same_time_lose_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        command(&root, "s", Some(json!({"upsert": [{"id": "shared", "label": "x"}]}))).unwrap();
        let shared = nid(&root, "s", "shared");
        let threads: Vec<_> = (0..8).map(|t| { let (root, shared) = (root.clone(), shared.clone()); std::thread::spawn(move || {
            for i in 0..10 {
                if t % 2 == 0 { command(&root, "s", Some(json!({"upsert": [{"id": format!("n{t}-{i}"), "label": "x"}]}))).unwrap(); }
                else { person(&root, json!([{"id": shared, "detail": format!("{t}-{i}")}])).unwrap(); }
            }
        }) }).collect();
        for h in threads { h.join().unwrap(); }
        assert_eq!(map_nodes(&root, "s").unwrap().len(), 41, "the session's writes all stay while the person writes too");
        assert_eq!(crate::db::events(&crate::db::open(&root).unwrap(), "nid = ? AND source = 'person'", &[&shared]).unwrap().len(), 40, "and each of the person's is told");
    }

    #[test]
    fn a_node_is_stored_once_and_shown_by_every_session_that_links_it() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let old = command(dir.path(), "old", Some(json!({"upsert": [{"id":"t","label":"topic"},{"id":"a","parent":"t","label":"task","state":"plan","requires":"t"},
            {"id":"d","label":"decision","links":[{"to":"a","rel":"related"}]}], "current": "a"})))?;
        let file: Value = serde_json::from_slice(&fs::read(dir.path().join("old.json"))?)?;
        assert_eq!(file["nodes"][1], json!({"id": "a", "nid": nid(dir.path(), "old", "a")}), "the map file names nodes, it holds none: {file}");
        assert!(file["nodes"][1].get("label").is_none());
        // Another session takes the map over: the same nodes, not copies.
        let new = take_over(dir.path(), "new", &old)?;
        assert_eq!((new["nodes"].clone(), new["current"].as_str(), new["sessionId"].as_str()), (old["nodes"].clone(), Some("a"), Some("new")));
        assert_eq!(read_json(&dir.path().join("latest.json"), Value::Null)?["sessionId"], "old", "taking over does not take the panel");
        // Changed in one, seen in the other at once.
        command(dir.path(), "new", Some(json!({"upsert": [{"id":"a","state":"done"}]})))?;
        assert_eq!(get(dir.path(), "old", "a").unwrap()["state"], "done");
        // Taken off one map: still on the other, with its parent and what points at it.
        command(dir.path(), "new", Some(json!({"remove": ["t"]})))?;
        assert_eq!(map_nodes(dir.path(), "new")?.iter().map(|n| n["id"].as_str().unwrap()).collect::<Vec<_>>(), ["d"], "the branch goes off this map");
        assert!(get(dir.path(), "new", "d").unwrap().get("links").is_none(), "a link to a node off the map is not shown");
        let a = get(dir.path(), "old", "a").unwrap();
        assert_eq!((a["parent"].as_str(), a["requires"].as_str()), (Some("t"), Some("t")));
        assert_eq!(get(dir.path(), "old", "d").unwrap()["links"], json!([{"to": "a", "rel": "related"}]), "the hidden link was kept");
        // A change on the map that does not show the link leaves it alone.
        command(dir.path(), "new", Some(json!({"upsert": [{"id":"d","label":"the decision","links":[]}]})))?;
        assert_eq!(get(dir.path(), "old", "d").unwrap()["links"], json!([{"to": "a", "rel": "related"}]));
        assert_eq!(get(dir.path(), "old", "d").unwrap()["label"], "the decision");
        // A map file from elsewhere has no node of the store: its nodes are made new, and start over.
        command(dir.path(), "other", Some(json!({"nodes":[]})))?;
        let m = start_from(dir.path(), "fresh", &json!({"nodes":[{"id":"x","label":"imported","opened":{"at":1,"turn":9}},{"id":"y","parent":"x","label":"under"}], "current": "y", "turn": 7}))?;
        assert_eq!((m["nodes"][0]["opened"]["turn"].as_u64(), m["nodes"][1]["parent"].as_str(), m["turn"].as_u64(), m["current"].is_null()), (Some(0), Some("x"), Some(0), true));
        assert_eq!(read_json(&dir.path().join("latest.json"), Value::Null)?["sessionId"], "other");
        assert!(take_over(dir.path(), "new", &json!({"nodes":[],"current":"gone"})).is_err());
        Ok(())
    }

    #[test]
    fn a_saved_map_names_its_nodes_and_shows_them_as_they_are_now() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let m = command(dir.path(), "s", Some(json!({"upsert": [{"id":"t","label":"top"},{"id":"a","parent":"t","label":"first name"}]})))?;
        let mut saved = saved_nodes(m["nodes"].as_array().unwrap());
        assert_eq!(saved[1], json!({"id": "a", "nid": m["nodes"][1]["nid"]}), "no content in the file");
        saved.push(json!({"id": "x", "label": "from elsewhere", "parent": "t"}));
        saved.push(json!({"id": "gone", "nid": "no-such-node"}));
        let path = dir.path().join("saved.json");
        fs::write(&path, json!({"title": "T", "from": "s", "nodes": saved}).to_string())?;
        command(dir.path(), "s", Some(json!({"upsert": [{"id":"a","label":"renamed since"}]})))?;
        let v = view_file(dir.path(), &path)?;
        let got: Vec<(&str, &str, Option<&str>)> = v["nodes"].as_array().unwrap().iter().map(|n| (n["id"].as_str().unwrap(), n["label"].as_str().unwrap(), n["parent"].as_str())).collect();
        assert_eq!(got, [("t", "top", None), ("a", "renamed since", Some("t")), ("x", "from elsewhere", Some("t"))]);
        assert_eq!(v["title"], "T");
        // A new session from it shows the same nodes; the one from elsewhere is made new.
        let new = start_from(dir.path(), "new", &v)?;
        assert_eq!(new["nodes"][1]["nid"], m["nodes"][1]["nid"]);
        assert_eq!(new["nodes"].as_array().unwrap().len(), 3);
        Ok(())
    }

    #[test]
    fn a_node_off_a_map_is_put_back_by_the_person() -> Result<()> {
        let dir = tempfile::tempdir()?;
        command(dir.path(), "s", Some(json!({"upsert": [{"id":"t","label":"top"},{"id":"a","parent":"t","label":"a"},{"id":"b","parent":"a","label":"b"}], "current": "a"})))?;
        let (a, t) = (nid(dir.path(), "s", "a"), nid(dir.path(), "s", "t"));
        assert_eq!(below(dir.path(), &t), 2);
        // Taken off the map by the session (with what is under it there); the nodes keep their place in the memory.
        let m = command(dir.path(), "s", Some(json!({"remove": ["a"]})))?;
        assert_eq!((m["nodes"].as_array().unwrap().len(), m["current"].is_null()), (1, true));
        // Back on the map: under its parent again, and what was under it too.
        let id = link(dir.path(), "s", &a)?;
        assert_eq!(get(dir.path(), "s", &id).unwrap()["parent"].as_str(), Some("t"));
        assert!(link(dir.path(), "s", &a).is_err() && link(dir.path(), "s", "no-such").is_err());
        let kinds: Vec<String> = crate::db::events(&crate::db::open(dir.path())?, "nid = ?", &[&a])?.iter().map(|e| e["type"].as_str().unwrap().to_string()).collect();
        assert_eq!(kinds, ["node.added", "session.unlinked", "session.linked"]);
        assert_eq!(below(dir.path(), &t), 2, "the memory still has them as they were");
        Ok(())
    }

    #[test]
    fn a_deletion_is_one_action_and_can_be_undone() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let sid = "aaaaaaaa-0000-0000-0000-000000000001";
        command(dir.path(), sid, Some(json!({"upsert": [{"id":"t","label":"top","isOpen":true},{"id":"a","parent":"t","label":"a","isOpen":true},
            {"id":"b","parent":"a","label":"b","isOpen":false},{"id":"c","parent":"a","label":"c","isOpen":true}]})))?;
        let get = |id: &str| get(dir.path(), sid, id);
        let n = |id: &str| nid(dir.path(), sid, id);
        let (a, b, c, t) = (n("a"), n("b"), n("c"), n("t"));
        // Alone: the children move up, and come back under it.
        let act = trash(dir.path(), &a, false)?;
        assert_eq!((get("a").unwrap()["state"].as_str(), get("b").unwrap()["parent"].as_str()), (Some("removed"), Some("t")));
        assert!(restore(dir.path(), &act)?.is_empty());
        assert_eq!((get("a").unwrap().get("state"), get("a").unwrap().get("isOpen"), get("b").unwrap()["parent"].as_str()), (None, None, Some("a")));
        assert_eq!(get("b").unwrap()["state"], "done", "a closed child keeps its own state");
        // The branch: all of it, and a node changed since stays as it is.
        let act = trash(dir.path(), &a, true)?;
        assert!(["a", "b", "c"].iter().all(|i| get(i).unwrap()["state"] == "removed"));
        let regrouped = command(dir.path(), sid, Some(json!({"nodes": [{"id":"t","label":"top","isOpen":true}]})))?;
        assert_eq!(regrouped["nodes"].as_array().unwrap().len(), 4, "a regrouping leaves the deleted nodes alone");
        assert_eq!(n("t"), t, "and a node it names again is the same node");
        person(dir.path(), json!([{"id": c, "label": "renamed"}]))?;
        assert_eq!(restore(dir.path(), &act)?, vec![c.clone()]);
        assert_eq!((is_open(&get("a").unwrap()), get("c").unwrap()["state"].as_str()), (true, Some("removed")));
        // For good: only deleted nodes go, with their events; what is under one and not deleted moves to the top level.
        trash(dir.path(), &a, false)?;
        assert_eq!(purge(dir.path(), &[a.clone(), c.clone(), t.clone()])?, 2, "only deleted nodes go");
        assert!(get("a").is_none() && get("c").is_none() && get("t").is_some());
        let conn = crate::db::open(dir.path())?;
        assert!(crate::db::events(&conn, "nid = ?", &[&a])?.is_empty(), "no event is left of a node deleted for good");
        assert_eq!(get("b").unwrap()["parent"], "t");
        let file: Value = serde_json::from_slice(&fs::read(dir.path().join(format!("{sid}.json")))?)?;
        assert_eq!(file["nodes"].as_array().unwrap().len(), 4, "the map file still names them; the view skips what is gone");
        command(dir.path(), sid, Some(json!({"upsert": [{"id":"t","detail":"written after"}]})))?;
        // Deleted long ago goes at start; 0 days keeps it.
        trash(dir.path(), &b, false)?;
        conn.execute("UPDATE nodes SET changed = json_set(changed, '$.at', 1) WHERE nid = ?", [&b])?;
        fs::write(dir.path().join("settings.json"), r#"{"trash_days":0}"#)?;
        assert_eq!(empty_old_trash(dir.path()), 0);
        fs::remove_file(dir.path().join("settings.json"))?;
        assert_eq!(empty_old_trash(dir.path()), 1);
        assert!(get("b").is_none());
        // Closed long ago, deleted now: it waits its 180 days like any other.
        command(dir.path(), sid, Some(json!({"upsert": [{"id":"old","label":"old","state":"done"}]})))?;
        conn.execute("UPDATE nodes SET closed = json_set(closed, '$.at', 1) WHERE nid = ?", [n("old")])?;
        trash(dir.path(), &n("old"), false)?;
        assert_eq!(empty_old_trash(dir.path()), 0);
        Ok(())
    }

    #[test]
    fn open_follows_the_state_and_the_mark_is_the_persons() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let up = |c: Value| command(dir.path(), "s", Some(json!({"upsert": [c]})));
        let map = up(json!({"id":"q","label":"question","isOpen":true,"mark":"important","nid":"mine","origin":"elsewhere"}))?;
        assert!(map["nodes"][0].get("mark").is_none(), "an agent cannot set the mark");
        assert!(map["nodes"][0]["nid"] != "mine" && map["nodes"][0]["origin"] == "s", "nor what the program gives a node");
        assert!(map["nodes"][0]["memid"].as_str().is_some_and(|m| !m.is_empty()), "a new node starts its own memory id");
        let map = up(json!({"id":"q","isOpen":false}))?;
        assert_eq!((map["nodes"][0]["state"].as_str(), map["nodes"][0].get("isOpen")), (Some("done"), None), "an older isOpen is read as the state and not kept");
        let map = up(json!({"id":"q","isOpen":true}))?;
        assert!(map["nodes"][0].get("state").is_none(), "reopening drops the closing state");
        let map = up(json!({"id":"q","state":"dropped"}))?;
        assert!(!is_open(&map["nodes"][0]));
        let map = up(json!({"id":"q","state":"postponed"}))?;
        assert!(is_open(&map["nodes"][0]));
        let map = up(json!({"id":"q","state":""}))?;
        assert!(map["nodes"][0].get("state").is_none(), "an empty state opens with no state");
        assert!(up(json!({"id":"q","suggest":{"state":"removed","why":"x"}})).is_err(), "only done or dropped can be suggested");
        let map = up(json!({"id":"q","suggest":{"state":"dropped","why":"settled elsewhere"}}))?;
        assert!(is_open(&map["nodes"][0]) && map["nodes"][0]["suggest"]["state"] == "dropped", "a suggestion leaves the node open");
        let q = nid(dir.path(), "s", "q");
        let map = person(dir.path(), json!([{"id": q, "state":"dropped","suggest":null}]))?;
        assert!(!is_open(&map["nodes"][0]) && map["nodes"][0].get("suggest").is_none(), "approved: the state is the person's, the suggestion goes");
        up(json!({"id":"q","state":""}))?;
        let map = command(dir.path(), "s", Some(json!({"upsert": [{"id":"o","label":"option","isOpen":true,"parent":"q"},
            {"id":"d","label":"decided","isOpen":false,"links":[{"to":"q","rel":"resolves"}]}]})))?;
        let got = |id: &str| map["nodes"].as_array().unwrap().iter().find(|n| n["id"] == id).cloned().unwrap();
        assert_eq!((got("q")["state"].as_str(), is_open(&got("o"))), (Some("done"), false), "resolves settles the problem and what is open under it");
        let o = nid(dir.path(), "s", "o");
        person(dir.path(), json!([{"id": o, "mark":"important"}]))?;
        assert_eq!(get(dir.path(), "s", "o").unwrap()["mark"], json!("important"), "the person may mark");
        assert!(person(dir.path(), json!([{"id": "no-such-node", "state": "done"}])).is_err(), "a node that is gone is not made anew");
        assert_eq!(crate::db::nodes(&crate::db::open(dir.path())?, "label = 'no-such-node'", &[])?.len(), 0);
        command(dir.path(), "s", Some(json!({"nodes": [{"id":"q","label":"question"},{"id":"o","label":"option","parent":"q"},{"id":"d","label":"decided","state":"done"}]})))?;
        assert_eq!(get(dir.path(), "s", "o").unwrap()["mark"], json!("important"), "a regrouping leaves the person's mark");
        person(dir.path(), json!([{"id": o, "mark":null}]))?;
        assert!(get(dir.path(), "s", "o").unwrap().get("mark").is_none(), "and take it off");
        assert_eq!(get(dir.path(), "s", "o").unwrap()["parent"], "q", "the person's change leaves the rest as it was");
        let memid = got("q")["memid"].clone();
        command(dir.path(), "s", Some(json!({"nodes": [{"id":"q","label":"regrouped","isOpen":true}]})))?;
        assert_eq!(get(dir.path(), "s", "q").unwrap()["memid"], memid, "a regrouping keeps the memory id");
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
        command(dir.path(), "s", Some(json!({"upsert": [{"id":"b","label":"B","isOpen":true,"requires":"¬(a)","due":"2026-12-01"}]})))?;
        let b = get(dir.path(), "s", "b").unwrap();
        assert_eq!(condition_ids(b["requires"].as_str().unwrap())?, ["a"], "the condition comes back naming this map's ids: {b}");
        Ok(())
    }

    #[test]
    fn invalid_map_cannot_replace_a_valid_map() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let initial = json!({"nodes":[{"id":"n1","label":"topic"}],"current":"n1"});
        command(dir.path(), "session1", Some(initial))?;
        let path = dir.path().join("session1.json");
        let before = fs::read(&path)?;
        let stored = || crate::db::nodes(&crate::db::open(dir.path()).unwrap(), "1", &[]).unwrap();
        let nodes_before = stored();
        for map in [
            json!({"nodes":[{"id":"n","label":"a"},{"id":"n","label":"b"}]}),
            json!({"nodes":[],"current":"missing"}),
            json!({"nodes":[{"id":"n","label":"a","parent":"missing"}]}),
            json!({"nodes":[{"id":"a","label":"a","parent":"b"},{"id":"b","label":"b","parent":"a"}]}),
            json!({"nodes":[{"id":"n","label":5}]}),
        ] {
            assert!(command(dir.path(), "session1", Some(map)).is_err());
            assert_eq!(fs::read(&path)?, before);
            assert_eq!(stored(), nodes_before, "nothing reaches the store either");
        }
        assert_eq!(command(dir.path(), "session1", None)?["nodes"][0]["label"], "topic");
        assert_eq!(command(dir.path(), "no-such-session", None)?, json!({}), "a session without a map file has no map");
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
        assert_eq!(command(dir.path(), "s1", None)?["turn"], 2);
        Ok(())
    }

    #[test]
    fn ids_cannot_leave_the_data_directory() {
        for id in ["../original", "/tmp/file", "", "a/b", "a.b"] { assert!(safe_id(id).is_err()); }
    }
}
