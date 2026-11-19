//! An agent's question or permission ask, answered in the answer panel instead of the terminal. The agent's hook
//! writes `cache/ask/<session>.json` and waits for `cache/ask/<session>.answer.json` (Claude Code and Codex
//! permissions, Claude Code's AskUserQuestion: their PermissionRequest hook decides). Codex's request_user_input
//! cannot be answered from a hook: it is held back and the answer goes to the session as its next prompt.
use std::{fs, path::{Path, PathBuf}, time::Duration};

use serde_json::{Value, json};

use crate::{clock::now_ms, i18n, map::{Result, read_json, safe_id, write_json}};

/// How long a hook waits for the panel; the hooks' own timeout is 600 seconds, and the terminal asks after this.
pub const WAIT_MS: i64 = 590_000;

fn dir(data: &Path) -> PathBuf { data.join("cache").join("ask") }
fn request_file(data: &Path, sid: &str) -> PathBuf { dir(data).join(format!("{sid}.json")) }
fn answer_file(data: &Path, sid: &str) -> PathBuf { dir(data).join(format!("{sid}.answer.json")) }
fn panel_file(data: &Path) -> PathBuf { dir(data).join("panel") }

/// The answer panel says it is open, each second; without it an ask goes to the terminal at once.
pub fn panel_alive(data: &Path) { let _ = crate::paths::write_atomic(&panel_file(data), now_ms().to_string()); }
fn panel_open(data: &Path) -> bool {
    fs::read_to_string(panel_file(data)).ok().and_then(|t| t.trim().parse::<i64>().ok()).is_some_and(|t| now_ms() - t < 5000)
}

/// Asks still waiting: one a hook waits on while its process lives, or one answered by a prompt.
pub fn pending(data: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = fs::read_dir(dir(data)).into_iter().flatten().flatten()
        .filter(|f| f.file_name().to_str().is_some_and(|n| n.ends_with(".json") && !n.ends_with(".answer.json")))
        .filter_map(|f| read_json(&f.path(), Value::Null).ok())
        .filter(|r| r["via"] == "prompt" || r["pid"].as_u64().is_some_and(|p| Path::new(&format!("/proc/{p}")).exists()))
        // Answered, and the hook has not taken it yet.
        .filter(|r| r["session"].as_str().is_none_or(|s| read_json(&answer_file(data, s), Value::Null).map_or(true, |a| a["at"] != r["at"])))
        .collect();
    out.sort_by_key(|r| r["at"].as_i64().unwrap_or(0));
    out
}

/// The person's answer to the ask made at `at`: decision allow, deny or terminal; answers by question text.
pub fn answer(data: &Path, sid: &str, at: i64, decision: &str, answers: &Value) -> Result<()> {
    let sid = safe_id(sid)?;
    write_json(&answer_file(data, sid), &json!({"at": at, "decision": decision, "answers": answers}))
}

pub fn clear(data: &Path, sid: &str) {
    let _ = fs::remove_file(request_file(data, sid));
    let _ = fs::remove_file(answer_file(data, sid));
}

/// A PermissionRequest inside Whaybench: shows it in the panel and waits. None when the person answers in the
/// terminal or nobody answers in time: the agent then asks there as it would.
pub fn wait(data: &Path, sid: &str, agent: &str, event: &Value) -> Result<Option<Value>> {
    if !panel_open(data) { return Ok(None); }
    let tool = event["tool_name"].as_str().unwrap_or("");
    let input = &event["tool_input"];
    let kind = if tool == "AskUserQuestion" { "question" } else { "permission" };
    let at = now_ms();
    clear(data, sid);
    write_json(&request_file(data, sid), &json!({"session": sid, "agent": agent, "kind": kind, "tool": tool, "input": input, "at": at, "pid": std::process::id(), "via": "hook"}))?;
    let got = loop {
        if let Ok(a) = read_json(&answer_file(data, sid), Value::Null) && a["at"] == at { break Some(a); }
        // A new prompt took the ask away, or another ask replaced it.
        if read_json(&request_file(data, sid), Value::Null).map_or(true, |r| r["at"] != at) { break None; }
        if now_ms() - at > WAIT_MS || !panel_open(data) { break None; }
        std::thread::sleep(Duration::from_millis(200));
    };
    clear(data, sid);
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
    write_json(&request_file(data, sid), &json!({"session": sid, "agent": agent, "kind": "question", "tool": event["tool_name"], "input": event["tool_input"], "at": now_ms(), "via": "prompt"}))?;
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
            if let Some(r) = pending(&r2).first() { answer(&r2, "s1", r["at"].as_i64().unwrap(), "allow", &json!({})).unwrap(); break; }
            std::thread::sleep(Duration::from_millis(50));
        });
        let out = wait(&root, "s1", "claude", &event).unwrap().unwrap();
        answerer.join().unwrap();
        assert_eq!(out["hookSpecificOutput"]["decision"]["behavior"], "allow");
        assert!(pending(&root).is_empty(), "nothing left once answered");
        hold(&root, "s2", "codex", &event).unwrap().unwrap();
        assert_eq!(pending(&root).len(), 1, "a held-back question waits without a process");
        clear(&root, "s2");
        assert!(pending(&root).is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
