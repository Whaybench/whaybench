//! The memory's store: one SQLite file, `memory/memory.db`. A node is stored once, as it is now (`nodes`); what
//! ties two nodes is in `node_links`; how a node came to be what it is, in time order, is in `events`.
//! A session keeps no node: its map file lists the nodes it shows.
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde_json::{Map, Value, json};

use crate::map::Result;

/// The schema's number, kept in the file (`user_version`): a later program reads it to bring an older file up to date.
const VERSION: i64 = 1;

/// Node fields stored as text.
const TEXT: [&str; 14] = ["nid", "memid", "label", "detail", "kind", "state", "parent", "at", "due", "requires", "assumes", "mark", "reason", "origin"];
/// Node fields stored as JSON: objects, and the program's stamps `{at, turn}`.
const JSON: [&str; 5] = ["suggest", "source", "opened", "changed", "closed"];

const SCHEMA: &str = "
CREATE TABLE nodes (
  nid TEXT PRIMARY KEY, memid TEXT NOT NULL, label TEXT NOT NULL, detail TEXT, kind TEXT, state TEXT, parent TEXT,
  at TEXT, due TEXT, requires TEXT, assumes TEXT, mark TEXT, reason TEXT, origin TEXT,
  suggest TEXT, source TEXT, opened TEXT, changed TEXT, closed TEXT);
CREATE INDEX nodes_memid ON nodes(memid);
CREATE INDEX nodes_state ON nodes(state);
CREATE INDEX nodes_parent ON nodes(parent);
CREATE INDEX nodes_due ON nodes(due) WHERE due IS NOT NULL;
CREATE INDEX nodes_at ON nodes(at) WHERE at IS NOT NULL;
CREATE INDEX nodes_changed ON nodes(coalesce(json_extract(changed, '$.at'), json_extract(opened, '$.at')));
CREATE TABLE node_links (from_nid TEXT NOT NULL, to_nid TEXT NOT NULL, rel TEXT NOT NULL, PRIMARY KEY (from_nid, to_nid, rel));
CREATE INDEX node_links_to ON node_links(to_nid);
CREATE TABLE events (
  seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE, source TEXT NOT NULL, type TEXT NOT NULL, time TEXT NOT NULL,
  nid TEXT, session TEXT, memid TEXT, turn INTEGER, action TEXT, path TEXT, data TEXT);
CREATE INDEX events_nid ON events(nid);
CREATE INDEX events_session ON events(session);
CREATE INDEX events_memid ON events(memid);
CREATE INDEX events_path ON events(path) WHERE path IS NOT NULL;
CREATE INDEX events_action ON events(action) WHERE action IS NOT NULL;
CREATE INDEX events_time ON events(time);
-- Search over names and details: three letters at a time, so a word is found inside a longer one (Korean particles).
CREATE VIRTUAL TABLE nodes_fts USING fts5(label, detail, content='nodes', tokenize='trigram');
CREATE TRIGGER nodes_fts_ai AFTER INSERT ON nodes BEGIN
  INSERT INTO nodes_fts(rowid, label, detail) VALUES (new.rowid, new.label, new.detail); END;
CREATE TRIGGER nodes_fts_ad AFTER DELETE ON nodes BEGIN
  INSERT INTO nodes_fts(nodes_fts, rowid, label, detail) VALUES ('delete', old.rowid, old.label, old.detail); END;
CREATE TRIGGER nodes_fts_au AFTER UPDATE ON nodes BEGIN
  INSERT INTO nodes_fts(nodes_fts, rowid, label, detail) VALUES ('delete', old.rowid, old.label, old.detail);
  INSERT INTO nodes_fts(rowid, label, detail) VALUES (new.rowid, new.label, new.detail); END;
";

/// Opens the store, making it on first use. Several programs hold it open at once (WAL): they read together and
/// write in turn, a writer waiting up to five seconds for another.
pub fn open(data: &Path) -> Result<Connection> {
    // Programs that start together on a new file all try to set it up; SQLite answers "busy" to the ones that lose
    // without waiting (changing the journal mode does not wait for a lock), so they try again.
    let started = std::time::Instant::now();
    loop {
        match open_once(data) {
            Err(e) if started.elapsed() < std::time::Duration::from_secs(5)
                && e.downcast_ref::<rusqlite::Error>().is_some_and(|e| e.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy)) => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            other => return other,
        }
    }
}

fn open_once(data: &Path) -> Result<Connection> {
    let dir = data.join("memory");
    std::fs::create_dir_all(&dir)?;
    let mut conn = Connection::open(dir.join("memory.db"))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    let version = |c: &Connection| c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0));
    let mut found = version(&conn)?;
    // Only a new file takes the write lock: a program that only reads does not make the others wait.
    if found == 0 {
        // Two programs may both find a new file: only the one that gets the lock first makes the tables.
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if version(&tx)? == 0 {
            tx.execute_batch(SCHEMA)?;
            tx.pragma_update(None, "user_version", VERSION)?;
        }
        tx.commit()?;
        found = version(&conn)?;
    }
    if found > VERSION {
        return Err(format!("memory.db was written by a newer Whaybench (schema {found}, this program knows {VERSION})").into());
    }
    Ok(conn)
}

fn column(key: &str, v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) if !JSON.contains(&key) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// Writes a node as given: the whole row, a field left out becomes empty. `nid`, `memid` and `label` are required.
pub fn node_put(conn: &Connection, node: &Value) -> Result<()> {
    for key in ["nid", "memid", "label"] {
        if node.get(key).and_then(Value::as_str).is_none_or(str::is_empty) { return Err(format!("a node requires {key}").into()); }
    }
    let cols: Vec<&str> = TEXT.iter().chain(JSON.iter()).copied().collect();
    let values: Vec<Option<String>> = cols.iter().map(|k| node.get(*k).and_then(|v| column(k, v))).collect();
    // An upsert, not a replace: the row keeps its rowid, which the search index follows.
    let sql = format!("INSERT INTO nodes ({}) VALUES ({}) ON CONFLICT(nid) DO UPDATE SET {}",
        cols.join(", "), vec!["?"; cols.len()].join(", "),
        cols.iter().skip(1).map(|c| format!("{c} = excluded.{c}")).collect::<Vec<_>>().join(", "));
    conn.execute(&sql, params_from_iter(values))?;
    Ok(())
}

fn node_from(row: &rusqlite::Row) -> rusqlite::Result<Value> {
    let mut out = Map::new();
    for (i, key) in TEXT.iter().chain(JSON.iter()).enumerate() {
        let Some(text) = row.get::<_, Option<String>>(i)? else { continue };
        let value = if JSON.contains(key) { serde_json::from_str(&text).unwrap_or(Value::Null) } else { Value::String(text) };
        if !value.is_null() { out.insert((*key).to_string(), value); }
    }
    Ok(Value::Object(out))
}

fn node_columns() -> String { TEXT.iter().chain(JSON.iter()).copied().collect::<Vec<_>>().join(", ") }

/// One node with its links (`links: [{to, rel}]`, left out when it has none).
pub fn node_get(conn: &Connection, nid: &str) -> Result<Option<Value>> {
    let found = conn.query_row(&format!("SELECT {} FROM nodes WHERE nid = ?", node_columns()), [nid], node_from).optional()?;
    let Some(mut node) = found else { return Ok(None) };
    let links = links_of(conn, nid)?;
    if !links.is_empty() { node["links"] = json!(links.iter().map(|(to, rel)| json!({"to": to, "rel": rel})).collect::<Vec<_>>()); }
    Ok(Some(node))
}

/// What a node points at: `(to, rel)`.
pub fn links_of(conn: &Connection, nid: &str) -> Result<Vec<(String, String)>> {
    let mut st = conn.prepare("SELECT to_nid, rel FROM node_links WHERE from_nid = ? ORDER BY rowid")?;
    let rows = st.query_map([nid], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// A node's links become exactly these.
pub fn links_set(conn: &Connection, nid: &str, links: &[(String, String)]) -> Result<()> {
    conn.execute("DELETE FROM node_links WHERE from_nid = ?", [nid])?;
    for (to, rel) in links {
        conn.execute("INSERT OR IGNORE INTO node_links (from_nid, to_nid, rel) VALUES (?, ?, ?)", params![nid, to, rel])?;
    }
    Ok(())
}

/// Adds one event (the shape `memory::event` makes, with `nid` and `session` in place of `subject`). An event
/// already in the store, by its id, is not added again.
pub fn event_add(conn: &Connection, e: &Value) -> Result<()> {
    let text = |k: &str| e.get(k).and_then(Value::as_str).map(String::from);
    conn.execute(
        "INSERT OR IGNORE INTO events (id, source, type, time, nid, session, memid, turn, action, path, data) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![text("id").ok_or("an event requires id")?, text("source").ok_or("an event requires source")?,
            text("type").ok_or("an event requires type")?, text("time").ok_or("an event requires time")?,
            text("nid"), text("session"), text("memid"), e.get("turn").and_then(Value::as_i64), text("action"), text("path"),
            e.get("data").filter(|d| !d.is_null()).map(Value::to_string)],
    )?;
    Ok(())
}

fn event_from(row: &rusqlite::Row) -> rusqlite::Result<Value> {
    let mut out = json!({"specversion": "1.0"});
    for (i, key) in ["id", "source", "type", "time", "nid", "session", "memid", "action", "path"].iter().enumerate() {
        if let Some(v) = row.get::<_, Option<String>>(i)? { out[*key] = json!(v); }
    }
    if let Some(turn) = row.get::<_, Option<i64>>(9)? { out["turn"] = json!(turn); }
    if let Some(data) = row.get::<_, Option<String>>(10)? { out["data"] = serde_json::from_str(&data).unwrap_or(Value::Null); }
    Ok(out)
}

/// Events in the order they were added, the ones `filter` picks: a SQL condition over the event columns with its values
/// (`"nid = ?"`, `"1"` for all).
pub fn events(conn: &Connection, filter: &str, values: &[&str]) -> Result<Vec<Value>> {
    let mut st = conn.prepare(&format!("SELECT id, source, type, time, nid, session, memid, action, path, turn, data FROM events WHERE {filter} ORDER BY seq"))?;
    let rows = st.query_map(params_from_iter(values), event_from)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Drops the events before a time (RFC 3339, as stored): the keeping limit. The nodes stay as they are.
pub fn events_drop_before(conn: &Connection, time: &str) -> Result<usize> {
    Ok(conn.execute("DELETE FROM events WHERE time < ?", [time])?)
}

/// A mark that differs whenever events were added or dropped: the last one's number and how many there are.
pub fn events_mark(conn: &Connection) -> Result<String> {
    Ok(conn.query_row("SELECT coalesce(max(seq), 0) || ':' || count(*) FROM events", [], |r| r.get(0))?)
}

/// Nodes a SQL condition over the node columns picks, with its values (`"memid = ?"`), without their links.
pub fn nodes(conn: &Connection, filter: &str, values: &[&str]) -> Result<Vec<Value>> {
    let mut st = conn.prepare(&format!("SELECT {} FROM nodes WHERE {filter}", node_columns()))?;
    let rows = st.query_map(params_from_iter(values), node_from)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// The nodes directly under a node.
pub fn children(conn: &Connection, nid: &str) -> Result<Vec<String>> {
    let mut st = conn.prepare("SELECT nid FROM nodes WHERE parent = ? ORDER BY rowid")?;
    let rows = st.query_map([nid], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Deletes nodes for good: their rows, every link from or to them, and all their events, together or not at all.
/// A node left under one of them moves to the top level. Returns how many nodes there were.
pub fn purge(conn: &mut Connection, nids: &[String]) -> Result<usize> {
    let tx = conn.transaction()?;
    let mut gone = 0;
    for nid in nids {
        gone += tx.execute("DELETE FROM nodes WHERE nid = ?", [nid])?;
        tx.execute("UPDATE nodes SET parent = NULL WHERE parent = ?", [nid])?;
        tx.execute("DELETE FROM node_links WHERE from_nid = ?1 OR to_nid = ?1", [nid])?;
        tx.execute("DELETE FROM events WHERE nid = ?", [nid])?;
    }
    tx.commit()?;
    Ok(gone)
}

/// Nodes whose name or detail holds any of the words (any case), most recently changed first. A word of three
/// letters or more is looked up in the search index; a shorter one is matched letter by letter.
pub fn search(conn: &Connection, words: &[String], limit: usize) -> Result<Vec<Value>> {
    let words: Vec<String> = words.iter().map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).collect();
    if words.is_empty() { return Ok(Vec::new()); }
    let (long, short): (Vec<&String>, Vec<&String>) = words.iter().partition(|w| w.chars().count() >= 3);
    let mut conds = Vec::new();
    let mut values: Vec<String> = Vec::new();
    if !long.is_empty() {
        conds.push("rowid IN (SELECT rowid FROM nodes_fts WHERE nodes_fts MATCH ?)".to_string());
        values.push(long.iter().map(|w| format!("\"{}\"", w.replace('"', "\"\""))).collect::<Vec<_>>().join(" OR "));
    }
    for w in short {
        conds.push("(instr(lower(label), ?) > 0 OR instr(lower(coalesce(detail, '')), ?) > 0)".to_string());
        values.extend([w.clone(), w.clone()]);
    }
    let sql = format!("SELECT {} FROM nodes WHERE {} ORDER BY coalesce(json_extract(changed, '$.at'), json_extract(opened, '$.at')) DESC LIMIT {limit}",
        node_columns(), conds.join(" OR "));
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map(params_from_iter(values), node_from)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(nid: &str, label: &str, at: i64) -> Value {
        json!({"nid": nid, "memid": format!("m-{nid}"), "label": label, "opened": {"at": at, "turn": 1}})
    }

    fn event(id: &str, kind: &str, nid: &str) -> Value {
        json!({"id": id, "source": "session/s1", "type": kind, "time": "2026-11-13T00:00:00+00:00", "nid": nid, "session": "s1", "turn": 2,
            "data": {"state": [null, "done"]}})
    }

    #[test]
    fn a_node_comes_back_as_it_was_written() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let conn = open(dir.path())?;
        let n = json!({"nid": "n1", "memid": "m1", "label": "Shortcut settings", "detail": "with a clash check", "kind": "task", "state": "plan",
            "parent": "n0", "due": "2026-12-01", "mark": "important", "reason": "came up while testing", "origin": "s1",
            "suggest": {"state": "done", "why": "merged"}, "source": {"file": "maps/a.json", "node": "x"},
            "opened": {"at": 5, "turn": 1}});
        node_put(&conn, &n)?;
        assert_eq!(node_get(&conn, "n1")?, Some(n.clone()));
        // Written again with fields left out: they are gone, and the row is still one.
        node_put(&conn, &json!({"nid": "n1", "memid": "m1", "label": "Shortcuts", "state": "done"}))?;
        assert_eq!(node_get(&conn, "n1")?, Some(json!({"nid": "n1", "memid": "m1", "label": "Shortcuts", "state": "done"})));
        assert_eq!(conn.query_row("SELECT count(*) FROM nodes", [], |r| r.get::<_, i64>(0))?, 1);
        assert!(node_put(&conn, &json!({"nid": "n2", "label": "no memory id"})).is_err());
        assert_eq!(node_get(&conn, "missing")?, None);
        Ok(())
    }

    #[test]
    fn links_and_events_follow_the_node_and_go_with_it() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut conn = open(dir.path())?;
        for (nid, label) in [("a", "A"), ("b", "B"), ("c", "C")] { node_put(&conn, &node(nid, label, 1))?; }
        links_set(&conn, "a", &[("b".into(), "resolves".into()), ("c".into(), "related".into())])?;
        links_set(&conn, "c", &[("b".into(), "cause".into())])?;
        assert_eq!(node_get(&conn, "a")?.unwrap()["links"], json!([{"to": "b", "rel": "resolves"}, {"to": "c", "rel": "related"}]));
        links_set(&conn, "a", &[("b".into(), "resolves".into())])?;
        assert_eq!(links_of(&conn, "a")?.len(), 1);

        event_add(&conn, &event("e1", "node.added", "a"))?;
        event_add(&conn, &event("e2", "node.changed", "b"))?;
        event_add(&conn, &event("e2", "node.changed", "b"))?;
        let of_b = events(&conn, "nid = ?", &["b"])?;
        assert_eq!(of_b.len(), 1, "the same event is kept once");
        assert_eq!((of_b[0]["type"].as_str(), of_b[0]["turn"].as_i64(), &of_b[0]["data"]["state"][1]), (Some("node.changed"), Some(2), &json!("done")));

        // Deleting b for good takes its row, its events and every link that touched it; a and c stay.
        assert_eq!(purge(&mut conn, &["b".into(), "missing".into()])?, 1);
        assert_eq!(node_get(&conn, "b")?, None);
        assert!(events(&conn, "nid = ?", &["b"])?.is_empty());
        assert!(links_of(&conn, "a")?.is_empty() && links_of(&conn, "c")?.is_empty());
        assert_eq!(events(&conn, "1", &[])?.len(), 1);
        // The keeping limit drops old events and leaves the nodes.
        let mark = events_mark(&conn)?;
        assert_eq!(events_drop_before(&conn, "2026-11-12T00:00:00+00:00")?, 0);
        assert_eq!(events_drop_before(&conn, "2026-11-14T00:00:00+00:00")?, 1);
        assert!(events_mark(&conn)? != mark && node_get(&conn, "a")?.is_some());
        Ok(())
    }

    #[test]
    fn search_finds_words_inside_names_and_details() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut conn = open(dir.path())?;
        // Letters of more than one byte too: a word is counted in letters, not bytes.
        node_put(&conn, &json!({"nid": "k1", "memid": "m", "label": "Shortcut settings for the caf\u{e9}", "detail": "with a clash check", "opened": {"at": 1}}))?;
        node_put(&conn, &json!({"nid": "k2", "memid": "m", "label": "Draw on the GPU", "opened": {"at": 2}}))?;
        node_put(&conn, &json!({"nid": "k3", "memid": "m", "label": "Release 0.5.2", "opened": {"at": 3}, "changed": {"at": 9}}))?;
        let found = |words: &[&str]| -> Result<Vec<String>> {
            Ok(search(&conn, &words.iter().map(|w| w.to_string()).collect::<Vec<_>>(), 10)?.iter().map(|n| n["nid"].as_str().unwrap().to_string()).collect())
        };
        assert_eq!(found(&["ortcut"])?, ["k1"], "inside a longer word");
        assert_eq!(found(&["clash"])?, ["k1"], "the detail is searched");
        assert_eq!(found(&["af\u{e9}"])?, ["k1"], "three letters of more than one byte");
        assert_eq!(found(&["f\u{e9}"])?, ["k1"], "two letters: no index, still found");
        assert_eq!(found(&["gpu"])?, ["k2"], "any case");
        assert_eq!(found(&["release", "draw"])?, ["k3", "k2"], "any of the words, most recently changed first");
        assert!(found(&["nowhere"])?.is_empty() && found(&[" "])?.is_empty());
        // The index follows a change and a deletion.
        node_put(&conn, &json!({"nid": "k1", "memid": "m", "label": "Key settings", "opened": {"at": 1}}))?;
        assert!(found(&["ortcut"])?.is_empty());
        purge(&mut conn, &["k2".into()])?;
        assert!(search(&conn, &["gpu".into()], 10)?.is_empty());
        Ok(())
    }

    #[test]
    fn two_programs_write_the_same_store() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().to_path_buf();
        let writers: Vec<_> = (0..4).map(|w| {
            let path = path.clone();
            std::thread::spawn(move || {
                let conn = open(&path).unwrap();
                for i in 0..25 { node_put(&conn, &node(&format!("w{w}-{i}"), "n", i)).unwrap(); }
            })
        }).collect();
        for w in writers { w.join().unwrap(); }
        let conn = open(dir.path())?;
        assert_eq!(conn.query_row("SELECT count(*) FROM nodes", [], |r| r.get::<_, i64>(0))?, 100);
        assert_eq!(conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?, VERSION);
        Ok(())
    }
}
