//! What each session has scheduled for itself: Claude Code's prompts on a cron (`CronCreate`, gone with the session,
//! a recurring one after 7 days) and a /loop's next wake-up (`ScheduleWakeup`). They live only in the session, so
//! the hooks write them down as they see the tools used, one file a session. Not map data.
use std::{fs, path::{Path, PathBuf}};

use serde_json::{Value, json};

use crate::clock::now_ms;

const WEEK: i64 = 7 * 86_400_000;
/// A one-shot past its time by this much has fired or is gone, though no prompt was seen to say so.
const GRACE: i64 = 10 * 60_000;

pub fn dir(root: &Path) -> PathBuf { root.join("cache").join("schedules") }
fn path(root: &Path, sid: &str) -> PathBuf { dir(root).join(format!("{sid}.json")) }

pub fn read(root: &Path, sid: &str) -> Vec<Value> {
    fs::read_to_string(path(root, sid)).ok().and_then(|t| serde_json::from_str::<Vec<Value>>(&t).ok()).unwrap_or_default()
}

fn write(root: &Path, sid: &str, jobs: &[Value]) -> std::io::Result<()> {
    if jobs.is_empty() { return fs::remove_file(path(root, sid)).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }); }
    fs::create_dir_all(dir(root))?;
    crate::paths::write_atomic(&path(root, sid), Value::from(jobs.to_vec()).to_string())
}

/// One cron job as the tools report it (`CronCreate`'s result with its input, or a `CronList` entry).
fn cron_job(id: &str, input: &Value, out: &Value, created: i64) -> Value {
    let recurring = out["recurring"].as_bool().or(input["recurring"].as_bool()).unwrap_or(true);
    // Claude Code gives the time with a fraction of a millisecond (1794929053214.331).
    let next = out["nextFireAt"].as_f64().map(|t| t as i64);
    let mut j = json!({"id": id, "kind": "cron", "prompt": input["prompt"], "cron": input["cron"], "human": out["humanSchedule"],
        "recurring": recurring, "next": next, "created": created});
    if recurring { j["expires"] = json!(created + WEEK); }
    j
}

/// Follows the session's schedule from one hook event.
pub fn on_event(root: &Path, sid: &str, name: &str, event: &Value) -> std::io::Result<()> {
    if name == "SessionEnd" { return write(root, sid, &[]); }
    let (input, out) = (&event["tool_input"], &event["tool_response"]);
    let mut jobs = read(root, sid);
    let before = jobs.clone();
    let now = now_ms();
    match (name, event["tool_name"].as_str().unwrap_or("")) {
        ("PostToolUse", "CronCreate") => if let Some(id) = out["id"].as_str() {
            jobs.retain(|j| j["id"] != id);
            jobs.push(cron_job(id, input, out, now));
        },
        ("PostToolUse", "CronDelete") => if let Some(id) = input["id"].as_str().or(out["id"].as_str()) { jobs.retain(|j| j["id"] != id); },
        // The whole list as Claude Code has it: what it no longer lists is gone, what it lists has its next time.
        ("PostToolUse", "CronList") => if let Some(list) = out["jobs"].as_array() {
            let kept = |id: &str| jobs.iter().find(|j| j["id"] == id).cloned();
            let mut fresh: Vec<Value> = jobs.iter().filter(|j| j["kind"] != "cron").cloned().collect();
            for l in list {
                let Some(id) = l["id"].as_str() else { continue };
                let old = kept(id);
                let mut j = cron_job(id, l, l, old.as_ref().and_then(|o| o["created"].as_i64()).unwrap_or(now));
                if let Some(last) = old.and_then(|o| o.get("last").cloned()) { j["last"] = last; }
                fresh.push(j);
            }
            jobs = fresh;
        },
        ("PostToolUse", "ScheduleWakeup") => {
            jobs.retain(|j| j["kind"] != "wakeup");
            if input["stop"] != true {
                let delay = input["delaySeconds"].as_f64().unwrap_or(0.0).clamp(60.0, 3600.0) as i64;
                jobs.push(json!({"id": "wakeup", "kind": "wakeup", "prompt": input["prompt"], "reason": input["reason"],
                    "recurring": false, "next": now + delay * 1000, "created": now}));
            }
        }
        // A job firing comes in as its prompt: a recurring one notes the time, a one-shot is done.
        ("UserPromptSubmit", _) => if let Some(p) = event["prompt"].as_str().map(str::trim) {
            jobs.retain_mut(|j| {
                if j["prompt"].as_str().map(str::trim) != Some(p) { return true; }
                j["last"] = json!(now);
                j["recurring"] == true
            });
        },
        _ => {}
    }
    if jobs != before { write(root, sid, &jobs)?; }
    Ok(())
}

/// The jobs that still stand: a one-shot until a little past its time, a recurring one until it expires.
pub fn live(root: &Path, sid: &str) -> Vec<Value> {
    let now = now_ms();
    read(root, sid).into_iter().filter(|j| if j["recurring"] == true {
        j["expires"].as_i64().is_none_or(|e| e > now)
    } else {
        j["next"].as_i64().is_some_and(|n| n + GRACE > now)
    }).collect()
}

/// Every session that has a schedule file.
pub fn sessions(root: &Path) -> Vec<String> {
    fs::read_dir(dir(root)).into_iter().flatten().flatten()
        .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".json").map(String::from)).collect()
}

/// A conversation moved to a new number takes its schedule along, as Claude Code carries the jobs over.
pub fn carry(root: &Path, old: &str, new: &str) {
    if path(root, new).exists() || !path(root, old).exists() { return; }
    let _ = fs::rename(path(root, old), path(root, new));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(name: &str, tool: &str, input: Value, out: Value) -> Value {
        json!({"hook_event_name": name, "tool_name": tool, "tool_input": input, "tool_response": out})
    }

    #[test]
    fn a_sessions_schedule_follows_the_tools() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        let soon = now_ms() + 3_600_000;
        // Shapes as Claude Code 2.1.326 sends them.
        on_event(r, "s", "PostToolUse", &ev("PostToolUse", "CronCreate", json!({"cron": "17 */2 * * *", "prompt": "check the build", "recurring": true}),
            json!({"id": "abe7b236", "humanSchedule": "Every 2 hours at :17", "recurring": true, "durable": false, "nextFireAt": soon as f64 + 0.331}))).unwrap();
        on_event(r, "s", "PostToolUse", &ev("PostToolUse", "CronCreate", json!({"cron": "41 23 31 12 *", "prompt": "say hi", "recurring": false}),
            json!({"id": "5a995f6b", "humanSchedule": "41 23 31 12 *", "recurring": false, "durable": false, "nextFireAt": soon + 1}))).unwrap();
        assert_eq!(live(r, "s").len(), 2);
        assert_eq!(read(r, "s")[0]["next"], soon, "a time with a fraction of a millisecond, as Claude Code sends it");
        assert!(read(r, "s")[0]["expires"].as_i64().is_some() && read(r, "s")[1].get("expires").is_none(), "only a recurring job expires");
        on_event(r, "s", "PostToolUse", &ev("PostToolUse", "CronDelete", json!({"id": "5a995f6b"}), json!({"id": "5a995f6b"}))).unwrap();
        assert_eq!(live(r, "s").len(), 1);
        // It fires: the recurring job notes the time and stays.
        on_event(r, "s", "UserPromptSubmit", &json!({"prompt": "check the build"})).unwrap();
        assert!(read(r, "s")[0]["last"].as_i64().is_some());
        // The list as Claude Code has it now: a job it no longer lists is gone, the time it fired is kept.
        on_event(r, "s", "PostToolUse", &ev("PostToolUse", "CronList", json!({}), json!({"jobs": [{"id": "abe7b236", "cron": "17 */2 * * *",
            "humanSchedule": "Every 2 hours at :17", "prompt": "check the build", "recurring": true, "nextFireAt": soon + 7_200_000}]}))).unwrap();
        let j = &read(r, "s")[0];
        assert!(j["next"] == soon + 7_200_000 && j["last"].as_i64().is_some());
        // A /loop's wake-up, at least a minute away, one at a time; stopping it takes it off.
        on_event(r, "s", "PostToolUse", &ev("PostToolUse", "ScheduleWakeup", json!({"delaySeconds": 5, "prompt": "go on", "reason": "watching CI"}), json!({}))).unwrap();
        let w = read(r, "s").into_iter().find(|j| j["kind"] == "wakeup").unwrap();
        assert!(w["next"].as_i64().unwrap() >= now_ms() + 59_000);
        on_event(r, "s", "PostToolUse", &ev("PostToolUse", "ScheduleWakeup", json!({"stop": true}), json!({}))).unwrap();
        assert!(read(r, "s").iter().all(|j| j["kind"] != "wakeup"));
        carry(r, "s", "t");
        assert_eq!(live(r, "t").len(), 1, "the jobs go along with a moved conversation");
        on_event(r, "t", "SessionEnd", &json!({})).unwrap();
        assert!(read(r, "t").is_empty() && sessions(r).is_empty());
    }

    #[test]
    fn a_one_shot_long_past_its_time_is_not_shown() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(r, "s", &[json!({"id": "a", "kind": "cron", "recurring": false, "next": now_ms() - GRACE - 1}),
            json!({"id": "b", "kind": "cron", "recurring": true, "expires": now_ms() - 1})]).unwrap();
        assert!(live(r, "s").is_empty(), "fired or gone, and an expired recurring job");
    }
}
