//! Context and plan usage, as Claude Code hands them to its status line: `whaybench statusline` keeps the last of
//! each session in the data folder and prints the line, and the notifications panel shows it under its sections.
use std::{fs, path::{Path, PathBuf}};

use serde_json::{Value, json};

fn dir(data: &Path) -> std::path::PathBuf { data.join("cache").join("usage") }

/// Keeps what matters of one status line input and returns the line to print. Fields Claude Code leaves out stay out.
pub fn record(data: &Path, input: &Value) -> String {
    let window = |k: &str| input["rate_limits"].get(k).filter(|w| w["used_percentage"].is_number()).map(|w| json!({"used": w["used_percentage"], "resets_at": w["resets_at"]}));
    let kept = json!({"context": input["context_window"]["used_percentage"], "five_hour": window("five_hour"), "seven_day": window("seven_day"), "at": crate::map::now_ms()});
    if let Some(sid) = input["session_id"].as_str() { keep(data, sid, &kept); }
    line(&kept)
}

/// One session's numbers. Only into a data folder Whaybench made; a status line run elsewhere just prints.
fn keep(data: &Path, sid: &str, kept: &Value) {
    if crate::map::safe_id(sid).is_ok() && data.is_dir() {
        if fs::create_dir_all(dir(data)).is_ok() { let _ = crate::paths::write_atomic(&dir(data).join(format!("{sid}.json")), kept.to_string()); }
        // Older than the weekly window, a session's numbers say nothing any more.
        let week = std::time::Duration::from_secs(7 * 86_400);
        for f in fs::read_dir(dir(data)).into_iter().flatten().flatten() {
            if f.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > week) { let _ = fs::remove_file(f.path()); }
        }
    }
}

/// Codex has no status line: the last `token_count` line of its session file holds the context and the plan windows.
/// The file is the hook's `transcript_path`, else found by the session number under `$CODEX_HOME/sessions`.
pub fn from_codex_rollout(data: &Path, sid: &str, agent: &str, event: &Value) {
    let path = event["transcript_path"].as_str().map(PathBuf::from)
        .or_else(|| find_named(&PathBuf::from(std::env::var_os("CODEX_HOME")?).join("sessions"), &format!("-{sid}.jsonl"), 4));
    let Some(text) = path.and_then(|p| fs::read_to_string(p).ok()) else { return };
    let Some(count) = text.lines().rev().filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .map(|v| v["payload"].clone()).find(|p| p["type"] == "token_count" && p["info"].is_object()) else { return };
    keep(data, sid, &codex_usage(&count, agent));
}

fn codex_usage(count: &Value, agent: &str) -> Value {
    let info = &count["info"];
    let context = match (info["last_token_usage"]["total_tokens"].as_f64(), info["model_context_window"].as_f64()) {
        (Some(used), Some(all)) if all > 0.0 => json!(used / all * 100.0),
        _ => Value::Null,
    };
    let mut kept = json!({"context": context, "five_hour": null, "seven_day": null, "at": crate::map::now_ms(), "agent": agent});
    for w in ["primary", "secondary"].map(|k| &count["rate_limits"][k]).into_iter().filter(|w| w["used_percent"].is_number()) {
        let key = match w["window_minutes"].as_i64() { Some(300) => "five_hour", Some(10080) => "seven_day", _ => continue };
        kept[key] = json!({"used": w["used_percent"], "resets_at": w["resets_at"]});
    }
    kept
}

fn find_named(dir: &Path, suffix: &str, depth: u32) -> Option<PathBuf> {
    fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let p = e.path();
        if p.is_dir() { (depth > 0).then(|| find_named(&p, suffix, depth - 1)).flatten() }
        else { p.file_name()?.to_str()?.ends_with(suffix).then_some(p) }
    })
}

/// The context of the given session (else the latest), and the plan windows of whichever session of the same agent
/// heard last: they belong to the account, and Claude Code's and Codex's are different accounts. A session not heard
/// from yet has no context of its own.
pub fn latest(data: &Path, sid: Option<&str>) -> Option<Value> {
    let mut all: Vec<(String, Value)> = fs::read_dir(dir(data)).ok()?.flatten().filter_map(|f| {
        let v: Value = serde_json::from_str(&fs::read_to_string(f.path()).ok()?).ok()?;
        Some((f.path().file_stem()?.to_string_lossy().to_string(), v))
    }).collect();
    all.sort_by_key(|(_, v)| std::cmp::Reverse(v["at"].as_i64().unwrap_or(0)));
    let agent_of_session = |s: &str| crate::map::read_json(&data.join("sessions").join(format!("{s}.json")), Value::Null).ok().map(|r| r["agent"].clone());
    let (context, agent) = match sid {
        Some(s) if crate::map::safe_id(s).is_ok() => match all.iter().find(|(k, _)| k == s) {
            Some((_, v)) => (v["context"].clone(), v["agent"].clone()),
            None => (Value::Null, agent_of_session(s).unwrap_or(Value::Null)),
        },
        _ => all.first().map(|(_, v)| (v["context"].clone(), v["agent"].clone()))?,
    };
    // A window whose reset has passed is over, as Claude Code drops it too.
    let now = chrono::Utc::now().timestamp();
    let live = |w: &Value| if w["resets_at"].as_i64().is_some_and(|t| t > now) { w.clone() } else { Value::Null };
    let window = |k: &str| all.iter().filter(|(_, v)| v["agent"] == agent).map(|(_, v)| live(&v[k])).find(|w| !w.is_null()).unwrap_or(Value::Null);
    Some(json!({"context": context, "five_hour": window("five_hour"), "seven_day": window("seven_day")}))
}

/// `Context 42% · 5h 23% resets 15:00 · 7d 41% resets 11-14 09:00`; what is missing is left out.
pub fn line(u: &Value) -> String {
    let pct = |v: &Value| v.as_f64().map(|p| format!("{}", p.round() as i64));
    let when = |w: &Value, fmt: &str| w["resets_at"].as_i64().and_then(|s| chrono::DateTime::from_timestamp(s, 0)).map(|t| t.with_timezone(&chrono::Local).format(fmt).to_string()).unwrap_or_default();
    let mut parts = Vec::new();
    if let Some(p) = pct(&u["context"]) { parts.push(crate::i18n::f("usage-context", &[("p", p)])); }
    for (k, key, fmt) in [("five_hour", "usage-five-hour", "%H:%M"), ("seven_day", "usage-seven-day", "%m-%d %H:%M")] {
        if let Some(p) = pct(&u[k]["used"]) { parts.push(crate::i18n::f(key, &[("p", p), ("at", when(&u[k], fmt))])); }
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_line_is_kept_per_session_and_the_plan_is_the_accounts() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = ("aaaaaaaa-0000-0000-0000-000000000001", "bbbbbbbb-0000-0000-0000-000000000002");
        let soon = chrono::Utc::now().timestamp() + 3600;
        let first = record(dir.path(), &json!({"session_id": a, "context_window": {"used_percentage": 41.6}}));
        assert_eq!(first, "Context 42%", "no plan windows before the first answer");
        std::thread::sleep(std::time::Duration::from_millis(3));
        let second = record(dir.path(), &json!({"session_id": b, "context_window": {"used_percentage": 8},
            "rate_limits": {"five_hour": {"used_percentage": 23.5, "resets_at": soon}, "seven_day": {"used_percentage": 50, "resets_at": 1738425600}}}));
        assert!(second.starts_with("Context 8% · 5h 24% resets "), "{second}");
        let u = latest(dir.path(), Some(a)).unwrap();
        assert_eq!((u["context"].as_f64(), u["five_hour"]["used"].as_f64()), (Some(41.6), Some(23.5)));
        assert!(u["seven_day"].is_null(), "a window whose reset has passed is not shown");
        assert_eq!(record(dir.path(), &json!({"session_id": "../x"})), "", "a bad id is not a path");
        assert!(!dir.path().join("cache/usage/../x.json").exists());
        assert!(latest(dir.path(), Some("cccccccc-0000-0000-0000-000000000003")).unwrap()["context"].is_null(), "a new session does not show another's context");
    }

    #[test]
    fn codex_numbers_come_from_its_session_file_and_stay_apart_from_claude_codes() {
        let dir = tempfile::tempdir().unwrap();
        let soon = chrono::Utc::now().timestamp() + 3600;
        record(dir.path(), &json!({"session_id": "claude-1", "context_window": {"used_percentage": 10}, "rate_limits": {"five_hour": {"used_percentage": 70, "resets_at": soon}}}));
        let file = dir.path().join("rollout.jsonl");
        let count = json!({"type": "event_msg", "payload": {"type": "token_count", "info": {"last_token_usage": {"total_tokens": 47500}, "model_context_window": 475000},
            "rate_limits": {"primary": {"used_percent": 2.0, "window_minutes": 300, "resets_at": soon}, "secondary": {"used_percent": 15.0, "window_minutes": 10080, "resets_at": soon}}}});
        fs::write(&file, format!("{}\n{{\"type\":\"other\"}}\n", count)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        from_codex_rollout(dir.path(), "codex-1", "codex", &json!({"transcript_path": file}));
        fs::create_dir_all(dir.path().join("sessions")).unwrap();
        fs::write(dir.path().join("sessions/codex-1.json"), json!({"agent": "codex"}).to_string()).unwrap();
        let c = latest(dir.path(), Some("codex-1")).unwrap();
        assert_eq!((c["context"].as_f64(), c["five_hour"]["used"].as_f64(), c["seven_day"]["used"].as_f64()), (Some(10.0), Some(2.0), Some(15.0)));
        let a = latest(dir.path(), Some("claude-1")).unwrap();
        assert_eq!(a["five_hour"]["used"].as_f64(), Some(70.0), "Claude Code's plan is not Codex's");
        fs::create_dir_all(dir.path().join("sessions")).unwrap();
        fs::write(dir.path().join("sessions/codex-2.json"), json!({"agent": "codex"}).to_string()).unwrap();
        assert_eq!(latest(dir.path(), Some("codex-2")).unwrap()["five_hour"]["used"].as_f64(), Some(2.0), "a new Codex session shows Codex's plan");
    }
}
