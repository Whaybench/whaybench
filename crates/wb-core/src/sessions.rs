//! What the records say about Claude Code sessions: where a session lives now, whether
//! it runs and where, which tab holds it. Records are written through `whaybench record`.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

use crate::{log as record, paths::Data, versions};

pub fn sessions_dir(d: &Data) -> PathBuf { d.root.join("sessions") }
pub fn alive_dir(d: &Data) -> PathBuf { d.cache().join("alive") }
pub fn state_dir(d: &Data) -> PathBuf { d.cache().join("state") }
pub fn focus_file(d: &Data) -> PathBuf { d.root.join("focus.json") }
pub fn tabs_dir(d: &Data) -> PathBuf { d.cache().join("term") }

fn read_json(p: &Path) -> Option<Value> { serde_json::from_str(&fs::read_to_string(p).ok()?).ok() }

fn write_atomic(p: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = p.parent() { fs::create_dir_all(dir)?; }
    let tmp = p.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, p)
}

pub fn record_of(d: &Data, id: &str) -> Option<Value> { read_json(&sessions_dir(d).join(format!("{id}.json"))) }

/// Where a session lives now: /background carries it on under a new id and the old record says where it
/// moved, possibly more than once.
pub fn current_id(d: &Data, id: &str) -> String {
    let mut now = id.to_string();
    let mut seen = vec![];
    while !seen.contains(&now) {
        seen.push(now.clone());
        match record_of(d, &now).and_then(|r| r.get("movedTo").and_then(Value::as_str).map(String::from)) {
            Some(next) if !next.is_empty() => now = next,
            _ => break,
        }
    }
    now
}

/// The numbers this conversation had before, nearest first. Only moves of the conversation itself: a new session that
/// took over another's map is a conversation of its own.
pub fn moved_from(d: &Data, id: &str) -> Vec<String> {
    let records = all(d);
    let (mut out, mut frontier) = (Vec::new(), vec![id.to_string()]);
    while let Some(here) = frontier.pop() {
        for r in &records {
            if r.get("movedTo").and_then(Value::as_str) != Some(&here) { continue; }
            let Some(old) = r.get("id").and_then(Value::as_str) else { continue };
            if old != id && !out.iter().any(|o| o == old) { out.push(old.to_string()); frontier.push(old.to_string()); }
        }
    }
    out
}

/// The newest session recorded for this tab that has not moved on. A tab can hold several in turn.
pub fn session_of_tab(d: &Data, tab: &str) -> Option<String> {
    let mut best: Option<(i64, String)> = None;
    for f in fs::read_dir(sessions_dir(d)).ok()?.flatten() {
        let Some(r) = read_json(&f.path()) else { continue };
        if r.get("tab").and_then(Value::as_str) != Some(tab) || r.get("movedTo").and_then(Value::as_str).is_some_and(|m| !m.is_empty()) { continue; }
        let at = r.get("lastActive").and_then(Value::as_i64).unwrap_or(0);
        let Some(id) = r.get("id").and_then(Value::as_str) else { continue };
        if best.as_ref().is_none_or(|(b, _)| at >= *b) { best = Some((at, id.to_string())); }
    }
    best.map(|b| b.1)
}

/// Where Claude Code says a conversation carried on: the last `continuedInSessionId` in its transcript.
fn continued_in_text(text: &str) -> Option<String> {
    const KEY: &str = "\"continuedInSessionId\":\"";
    let at = text.rfind(KEY)? + KEY.len();
    let rest = &text[at..];
    let id = &rest[..rest.find('"')?];
    (id.len() == 36 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')).then(|| id.to_string())
}

fn transcript_of(d: &Data, id: &str) -> Option<PathBuf> {
    let cwd = record_of(d, id)?.get("cwd")?.as_str()?.to_string();
    let home = std::env::var("HOME").ok()?;
    let folder: String = cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    Some(Path::new(&home).join(".claude/projects").join(folder).join(format!("{id}.jsonl")))
}

/// The session a conversation carried on in, by Claude Code's own record. Needed when /background came right
/// after /resume: the new session has no transcript yet, so the mod takes it for a spare process and does not
/// take the conversation over.
pub fn continued_in(d: &Data, id: &str) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = fs::File::open(transcript_of(d, id)?).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(256 * 1024))).ok()?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).ok()?;
    continued_in_text(&String::from_utf8_lossy(&bytes)).filter(|n| n != id)
}

/// Whether Claude Code keeps a conversation under this number (its transcript). Codex keeps its own, elsewhere.
pub fn saved(d: &Data, id: &str) -> bool {
    record_of(d, id).and_then(|r| r.get("agent").and_then(Value::as_str).map(String::from)).as_deref() == Some("codex")
        || transcript_of(d, id).is_some_and(|p| holds_conversation(&p))
}

/// Whether a transcript has the person's words in it. Claude Code also leaves files with only its own bookkeeping
/// lines, which it cannot resume ("No conversation found"). The first person's line comes early, so only the start
/// is read.
fn holds_conversation(path: &Path) -> bool {
    use std::io::{BufRead, Read};
    let Ok(f) = fs::File::open(path) else { return false };
    std::io::BufReader::new(f.take(256 * 1024)).split(b'\n').map_while(Result::ok)
        .any(|line| String::from_utf8_lossy(&line).contains("\"type\":\"user\""))
}

/// A number nothing can be opened by: not running, nothing saved under it, and no earlier number carried on into it
/// with its conversation. Left behind when a session handed to the background ended before it saved anything; the
/// conversation itself lives on under another number.
pub fn ghost(d: &Data, all: &[Value], id: &str) -> bool {
    !alive(d, id) && !saved(d, id)
        && !all.iter().any(|r| r.get("movedTo").and_then(Value::as_str) == Some(id) && r.get("id").and_then(Value::as_str).is_some_and(|p| saved(d, p)))
}

/// The number to resume a conversation by. A session carried on in the background that ended before it
/// wrote anything has no transcript, and Claude Code answers "No conversation found"; the conversation is
/// then in the nearest earlier number that has one.
pub fn resumable(d: &Data, id: &str) -> String {
    if record_of(d, id).and_then(|r| r.get("agent").and_then(Value::as_str).map(String::from)).as_deref() == Some("codex") { return id.into(); }
    resumable_by(d, id, |n| transcript_of(d, n).is_some_and(|p| holds_conversation(&p)))
}

fn resumable_by(d: &Data, id: &str, saved: impl Fn(&str) -> bool) -> String {
    if saved(id) { return id.into(); }
    moved_from(d, id).into_iter().find(|n| saved(n)).unwrap_or_else(|| id.into())
}

/// Finishes the take-over the mod did not do: the new record gets the conversation's name and tab, its map, and
/// the old record says where it went.
pub fn adopt(d: &Data, old: &str, new: &str) {
    if old == new { return; }
    let Some(mut from) = record_of(d, old) else { return };
    let mut to = record_of(d, new).unwrap_or_else(|| serde_json::json!({"id": new}));
    for key in ["cwd", "firstPrompt", "lastPrompt", "tab"] {
        if to.get(key).and_then(Value::as_str).is_none_or(str::is_empty) {
            if let Some(v) = from.get(key) { to[key] = v.clone(); }
        }
    }
    if to.get("lastActive").and_then(Value::as_i64).unwrap_or(0) == 0 {
        if let Some(v) = from.get("lastActive") { to["lastActive"] = v.clone(); }
    }
    let _ = write_atomic(&sessions_dir(d).join(format!("{new}.json")), &to.to_string());
    let (map_old, map_new) = (d.root.join(format!("{old}.json")), d.root.join(format!("{new}.json")));
    if map_old.exists() && !map_new.exists() { let _ = fs::copy(map_old, map_new); }
    from["movedTo"] = Value::String(new.to_string());
    let _ = write_atomic(&sessions_dir(d).join(format!("{old}.json")), &from.to_string());
    mark_off(d, old);
    record::decide("session", "number change seen", "the old transcript's continuedInSessionId points to it and that session runs", &[("old", old.to_string()), ("new", new.to_string()), ("source", "continuedIn".into())]);
}

/// Where a session lives now, also when the mod missed the move: if the session itself is not running and Claude
/// Code says it continued in one that is, that one takes over.
/// A recorded move can be stale: a number the conversation moved to that ended before saving anything, after which
/// the conversation went on under an older number. So each number on the way is asked, newest first.
pub fn locate(d: &Data, id: &str) -> String {
    let now = current_id(d, id);
    if agent(&now).is_some() { return now; }
    let mut way = vec![id.to_string()];
    while let Some(next) = record_of(d, way.last().unwrap()).and_then(|r| r.get("movedTo")?.as_str().map(String::from)).filter(|n| !n.is_empty() && !way.contains(n)) {
        way.push(next);
    }
    for from in way.iter().rev() {
        // A carried-on conversation's own transcript names itself too: that is not a move.
        if let Some(next) = continued_in(d, from).filter(|n| n != from && agent(n).is_some()) { adopt(d, from, &next); return next; }
    }
    now
}

/// The conversation goes on under this number again: a move recorded on it did not hold.
pub fn moved_back(d: &Data, id: &str) {
    let Some(mut r) = record_of(d, id) else { return };
    if r.as_object_mut().and_then(|o| o.remove("movedTo")).is_some() {
        let _ = write_atomic(&sessions_dir(d).join(format!("{id}.json")), &r.to_string());
    }
}

pub fn claude_program() -> PathBuf {
    let path = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var("HOME").ok();
    versions::find_program("claude", &path, home.as_deref().map(Path::new)).unwrap_or_else(|| "claude".into())
}

static AGENTS: Mutex<Option<(Instant, Vec<Value>)>> = Mutex::new(None);

/// The next `agents()` asks Claude Code again instead of using the list kept from up to 3 seconds ago.
pub fn refresh_agents() { *AGENTS.lock().unwrap() = None; }

/// Running sessions as Claude Code reports them, kept for 3 seconds. `kind` "interactive" is held by a
/// terminal; anything else runs in the background.
pub fn agents() -> Vec<Value> {
    if crate::codex::active() { return Vec::new(); }
    let mut g = AGENTS.lock().unwrap();
    if let Some((at, list)) = g.as_ref() {
        if at.elapsed() < Duration::from_secs(3) { return list.clone(); }
    }
    let out = Command::new(claude_program()).args(["agents", "--json"]).stdin(Stdio::null()).stderr(Stdio::null()).output();
    let list = out.ok().and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok()).and_then(|v| v.as_array().cloned())
        .or_else(|| g.as_ref().map(|x| x.1.clone())).unwrap_or_default();
    *g = Some((Instant::now(), list.clone()));
    list
}

/// The name `claude attach` knows a background session by. An agent keeps its first short id when the conversation
/// in it moves to a new session number, so it cannot be taken from the session number.
pub fn attach_name(id: &str) -> String {
    agent(id).and_then(|a| a.get("id")?.as_str().map(String::from)).unwrap_or_else(|| id.chars().take(8).collect())
}

/// Whether the person has said anything in this session. Without that there is nothing to come back to.
pub fn has_conversation(d: &Data, id: &str) -> bool {
    if record_of(d, id).and_then(|r| r.get("firstPrompt")?.as_str().map(|p| !p.is_empty())).unwrap_or(false) { return true; }
    // The record can miss a prompt; Claude Code's own record cannot. Ending a session is not to be undone on a guess.
    let Some(home) = std::env::var_os("HOME") else { return true };
    let name = format!("{id}.jsonl");
    fs::read_dir(Path::new(&home).join(".claude/projects")).into_iter().flatten().flatten().any(|e| {
        fs::read_to_string(e.path().join(&name)).is_ok_and(|t| t.contains("\"type\":\"user\""))
    })
}

pub fn agent(id: &str) -> Option<Value> { agents().into_iter().find(|a| a.get("sessionId").and_then(Value::as_str) == Some(id)) }

pub fn background(id: &str) -> bool { agent(id).is_some_and(|a| a.get("kind").and_then(Value::as_str) != Some("interactive")) }

/// Whether a process holds this session: Claude Code's list, else the mod's heartbeat (30 seconds).
pub fn alive(d: &Data, id: &str) -> bool {
    if agent(id).is_some() { return true; }
    heartbeat(&alive_dir(d).join(id))
}

fn heartbeat(p: &Path) -> bool {
    if fs::read_to_string(p).is_ok_and(|t| t.trim() == "0") { return false; }
    fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok()).is_some_and(|a| a < Duration::from_secs(30))
}

pub fn mark_off(d: &Data, id: &str) { let _ = write_atomic(&alive_dir(d).join(id), "0"); }

/// A session the person ended: kept on disk, listed under the hidden ones, never left as "working".
pub fn mark_ended(d: &Data, id: &str) {
    let p = sessions_dir(d).join(format!("{id}.json"));
    if let Some(mut r) = read_json(&p) {
        r["ended"] = Value::Bool(true);
        let _ = write_atomic(&p, &r.to_string());
    }
    let at = SystemTime::now().duration_since(UNIX_EPOCH).map(|t| t.as_millis() as u64).unwrap_or(0);
    let _ = write_atomic(&state_dir(d).join(format!("{id}.json")), &serde_json::json!({"state":"done","note":"ended","at":at}).to_string());
}

pub fn stop(id: &str) {
    let short: String = id.chars().take(8).collect();
    let _ = Command::new(claude_program()).args(["stop", &short]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

/// Every recorded session, most recently active first.
pub fn all(d: &Data) -> Vec<Value> {
    let mut out: Vec<Value> = fs::read_dir(sessions_dir(d)).into_iter().flatten().flatten().filter_map(|f| read_json(&f.path())).filter(|r| r.get("id").and_then(Value::as_str).is_some()).collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.get("lastActive").and_then(Value::as_i64).unwrap_or(0)));
    out
}

pub fn map_of(d: &Data, id: &str) -> Option<Value> { read_json(&d.root.join(format!("{id}.json"))) }

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
        if moved || done.iter().any(|x| x == id) || !ghost(d, &all, id) { continue; }
        let Some(map) = map_of(d, id) else { continue };
        let nodes = map.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
        if nodes.is_empty() { continue; }
        let path = crate::map::free_path(&d.maps(), &crate::map::file_name(&nodes, None, "auto-"));
        let body = serde_json::json!({"format": "whaybench-map", "version": map.get("version"), "title": session_name(d, id),
            "exported": chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(), "from": id, "auto": true, "nodes": nodes});
        if fs::create_dir_all(d.maps()).and_then(|_| fs::write(&path, body.to_string())).is_ok() {
            record::decide("session", "map saved", "its conversation is no longer kept", &[("session", id.to_string()), ("file", path.display().to_string())]);
            done.push(id.to_string());
            saved += 1;
        }
    }
    if saved > 0 { let _ = write_atomic(&file, &Value::from(done).to_string()); }
    saved
}

/// A node label as a name: the part before ' → ' (the topic, not its conclusion).
fn short(label: &str, n: usize) -> String {
    let head = label.split(" → ").next().unwrap_or("").trim();
    if head.chars().count() <= n { head.to_string() } else { format!("{}…", head.chars().take(n - 1).collect::<String>()) }
}

/// (main topic, what it is on now): the top branch with the most nodes, and the step under it holding the current one.
pub fn map_topics(map: &Value) -> (String, String) {
    let nodes = map.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
    if nodes.is_empty() { return Default::default(); }
    let sid = |n: &Value| n.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let parent = |n: &Value| n.get("parent").and_then(Value::as_str).unwrap_or("").to_string();
    let by: std::collections::HashMap<String, &Value> = nodes.iter().map(|n| (sid(n), n)).collect();
    let top_of = |start: &Value| -> String {
        let (mut x, mut seen) = (start, Vec::new());
        while let Some(up) = by.get(&parent(x)) {
            if seen.contains(&sid(x)) { break; }
            seen.push(sid(x));
            x = up;
        }
        sid(x)
    };
    let mut counts: Vec<(String, usize)> = Vec::new();
    for n in &nodes {
        let t = top_of(n);
        match counts.iter_mut().find(|c| c.0 == t) { Some(c) => c.1 += 1, None => counts.push((t, 1)) }
    }
    // Most nodes; of equals, the one that comes last in the map.
    let position = |id: &str| nodes.iter().position(|n| sid(n) == id).unwrap_or(0);
    let main = counts.iter().max_by_key(|c| (c.1, position(&c.0))).map(|c| c.0.clone()).unwrap_or_default();
    let label = |id: &str| by.get(id).and_then(|n| n.get("label")).and_then(Value::as_str).unwrap_or("").to_string();
    let current = map.get("current").and_then(Value::as_str).unwrap_or("");
    let mut now = String::new();
    if let Some(cur) = by.get(current) {
        if top_of(cur) != main {
            now = label(&top_of(cur));
        } else {
            let mut step = (*cur).clone();
            let mut guard = 0;
            let mut last = String::new();
            while sid(&step) != main && guard < 200 {
                last = sid(&step);
                match by.get(&parent(&step)) { Some(up) => step = (*up).clone(), None => break }
                guard += 1;
            }
            if !last.is_empty() { now = label(&last); }
        }
    }
    (short(&label(&main), 26), short(&now, 22))
}

/// Names from the maps, made distinct: main topic, then with what it is on, then with the date of last activity.
pub fn map_titles(d: &Data) -> std::collections::HashMap<String, String> {
    use std::collections::HashMap;
    let mut rows: Vec<(String, String, String, i64)> = Vec::new();
    for info in all(d) {
        if info.get("movedTo").and_then(Value::as_str).is_some_and(|m| !m.is_empty()) { continue; }
        let id = info["id"].as_str().unwrap_or("").to_string();
        let Some(map) = map_of(d, &id) else { continue };
        let (main, now) = map_topics(&map);
        if !main.is_empty() { rows.push((id, main, now, info.get("lastActive").and_then(Value::as_i64).unwrap_or(0))); }
    }
    let mut names: HashMap<String, String> = HashMap::new();
    for (id, main, now, _) in &rows {
        let same = rows.iter().filter(|r| r.1 == *main).count();
        names.insert(id.clone(), if same == 1 || now.is_empty() { main.clone() } else { format!("{main} · {now}") });
    }
    let snapshot = names.clone();
    for (id, _, _, at) in &rows {
        let name = &snapshot[id];
        if snapshot.values().filter(|v| *v == name).count() > 1 {
            let stamp = chrono::DateTime::from_timestamp_millis(*at).map(|t| t.with_timezone(&chrono::Local).format("%m-%d %H:%M").to_string()).unwrap_or_default();
            names.insert(id.clone(), format!("{name} · {stamp}"));
        }
    }
    names
}

/// The session's name: its main topic from its map; without a map, the name Claude Code gives it
/// (without the " (n)" it adds per /background), remembered after the session stops; else the first prompt.
pub fn session_name(d: &Data, id: &str) -> String {
    let file = d.root.join("names.json");
    let mut names = read_json(&file).unwrap_or_else(|| serde_json::json!({}));
    let running = strip_count(agent(id).as_ref().and_then(|a| a.get("name")).and_then(Value::as_str).unwrap_or(""));
    if !running.is_empty() && names.get(id).and_then(Value::as_str) != Some(&running) {
        names[id] = Value::String(running.clone());
        let _ = write_atomic(&file, &names.to_string());
    }
    if let Some(n) = map_titles(d).get(id) { return n.clone(); }
    if !running.is_empty() { return running; }
    if let Some(n) = names.get(id).and_then(Value::as_str) { return n.to_string(); }
    record_of(d, id).and_then(|r| r.get("firstPrompt").and_then(Value::as_str).map(String::from)).unwrap_or_default()
}

pub fn tab_title(d: &Data, id: &str) -> String { session_name(d, id) }

pub fn unend(d: &Data, id: &str) {
    let p = sessions_dir(d).join(format!("{id}.json"));
    if let Some(mut r) = read_json(&p) {
        r["ended"] = Value::Bool(false);
        let _ = write_atomic(&p, &r.to_string());
    }
}

pub fn state_of(d: &Data, id: &str) -> Option<String> { read_json(&state_dir(d).join(format!("{id}.json")))?.get("state")?.as_str().map(String::from) }

pub fn context_percent(d: &Data, id: &str) -> Option<i64> { fs::read_to_string(d.cache().join("context").join(id)).ok()?.trim().parse().ok() }

pub fn clear_focus(d: &Data) { let _ = fs::remove_file(focus_file(d)); }

pub fn latest(d: &Data) -> Option<String> { Some(current_id(d, read_json(&latest_map(d))?.get("sessionId")?.as_str()?)) }

fn strip_count(name: &str) -> String {
    let t = name.trim();
    if let Some(open) = t.rfind(" (") {
        let inner = &t[open + 2..];
        if inner.ends_with(')') && inner[..inner.len() - 1].chars().all(|c| c.is_ascii_digit()) && inner.len() > 1 { return t[..open].trim().to_string(); }
    }
    t.to_string()
}

pub fn set_focus(d: &Data, id: &str, who: &str) {
    let before = read_json(&focus_file(d)).and_then(|v| v.get("sessionId").and_then(Value::as_str).map(String::from)).unwrap_or_default();
    if before == id { return; }
    record::decide("session", "session chosen", "the session of the tab in front", &[("session", id.to_string()), ("by", who.to_string()), ("before", before)]);
    let _ = write_atomic(&focus_file(d), &serde_json::json!({"sessionId": id}).to_string());
}

pub fn latest_map(d: &Data) -> PathBuf { d.root.join("latest.json") }

/// The chosen session, followed to where it lives now.
pub fn focus(d: &Data) -> Option<String> {
    let id = read_json(&focus_file(d))?.get("sessionId")?.as_str()?.to_string();
    Some(current_id(d, &id))
}

/// The session the program is on: the chosen one, else the one whose map changed last.
pub fn target(d: &Data) -> Option<String> {
    focus(d).or_else(|| {
        let id = read_json(&latest_map(d))?.get("sessionId")?.as_str()?.to_string();
        Some(current_id(d, &id))
    })
}

/// A request's kind for the record: its leading [tag], a slash command, or the person's own text.
pub fn request_kind(text: &str) -> String {
    let t = text.trim_start();
    if t.starts_with('[') {
        if let Some(end) = t.char_indices().take(40).find(|(_, c)| *c == ']').map(|(i, _)| i) { return t[1..end].to_string(); }
    }
    if t.starts_with('/') { return format!("command {}", t.split_whitespace().next().unwrap_or(t)); }
    "person's input".into()
}

/// Queues text for a session's map mod to submit as if typed. Only for a session running now: the mod submits
/// whatever is queued when a session next starts, so a request left for a stopped one would run hours later,
/// after the work was done elsewhere. True when queued.
pub fn queue_prompt(d: &Data, id: &str, text: &str) -> bool {
    let now = current_id(d, id);
    let kind = request_kind(text);
    if !alive(d, &now) {
        record::decide("request", "request not sent", "the session is not running (no heartbeat)", &[("session", id.to_string()), ("kind", kind)]);
        return false;
    }
    if record_of(d, &now).and_then(|r| r.get("agent").and_then(Value::as_str).map(String::from)).as_deref() == Some("codex") {
        let Some(tab) = open_tabs(d).into_iter().find(|t| t.session.as_deref() == Some(&now)) else { return false };
        let seq = SystemTime::now().duration_since(UNIX_EPOCH).map(|t| t.as_nanos()).unwrap_or(0);
        return write_atomic(&tab_inbox(d, &tab.tab).join(format!("{seq}.json")), &serde_json::json!({"text": text, "enter": true}).to_string()).is_ok();
    }
    let path = d.cache().join("inbox").join(format!("{now}.json"));
    let mut queued = read_json(&path).and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let seq = SystemTime::now().duration_since(UNIX_EPOCH).map(|t| t.as_micros() as u64).unwrap_or(0);
    queued.push(serde_json::json!({"seq": seq, "text": text}));
    let skip = queued.len().saturating_sub(20);
    let kept: Vec<Value> = queued.into_iter().skip(skip).collect();
    if write_atomic(&path, &Value::Array(kept).to_string()).is_err() { return false; }
    record::decide("request", "request queued", "the session is running", &[("request", seq.to_string()), ("session", now), ("kind", kind), ("chars", text.chars().count().to_string())]);
    true
}

/// A tab's own record of what it shows, for the program and the other tabs.
pub fn tab_file(d: &Data, tab: &str) -> PathBuf { tabs_dir(d).join(format!("{tab}.json")) }
pub fn tab_inbox(d: &Data, tab: &str) -> PathBuf { tabs_dir(d).join(format!("{tab}.in")) }

pub fn write_tab(d: &Data, tab: &str, session: Option<&str>, attached: bool) {
    let v = serde_json::json!({"tab": tab, "pid": std::process::id(), "session": session, "attached": attached});
    let _ = write_atomic(&tab_file(d, tab), &v.to_string());
}

pub struct OpenTab { pub tab: String, pub session: Option<String>, pub attached: bool }

/// Tabs whose relay still runs.
pub fn open_tabs(d: &Data) -> Vec<OpenTab> {
    let mut out = Vec::new();
    for f in fs::read_dir(tabs_dir(d)).into_iter().flatten().flatten() {
        if f.path().extension().is_none_or(|e| e != "json") { continue; }
        let Some(v) = read_json(&f.path()) else { continue };
        let pid = v.get("pid").and_then(Value::as_u64).unwrap_or(0);
        if pid == 0 || !Path::new(&format!("/proc/{pid}")).exists() { let _ = fs::remove_file(f.path()); continue; }
        out.push(OpenTab {
            tab: v.get("tab").and_then(Value::as_str).unwrap_or_default().to_string(),
            session: v.get("session").and_then(Value::as_str).map(String::from),
            attached: v.get("attached").and_then(Value::as_bool).unwrap_or(false),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn data(name: &str) -> Data {
        let d = std::env::temp_dir().join(format!("wb-s-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("sessions")).unwrap();
        Data::new(d)
    }
    fn rec(d: &Data, id: &str, v: Value) { fs::write(sessions_dir(d).join(format!("{id}.json")), v.to_string()).unwrap(); }

    #[test]
    fn moves_are_followed_to_the_end_and_a_loop_stops() {
        let d = data("moved");
        rec(&d, "a", serde_json::json!({"id":"a","movedTo":"b"}));
        rec(&d, "b", serde_json::json!({"id":"b","movedTo":"c"}));
        rec(&d, "c", serde_json::json!({"id":"c"}));
        assert_eq!(current_id(&d, "a"), "c");
        assert_eq!(current_id(&d, "none"), "none");
        rec(&d, "x", serde_json::json!({"id":"x","movedTo":"y"}));
        rec(&d, "y", serde_json::json!({"id":"y","movedTo":"x"}));
        assert!(["x", "y"].contains(&current_id(&d, "x").as_str()));
    }
    #[test]
    fn a_tab_is_paired_with_its_newest_session_that_has_not_moved() {
        let d = data("pair");
        rec(&d, "old", serde_json::json!({"id":"old","tab":"t1","lastActive":5}));
        rec(&d, "new", serde_json::json!({"id":"new","tab":"t1","lastActive":9}));
        rec(&d, "gone", serde_json::json!({"id":"gone","tab":"t1","lastActive":99,"movedTo":"new"}));
        rec(&d, "other", serde_json::json!({"id":"other","tab":"t2","lastActive":100}));
        assert_eq!(session_of_tab(&d, "t1").as_deref(), Some("new"));
        assert_eq!(session_of_tab(&d, "t3"), None);
    }
    #[test]
    fn a_take_over_keeps_the_time_of_last_activity() {
        let d = data("adopt-time");
        rec(&d, "x", serde_json::json!({"id":"x","lastActive":7}));
        rec(&d, "y", serde_json::json!({"id":"y"}));
        adopt(&d, "x", "y");
        assert_eq!(record_of(&d, "y").unwrap()["lastActive"], 7);
    }
    #[test]
    fn claude_codes_note_of_where_it_continued_is_read() {
        let id = "00000000-0000-4000-8000-000000000002";
        let text = format!("x\n{{\"continuedInSessionId\":\"00000000-0000-4000-8000-000000000001\"}}\n{{\"continuedInSessionId\":\"{id}\",\"a\":1}}\ny");
        assert_eq!(continued_in_text(&text).as_deref(), Some(id));
        assert_eq!(continued_in_text("nothing"), None);
        assert_eq!(continued_in_text("\"continuedInSessionId\":\"short\""), None);
    }
    #[test]
    fn a_missed_take_over_is_finished() {
        let d = data("adopt");
        rec(&d, "old", serde_json::json!({"id":"old","cwd":"/h","firstPrompt":"question","tab":"t1","lastActive":5}));
        rec(&d, "new", serde_json::json!({"id":"new","cwd":"/h","lastActive":9}));
        fs::write(d.root.join("old.json"), "{\"nodes\":[]}").unwrap();
        adopt(&d, "old", "new");
        let n = record_of(&d, "new").unwrap();
        assert_eq!((n["firstPrompt"].as_str(), n["tab"].as_str(), n["id"].as_str()), (Some("question"), Some("t1"), Some("new")));
        assert!(d.root.join("new.json").exists(), "the map goes along");
        assert_eq!(current_id(&d, "old"), "new");
        assert_eq!(session_of_tab(&d, "t1").as_deref(), Some("new"));
    }
    #[test]
    fn requests_go_only_to_a_running_session_and_keep_the_last_20() {
        let d = data("queue");
        fs::create_dir_all(alive_dir(&d)).unwrap();
        rec(&d, "old", serde_json::json!({"id":"old","movedTo":"new"}));
        rec(&d, "new", serde_json::json!({"id":"new"}));
        assert!(!queue_prompt(&d, "old", "text"), "not running");
        assert!(!d.cache().join("inbox/new.json").exists());
        fs::write(alive_dir(&d).join("new"), "1").unwrap();
        for i in 0..25 { assert!(queue_prompt(&d, "old", &format!("text{i}"))); }
        let q = read_json(&d.cache().join("inbox/new.json")).unwrap();
        let q = q.as_array().unwrap();
        assert_eq!(q.len(), 20, "goes to where the session lives now");
        assert_eq!(q[19]["text"], "text24");
    }
    #[test]
    fn codex_requests_require_a_tab_and_go_to_its_relay() {
        let d = data("codex-input");
        rec(&d, "codex1", serde_json::json!({"id":"codex1","agent":"codex"}));
        fs::create_dir_all(alive_dir(&d)).unwrap();
        fs::write(alive_dir(&d).join("codex1"), "1").unwrap();
        assert!(!queue_prompt(&d, "codex1", "hello"));
        write_tab(&d, "codextab", Some("codex1"), false);
        assert!(queue_prompt(&d, "codex1", "first line\nsecond line"));
        let p = fs::read_dir(tab_inbox(&d, "codextab")).unwrap().next().unwrap().unwrap().path();
        let v = read_json(&p).unwrap();
        assert_eq!(v["text"], "first line\nsecond line");
        assert_eq!(v["enter"], true);
        assert!(!d.cache().join("inbox/codex1.json").exists());
    }
    #[test]
    fn a_session_without_a_transcript_resumes_where_its_conversation_is() {
        let d = data("resumable");
        rec(&d, "a", serde_json::json!({"id":"a","movedTo":"b"}));
        rec(&d, "b", serde_json::json!({"id":"b","movedTo":"c"}));
        rec(&d, "c", serde_json::json!({"id":"c"}));
        assert_eq!(resumable_by(&d, "c", |n| n == "a"), "a");
        assert_eq!(resumable_by(&d, "c", |n| n == "a" || n == "b"), "b");
        assert_eq!(resumable_by(&d, "c", |n| n == "c"), "c");
        assert_eq!(resumable_by(&d, "c", |_| false), "c");
    }
    #[test]
    fn a_number_with_nothing_behind_it_is_a_ghost_unless_a_saved_one_went_on_in_it() {
        let d = data("ghost");
        rec(&d, "a", serde_json::json!({"id":"a","movedTo":"b"}));
        rec(&d, "b", serde_json::json!({"id":"b"}));
        rec(&d, "c", serde_json::json!({"id":"c","agent":"codex"}));
        let all = all(&d);
        // No transcripts here (no HOME projects for these ids): "a" is not saved, so "b" has nothing behind it.
        assert!(ghost(&d, &all, "b"));
        assert!(!ghost(&d, &all, "c"), "Codex keeps its own conversations");
        fs::create_dir_all(alive_dir(&d)).unwrap();
        fs::write(alive_dir(&d).join("b"), "1").unwrap();
        assert!(!ghost(&d, &all, "b"), "a running one is no ghost");
    }
    #[test]
    fn the_map_of_a_conversation_no_longer_kept_is_saved_once_from_its_last_number() {
        let d = data("lost-maps");
        rec(&d, "a", serde_json::json!({"id":"a","movedTo":"b"}));
        rec(&d, "b", serde_json::json!({"id":"b"}));
        let map = |label: &str| serde_json::json!({"nodes":[{"id":"n","label":label,"opened":{"at":1_790_000_000_000_i64}}]}).to_string();
        fs::write(d.root.join("a.json"), map("early")).unwrap();
        fs::write(d.root.join("b.json"), map("whole")).unwrap();
        assert_eq!(save_lost_maps(&d), 1, "only the last number");
        let files: Vec<_> = fs::read_dir(d.maps()).unwrap().flatten().map(|f| f.path()).collect();
        assert_eq!(files.len(), 1);
        assert!(files[0].file_name().unwrap().to_string_lossy().starts_with("auto-map-"));
        let v = read_json(&files[0]).unwrap();
        assert_eq!((v["from"].as_str(), v["auto"].as_bool(), v["nodes"][0]["label"].as_str()), (Some("b"), Some(true), Some("whole")));
        assert_eq!(save_lost_maps(&d), 0, "once");
    }
    #[test]
    fn a_move_that_did_not_hold_is_taken_back() {
        let d = data("moved-back");
        rec(&d, "a", serde_json::json!({"id":"a","movedTo":"b","firstPrompt":"text"}));
        assert_eq!(current_id(&d, "a"), "b");
        moved_back(&d, "a");
        assert_eq!(current_id(&d, "a"), "a");
        assert_eq!(record_of(&d, "a").unwrap()["firstPrompt"], "text");
    }
    #[test]
    fn request_kinds() {
        assert_eq!(request_kind("[map update] text"), "map update");
        assert_eq!(request_kind("/exit now"), "command /exit");
        assert_eq!(request_kind("plain text"), "person's input");
    }
    #[test]
    fn the_chosen_session_comes_first() {
        let d = data("target");
        assert_eq!(target(&d), None);
        fs::write(latest_map(&d), r#"{"sessionId":"m"}"#).unwrap();
        assert_eq!(target(&d).as_deref(), Some("m"));
        rec(&d, "a", serde_json::json!({"id":"a","movedTo":"b"}));
        fs::write(focus_file(&d), r#"{"sessionId":"a"}"#).unwrap();
        assert_eq!(target(&d).as_deref(), Some("b"));
    }
    #[test]
    fn ending_marks_the_record_and_the_state() {
        let d = data("end");
        rec(&d, "a", serde_json::json!({"id":"a"}));
        mark_ended(&d, "a");
        assert_eq!(record_of(&d, "a").unwrap()["ended"], Value::Bool(true));
        let s = read_json(&state_dir(&d).join("a.json")).unwrap();
        assert_eq!((s["state"].as_str(), s["note"].as_str()), (Some("done"), Some("ended")));
    }
    #[test]
    fn heartbeat_zero_means_off() {
        let d = data("beat");
        fs::create_dir_all(alive_dir(&d)).unwrap();
        let p = alive_dir(&d).join("a");
        fs::write(&p, "123").unwrap();
        assert!(heartbeat(&p));
        mark_off(&d, "a");
        assert!(!heartbeat(&p));
        assert!(!heartbeat(&alive_dir(&d).join("none")));
    }
    #[test]
    fn a_map_names_a_session_by_its_biggest_branch() {
        let m = serde_json::json!({"current":"c","nodes":[{"id":"a","label":"small branch"},{"id":"b","label":"big topic → conclusion"},{"id":"c","parent":"b","label":"first step"},{"id":"d","parent":"c","label":"detail"}]});
        assert_eq!(map_topics(&m), ("big topic".to_string(), "first step".to_string()));
        assert_eq!(map_topics(&serde_json::json!({"nodes":[]})), Default::default());
    }
    #[test]
    fn the_same_topic_in_two_sessions_is_told_apart() {
        let d = data("titles");
        rec(&d, "s1", serde_json::json!({"id":"s1","lastActive":1000}));
        rec(&d, "s2", serde_json::json!({"id":"s2","lastActive":2000}));
        rec(&d, "s3", serde_json::json!({"id":"s3","firstPrompt":"first question"}));
        let map = |cur: &str| serde_json::json!({"current":cur,"nodes":[{"id":"a","label":"same topic"},{"id":"x","parent":"a","label":"one"},{"id":"y","parent":"a","label":"two"}]});
        fs::write(d.root.join("s1.json"), map("x").to_string()).unwrap();
        fs::write(d.root.join("s2.json"), map("y").to_string()).unwrap();
        let t = map_titles(&d);
        assert_eq!(t["s1"], "same topic · one");
        assert_eq!(t["s2"], "same topic · two");
        assert_eq!(session_name(&d, "s3"), "first question", "without a map, the first prompt");
    }
    #[test]
    fn the_count_claude_code_adds_is_removed() {
        assert_eq!(strip_count("tidy up (3)"), "tidy up");
        assert_eq!(strip_count("tidy up"), "tidy up");
        assert_eq!(strip_count("tidy (up)"), "tidy (up)");
        assert_eq!(strip_count(""), "");
    }
    #[test]
    fn tabs_with_a_dead_relay_do_not_count() {
        let d = data("tabs");
        write_tab(&d, "t1", Some("s1"), true);
        fs::write(tab_file(&d, "t2"), r#"{"tab":"t2","pid":999999999,"session":"s2"}"#).unwrap();
        let open = open_tabs(&d);
        assert_eq!(open.len(), 1);
        assert_eq!((open[0].tab.as_str(), open[0].session.as_deref(), open[0].attached), ("t1", Some("s1"), true));
        assert!(!tab_file(&d, "t2").exists());
    }
    #[test]
    fn a_transcript_without_the_persons_words_is_not_a_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let (empty, full) = (dir.path().join("e.jsonl"), dir.path().join("f.jsonl"));
        fs::write(&empty, "{\"type\":\"session-owner\"}\n{\"type\":\"cost-state\"}\n").unwrap();
        fs::write(&full, "{\"type\":\"session-owner\"}\n{\"type\":\"user\",\"message\":{}}\n").unwrap();
        assert!(!holds_conversation(&empty));
        assert!(holds_conversation(&full));
        assert!(!holds_conversation(&dir.path().join("none.jsonl")));
    }

}
