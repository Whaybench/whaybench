//! Codex CLI as the agent of a run (`whaybench --agent codex`). Codex runs no Claude Code mod,
//! so its hooks call `whaybench hook codex`, which writes the same shared files the mod writes: the
//! session record, its state, the heartbeat, the answer, and the map turn.
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use serde_json::{Value, json};

use crate::{i18n, map::{self, Result, now_ms, read_json, safe_id, text, write_json}, session_record};

/// Set for everything a Codex run starts: the terminal relay, the panels and the hooks.
pub const ENV_AGENT: &str = "WHAYBENCH_AGENT";
const EVENTS: [&str; 8] = ["SessionStart", "UserPromptSubmit", "PreToolUse", "PermissionRequest",
    "PostToolUse", "Stop", "Interrupt", "SessionEnd"];

pub fn active() -> bool {
    std::env::var(ENV_AGENT).is_ok_and(|a| a == "codex")
}

/// Arguments for the Codex CLI: a session coming back is `codex resume ID`, as Codex has no attach.
pub fn args(args: &[String]) -> Vec<String> {
    let mut out: Vec<String> = ["--no-daemon", "--sandbox", "workspace-write"].map(String::from).into();
    if args.len() == 2 && (args[0] == "--resume" || args[0] == "attach") {
        out.extend(["resume".into(), args[1].clone()]);
    } else { out.extend(args.iter().cloned()); }
    out
}

fn shell_quote(path: &Path) -> Result<String> {
    let path = path.to_str().ok_or("command path is not UTF-8")?;
    Ok(format!("'{}'", path.replace('\'', "'\\''")))
}

/// One Codex hook event (stdin of `whaybench hook codex`); the result is what Codex reads on stdout.
pub fn hook(data: &Path, event: &Value, tab: Option<&str>, exe: &Path) -> Result<Value> {
    let sid = safe_id(text(event, "session_id")?)?;
    let name = text(event, "hook_event_name")?;
    if !EVENTS.contains(&name) { return Err("unsupported hook event".into()); }
    let prompt = if name == "UserPromptSubmit" { Some(text(event, "prompt")?) } else { None };
    let at = now_ms();
    let tab = tab.map(safe_id).transpose()?;
    let mut record = session_record::touch(data, sid, session_record::Touch { prompt, tab, cwd: Some(text(event, "cwd")?), agent: Some("codex") })?;
    let state = match name {
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" => "working",
        "PermissionRequest" => "waiting",
        _ => "done",
    };
    session_record::state(data, sid, state, None)?;
    session_record::alive(data, sid, name != "SessionEnd")?;
    let map_path = data.join(format!("{sid}.json"));
    if !map_path.exists() {
        write_json(&map_path, &json!({"sessionId":sid,"nodes":[],"current":null,"version":2,"turn":0,"updatedAt":at}))?;
    }
    if name == "SessionStart" || name == "UserPromptSubmit" {
        // The panel follows latest.json until the person selects a session; a resumed conversation
        // publishes its existing map rather than replacing its nodes.
        let m = read_json(&map_path, Value::Null)?;
        map::validate(&m)?;
        write_json(&data.join("latest.json"), &m)?;
    }
    if name == "Stop" {
        let turn = event.get("turn_id").and_then(Value::as_str);
        let already_counted = turn.is_some() && record.get("lastCompletedTurn").and_then(Value::as_str) == turn;
        if !already_counted && event.get("stop_hook_active") != Some(&json!(true)) {
            map::end_turn(data, sid)?;
            if let Some(turn) = turn {
                record["lastCompletedTurn"] = json!(turn);
                write_json(&data.join("sessions").join(format!("{sid}.json")), &record)?;
            }
        }
        if let Some(answer) = event.get("last_assistant_message").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            // shortcut: records the final assistant message; collect all blocks when full-turn history is added.
            session_record::answer(data, sid, &json!({"text": answer, "turnId": turn}))?;
        }
    }
    if name == "SessionStart" || name == "UserPromptSubmit" {
        let command = shell_quote(exe)?;
        let vars = [("session", sid.to_string()), ("command", command)];
        let context = format!("{}\n{}", i18n::ai_f("codex-context", &vars), i18n::ai_f("codex-map-context", &vars));
        return Ok(json!({"hookSpecificOutput":{"hookEventName":name,"additionalContext":context}}));
    }
    Ok(json!({}))
}

fn hook_config(exe: &Path) -> Result<Value> {
    let command = format!("{} hook codex", shell_quote(exe)?);
    let mut hooks = serde_json::Map::new();
    for event in EVENTS {
        hooks.insert(event.into(), json!([{"hooks":[{"type":"command","command":command,"timeout":3}]}]));
    }
    Ok(json!({"hooks":hooks}))
}

fn model_config(source: &Path) -> Result<String> {
    let source = match fs::read_to_string(source.join("config.toml")) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let source: toml::Table = source.parse()?;
    let mut config = toml::Table::new();
    for key in ["model", "model_reasoning_effort"] {
        if let Some(value) = source.get(key) { config.insert(key.into(), value.clone()); }
    }
    Ok(toml::to_string(&config)?)
}

fn create_private(path: &Path, bytes: &[u8]) -> Result<()> {
    match OpenOptions::new().write(true).create_new(true).mode(0o600).open(path) {
        Ok(mut file) => { file.write_all(bytes)?; Ok(()) },
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Codex's own folder for this program, `<data>/codex`, used as CODEX_HOME: the hooks are written there,
/// not in the person's ~/.codex, so Codex used elsewhere is not changed. The login and the model choice
/// are copied from `source` once; the hook definitions are written each time, which Codex asks to trust.
pub fn prepare(data: &Path, source: &Path, exe: &Path) -> Result<PathBuf> {
    let home = data.join("codex");
    fs::DirBuilder::new().recursive(true).mode(0o700).create(&home)?;
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700))?;
    let auth = home.join("auth.json");
    if !auth.exists() {
        match fs::read(source.join("auth.json")) {
            Ok(bytes) => create_private(&auth, &bytes)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {},
            Err(e) => return Err(e.into()),
        }
    }
    let config = home.join("config.toml");
    if !config.exists() { create_private(&config, model_config(source)?.as_bytes())?; }
    write_json(&home.join("hooks.json"), &hook_config(exe)?)?;
    Ok(home)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(name: &str) -> Value {
        json!({"session_id":"codex-1","hook_event_name":name,"cwd":"work"})
    }

    fn call(data: &Path, event: &Value) -> Result<Value> {
        hook(data, event, Some("tab1"), &data.join("whaybench"))
    }


    #[test]
    fn invalid_events_and_ids_do_not_write_records() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for id in ["../original", "/tmp/file", "", "a/b", "a.b"] {
            let mut e = event("SessionStart");
            e["session_id"] = json!(id);
            assert!(call(dir.path(), &e).is_err());
        }
        assert!(call(dir.path(), &event("UnknownEvent")).is_err());
        assert!(call(dir.path(), &event("UserPromptSubmit")).is_err());
        assert_eq!(fs::read_dir(dir.path())?.count(), 0);
        Ok(())
    }

    #[test]
    fn preparation_writes_hooks_and_keeps_the_private_login() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let data = dir.path().join("data ' é");
        let source = dir.path().join("source");
        fs::create_dir_all(&source)?;
        fs::write(source.join("auth.json"), "test-login")?;
        fs::write(source.join("config.toml"), "model = 'chosen-model'\nmodel_reasoning_effort = 'high'\n[plugins.other]\nenabled = true\n")?;
        let exe = dir.path().join("bin/whaybench");
        let home = prepare(&data, &source, &exe)?;
        assert_eq!(home, data.join("codex"));
        assert_eq!(fs::metadata(&home)?.permissions().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(home.join("auth.json"))?.permissions().mode() & 0o777, 0o600);
        let config: toml::Table = fs::read_to_string(home.join("config.toml"))?.parse()?;
        assert_eq!(config["model"].as_str(), Some("chosen-model"));
        assert_eq!(config.len(), 2);
        fs::write(home.join("auth.json"), "refreshed-login")?;
        prepare(&data, &source, &exe)?;
        assert_eq!(fs::read_to_string(home.join("auth.json"))?, "refreshed-login");
        let hooks = read_json(&home.join("hooks.json"), Value::Null)?;
        assert_eq!(hooks["hooks"].as_object().unwrap().len(), EVENTS.len());
        for name in EVENTS {
            assert_eq!(hooks["hooks"][name][0]["hooks"][0]["command"], format!("{} hook codex", shell_quote(&exe)?));
        }
        Ok(())
    }

    #[test]
    fn corrupt_existing_records_are_reported_and_preserved() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("sessions/codex-1.json");
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, "{broken")?;
        assert!(call(dir.path(), &event("SessionStart")).is_err());
        assert_eq!(fs::read_to_string(path)?, "{broken");
        Ok(())
    }

    #[test]
    fn codex_arguments_keep_the_sandbox_and_resume_natively() {
        let a = args(&["--resume".into(), "sid".into()]);
        assert_eq!(a, ["--no-daemon", "--sandbox", "workspace-write", "resume", "sid"]);
        assert_eq!(args(&[]).len(), 3);
        assert!(!a.iter().any(|s| s.contains("bypass")));
    }
}
