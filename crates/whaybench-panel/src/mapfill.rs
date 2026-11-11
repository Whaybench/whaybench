//! The side worker of a map style: fills fields the style needs and the nodes lack, outside the
//! conversation. It asks a small model once, without tools or plugins and saving nothing, and leaves a patch the map mod
//! applies; the panel redraws when the map file changes.
use std::{fs, io::Write, path::{Path, PathBuf}, process::{Command, Stdio}, time::Instant};

use serde_json::{Value, json};
use wb_core::{log as record, paths::Data, sessions};
use wb_store::{arr, mapstyles::{self, Style}, s};

const TRANSCRIPT_LIMIT: usize = 90_000;

fn transcript_path(id: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    fs::read_dir(Path::new(&home).join(".claude/projects")).ok()?.flatten().map(|e| e.path().join(format!("{id}.jsonl"))).find(|p| p.is_file())
}

/// The conversation as text: the person's and the AI's words, each cut short, the end of it kept.
pub fn transcript_text(raw: &str) -> String {
    let mut parts = Vec::new();
    for line in raw.lines() {
        let Ok(o) = serde_json::from_str::<Value>(line) else { continue };
        let kind = s(&o, "type");
        if !["user", "assistant"].contains(&kind) || o.get("isMeta") == Some(&Value::Bool(true)) { continue; }
        let content = o.get("message").and_then(|m| m.get("content")).cloned().unwrap_or_default();
        let text = match &content {
            Value::String(t) => t.clone(),
            Value::Array(a) => a.iter().filter(|x| s(x, "type") == "text").map(|x| s(x, "text").to_string()).collect::<Vec<_>>().join(" "),
            _ => String::new(),
        };
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() || text.starts_with('<') { continue; }
        parts.push(format!("{}: {}", if kind == "user" { wb_core::i18n::t("user") } else { "AI" }, text.chars().take(2000).collect::<String>()));
    }
    let whole = parts.join("\n");
    let count = whole.chars().count();
    whole.chars().skip(count.saturating_sub(TRANSCRIPT_LIMIT)).collect()
}

/// Only what was asked for: known nodes, known fields.
pub fn clean_answer(text: &str, need: &[(String, Vec<String>)]) -> Option<Vec<Value>> {
    let (from, to) = (text.find('[')?, text.rfind(']')?);
    let items: Vec<Value> = serde_json::from_str(text.get(from..=to)?).ok()?;
    let wanted: Vec<&String> = need.iter().flat_map(|(_, ids)| ids).collect();
    Some(items.into_iter().filter(|i| wanted.iter().any(|w| w.as_str() == s(i, "id"))).map(|i| {
        let mut out = json!({"id": s(&i, "id")});
        for (f, ids) in need {
            if ids.iter().any(|x| x == s(&i, "id")) { if let Some(v) = i.get(f) { out[f] = v.clone(); } }
        }
        out
    }).collect())
}

/// Runs in a thread of its own. What happened goes in the record.
pub fn run(d: &Data, session: &str, style: &Style, scope: Option<&str>) {
    let log = |level_warn: bool, text: String| {
        let fields = [(wb_core::i18n::t("sessions"), session.chars().take(8).collect::<String>()), ("style", style.name.clone())];
        if level_warn { record::warn("map", "map fill", &text, &fields); } else { record::decide("map", "map fill", &text, &fields); }
    };
    let Some(data) = sessions::map_of(d, session) else { log(true, "no map".into()); return };
    let nodes = mapstyles::scoped(&data, scope);
    let need = mapstyles::missing(style, &nodes, s(&data, "current"));
    if need.is_empty() { log(false, "nothing missing".into()); return; }
    let all = arr(&data, "nodes");
    let label_of = |id: &str| all.iter().find(|n| s(n, "id") == id).map(|n| s(n, "label").to_string()).unwrap_or_default();
    let wanted: Vec<&String> = need.iter().flat_map(|(_, ids)| ids).collect();
    let listing: Vec<String> = all.iter().filter(|n| wanted.iter().any(|w| w.as_str() == s(n, "id"))).map(|n| {
        let parent = label_of(s(n, "parent"));
        format!("- {}: {}{}  [{}]", s(n, "id"), s(n, "label"), if parent.is_empty() { String::new() } else { wb_core::i18n::f("parent", &[("parent", (parent).to_string())]) }, if !wb_core::map::is_open(n) { wb_core::i18n::t("done") } else { wb_core::i18n::t("open-3") })
    }).collect();
    let asks: Vec<String> = need.iter().map(|(f, ids)| wb_core::i18n::f("nodes", &[("f", (f).to_string()), ("a1", (mapstyles::filled_hint(f)).to_string()), ("a2", (ids.join(", ")).to_string())])).collect();
    let talk = transcript_path(session).and_then(|p| fs::read_to_string(p).ok()).map(|t| transcript_text(&t)).unwrap_or_default();
    let prompt = wb_core::i18n::f("output-only-the-json-array-as", &[("talk", (talk).to_string()), ("a1", (listing.join("\n")).to_string()), ("a2", (asks.join("\n")).to_string())]);
    // An empty folder: no project settings, so no plugins load.
    let work = d.cache().join("fill-work");
    let _ = fs::create_dir_all(&work);
    let started = Instant::now();
    let child = Command::new(sessions::claude_program()).args(["-p", "--model", "haiku", "--tools", "", "--no-session-persistence", "--setting-sources", "project", "--strict-mcp-config", "--output-format", "text", "--system-prompt", wb_core::i18n::t("a-worker-that-fills-conversation-map")])
        .current_dir(&work).env_remove("WHAYBENCH").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let mut child = match child { Ok(c) => c, Err(e) => { log(true, format!("run failed: {e}")); return } };
    if let Some(mut input) = child.stdin.take() { let _ = input.write_all(prompt.as_bytes()); }
    let pid = child.id();
    // Four minutes at most.
    std::thread::spawn(move || { std::thread::sleep(std::time::Duration::from_secs(240)); let _ = Command::new("kill").arg(pid.to_string()).stderr(Stdio::null()).status(); });
    let out = match child.wait_with_output() { Ok(o) => o, Err(e) => { log(true, format!("run failed: {e}")); return } };
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let Some(items) = clean_answer(&text, &need) else {
        log(true, format!("answer not understood · {} {}", text.chars().take(200).collect::<String>(), String::from_utf8_lossy(&out.stderr).chars().take(200).collect::<String>()));
        return;
    };
    let dir = d.root.join("map-patches");
    let _ = fs::create_dir_all(&dir);
    let tmp = dir.join(format!(".{session}.tmp"));
    if fs::write(&tmp, Value::Array(items.clone()).to_string()).is_ok() { let _ = fs::rename(&tmp, dir.join(format!("{session}.json"))); }
    log(false, format!("{}/{} nodes written · {}s", items.len(), wanted.len(), started.elapsed().as_secs()));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_conversation_leaves_out_tools_and_the_program() {
        let raw = [
            r#"{"type":"user","message":{"content":"first question"}}"#,
            r#"{"type":"user","isMeta":true,"message":{"content":"hidden"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"the  answer"},{"type":"tool_use","name":"Bash"}]}}"#,
            r#"{"type":"user","message":{"content":"<command-name>/x</command-name>"}}"#,
            "not json",
        ].join("\n");
        assert_eq!(transcript_text(&raw), "User: first question\nAI: the answer");
    }
    #[test]
    fn an_answer_keeps_only_what_was_asked() {
        let need = vec![("kind".to_string(), vec!["a".to_string()]), ("reason".to_string(), vec!["b".to_string()])];
        let items = clean_answer("here: [{\"id\":\"a\",\"kind\":\"note\",\"reason\":\"x\"},{\"id\":\"b\",\"reason\":\"y\",\"label\":\"changed\"},{\"id\":\"z\",\"kind\":\"note\"}] end", &need).unwrap();
        assert_eq!(items, vec![json!({"id":"a","kind":"note"}), json!({"id":"b","reason":"y"})]);
        assert!(clean_answer("no answer", &need).is_none());
    }
}
