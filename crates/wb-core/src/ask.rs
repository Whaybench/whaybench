//! An agent's question or permission ask, answered in the answer panel instead of the terminal. The agent's hook
//! writes `cache/ask/<id>.json` and waits for `cache/ask/<id>.answer.json` (Claude Code and Codex
//! permissions, Claude Code's AskUserQuestion: their PermissionRequest hook decides). Codex's request_user_input
//! cannot be answered from a hook: it is held back and the answer goes to the session as its next prompt.
use std::{fs, path::{Path, PathBuf}, time::Duration};

use serde_json::{Value, json};

use crate::{clock::now_ms, i18n, map::{Result, read_json, safe_id, write_json}};

/// How long a hook waits for the panel; the hooks' own timeout is 600 seconds, and the terminal asks after this.
pub const WAIT_MS: i64 = 590_000;

fn dir(data: &Path) -> PathBuf { data.join("cache").join("ask") }
pub(crate) fn request_file(data: &Path, id: &str) -> PathBuf { dir(data).join(format!("{id}.json")) }
fn answer_file(data: &Path, id: &str) -> PathBuf { dir(data).join(format!("{id}.answer.json")) }
fn panel_file(data: &Path) -> PathBuf { dir(data).join("panel") }
/// Written by the answer panel once it has read an ask: what shows it is not stuck.
fn seen_file(data: &Path, id: &str) -> PathBuf { dir(data).join(format!("{id}.seen")) }
/// How long an ask waits for the panel to read it before the panel is taken for stuck.
const SEEN_MS: i64 = 4000;

/// The answer panel says it is open by holding a lock (flock) on a file of each root while it runs: no writing on a
/// clock, and gone with the panel however it ends. A project's hook may run in its container, in its own process space,
/// where the lock still shows. A vault's lock is beside it, not in it: a file held open there would keep the vault
/// from closing. Without a live answer panel an ask goes to the terminal at once.
fn beside(root: &Path) -> PathBuf { root.with_extension("answer-panel") }

/// Takes the locks for the host and each open project, and lets go of those of projects that closed.
pub fn panel_alive(host: &crate::paths::Data) {
    use std::{collections::HashMap, sync::Mutex};
    static HELD: Mutex<Option<HashMap<PathBuf, HashMap<PathBuf, fs::File>>>> = Mutex::new(None);
    // Where its folder is made: the host's, a vault's runtime folder, an open project's data where it rests (never
    // through the link of its open place, which may go meanwhile and leave a real folder in the way of the next open).
    let mut want: Vec<(PathBuf, Option<PathBuf>)> = vec![(panel_file(&host.root), Some(dir(&host.root)))];
    for st in crate::roots::stubs(host).into_iter().filter(|s| s.open()) {
        if st.vault {
            // Seen from the host, and from the container, where the folder that holds the vault is bound over cache/open.
            want.push((beside(&st.open_at), None));
            want.push((beside(&crate::vault::mountpoint(&st)), None));
        } else {
            want.push((panel_file(&st.open_at), Some(dir(&st.stored().root))));
        }
    }
    let mark = std::process::id().to_string();
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    let held = held.get_or_insert_with(HashMap::new).entry(host.root.clone()).or_default();
    held.retain(|p, _| want.iter().any(|(w, _)| w == p));
    for (p, make) in want {
        if held.contains_key(&p) { continue; }
        if let Some(dir) = make { let _ = fs::create_dir_all(dir); }
        let Ok(f) = fs::OpenOptions::new().create(true).truncate(false).write(true).open(&p) else { continue };
        use std::os::fd::AsRawFd;
        // Safety: flock on a descriptor kept open here; the lock goes with it.
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            // Its process number in it: a hook that finds the panel stuck ends it, and Whaybench starts it again.
            use std::io::Write;
            let mut w = &f;
            let _ = f.set_len(0).and_then(|_| w.write_all(mark.as_bytes()));
            held.insert(p, f);
        }
    }
}

/// A panel of before the lock (it writes the time each second), running until Whaybench starts again.
fn old_panel(data: &Path) -> bool {
    let then: Option<i64> = fs::read_to_string(panel_file(data)).ok().and_then(|t| t.trim().parse().ok());
    then.is_some_and(|t| t > 1_000_000_000_000 && (now_ms() - t).abs() < 5000)
}

/// Open while some answer panel holds the lock of this root (in its data, or beside it for a vault); or, from a panel
/// of before the lock that runs until Whaybench starts again, while the time it writes each second is new.
fn panel_open(data: &Path) -> bool {
    use std::os::fd::AsRawFd;
    if old_panel(data) { return true; }
    [panel_file(data), beside(data)].iter().any(|p| fs::File::open(p).is_ok_and(|f| {
        // Safety: flock on a descriptor owned here; a shared lock taken is let go when it closes.
        unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EWOULDBLOCK) }
    }))
}

/// An ask id: a session id with its suffix, so a session id at its own limit still fits.
fn valid_id(id: &str) -> Result<&str> {
    if id.is_empty() || id.len() > 200 || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-') { return Err("invalid ask id".into()); }
    Ok(id)
}

/// Each ask has its own files: a session can ask twice at once (subagents run in parallel), even in one millisecond.
fn new_id(sid: &str) -> String {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    format!("{sid}-{}-{}-{}", now_ms(), std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

/// Asks still waiting, oldest first: one a hook waits on while its process lives (no longer than the hooks'
/// timeout, so a reused process number shows nothing), or one answered by a prompt. Only well-formed ids.
pub fn pending(data: &Path) -> Vec<Value> {
    let foreign = crate::roots::is_project_root(&crate::paths::Data::new(data.to_path_buf()));
    let mut out: Vec<Value> = crate::paths::list_dir(&dir(data)).into_iter()
        .filter(|f| f.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(".json") && !n.ends_with(".answer.json")))
        .filter_map(|f| read_json(&f, Value::Null).ok())
        .filter(|r| r["id"].as_str().is_some_and(|i| valid_id(i).is_ok()) && r["session"].as_str().is_some_and(|s| safe_id(s).is_ok()))
        // A project's hook may run in its own process namespace: its number says nothing here, the time limit holds.
        .filter(|r| r["via"] == "prompt" || ((foreign || r["pid"].as_u64().is_some_and(|p| Path::new(&format!("/proc/{p}")).exists()))
            && now_ms() - r["at"].as_i64().unwrap_or(0) < WAIT_MS + 10_000))
        // Answered, and the hook has not taken it yet.
        .filter(|r| !answer_file(data, r["id"].as_str().unwrap_or("")).exists())
        .collect();
    out.sort_by_key(|r| r["at"].as_i64().unwrap_or(0));
    out
}

/// Asks waiting in every root a caller sees (the host's and the open projects'), oldest first.
pub fn pending_all(d: &crate::paths::Data) -> Vec<Value> {
    // An ask counts only in the root its session is in: a project cannot ask in the name of a host conversation.
    let mine = |r: &crate::paths::Data, a: &Value| a["session"].as_str().and_then(|s| crate::roots::known_session(d, s)).is_some_and(|k| k.root == r.root);
    let mut out: Vec<Value> = crate::roots::all_of(d).iter().flat_map(|r| pending(&r.root).into_iter().filter(|a| mine(r, a)).collect::<Vec<_>>()).collect();
    out.sort_by_key(|r| r["at"].as_i64().unwrap_or(0));
    out
}

/// The data root an ask was written in, where its answer goes. None: it is gone (answered, or its project closed),
/// and the answer goes nowhere.
pub fn root_of(d: &crate::paths::Data, id: &str) -> Option<std::path::PathBuf> {
    crate::roots::all_of(d).into_iter().map(|r| r.root).find(|r| crate::paths::is_file(&request_file(r, id)))
}

/// Whether a question of this session is held for the person, to be answered by a prompt.
pub fn held(data: &Path, sid: &str) -> bool { pending(data).iter().any(|r| r["via"] == "prompt" && r["session"] == sid) }

/// `answer` in the root the ask is in; an ask gone meanwhile (its project closed) takes no answer.
pub fn answer_in(d: &crate::paths::Data, id: &str, decision: &str, answers: &Value) -> Result<()> {
    match root_of(d, id) { Some(r) => answer(&r, id, decision, answers), None => Err("the question is no longer asked".into()) }
}

/// `clear` in the root the ask is in.
pub fn clear_in(d: &crate::paths::Data, id: &str) { if let Some(r) = root_of(d, id) { clear(&r, id); } }

/// The person's answer to one ask: decision allow, deny or terminal; answers by question text.
/// An ask that ended meanwhile (the hook timed out, the agent stopped) takes no answer.
pub fn answer(data: &Path, id: &str, decision: &str, answers: &Value) -> Result<()> {
    let id = valid_id(id)?;
    if !crate::paths::is_file(&request_file(data, id)) { return Err(i18n::t("ask-gone").into()); }
    write_json(&answer_file(data, id), &json!({"decision": decision, "answers": answers}))
}

/// Takes one ask away, answered or not.
pub fn clear(data: &Path, id: &str) {
    let Ok(id) = valid_id(id) else { return };
    // In a project, the entries themselves: never what a link left there points to.
    let _ = crate::paths::remove_file(&request_file(data, id));
    let _ = crate::paths::remove_file(&answer_file(data, id));
    let _ = crate::paths::remove_file(&seen_file(data, id));
    let _ = crate::paths::remove_file(&ping_file(data, id));
}

/// The answer panel says it has read every ask waiting in its roots, shown or not (behind the one shown, or of a
/// session it does not list): it runs. It answers each ask's last ping (`<id>.ping`) with the same word.
pub fn mark_seen_all(d: &crate::paths::Data) {
    for r in crate::roots::all_of(d) {
        for a in pending(&r.root) {
            let Some(id) = a["id"].as_str().and_then(|i| valid_id(i).ok()) else { continue };
            let ping = fs::read_to_string(ping_file(&r.root, id)).unwrap_or_default();
            if fs::read_to_string(seen_file(&r.root, id)).ok().as_deref() != Some(ping.as_str()) {
                let _ = crate::paths::write_atomic(&seen_file(&r.root, id), ping);
                // Taken away meanwhile (answered, cleared): its mark goes too, not left behind.
                if !request_file(&r.root, id).exists() { let _ = crate::paths::remove_file(&seen_file(&r.root, id)); }
            }
        }
    }
}

/// Asked again how often while an ask waits: a panel stuck after it showed the card is found within this and SEEN_MS.
const PING_MS: i64 = 10_000;
fn ping_file(data: &Path, id: &str) -> PathBuf { dir(data).join(format!("{id}.ping")) }

/// Whether the panel has answered `word` (the ask's last ping, or "" for the ask itself).
fn seen(data: &Path, id: &str, word: &str) -> bool { fs::read_to_string(seen_file(data, id)).is_ok_and(|t| t == word) }

/// The panel did not read an ask in time: it holds its lock but does not run. Said in this root's ask folder, the
/// one place every hook can write and Whaybench sees (a project's hook may run in a container that sees no process of
/// the host): `stuck` holds the number of the panel holding the lock. Whaybench, which started that panel, ends it and
/// starts it again the way it was started; the ask goes to the terminal.
fn stuck(data: &Path) {
    use std::os::fd::AsRawFd;
    let held = [panel_file(data), beside(data)].into_iter().find(|p| fs::File::open(p).is_ok_and(|f| {
        // Safety: flock on a descriptor owned here; a shared lock taken is let go when it closes.
        unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) != 0 }
    }));
    let Some(pid) = held.and_then(|p| fs::read_to_string(p).ok()).and_then(|t| t.split_whitespace().next()?.parse::<u32>().ok()) else { return };
    let _ = crate::paths::write_atomic(&dir(data).join("stuck"), pid.to_string());
}

/// The number of the answer panel a hook said is stuck in `root`, if one did: read by Whaybench.
pub fn stuck_panel(root: &Path) -> Option<u32> { crate::paths::read_text(&dir(root).join("stuck")).ok()?.trim().parse().ok() }

/// Waits for the panel to answer `word` for ask `id`, counted from now (after the ask is written: a slow write is not
/// the panel's). True when it did, or the ask was answered or taken away meanwhile, or the panel is one of before the
/// lock (it cannot answer).
fn read_in_time(data: &Path, id: &str, word: &str) -> bool {
    if old_panel(data) { return true; }
    let at = now_ms();
    while now_ms() - at < SEEN_MS {
        if seen(data, id, word) || read_json(&answer_file(data, id), Value::Null).is_ok_and(|a| !a.is_null()) { return true; }
        if !request_file(data, id).exists() { return true; }
        std::thread::sleep(Duration::from_millis(100));
    }
    if seen(data, id, word) { return true; }
    stuck(data);
    false
}

/// Which of a session's asks a moment takes away. Claude Code shows its own dialog beside the card and does not tell
/// the hook when the person answers there, so what is left is cleared when it can no longer be answered.
pub enum Clear<'a> {
    /// The main turn ended (stopped, or idle after an interrupt or a denial): its own asks. A held-back Codex question
    /// waits past it (Codex ends its turn right after holding it), and so does a subagent's ask.
    TurnEnded,
    /// A new prompt: the held-back questions too; a subagent may still wait.
    Prompt,
    /// The session starts or ends: everything.
    All,
    /// A subagent stopped: its asks.
    Subagent(&'a str),
}

pub fn clear_session(data: &Path, sid: &str, which: Clear) {
    for r in fs::read_dir(dir(data)).into_iter().flatten().flatten().filter_map(|f| read_json(&f.path(), Value::Null).ok()) {
        if r["session"] != sid { continue; }
        let (held, sub) = (r["via"] == "prompt", !r["subagent"].is_null());
        let goes = match which {
            Clear::TurnEnded => !held && !sub,
            Clear::Prompt => !sub,
            Clear::All => true,
            Clear::Subagent(a) => r["subagent"] == a,
        };
        if goes && let Some(id) = r["id"].as_str() { clear(data, id); }
    }
}

/// The call ran or failed: it was allowed elsewhere (in the terminal, or by auto mode), so its card goes and its hook
/// ends. Only the oldest same call: another one alike (a parallel subagent's) still waits.
pub fn ran(data: &Path, sid: &str, event: &Value) {
    // A question answered in the terminal comes back with its answers and annotations added (Claude Code 2.1.329).
    let bare = |v: &Value| { let mut v = v.clone(); if let Some(o) = v.as_object_mut() { o.remove("answers"); o.remove("annotations"); } v };
    let input = bare(&event["tool_input"]);
    let mut same: Vec<Value> = fs::read_dir(dir(data)).into_iter().flatten().flatten().filter_map(|f| read_json(&f.path(), Value::Null).ok())
        .filter(|r| r["session"] == sid && r["via"] != "prompt" && r["tool"] == event["tool_name"] && r["subagent"] == event["agent_id"] && bare(&r["input"]) == input)
        .collect();
    same.sort_by_key(|r| r["at"].as_i64().unwrap_or(0));
    if let Some(id) = same.first().and_then(|r| r["id"].as_str()) { clear(data, id); }
}

/// A PermissionRequest inside Whaybench: shows it in the panel and waits. None when the person answers in the
/// terminal or nobody answers in time: the agent then asks there as it would.
pub fn wait(data: &Path, sid: &str, agent: &str, event: &Value) -> Result<Option<Value>> {
    if !panel_open(data) { return Ok(None); }
    let tool = event["tool_name"].as_str().unwrap_or("");
    let input = &event["tool_input"];
    let kind = if tool == "AskUserQuestion" { "question" } else { "permission" };
    let (id, at) = (new_id(sid), now_ms());
    // A subagent's ask (it shares the session id) is told apart, so the main turn's end leaves it.
    write_json(&request_file(data, &id), &json!({"id": id, "session": sid, "agent": agent, "subagent": event["agent_id"], "kind": kind, "tool": tool, "input": input, "at": at, "pid": std::process::id()}))?;
    if !read_in_time(data, &id, "") { clear(data, &id); return Ok(None); }
    // While it waits the panel is asked now and then whether it still runs; its lock gone for a moment (it is started
    // again) is waited out.
    let (mut pinged, mut pings, mut gone_since) = (now_ms(), 0u32, None::<i64>);
    let got = loop {
        if let Ok(a) = read_json(&answer_file(data, &id), Value::Null) && !a.is_null() { break Some(a); }
        // A new prompt took the ask away.
        if !request_file(data, &id).exists() { break None; }
        if now_ms() - at > WAIT_MS { break None; }
        if panel_open(data) { gone_since = None; } else if now_ms() - *gone_since.get_or_insert(now_ms()) > 3000 { break None; }
        if gone_since.is_none() && now_ms() - pinged > PING_MS {
            pings += 1;
            let _ = crate::paths::write_atomic(&ping_file(data, &id), pings.to_string());
            if !read_in_time(data, &id, &pings.to_string()) { break None; }
            pinged = now_ms();
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    clear(data, &id);
    Ok(got.and_then(|a| decision(kind, input, &a)))
}

/// The hook's output for an answer; None for 'terminal'.
fn decision(kind: &str, input: &Value, a: &Value) -> Option<Value> {
    let d = match a["decision"].as_str()? {
        "allow" if kind == "question" => {
            let mut changed = input.clone();
            changed["answers"] = a["answers"].clone();
            json!({"behavior": "allow", "updatedInput": changed})
        }
        "allow" => json!({"behavior": "allow"}),
        "deny" => json!({"behavior": "deny", "message": i18n::ai("ask-denied")}),
        _ => return None,
    };
    Some(json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": d}}))
}

/// Codex's request_user_input: kept for the panel; the hook holds the call back with the returned reason. None
/// when no answer panel is open: the agent asks in the terminal.
pub fn hold(data: &Path, sid: &str, agent: &str, event: &Value) -> Result<Option<String>> {
    if !panel_open(data) { return Ok(None); }
    let id = new_id(sid);
    write_json(&request_file(data, &id), &json!({"id": id, "session": sid, "agent": agent, "kind": "question", "tool": event["tool_name"], "input": event["tool_input"], "at": now_ms(), "via": "prompt"}))?;
    if !read_in_time(data, &id, "") { clear(data, &id); return Ok(None); }
    Ok(Some(i18n::ai("ask-shown").to_string()))
}

/// The prompt that carries a held-back question's answers.
pub fn reply_prompt(answers: &[(String, String)]) -> String {
    let lines: Vec<String> = answers.iter().map(|(q, a)| format!("{q}: {a}")).collect();
    format!("{}\n{}", i18n::ai("ask-reply"), lines.join("\n"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_ask_a_stuck_panel_does_not_read_goes_to_the_terminal_soon() {
        let dir = tempfile::tempdir().unwrap();
        let d = crate::paths::Data::new(dir.path().to_path_buf());
        super::panel_alive(&d);
        let event = serde_json::json!({"tool_name": "Bash", "tool_input": {"command": "ls"}});
        let t = std::time::Instant::now();
        assert!(super::wait(dir.path(), "s1", "claude", &event).unwrap().is_none(), "the terminal asks");
        assert!(t.elapsed() < std::time::Duration::from_secs(6), "within seconds, not ten minutes");
        assert!(super::pending(dir.path()).is_empty(), "the ask is taken away");
        assert_eq!(super::stuck_panel(dir.path()), Some(std::process::id()), "Whaybench is told which panel is stuck");
    }

    #[test]
    fn the_answer_panel_is_open_only_while_it_holds_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!super::panel_open(dir.path()), "no file");
        std::fs::create_dir_all(super::dir(dir.path())).unwrap();
        std::fs::write(super::panel_file(dir.path()), "1795680000000").unwrap();
        assert!(!super::panel_open(dir.path()), "a file nobody holds (the old form, a time long past)");
        std::fs::write(super::panel_file(dir.path()), super::now_ms().to_string()).unwrap();
        assert!(super::panel_open(dir.path()), "an old panel's time, just written");
        std::fs::write(super::panel_file(dir.path()), "1795680000000").unwrap();
        super::panel_alive(&crate::paths::Data::new(dir.path().to_path_buf()));
        assert!(super::panel_open(dir.path()), "held");
    }

    use super::*;

    #[test]
    fn an_answer_becomes_the_hook_decision() {
        let input = json!({"questions": [{"question": "Which?", "options": [{"label": "A"}]}]});
        let allow = decision("question", &input, &json!({"decision": "allow", "answers": {"Which?": "A"}})).unwrap();
        let d = &allow["hookSpecificOutput"]["decision"];
        assert_eq!(d["behavior"], "allow");
        assert_eq!(d["updatedInput"]["answers"]["Which?"], "A");
        assert_eq!(d["updatedInput"]["questions"], input["questions"], "the questions go back with the answers");
        assert!(decision("permission", &json!({}), &json!({"decision": "allow"})).unwrap()["hookSpecificOutput"]["decision"].get("updatedInput").is_none());
        assert_eq!(decision("permission", &json!({}), &json!({"decision": "deny"})).unwrap()["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert!(decision("permission", &json!({}), &json!({"decision": "terminal"})).is_none(), "the terminal asks");
    }

    #[test]
    fn the_hook_takes_the_panels_answer_and_cleans_up() {
        let root = std::env::temp_dir().join(format!("wb-ask-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let event = json!({"tool_name": "Bash", "tool_input": {"command": "ls"}});
        assert!(wait(&root, "s1", "claude", &event).unwrap().is_none(), "no panel: the terminal asks at once");
        assert!(hold(&root, "s2", "codex", &event).unwrap().is_none());
        // The panel's part: it reads what waits.
        let reader_root = root.clone();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop2 = stop.clone();
        let reader = std::thread::spawn(move || while !stop2.load(std::sync::atomic::Ordering::Relaxed) {
            mark_seen_all(&crate::paths::Data::new(reader_root.clone()));
            std::thread::sleep(Duration::from_millis(50));
        });
        panel_alive(&crate::paths::Data::new(root.clone()));
        let r2 = root.clone();
        let answerer = std::thread::spawn(move || loop {
            if let Some(r) = pending(&r2).first() { answer(&r2, r["id"].as_str().unwrap(), "allow", &json!({})).unwrap(); break; }
            std::thread::sleep(Duration::from_millis(50));
        });
        let out = wait(&root, "s1", "claude", &event).unwrap().unwrap();
        answerer.join().unwrap();
        assert_eq!(out["hookSpecificOutput"]["decision"]["behavior"], "allow");
        assert!(pending(&root).is_empty(), "nothing left once answered");
        hold(&root, "s2", "codex", &event).unwrap().unwrap();
        assert_eq!(pending(&root).len(), 1, "a held-back question waits without a process");
        clear_session(&root, "s2", Clear::TurnEnded);
        assert_eq!(pending(&root).len(), 1, "a turn's end leaves a held-back question");
        clear_session(&root, "s2", Clear::Prompt);
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        reader.join().unwrap();
        assert!(pending(&root).is_empty());
        // Two asks of one session at once: each gets its own answer.
        let (a, b) = (root.clone(), root.clone());
        let deny = json!({"tool_name": "Bash", "tool_input": {"command": "rm x"}});
        let first = std::thread::spawn(move || wait(&a, "s3", "claude", &json!({"tool_name": "Bash", "tool_input": {"command": "ls"}})).unwrap());
        let second = std::thread::spawn(move || wait(&b, "s3", "claude", &deny).unwrap());
        let (mut answered, mut tries) = (0, 0);
        while answered < 2 {
            tries += 1;
            assert!(tries < 100, "both asks show");
            for r in pending(&root) {
                let d = if r["input"]["command"] == "ls" { "allow" } else { "deny" };
                answer(&root, r["id"].as_str().unwrap(), d, &json!({})).unwrap();
                answered += 1;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(first.join().unwrap().unwrap()["hookSpecificOutput"]["decision"]["behavior"], "allow");
        assert_eq!(second.join().unwrap().unwrap()["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert!(fs::read_dir(dir(&root)).unwrap().flatten().all(|f| f.file_name() == "panel"), "nothing left behind");
        // The call ran (allowed in the terminal): the oldest same call's hook ends with no decision, and its ask
        // takes no answer; the same call of another waits on.
        let spawn = |root: &PathBuf, sub: Value| { let c = root.clone(); std::thread::spawn(move || wait(&c, "s4", "claude", &json!({"tool_name": "Bash", "tool_input": {"command": "x"}, "agent_id": sub})).unwrap()) };
        let first = spawn(&root, Value::Null);
        while pending(&root).is_empty() { std::thread::sleep(Duration::from_millis(20)); }
        let second = spawn(&root, Value::Null);
        let sub = spawn(&root, json!("a1"));
        while pending(&root).len() < 3 { std::thread::sleep(Duration::from_millis(20)); }
        let id = pending(&root)[0]["id"].as_str().unwrap().to_string();
        ran(&root, "s4", &json!({"tool_name": "Bash", "tool_input": {"command": "other"}}));
        assert_eq!(pending(&root).len(), 3, "another call leaves them");
        ran(&root, "s4", &json!({"tool_name": "Bash", "tool_input": {"command": "x"}}));
        assert!(first.join().unwrap().is_none(), "the oldest one's hook ends");
        assert!(answer(&root, &id, "allow", &json!({})).is_err(), "its ask is over");
        assert_eq!(pending(&root).len(), 2, "the same call of another still waits");
        clear_session(&root, "s4", Clear::TurnEnded);
        assert!(second.join().unwrap().is_none());
        assert_eq!(pending(&root).len(), 1, "a subagent's ask outlives the main turn");
        clear_session(&root, "s4", Clear::Subagent("a1"));
        assert!(sub.join().unwrap().is_none());
        assert!(valid_id(&format!("{}-1-2-3", "a".repeat(128))).is_ok(), "a session id at its limit still makes an ask id");
        let _ = fs::remove_dir_all(&root);
    }
}
