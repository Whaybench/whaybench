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
fn request_file(data: &Path, id: &str) -> PathBuf { dir(data).join(format!("{id}.json")) }
fn answer_file(data: &Path, id: &str) -> PathBuf { dir(data).join(format!("{id}.answer.json")) }
fn panel_file(data: &Path) -> PathBuf { dir(data).join("panel") }

/// The answer panel says it is open, each second; without it an ask goes to the terminal at once.
pub fn panel_alive(data: &Path) { let _ = crate::paths::write_atomic(&panel_file(data), now_ms().to_string()); }
fn panel_open(data: &Path) -> bool {
    fs::read_to_string(panel_file(data)).ok().and_then(|t| t.trim().parse::<i64>().ok()).is_some_and(|t| now_ms() - t < 5000)
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
    let mut out: Vec<Value> = fs::read_dir(dir(data)).into_iter().flatten().flatten()
        .filter(|f| f.file_name().to_str().is_some_and(|n| n.ends_with(".json") && !n.ends_with(".answer.json")))
        .filter_map(|f| read_json(&f.path(), Value::Null).ok())
        .filter(|r| r["id"].as_str().is_some_and(|i| valid_id(i).is_ok()) && r["session"].as_str().is_some_and(|s| safe_id(s).is_ok()))
        .filter(|r| r["via"] == "prompt" || (r["pid"].as_u64().is_some_and(|p| Path::new(&format!("/proc/{p}")).exists())
            && now_ms() - r["at"].as_i64().unwrap_or(0) < WAIT_MS + 10_000))
        // Answered, and the hook has not taken it yet.
        .filter(|r| !answer_file(data, r["id"].as_str().unwrap_or("")).exists())
        .collect();
    out.sort_by_key(|r| r["at"].as_i64().unwrap_or(0));
    out
}

/// The person's answer to one ask: decision allow, deny or terminal; answers by question text.
/// An ask that ended meanwhile (the hook timed out, the agent stopped) takes no answer.
pub fn answer(data: &Path, id: &str, decision: &str, answers: &Value) -> Result<()> {
    let id = valid_id(id)?;
    if !request_file(data, id).exists() { return Err(i18n::t("ask-gone").into()); }
    write_json(&answer_file(data, id), &json!({"decision": decision, "answers": answers}))
}

/// Takes one ask away, answered or not.
pub fn clear(data: &Path, id: &str) {
    let Ok(id) = valid_id(id) else { return };
    let _ = fs::remove_file(request_file(data, id));
    let _ = fs::remove_file(answer_file(data, id));
}

/// Takes away a session's held-back questions: a new prompt answers or passes over them, a new start or an end
/// leaves none. An ask a hook waits on is not touched: a subagent may still wait on it, and it ends with its hook.
pub fn clear_session(data: &Path, sid: &str) {
    for r in fs::read_dir(dir(data)).into_iter().flatten().flatten().filter_map(|f| read_json(&f.path(), Value::Null).ok()) {
        if r["session"] == sid && r["via"] == "prompt" && let Some(id) = r["id"].as_str() { clear(data, id); }
    }
}

/// A PermissionRequest inside Whaybench: shows it in the panel and waits. None when the person answers in the
/// terminal or nobody answers in time: the agent then asks there as it would.
pub fn wait(data: &Path, sid: &str, agent: &str, event: &Value) -> Result<Option<Value>> {
    if !panel_open(data) { return Ok(None); }
    let tool = event["tool_name"].as_str().unwrap_or("");
    let input = &event["tool_input"];
    let kind = if tool == "AskUserQuestion" { "question" } else { "permission" };
    let (id, at) = (new_id(sid), now_ms());
    write_json(&request_file(data, &id), &json!({"id": id, "session": sid, "agent": agent, "kind": kind, "tool": tool, "input": input, "at": at, "pid": std::process::id()}))?;
    let got = loop {
        if let Ok(a) = read_json(&answer_file(data, &id), Value::Null) && !a.is_null() { break Some(a); }
        // A new prompt took the ask away.
        if !request_file(data, &id).exists() { break None; }
        if now_ms() - at > WAIT_MS || !panel_open(data) { break None; }
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
    Ok(Some(i18n::ai("ask-shown").to_string()))
}

/// The prompt that carries a held-back question's answers.
pub fn reply_prompt(answers: &[(String, String)]) -> String {
    let lines: Vec<String> = answers.iter().map(|(q, a)| format!("{q}: {a}")).collect();
    format!("{}\n{}", i18n::ai("ask-reply"), lines.join("\n"))
}

#[cfg(test)]
mod tests {
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
        panel_alive(&root);
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
        clear_session(&root, "s2");
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
        // A new prompt leaves an ask a hook still waits on; an ended ask takes no answer.
        let c = root.clone();
        let waiting = std::thread::spawn(move || wait(&c, "s4", "claude", &json!({"tool_name": "Bash", "tool_input": {}})).unwrap());
        let id = loop { if let Some(r) = pending(&root).first() { break r["id"].as_str().unwrap().to_string(); } std::thread::sleep(Duration::from_millis(20)); };
        clear_session(&root, "s4");
        answer(&root, &id, "deny", &json!({})).unwrap();
        assert!(waiting.join().unwrap().is_some(), "the hook still got its answer");
        assert!(answer(&root, &id, "allow", &json!({})).is_err(), "the ask is over");
        assert!(valid_id(&format!("{}-1-2-3", "a".repeat(128))).is_ok(), "a session id at its limit still makes an ask id");
        let _ = fs::remove_dir_all(&root);
    }
}
