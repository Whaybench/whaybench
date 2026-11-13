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

use crate::{log as record, paths::{Data, write_atomic}, versions};

mod names;
mod retention;
mod tabs;
pub use names::*;
pub use retention::*;
pub use tabs::*;

pub fn sessions_dir(d: &Data) -> PathBuf { d.root.join("sessions") }
pub fn alive_dir(d: &Data) -> PathBuf { d.cache().join("alive") }
pub fn state_dir(d: &Data) -> PathBuf { d.cache().join("state") }
pub fn focus_file(d: &Data) -> PathBuf { d.root.join("focus.json") }
pub fn tabs_dir(d: &Data) -> PathBuf { d.cache().join("term") }

fn read_json(p: &Path) -> Option<Value> { serde_json::from_str(&fs::read_to_string(p).ok()?).ok() }


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

/// After /resume picked a session that runs in the background, Claude Code does not move it into this tab: this
/// tab's program becomes a window onto it, its own number stands for nothing, and the session the tab was in goes on
/// in the background, empty when nobody spoke in it. Claude Code notes the opening in the transcript as a system
/// row; only that row counts (the same words can be in the conversation), and only from the last 90 seconds.
/// The session opened, and the empty one carried on, if any.
pub fn resumed_elsewhere(d: &Data, me: &str) -> Option<(String, Option<String>)> {
    let text = fs::read_to_string(transcript_of(d, me)?).ok()?;
    let short = opened_short(&text, chrono::Utc::now())?;
    let target = fs::read_dir(sessions_dir(d)).ok()?.flatten().filter_map(|f| f.file_name().to_string_lossy().strip_suffix(".json").map(String::from))
        .find(|n| n.starts_with(&short))?;
    if target == me { return None; }
    let carried = continued_in_text(&text).filter(|c| c != &target && !has_conversation(d, c));
    Some((target, carried))
}

/// The short number of the background session a transcript says was opened in the last 90 seconds.
fn opened_short(text: &str, now: chrono::DateTime<chrono::Utc>) -> Option<String> {
    let mut short = None;
    for line in text.lines().filter(|l| l.contains("running in the background (")) {
        let Ok(row) = serde_json::from_str::<Value>(line) else { continue };
        if row["type"] != "system" || row["subtype"] != "local_command" { continue; }
        let content = row["content"].as_str().unwrap_or("");
        let Some(rest) = content.strip_prefix("<local-command-stdout>Opening ") else { continue };
        let Some(at) = rest.find("running in the background (") else { continue };
        let id = &rest[at + "running in the background (".len()..];
        let id = id.split(')').next().unwrap_or("");
        if id.len() != 8 || !id.bytes().all(|b| b.is_ascii_hexdigit()) { continue; }
        let when = row["timestamp"].as_str().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok());
        if when.is_some_and(|t| (now - t.with_timezone(&chrono::Utc)).num_seconds() < 90) { short = Some(id.to_string()); }
    }
    short
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

/// Whether Claude Code keeps a conversation under this number (its transcript). An add-on agent keeps its own, elsewhere.
pub fn saved(d: &Data, id: &str) -> bool {
    record_of(d, id).is_some_and(|r| r.get("agent").is_some())
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
    if record_of(d, id).is_some_and(|r| r.get("agent").is_some()) { return id.into(); }
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

/// Whether the person set Claude Code's answer language themselves (`language` in ~/.claude/settings.json).
pub fn claude_language_set() -> bool {
    let Some(home) = std::env::var_os("HOME") else { return false };
    std::fs::read_to_string(std::path::Path::new(&home).join(".claude/settings.json")).ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .is_some_and(|v| v["language"].as_str().is_some_and(|l| !l.is_empty()))
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
    if crate::agents::other() { return Vec::new(); }
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

/// A session's map with its nodes' content (`map::view`); None for a session without a map file.
/// shortcut: opens the store on every call; share one connection if a panel's refresh shows up in a profile.
pub fn map_of(d: &Data, id: &str) -> Option<Value> { crate::map::view(&d.root, id).ok().filter(|m| m.get("nodes").is_some()) }

pub fn unend(d: &Data, id: &str) {
    let p = sessions_dir(d).join(format!("{id}.json"));
    if let Some(mut r) = read_json(&p) {
        r["ended"] = Value::Bool(false);
        let _ = write_atomic(&p, &r.to_string());
    }
}

pub fn state_of(d: &Data, id: &str) -> Option<String> { read_json(&state_dir(d).join(format!("{id}.json")))?.get("state")?.as_str().map(String::from) }


pub fn clear_focus(d: &Data) { let _ = fs::remove_file(focus_file(d)); }

pub fn latest(d: &Data) -> Option<String> { Some(current_id(d, read_json(&latest_map(d))?.get("sessionId")?.as_str()?)) }

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

#[cfg(test)]
mod tests {
    use super::*;
    pub(crate) fn data(name: &str) -> Data {
        let d = std::env::temp_dir().join(format!("wb-s-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("sessions")).unwrap();
        Data::new(d)
    }
    pub(crate) fn rec(d: &Data, id: &str, v: Value) { fs::write(sessions_dir(d).join(format!("{id}.json")), v.to_string()).unwrap(); }

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
        // /resume into a background session: only Claude Code's own system row counts, and only a recent one.
        let now = chrono::Utc::now();
        let row = |kind: &str, ago: i64| serde_json::json!({"type": kind, "subtype": "local_command", "timestamp": (now - chrono::Duration::seconds(ago)).to_rfc3339(),
            "content": "<local-command-stdout>Opening Fix the store, running in the background (5813148b)</local-command-stdout>"}).to_string();
        assert_eq!(opened_short(&row("system", 5), now).as_deref(), Some("5813148b"));
        assert_eq!(opened_short(&row("user", 5), now), None, "the same words in the conversation do not count");
        assert_eq!(opened_short(&row("system", 200), now), None, "an old opening does not count");
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
    fn a_move_that_did_not_hold_is_taken_back() {
        let d = data("moved-back");
        rec(&d, "a", serde_json::json!({"id":"a","movedTo":"b","firstPrompt":"text"}));
        assert_eq!(current_id(&d, "a"), "b");
        moved_back(&d, "a");
        assert_eq!(current_id(&d, "a"), "a");
        assert_eq!(record_of(&d, "a").unwrap()["firstPrompt"], "text");
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
