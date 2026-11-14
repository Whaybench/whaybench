//! The agents' hooks, `whaybench hook claude|<add-on id>`: one handler for the standard events Claude Code and the agent add-ons
//! send (session start, prompt, tool use, notification, stop, end), so the session's record, what the agent is told
//! each turn, its file records and answers are the same whatever the agent. The map tool itself is `whaybench mcp`.
use std::{collections::{HashMap, HashSet}, fs, path::Path};

use serde_json::{Value, json};

use crate::{i18n, map::{self, Result, now_ms, read_json, safe_id, text, write_json}, memory, session_record};

pub const EVENTS: [&str; 10] = ["SessionStart", "UserPromptSubmit", "PreToolUse", "PermissionRequest", "PostToolUse",
    "Notification", "Stop", "Interrupt", "SessionEnd", "PreCompact"];
const EDITS_KEPT: usize = 200;
const TEXT_KEPT: usize = 40_000;

/// Who runs the hook, and whether it runs inside Whaybench (the view panel is there).
pub struct Agent<'a> { pub name: &'a str, pub tab: Option<&'a str>, pub inside: bool }

fn files_dir(data: &Path) -> std::path::PathBuf { data.join("files") }
fn state_file(data: &Path, kind: &str, sid: &str) -> std::path::PathBuf { data.join("cache").join("hook").join(format!("{sid}.{kind}.json")) }

/// One event; the result is what the agent reads on stdout.
pub fn run(data: &Path, a: &Agent, event: &Value) -> Result<Value> {
    let sid = safe_id(text(event, "session_id")?)?;
    let name = text(event, "hook_event_name")?;
    if !EVENTS.contains(&name) { return Err("unsupported hook event".into()); }
    // The session list shows what the person asked: neither commands nor Whaybench's own requests.
    let prompt = if name == "UserPromptSubmit" { Some(text(event, "prompt")?) } else { None };
    let shown = prompt.filter(|p| !p.starts_with('/') && !p.starts_with('[')).map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect::<String>());
    let tab = a.tab.map(safe_id).transpose()?;
    let agent = (a.name != crate::agents::CLAUDE).then_some(a.name);
    // Only a start or a prompt is activity: the other events must not undo what a record says of the past, such as
    // the person having ended the session (the end comes after Whaybench marks it).
    let existing = read_json(&data.join("sessions").join(format!("{sid}.json")), Value::Null)?;
    let mut record = if name == "SessionStart" || name == "UserPromptSubmit" || existing.is_null() {
        session_record::touch(data, sid, session_record::Touch { prompt: shown.as_deref(), tab, cwd: event.get("cwd").and_then(Value::as_str), agent })?
    } else { existing };
    let (state, note) = match name {
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" => ("working", None),
        "PermissionRequest" => ("waiting", None),
        "Notification" if event["notification_type"] != "idle_prompt" => ("waiting", event.get("message").and_then(Value::as_str)),
        // A session can end mid-turn: no Stop comes, so the end settles it.
        "SessionEnd" => ("done", Some("ended")),
        "PreCompact" => ("", None),
        _ => ("done", None),
    };
    if !state.is_empty() { session_record::state(data, sid, state, note)?; }
    // The turn's own question for its answer, unfiltered: the record's lastPrompt leaves out commands and Whaybench's
    // requests (they start with / or [), and the answer to one would be kept under the question before it.
    if let Some(p) = prompt { write_json(&state_file(data, "prompt", sid), &json!(p))?; }
    session_record::alive(data, sid, name != "SessionEnd")?;
    let session_map = map::ensure(data, sid)?;
    if name == "SessionStart" || name == "UserPromptSubmit" {
        // The panel follows latest.json until the person selects a session; a resumed conversation shows its map.
        write_json(&data.join("latest.json"), &session_map)?;
    }
    // The agent's process is this session from now on: the MCP server, its child too, finds the session by it.
    if name == "SessionStart" || name == "UserPromptSubmit" { note_agent_process(data, sid); }
    // The map a tab started from comes in once, when the agent starts; /clear starts a session in the same process.
    let starting = event["source"].as_str().is_none_or(|s| s == "startup");
    let carried = if name == "SessionStart" { adopt_fork(data, sid, a, event).or_else(|| if starting { carry_in(data, sid) } else { None }) } else { None };
    let tool = event["tool_name"].as_str().unwrap_or("");
    match name {
        "Stop" => {
            stop(data, a, sid, event, &mut record)?;
            if a.name != crate::agents::CLAUDE && crate::agents::load(data, a.name).is_ok_and(|g| g.usage.as_deref() == Some("codex-rollout")) {
                crate::usage::from_codex_rollout(data, sid, a.name, event);
            }
        }
        "PreToolUse" if is_shell(tool) => write_json(&state_file(data, "bash", sid), &json!(now_ms()))?,
        "PreToolUse" => if let Some(reason) = held_back(data, sid, a, tool, &event["tool_input"]) {
            return Ok(json!({"hookSpecificOutput": {"hookEventName": name, "permissionDecision": "deny", "permissionDecisionReason": reason}}));
        },
        "PostToolUse" => if let Some(context) = after_tool(data, sid, a, tool, event)? { return Ok(context_out(name, &context)); },
        _ => {}
    }
    if name == "SessionStart" || name == "UserPromptSubmit" {
        let mut parts = Vec::new();
        if a.name != crate::agents::CLAUDE {
            parts.extend(crate::agents::load(data, a.name)?.context(sid)?);
        } else {
            parts.push(i18n::ai("map-note").to_string());
        }
        if a.inside { parts.push(i18n::ai(if name == "SessionStart" { "view-note" } else { "view-short" }).to_string()); }
        if let Some(c) = carried { parts.push(c); }
        // The relay saw /rewind typed: the map did not roll back with the conversation.
        if name == "UserPromptSubmit" && fs::remove_file(state_file(data, "rewound", sid)).is_ok() { parts.push(i18n::ai("rewind-note").to_string()); }
        // The whole memory list when the conversation starts, resumes or was compacted; on a prompt, what changed.
        if name == "SessionStart" {
            let lines = memory::list(data, sid, 100);
            memory::mark_seen(data, sid);
            if !lines.is_empty() { parts.push(i18n::ai_f("memory-whole", &[("lines", lines.join("\n"))])); }
        } else {
            let lines = memory::delta(data, sid);
            if !lines.is_empty() { parts.push(i18n::ai_f("memory-changes", &[("lines", lines.join("\n"))])); }
            if let Some(changed) = changed_files_note(data, sid) { parts.push(changed); }
        }
        return Ok(context_out(name, &parts.join("\n\n")));
    }
    Ok(json!({}))
}

fn context_out(name: &str, context: &str) -> Value { json!({"hookSpecificOutput": {"hookEventName": name, "additionalContext": context}}) }

fn is_shell(tool: &str) -> bool { ["Bash", "shell", "local_shell", "exec_command"].contains(&tool) }

/// The end of a turn: count it once, and keep its answer.
fn stop(data: &Path, a: &Agent, sid: &str, event: &Value, record: &mut Value) -> Result<()> {
    let turn = event.get("turn_id").and_then(Value::as_str);
    let counted = turn.is_some() && record.get("lastCompletedTurn").and_then(Value::as_str) == turn;
    if counted || event.get("stop_hook_active") == Some(&json!(true)) { return Ok(()); }
    map::end_turn(data, sid)?;
    if let Some(turn) = turn {
        record["lastCompletedTurn"] = json!(turn);
        write_json(&data.join("sessions").join(format!("{sid}.json")), record)?;
    }
    let last = event.get("last_assistant_message").and_then(Value::as_str).unwrap_or("");
    // Claude Code's transcript holds the turn's every block of text; the last message alone misses what came before tools.
    let whole = if a.name == "claude" { event.get("transcript_path").and_then(Value::as_str).map(|p| whole_turn(Path::new(p), last)).unwrap_or_default() } else { String::new() };
    let answer = if whole.trim().is_empty() { last.to_string() } else { whole };
    let mut input = json!({"text": answer, "turnId": turn});
    if let Some(p) = read_json(&state_file(data, "prompt", sid), Value::Null)?.as_str() { input["prompt"] = json!(p); }
    if !answer.trim().is_empty() { session_record::answer(data, sid, &input)?; }
    Ok(())
}

/// The text of the turn since the person's last message, from a Claude Code transcript.
pub fn whole_turn(transcript: &Path, last: &str) -> String {
    let asked = |d: &Value| {
        let c = &d["message"]["content"];
        if d["type"] != "user" || d["isMeta"] == true { return false; }
        if let Some(s) = c.as_str() { return !s.starts_with('<'); }
        c.as_array().is_some_and(|x| x.iter().any(|b| b["type"] == "text") && !x.iter().any(|b| b["type"] == "tool_result"))
    };
    // A long conversation's file runs to tens of MB: its end holds the turn, unless the turn alone is longer.
    let parse = |text: String| -> Vec<Value> { text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect() };
    let mut rows = parse(crate::paths::read_tail(transcript, 2 * 1024 * 1024).unwrap_or_default());
    if !rows.iter().any(asked) { rows = parse(fs::read_to_string(transcript).unwrap_or_default()); }
    let start = rows.iter().rposition(asked).unwrap_or(0);
    let parts: Vec<String> = rows[(start + 1).min(rows.len())..].iter().filter(|d| d["type"] == "assistant")
        .filter_map(|d| d["message"]["content"].as_array().map(|c| c.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<String>().trim().to_string()))
        .filter(|t| !t.is_empty()).collect();
    let whole = parts.join("\n\n");
    if whole.is_empty() || whole.ends_with(last.trim()) { whole } else { format!("{whole}\n\n{}", last.trim()) }
}

/// The files a tool call read or changed: (path, "read" | "edit").
fn touched(tool: &str, input: &Value) -> Vec<(String, &'static str)> {
    let path = input.get("file_path").or_else(|| input.get("notebook_path")).and_then(Value::as_str).map(String::from);
    match tool {
        "Read" => path.map(|p| vec![(p, "read")]).unwrap_or_default(),
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => path.map(|p| vec![(p, "edit")]).unwrap_or_default(),
        // Codex changes files with apply_patch: its patch names each file it adds, updates or deletes.
        "apply_patch" => input.to_string().replace("\\n", "\n").lines()
            .filter_map(|l| ["*** Add File: ", "*** Update File: ", "*** Delete File: "].iter().find_map(|h| l.trim_start_matches('"').strip_prefix(h)))
            .map(|p| (p.trim().trim_end_matches('"').to_string(), "edit")).collect(),
        _ => Vec::new(),
    }
}

/// After a tool: its run time when it was a shell command, the files it read or changed with the node the
/// conversation was on, the edit itself, and the decisions that come with a file read for the first time.
fn after_tool(data: &Path, sid: &str, a: &Agent, tool: &str, event: &Value) -> Result<Option<String>> {
    if is_shell(tool) {
        let from = read_json(&state_file(data, "bash", sid), json!(now_ms()))?.as_u64().unwrap_or(now_ms());
        let file = files_dir(data).join(format!("{sid}.bash.json"));
        let mut runs = read_json(&file, json!([]))?.as_array().cloned().unwrap_or_default();
        runs.push(json!([from, now_ms()]));
        let skip = runs.len().saturating_sub(500);
        write_json(&file, &json!(runs[skip..]))?;
        return Ok(None);
    }
    let input = &event["tool_input"];
    let mut told = None;
    for (path, op) in touched(tool, input) {
        let path = if Path::new(&path).is_absolute() { path } else { Path::new(event["cwd"].as_str().unwrap_or("")).join(&path).to_string_lossy().to_string() };
        let node = map::command(data, sid, None).ok().and_then(|m| m["current"].as_str().map(String::from));
        let file = files_dir(data).join(format!("{sid}.json"));
        let mut touches = read_json(&file, json!([]))?.as_array().cloned().unwrap_or_default();
        touches.push(json!({"path": path, "op": op, "at": now_ms(), "node": node}));
        let skip = touches.len().saturating_sub(1000);
        write_json(&file, &json!(touches[skip..]))?;
        memory::file_touched(data, sid, op, &path, node.as_deref())?;
        if a.name == "claude" && (tool == "Edit" || tool == "Write") {
            let cut = |t: &str| if t.chars().count() > TEXT_KEPT { format!("{}\n…", t.chars().take(TEXT_KEPT).collect::<String>()) } else { t.to_string() };
            let (before, after, whole) = if tool == "Edit" { (input["old_string"].as_str().unwrap_or(""), input["new_string"].as_str().unwrap_or(""), false) }
                else { (event["tool_response"]["originalFile"].as_str().unwrap_or(""), input["content"].as_str().unwrap_or(""), true) };
            let edits_file = files_dir(data).join(format!("{sid}.edits.json"));
            let mut edits = read_json(&edits_file, json!([]))?.as_array().cloned().unwrap_or_default();
            edits.push(json!({"path": path, "at": now_ms(), "node": node, "before": cut(before), "after": cut(after), "whole": whole}));
            let skip = edits.len().saturating_sub(EDITS_KEPT);
            write_json(&edits_file, &json!(edits[skip..]))?;
        }
        if op == "read" {
            let told_file = state_file(data, "decisions", sid);
            let mut seen: Vec<String> = serde_json::from_value(read_json(&told_file, json!([]))?).unwrap_or_default();
            if !seen.contains(&path) {
                seen.push(path.clone());
                write_json(&told_file, &json!(seen))?;
                let lines = decisions_for(data, sid, &path);
                if !lines.is_empty() { told = Some(i18n::ai_f("file-decisions", &[("path", path.clone()), ("lines", lines.join("\n"))])); }
            }
        }
    }
    Ok(told)
}

/// The sessions whose maps this one carries on, itself first.
fn lineage(data: &Path, sid: &str) -> Vec<String> {
    let links = read_json(&data.join("forks").join("inherited.json"), json!({})).unwrap_or(json!({}));
    let mut ids = vec![sid.to_string()];
    while let Some(next) = links[ids.last().unwrap().as_str()].as_str() { if ids.len() > 30 || ids.iter().any(|i| i == next) { break; } ids.push(next.to_string()); }
    ids
}

/// `*` within a name, `**` across folders, `?` one character.
fn glob(pattern: &str, path: &str) -> bool {
    fn go(p: &[u8], s: &[u8]) -> bool {
        match p {
            [] => s.is_empty(),
            [b'*', b'*', rest @ ..] => (0..=s.len()).any(|i| go(rest, &s[i..])),
            [b'*', rest @ ..] => (0..=s.len()).take_while(|i| *i == 0 || s[i - 1] != b'/').any(|i| go(rest, &s[i..])),
            [b'?', rest @ ..] => !s.is_empty() && s[0] != b'/' && go(rest, &s[1..]),
            [c, rest @ ..] => s.first() == Some(c) && go(rest, &s[1..]),
        }
    }
    go(pattern.as_bytes(), path.as_bytes())
}

/// The settled decisions of the branches a file belongs to: attached to a node, or touched while the talk was on it.
pub fn decisions_for(data: &Path, sid: &str, path: &str) -> Vec<String> {
    let ids = lineage(data, sid);
    let mut nodes_of: HashSet<String> = HashSet::new();
    for id in &ids {
        let attached = read_json(&data.join("node-files").join(format!("{id}.json")), json!({})).unwrap_or(json!({}));
        for (node, patterns) in attached.as_object().into_iter().flatten() {
            if patterns.as_array().into_iter().flatten().filter_map(Value::as_str).any(|p| glob(p, path)) { nodes_of.insert(node.clone()); }
        }
        for t in read_json(&files_dir(data).join(format!("{id}.json")), json!([])).unwrap_or(json!([])).as_array().into_iter().flatten() {
            if t["path"] == path { if let Some(n) = t["node"].as_str() { nodes_of.insert(n.to_string()); } }
        }
    }
    let nodes = map::command(data, sid, None).ok().and_then(|m| m["nodes"].as_array().cloned()).unwrap_or_default();
    let by: HashMap<&str, &Value> = nodes.iter().filter_map(|n| Some((n["id"].as_str()?, n))).collect();
    let decision = |id: &str| by.get(id).is_some_and(|n| !map::is_open(n) && n["kind"] == "conclusion");
    let parent = |id: &str| by.get(id).and_then(|n| n["parent"].as_str()).unwrap_or("").to_string();
    let children = |id: &str| nodes.iter().filter(|c| c["parent"] == id).filter_map(|c| c["id"].as_str().map(String::from)).collect::<Vec<_>>();
    let mut found: Vec<String> = Vec::new();
    let mut add = |id: &str| if decision(id) && !found.iter().any(|f| f == id) { found.push(id.to_string()); };
    for id in &nodes_of {
        if !by.contains_key(id.as_str()) { continue; }
        add(id);
        // Below a top-level branch: its settled children, and its siblings' settled ends (the same part of the work).
        let up = parent(id);
        if by.contains_key(up.as_str()) {
            let mut stack = vec![(id.clone(), 0)];
            while let Some((pid, depth)) = stack.pop() {
                for c in children(&pid) { add(&c); if depth < 4 { stack.push((c, depth + 1)); } }
            }
            if by.contains_key(parent(&up).as_str()) { for c in children(&up) { add(&c); } }
        }
        let mut p = up;
        for _ in 0..10 { if !by.contains_key(p.as_str()) { break; } add(&p); p = parent(&p); }
    }
    found.iter().take(15).map(|id| format!("- {}", by[id.as_str()]["label"].as_str().unwrap_or(""))).collect()
}

/// Files this session read or changed that changed on disk after its last touch, and not while one of its shell
/// commands ran: most likely the person saved them. Each change once: `only` limits it to one file.
fn changed_since(data: &Path, sid: &str, only: Option<&str>) -> Vec<(String, u64)> {
    let mut last: HashMap<String, u64> = HashMap::new();
    for t in read_json(&files_dir(data).join(format!("{sid}.json")), json!([])).unwrap_or(json!([])).as_array().into_iter().flatten() {
        let (Some(p), Some(at)) = (t["path"].as_str(), t["at"].as_u64()) else { continue };
        if only.is_none_or(|o| o == p) { let e = last.entry(p.to_string()).or_default(); *e = (*e).max(at); }
    }
    let runs: Vec<(u64, u64)> = read_json(&files_dir(data).join(format!("{sid}.bash.json")), json!([])).unwrap_or(json!([])).as_array().into_iter().flatten()
        .filter_map(|r| Some((r[0].as_u64()?, r[1].as_u64()?))).collect();
    let told = read_json(&state_file(data, "changes", sid), json!({})).unwrap_or(json!({}));
    let mut out = Vec::new();
    for (path, touched) in last {
        let Ok(at) = fs::metadata(&path).and_then(|m| m.modified()) else { continue };
        let at = at.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let own = runs.iter().any(|(f, t)| at + 500 >= *f && at <= t + 1500);
        if at > touched + 1500 && told[&path].as_u64() != Some(at) && !own { out.push((path, at)); }
    }
    out
}

fn tell_changes(data: &Path, sid: &str, changes: &[(String, u64)]) {
    let file = state_file(data, "changes", sid);
    let mut told = read_json(&file, json!({})).unwrap_or(json!({}));
    for (p, at) in changes { told[p] = json!(at); }
    let _ = write_json(&file, &told);
}

fn clock(ms: u64) -> String { chrono::DateTime::from_timestamp_millis(ms as i64).map(|t| t.with_timezone(&chrono::Local).format("%H:%M:%S").to_string()).unwrap_or_default() }

/// For the next prompt: files the person changed since the agent read or changed them.
fn changed_files_note(data: &Path, sid: &str) -> Option<String> {
    let changes = changed_since(data, sid, None);
    if changes.is_empty() { return None; }
    tell_changes(data, sid, &changes);
    let lines: Vec<String> = changes.iter().map(|(p, at)| {
        let vim = vim_holding(p).map(|(_, dirty)| format!(", still open in vim{}", if dirty { " with unsaved changes" } else { "" })).unwrap_or_default();
        format!("- {p} (changed at {}{vim})", clock(*at))
    }).collect();
    Some(i18n::ai_f("files-changed", &[("lines", lines.join("\n"))]))
}

/// A file open in vim has a swap file beside it naming vim's pid (bytes 24..28) and whether it has unsaved changes.
fn vim_holding(path: &str) -> Option<(u32, bool)> {
    let p = Path::new(path);
    let (dir, base) = (p.parent()?, p.file_name()?.to_string_lossy().to_string());
    for ext in ["swp", "swo", "swn"] {
        let Ok(b) = fs::read(dir.join(format!(".{base}.{ext}"))) else { continue };
        if b.len() < 28 { continue; }
        let pid = u32::from_le_bytes([b[24], b[25], b[26], b[27]]);
        if pid != 0 && Path::new(&format!("/proc/{pid}")).exists() { return Some((pid, b.get(1007) == Some(&b'U'))); }
    }
    None
}

/// Before an edit: held back when the file changed on disk since the agent's last look, and the first time it is open in vim.
fn held_back(data: &Path, sid: &str, a: &Agent, tool: &str, input: &Value) -> Option<String> {
    // shortcut: Claude Code's edit tools only; Codex's apply_patch is not held back until its deny output is checked.
    if a.name != "claude" || !["Edit", "MultiEdit", "Write", "NotebookEdit"].contains(&tool) { return None; }
    let path = input.get("file_path").or_else(|| input.get("notebook_path")).and_then(Value::as_str)?;
    if let Some(change) = changed_since(data, sid, Some(path)).into_iter().next() {
        tell_changes(data, sid, std::slice::from_ref(&change));
        return Some(i18n::ai_f("edit-changed", &[("path", path.to_string()), ("at", clock(change.1))]));
    }
    let (pid, dirty) = vim_holding(path)?;
    let file = state_file(data, "vim", sid);
    let mut warned: Vec<String> = serde_json::from_value(read_json(&file, json!([])).ok()?).unwrap_or_default();
    let key = format!("{path}:{pid}");
    if warned.contains(&key) { return None; }
    warned.push(key);
    let _ = write_json(&file, &json!(warned));
    Some(i18n::ai_f("edit-in-vim", &[("path", path.to_string()), ("pid", pid.to_string()), ("dirty", (if dirty { "with unsaved changes" } else { "no unsaved changes" }).to_string())]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(sid: &str, name: &str, extra: Value) -> Value {
        let mut e = json!({"session_id": sid, "hook_event_name": name, "cwd": "/w"});
        for (k, v) in extra.as_object().unwrap() { e[k] = v.clone(); }
        e
    }

    #[test]
    fn an_answer_is_kept_with_its_own_question_even_a_request() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        let a = Agent { name: "codex", tab: None, inside: true };
        let sid = "s-ask";
        let ev = |name: &str, extra: Value| { let mut e = json!({"session_id": sid, "hook_event_name": name, "cwd": "w"}); for (k, v) in extra.as_object().unwrap() { e[k] = v.clone(); } e };
        crate::addons::install_bundled(&crate::paths::Data::new(data.to_path_buf()));
        run(data, &a, &ev("UserPromptSubmit", json!({"prompt": "install it"}))).unwrap();
        run(data, &a, &ev("Stop", json!({"last_assistant_message": "installed", "turn_id": "t1"}))).unwrap();
        run(data, &a, &ev("UserPromptSubmit", json!({"prompt": "[from another session] does read work?"}))).unwrap();
        run(data, &a, &ev("Stop", json!({"last_assistant_message": "it works", "turn_id": "t2"}))).unwrap();
        let log = fs::read_to_string(data.join("answer-log/s-ask.jsonl")).unwrap();
        let rows: Vec<Value> = log.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!((rows[0]["prompt"].as_str(), rows[1]["prompt"].as_str()), (Some("install it"), Some("[from another session] does read work?")));
        assert_eq!(read_json(&data.join("sessions/s-ask.json"), Value::Null).unwrap()["lastPrompt"], "install it", "the list still leaves requests out");
    }

    #[test]
    fn a_claude_session_is_recorded_told_and_kept_from_clashing() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        let a = Agent { name: "claude", tab: Some("t1"), inside: true };
        let sid = "aaaaaaaa-0000-0000-0000-000000000001";
        let start = run(data, &a, &ev(sid, "SessionStart", json!({"source": "startup"}))).unwrap();
        let told = start["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
        assert!(told.contains("call the context_map tool") && told.contains("view panel beside this conversation"), "{told}");
        run(data, &a, &ev(sid, "UserPromptSubmit", json!({"prompt": "/clear"}))).unwrap();
        assert!(read_json(&data.join("sessions").join(format!("{sid}.json")), Value::Null).unwrap().get("firstPrompt").is_none(), "commands are not what was asked");
        // A decision attached to the files of a branch comes with the first read of such a file.
        map::command(data, sid, Some(json!({"upsert": [{"id": "t", "label": "App"}, {"id": "b", "parent": "t", "label": "Store"}, {"id": "d", "parent": "b", "label": "Store → SQLite", "kind": "conclusion", "state": "done"}], "current": "b"}))).unwrap();
        let file = data.join("f.rs");
        fs::write(&file, "x").unwrap();
        let path = file.to_string_lossy().to_string();
        write_json(&data.join("node-files").join(format!("{sid}.json")), &json!({"b": [format!("{}/*.rs", data.display())]})).unwrap();
        let read = |r: &Agent| run(data, r, &ev(sid, "PostToolUse", json!({"tool_name": "Read", "tool_input": {"file_path": path}}))).unwrap();
        let first = read(&a);
        assert!(first["hookSpecificOutput"]["additionalContext"].as_str().unwrap().contains("- Store → SQLite"), "{first}");
        assert_eq!(read(&a), json!({}), "told once per file");
        let touches = read_json(&data.join("files").join(format!("{sid}.json")), json!([])).unwrap();
        assert_eq!((touches[0]["op"].as_str(), touches[0]["node"].as_str()), (Some("read"), Some("b")));
        // Changed on disk after the read, not by a shell command of the agent: the next edit is held back once.
        std::thread::sleep(std::time::Duration::from_millis(1600));
        fs::write(&file, "person").unwrap();
        let edit = ev(sid, "PreToolUse", json!({"tool_name": "Edit", "tool_input": {"file_path": path, "old_string": "x", "new_string": "y"}}));
        let held = run(data, &a, &edit).unwrap();
        assert_eq!(held["hookSpecificOutput"]["permissionDecision"], "deny");
        assert!(held["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap().contains("was changed on disk"));
        assert_eq!(run(data, &a, &edit).unwrap(), json!({}), "the same change is told once");
        // Open in vim: held back the first time only.
        let mut swap = vec![0u8; 1008];
        swap[24..28].copy_from_slice(&std::process::id().to_le_bytes());
        swap[1007] = b'U';
        fs::write(data.join(".f.rs.swp"), swap).unwrap();
        assert!(run(data, &a, &edit).unwrap()["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap().contains("open in vim"));
        assert_eq!(run(data, &a, &edit).unwrap(), json!({}));
        run(data, &a, &ev(sid, "PostToolUse", json!({"tool_name": "Edit", "tool_input": {"file_path": path, "old_string": "x", "new_string": "y"}}))).unwrap();
        assert_eq!(read_json(&data.join("files").join(format!("{sid}.edits.json")), json!([])).unwrap()[0]["after"], "y");
        // The turn's whole text comes from the transcript, not only its last block.
        let transcript = data.join("t.jsonl");
        fs::write(&transcript, [json!({"type": "user", "message": {"content": "do it"}}), json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "First part."}]}}),
            json!({"type": "user", "message": {"content": [{"type": "tool_result"}]}}), json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "Done."}]}})]
            .iter().map(|r| r.to_string()).collect::<Vec<_>>().join("\n")).unwrap();
        run(data, &a, &ev(sid, "Stop", json!({"transcript_path": transcript, "last_assistant_message": "Done."}))).unwrap();
        let answer = read_json(&data.join("answers").join(format!("{sid}.json")), Value::Null).unwrap();
        assert!(answer.to_string().contains("First part.\\n\\nDone."), "{answer}");
    }

    #[test]
    fn a_session_the_person_ended_stays_ended() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        let a = Agent { name: "claude", tab: Some("t1"), inside: true };
        let sid = "aaaaaaaa-0000-0000-0000-000000000002";
        run(data, &a, &ev(sid, "SessionStart", json!({}))).unwrap();
        let d = crate::paths::Data::new(data.to_path_buf());
        crate::sessions::mark_ended(&d, sid);
        for name in ["Stop", "SessionEnd"] { run(data, &a, &ev(sid, name, json!({}))).unwrap(); }
        assert_eq!(read_json(&data.join("sessions").join(format!("{sid}.json")), Value::Null).unwrap()["ended"], true, "the end does not undo the person's end");
        run(data, &a, &ev(sid, "UserPromptSubmit", json!({"prompt": "again"}))).unwrap();
        assert!(read_json(&data.join("sessions").join(format!("{sid}.json")), Value::Null).unwrap().get("ended").is_none(), "talked to again: active");
    }

    #[test]
    fn a_tab_started_from_a_map_takes_it_once() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        let a = Agent { name: "claude", tab: None, inside: true };
        let from = "aaaaaaaa-0000-0000-0000-00000000000f";
        map::command(data, from, Some(json!({"upsert": [{"id": "t", "label": "Kept"}]}))).unwrap();
        // shortcut: the process environment is shared by the tests; only this test sets the variable.
        unsafe { std::env::set_var("WHAYBENCH_MAP_FROM", from) };
        let (first, cleared) = ("aaaaaaaa-0000-0000-0000-0000000000a1", "aaaaaaaa-0000-0000-0000-0000000000a2");
        run(data, &a, &ev(first, "SessionStart", json!({"source": "startup"}))).unwrap();
        run(data, &a, &ev(cleared, "SessionStart", json!({"source": "clear"}))).unwrap();
        unsafe { std::env::remove_var("WHAYBENCH_MAP_FROM") };
        assert!(!map_is_empty(data, first), "the tab's first session takes the map");
        assert!(map_is_empty(data, cleared), "/clear in the same process does not take it again");
    }

    #[test]
    fn the_session_is_found_by_the_agent_process() {
        let dir = tempfile::tempdir().unwrap();
        assert!(agent_process().is_some(), "a process above the tests");
        assert_eq!(session_of_agent(dir.path()), None);
        note_agent_process(dir.path(), "s1");
        assert_eq!(session_of_agent(dir.path()).as_deref(), Some("s1"), "a hook and the MCP server under the same agent agree");
    }

    #[test]
    fn codex_patches_name_their_files_and_globs_match_folders() {
        let patch = json!({"input": "*** Begin Patch\n*** Update File: src/a.rs\n@@\n*** Add File: b.md\n*** End Patch"});
        assert_eq!(touched("apply_patch", &patch), vec![("src/a.rs".to_string(), "edit"), ("b.md".to_string(), "edit")]);
        assert!(glob("/w/src/**", "/w/src/a/b.rs") && glob("/w/*.rs", "/w/a.rs") && !glob("/w/*.rs", "/w/a/b.rs") && glob("a?c", "abc"));
    }
}

/// The map in short for a session that took it over: none of the conversation behind it comes along.
pub fn carried_summary(m: &Value, from: &str) -> String {
    let nodes: Vec<&Value> = m["nodes"].as_array().into_iter().flatten().collect();
    let by: HashMap<&str, &Value> = nodes.iter().filter_map(|n| Some((n["id"].as_str()?, *n))).collect();
    let mut path = Vec::new();
    let mut at = m["current"].as_str().and_then(|c| by.get(c));
    while let Some(n) = at { if path.len() > 50 { break; } path.insert(0, n["label"].as_str().unwrap_or("").to_string()); at = n["parent"].as_str().and_then(|p| by.get(p)); }
    let pick = |f: &dyn Fn(&Value) -> bool, cap: usize| { let v: Vec<&&Value> = nodes.iter().filter(|n| f(n)).collect(); let skip = v.len().saturating_sub(cap); v[skip..].iter().map(|n| {
        let why = n["detail"].as_str().or(n["reason"].as_str()).map(|r| format!(" (reason: {})", r.split(['.', '\n']).next().unwrap_or("").trim())).unwrap_or_default();
        format!("- {}{why}", n["label"].as_str().unwrap_or("")) }).collect::<Vec<_>>().join("\n") };
    let state = |n: &Value| n["state"].as_str().unwrap_or("").to_string();
    let parts = [
        i18n::ai_f("carried-head", &[("from", from.to_string())]),
        if path.is_empty() { String::new() } else { i18n::ai_f("carried-position", &[("path", path.join(" › "))]) },
        Some(pick(&|n| map::is_open(n) && state(n) != "plan" && state(n) != "doing", 20)).filter(|l| !l.is_empty()).map(|l| i18n::ai_f("carried-open", &[("lines", l)])).unwrap_or_default(),
        Some(pick(&|n| map::is_open(n) && state(n) == "plan", 10)).filter(|l| !l.is_empty()).map(|l| i18n::ai_f("carried-later", &[("lines", l)])).unwrap_or_default(),
        Some(pick(&|n| !map::is_open(n) && n["kind"] == "conclusion", 20)).filter(|l| !l.is_empty()).map(|l| i18n::ai_f("carried-decided", &[("lines", l)])).unwrap_or_default(),
    ];
    parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join("\n\n")
}

fn map_is_empty(data: &Path, sid: &str) -> bool {
    map::command(data, sid, None).ok().and_then(|m| m["nodes"].as_array().map(Vec::is_empty)).unwrap_or(true)
}

/// A tab started from another map (`WHAYBENCH_MAP_FROM`: a saved map's file, or a session id): it becomes this
/// session's map. The program's first prompt (`WHAYBENCH_MAP_PROMPT`) goes to the tab with the summary, for the
/// relay to type; without one the summary is told at the start.
fn carry_in(data: &Path, sid: &str) -> Option<String> {
    let from = std::env::var("WHAYBENCH_MAP_FROM").ok().filter(|f| !f.is_empty())?;
    if !map_is_empty(data, sid) { return None; }
    let label = if from.starts_with('/') {
        let body = map::view_file(data, Path::new(&from)).ok()?;
        if body["nodes"].as_array().is_none_or(Vec::is_empty) { return None; }
        // A part of a session's map passed on in a display style keeps where that conversation was.
        if body.get("style").is_some() {
            map::take_over(data, sid, &body).ok()?;
            let _ = crate::paths::write_atomic(&crate::memory::part_carried(data, sid), body["style"].as_str().unwrap_or(""));
        } else { map::start_from(data, sid, &body).ok()?; }
        format!("imported map '{}'", body["title"].as_str().unwrap_or(&from))
    } else {
        let source = read_json(&data.join(format!("{}.json", safe_id(&from).ok()?)), Value::Null).ok()?;
        source.get("nodes")?;
        crate::map_tool::call(data, sid, &json!({"inherit": from})).ok()?;
        i18n::ai_f("carried-session", &[("id", from.chars().take(8).collect())])
    };
    let summary = carried_summary(&map::command(data, sid, None).ok()?, &label);
    match std::env::var("WHAYBENCH_MAP_PROMPT").ok().filter(|p| !p.is_empty()) {
        // No tab to type it in yet: the summary is told at the start instead.
        Some(asked) if crate::sessions::queue_prompt(&crate::paths::Data::new(data.to_path_buf()), sid, &format!("{asked}\n\n{summary}")) => None,
        _ => Some(summary),
    }
}

fn fork_note(data: &Path) -> std::path::PathBuf { data.join("cache").join("forks").join("pending.json") }

/// The relay saw /background: the conversation goes on in a new session, which takes this map over when it starts.
pub fn note_fork(data: &Path, from: &str) {
    let cwd = read_json(&data.join("sessions").join(format!("{from}.json")), Value::Null).ok().and_then(|r| r["cwd"].as_str().map(String::from)).unwrap_or_default();
    let _ = write_json(&fork_note(data), &json!({"from": from, "cwd": cwd, "at": now_ms()}));
}

/// The relay saw /rewind: the next prompt asks the agent to set the map straight.
pub fn note_rewind(data: &Path, sid: &str) { let _ = write_json(&state_file(data, "rewound", sid), &json!(now_ms())); }

/// A session started within a minute of a /background in the same folder, with a conversation of its own, carries
/// that session on: its map and record. Claude Code's spare processes start with an id but no conversation.
fn adopt_fork(data: &Path, sid: &str, a: &Agent, event: &Value) -> Option<String> {
    if a.name != "claude" { return None; }
    let note = read_json(&fork_note(data), Value::Null).ok()?;
    let from = note["from"].as_str()?.to_string();
    let cwd = event["cwd"].as_str().unwrap_or("");
    let near = |x: &str, y: &str| x == y || x.starts_with(&format!("{}/", y.trim_end_matches('/'))) || y.starts_with(&format!("{}/", x.trim_end_matches('/')));
    if from == sid || now_ms().saturating_sub(note["at"].as_u64().unwrap_or(0)) > 60_000 || !near(note["cwd"].as_str().unwrap_or(""), cwd) { return None; }
    let talked = event["transcript_path"].as_str().and_then(|p| fs::metadata(p).ok()).is_some_and(|m| m.len() > 0);
    if !talked { return None; }
    let _ = write_json(&fork_note(data), &json!({}));
    crate::map_tool::call(data, sid, &json!({"inherit": from})).ok()?;
    crate::session_record::carry_from(data, sid, &from).ok()?;
    // The carried record has no tab; this session is in the tab it started in.
    crate::session_record::touch(data, sid, crate::session_record::Touch { prompt: None, tab: a.tab, cwd: None, agent: None }).ok()?;
    None
}

/// One process: its name, parent and start time (`/proc/<pid>/stat`).
fn proc_stat(pid: u32) -> Option<(String, u32, u64)> {
    let text = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (open, close) = (text.find('(')?, text.rfind(')')?);
    let rest: Vec<&str> = text[close + 1..].split_whitespace().collect();
    Some((text[open + 1..close].to_string(), rest.get(1)?.parse().ok()?, rest.get(19)?.parse().ok()?))
}

/// The agent this program runs under: the nearest process above it that is not a shell. A hook runs through a
/// shell, the MCP server straight from the agent, so both arrive at the same process. Its id and start time
/// (an id can be used again later).
pub fn agent_process() -> Option<(u32, u64)> {
    const SHELLS: [&str; 6] = ["sh", "bash", "dash", "zsh", "env", "timeout"];
    let mut pid = std::os::unix::process::parent_id();
    for _ in 0..8 {
        let (comm, parent, start) = proc_stat(pid)?;
        if !SHELLS.contains(&comm.as_str()) { return Some((pid, start)); }
        pid = parent;
    }
    None
}

fn agent_file(data: &Path, pid: u32) -> std::path::PathBuf { data.join("cache").join("agent-process").join(format!("{pid}.json")) }

fn note_agent_process(data: &Path, sid: &str) {
    let Some((pid, start)) = agent_process() else { return };
    let _ = write_json(&agent_file(data, pid), &json!({"session": sid, "start": start}));
    // A note outlives its process; a week on, nothing asks for it.
    let week = std::time::Duration::from_secs(7 * 86_400);
    for f in fs::read_dir(data.join("cache").join("agent-process")).into_iter().flatten().flatten() {
        if f.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > week) { let _ = fs::remove_file(f.path()); }
    }
}

/// The session of the agent this program runs under, as its hooks last noted it.
pub fn session_of_agent(data: &Path) -> Option<String> {
    let (pid, start) = agent_process()?;
    let noted = read_json(&agent_file(data, pid), Value::Null).ok()?;
    (noted["start"].as_u64() == Some(start)).then(|| noted["session"].as_str().map(String::from)).flatten()
}
