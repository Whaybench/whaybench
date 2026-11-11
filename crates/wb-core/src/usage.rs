//! Context and plan usage, as Claude Code hands them to its status line: `whaybench statusline` keeps the last of
//! each session in the data folder and prints the line, and the alerts panel shows it under its sections.
use std::{fs, path::Path};

use serde_json::{Value, json};

fn dir(data: &Path) -> std::path::PathBuf { data.join("cache").join("usage") }

/// Keeps what matters of one status line input and returns the line to print. Fields Claude Code leaves out stay out.
pub fn record(data: &Path, input: &Value) -> String {
    let window = |k: &str| input["rate_limits"].get(k).filter(|w| w["used_percentage"].is_number()).map(|w| json!({"used": w["used_percentage"], "resets_at": w["resets_at"]}));
    let kept = json!({"context": input["context_window"]["used_percentage"], "five_hour": window("five_hour"), "seven_day": window("seven_day"), "at": crate::map::now_ms()});
    // Only into a data folder Whaybench made; a status line run elsewhere just prints.
    if let Some(sid) = input["session_id"].as_str().filter(|s| crate::map::safe_id(s).is_ok() && data.is_dir()) {
        if fs::create_dir_all(dir(data)).is_ok() { let _ = fs::write(dir(data).join(format!("{sid}.json")), kept.to_string()); }
        // Older than the weekly window, a session's numbers say nothing any more.
        let week = std::time::Duration::from_secs(7 * 86_400);
        for f in fs::read_dir(dir(data)).into_iter().flatten().flatten() {
            if f.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > week) { let _ = fs::remove_file(f.path()); }
        }
    }
    line(&kept)
}

/// The context of the given session (else the latest), and the plan windows of whichever session heard last:
/// they belong to the account, not the session.
pub fn latest(data: &Path, sid: Option<&str>) -> Option<Value> {
    let mut all: Vec<(String, Value)> = fs::read_dir(dir(data)).ok()?.flatten().filter_map(|f| {
        let v: Value = serde_json::from_str(&fs::read_to_string(f.path()).ok()?).ok()?;
        Some((f.path().file_stem()?.to_string_lossy().to_string(), v))
    }).collect();
    all.sort_by_key(|(_, v)| std::cmp::Reverse(v["at"].as_i64().unwrap_or(0)));
    let mine = sid.and_then(|s| all.iter().find(|(k, _)| k == s)).or(all.first())?;
    // A window whose reset has passed is over, as Claude Code drops it too.
    let now = chrono::Utc::now().timestamp();
    let live = |w: &Value| if w["resets_at"].as_i64().is_some_and(|t| t > now) { w.clone() } else { Value::Null };
    let window = |k: &str| all.iter().map(|(_, v)| live(&v[k])).find(|w| !w.is_null()).unwrap_or(Value::Null);
    Some(json!({"context": mine.1["context"], "five_hour": window("five_hour"), "seven_day": window("seven_day")}))
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
    }
}
