//! What each session has scheduled for itself: Claude Code's prompts on a cron (`CronCreate`) and a self-paced /loop's
//! next wake-up (`ScheduleWakeup`). Claude Code keeps them in the session: `/background` carries them to the new
//! number, `--resume` brings back the cron jobs still due (not a wake-up, not a one-shot whose time has passed), and a
//! recurring job ends 7 days after it was made. The hooks write down what they see the tools do, one file a session.
//! Not map data.
use std::{fs, path::{Path, PathBuf}};

use serde_json::{Value, json};

use crate::clock::now_ms;

const DAY: i64 = 86_400_000;
const WEEK: i64 = 7 * DAY;
/// A job's prompt that comes in this long before its time is the person's own words, not the job firing.
const EARLY: i64 = 120_000;

pub fn dir(root: &Path) -> PathBuf { root.join("cache").join("schedules") }
fn path(root: &Path, sid: &str) -> PathBuf { dir(root).join(format!("{sid}.json")) }

pub fn read(root: &Path, sid: &str) -> Vec<Value> {
    crate::paths::read_text(&path(root, sid)).ok().and_then(|t| serde_json::from_str::<Vec<Value>>(&t).ok()).unwrap_or_default()
}

fn write(root: &Path, sid: &str, jobs: &[Value]) -> std::io::Result<()> {
    if jobs.is_empty() { return crate::paths::remove_file(&path(root, sid)).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }); }
    crate::paths::create_dir_all(&dir(root))?;
    crate::paths::write_atomic(&path(root, sid), Value::from(jobs.to_vec()).to_string())
}

/// A time as Claude Code gives it: milliseconds, with a fraction (1794929053214.331), or RFC 3339 text.
fn time(v: &Value) -> Option<i64> { v.as_f64().map(|t| t as i64).or_else(|| v.as_str().and_then(crate::clock::parse_ms)) }

/// One cron job as the tools report it: `CronCreate`'s result with its input, or a `CronList` entry. A list entry
/// says nothing of `recurring` for a one-shot.
fn cron_job(id: &str, input: &Value, out: &Value, created: i64, recurring_default: bool) -> Value {
    let recurring = out["recurring"].as_bool().or(input["recurring"].as_bool()).unwrap_or(recurring_default);
    let mut j = json!({"id": id, "kind": "cron", "prompt": input["prompt"], "cron": input["cron"], "human": out["humanSchedule"],
        "recurring": recurring, "next": time(&out["nextFireAt"]), "created": created});
    if recurring { j["expires"] = json!(created + WEEK); }
    j
}

/// Follows the session's schedule from one hook event.
pub fn on_event(root: &Path, sid: &str, name: &str, event: &Value) -> crate::map::Result<()> {
    // Tools called side by side run their hooks at once: one at a time, or a job is lost.
    let _lock = crate::map::lock(root, &format!("schedules-{sid}"))?;
    let (input, out) = (&event["tool_input"], &event["tool_response"]);
    let mut jobs = read(root, sid);
    let before = jobs.clone();
    let now = now_ms();
    match (name, event["tool_name"].as_str().unwrap_or("")) {
        // A wake-up lives only while the process does; the cron jobs come back with the conversation.
        ("SessionEnd", _) => jobs.retain(|j| j["kind"] != "wakeup"),
        ("SessionStart", _) if event["source"] == "resume" => jobs.retain(|j| j["kind"] != "wakeup" && standing(j, now)),
        ("PostToolUse", "CronCreate") => if let Some(id) = out["id"].as_str() {
            jobs.retain(|j| j["id"] != id);
            jobs.push(cron_job(id, input, out, now, true));
        },
        ("PostToolUse", "CronDelete") => if let Some(id) = input["id"].as_str().or(out["id"].as_str()) { jobs.retain(|j| j["id"] != id); },
        // The whole list as Claude Code has it, a wake-up among it as a one-shot: it replaces what was written down,
        // keeping when each job was made and last ran. A subagent's list is its own only.
        ("PostToolUse", "CronList") if event.get("agent_id").is_none() => if let Some(list) = out["jobs"].as_array() {
            let old = |id: &str| jobs.iter().find(|j| j["id"] == id).cloned();
            jobs = list.iter().filter_map(|l| {
                let id = l["id"].as_str()?;
                let was = old(id);
                let mut j = cron_job(id, l, l, was.as_ref().and_then(|o| o["created"].as_i64()).unwrap_or(now), false);
                if let Some(last) = was.and_then(|o| o.get("last").cloned()) { j["last"] = last; }
                Some(j)
            }).collect();
        },
        ("PostToolUse", "ScheduleWakeup") => {
            jobs.retain(|j| j["kind"] != "wakeup");
            if input["stop"] != true {
                let delay = input["delaySeconds"].as_f64().unwrap_or(0.0).clamp(60.0, 3600.0) as i64;
                let next = time(&out["scheduledFor"]).unwrap_or(now + delay * 1000);
                jobs.push(json!({"id": "wakeup", "kind": "wakeup", "prompt": input["prompt"], "reason": input["reason"],
                    "recurring": false, "next": next, "created": now}));
            }
        }
        // A job firing comes in as its prompt, at or after its time: a recurring one notes when, a one-shot is done.
        // shortcut: told apart by its words and time only; the same words typed by the person after the time count too.
        ("UserPromptSubmit", _) => if let Some(p) = event["prompt"].as_str().map(str::trim) {
            jobs.retain_mut(|j| {
                let due = j["next"].as_i64().is_none_or(|n| now >= n - EARLY);
                if !due || j["prompt"].as_str().map(str::trim) != Some(p) { return true; }
                j["last"] = json!(now);
                j["recurring"] == true
            });
        },
        _ => {}
    }
    if jobs != before { write(root, sid, &jobs)?; }
    Ok(())
}

/// Whether a job still stands: a recurring one until it expires; a one-shot until it fires (it waits while the
/// session is busy), and at most a day past its time.
fn standing(j: &Value, now: i64) -> bool {
    if j["recurring"] == true { j["expires"].as_i64().is_none_or(|e| e > now) } else { j["next"].as_i64().is_some_and(|n| n + DAY > now) }
}

/// The jobs that still stand.
pub fn live(root: &Path, sid: &str) -> Vec<Value> {
    let now = now_ms();
    read(root, sid).into_iter().filter(|j| standing(j, now)).collect()
}

/// Every session that has a schedule file.
pub fn sessions(root: &Path) -> Vec<String> {
    crate::paths::list_dir(&dir(root)).into_iter()
        .filter_map(|p| p.file_name()?.to_string_lossy().strip_suffix(".json").map(String::from)).collect()
}

/// A conversation moved to a new number takes its schedule along, as Claude Code carries the jobs over; what the new
/// number has written down already stays.
pub fn carry(root: &Path, old: &str, new: &str) {
    if old == new { return; }
    let Ok(_lock) = crate::map::lock(root, &format!("schedules-{new}")) else { return };
    let mut jobs = read(root, new);
    for j in read(root, old) { if !jobs.iter().any(|k| k["id"] == j["id"]) { jobs.push(j); } }
    if write(root, new, &jobs).is_ok() { let _ = crate::paths::remove_file(&path(root, old)); }
}

/// Files with nothing left standing go (a session that ended long ago, never resumed).
pub fn tidy(root: &Path) {
    let now = now_ms();
    for sid in sessions(root) {
        if read(root, &sid).iter().all(|j| !standing(j, now)) { let _ = fs::remove_file(path(root, &sid)); }
    }
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
        // The person typing a job's words before its time does not fire it.
        on_event(r, "s", "UserPromptSubmit", &json!({"prompt": "say hi"})).unwrap();
        assert_eq!(live(r, "s").len(), 2);
        // The list as Claude Code has it: a one-shot comes without `recurring`; a subagent's list is its own.
        let list = json!({"jobs": [{"id": "abe7b236", "cron": "17 */2 * * *", "humanSchedule": "Every 2 hours at :17", "prompt": "check the build", "recurring": true, "nextFireAt": soon},
            {"id": "5a995f6b", "cron": "41 23 31 12 *", "prompt": "say hi", "nextFireAt": soon + 1}]});
        let mut sub = ev("PostToolUse", "CronList", json!({}), json!({"jobs": []}));
        sub["agent_id"] = json!("a1");
        on_event(r, "s", "PostToolUse", &sub).unwrap();
        assert_eq!(live(r, "s").len(), 2, "a subagent's empty list leaves the session's jobs");
        on_event(r, "s", "PostToolUse", &ev("PostToolUse", "CronList", json!({}), list)).unwrap();
        assert_eq!(read(r, "s")[1]["recurring"], false, "a listed one-shot stays one");
        on_event(r, "s", "PostToolUse", &ev("PostToolUse", "CronDelete", json!({"id": "5a995f6b"}), json!({"id": "5a995f6b"}))).unwrap();
        assert_eq!(live(r, "s").len(), 1);
        // A /loop's wake-up at the time Claude Code gives; one at a time; the list shows it too, so the list replaces it.
        on_event(r, "s", "PostToolUse", &ev("PostToolUse", "ScheduleWakeup", json!({"delaySeconds": 5, "prompt": "go on", "reason": "watching CI"}), json!({"scheduledFor": soon + 2}))).unwrap();
        assert_eq!(read(r, "s").iter().find(|j| j["kind"] == "wakeup").unwrap()["next"], soon + 2);
        on_event(r, "s", "PostToolUse", &ev("PostToolUse", "ScheduleWakeup", json!({"stop": true}), json!({}))).unwrap();
        assert!(read(r, "s").iter().all(|j| j["kind"] != "wakeup"));
        on_event(r, "s", "PostToolUse", &ev("PostToolUse", "ScheduleWakeup", json!({"delaySeconds": 5, "prompt": "go on"}), json!({}))).unwrap();
        assert!(read(r, "s").iter().find(|j| j["kind"] == "wakeup").unwrap()["next"].as_i64().unwrap() >= now_ms() + 59_000, "without a time given: at least a minute");
        // The session ends: the wake-up goes, the cron job waits to be resumed.
        on_event(r, "s", "SessionEnd", &json!({})).unwrap();
        assert_eq!(read(r, "s").iter().map(|j| j["id"].as_str().unwrap()).collect::<Vec<_>>(), ["abe7b236"]);
        // Moved to a new number that has a job of its own already: both stand there, the old file goes.
        on_event(r, "t", "PostToolUse", &ev("PostToolUse", "CronCreate", json!({"cron": "1 1 1 1 *", "prompt": "x", "recurring": false}), json!({"id": "n1", "recurring": false, "nextFireAt": soon}))).unwrap();
        carry(r, "s", "t");
        assert_eq!(live(r, "t").len(), 2);
        assert_eq!(sessions(r), ["t"]);
    }

    #[test]
    fn what_no_longer_stands() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        let now = now_ms();
        write(r, "s", &[json!({"id": "late", "kind": "cron", "prompt": "p", "recurring": false, "next": now - 3_600_000}),
            json!({"id": "gone", "kind": "cron", "recurring": false, "next": now - DAY - 1}),
            json!({"id": "old", "kind": "cron", "recurring": true, "expires": now - 1}),
            json!({"id": "wakeup", "kind": "wakeup", "recurring": false, "next": now + 60_000})]).unwrap();
        let ids = |r: &Path| live(r, "s").iter().map(|j| j["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(ids(r), ["late", "wakeup"], "a one-shot waits while the session is busy, a day at most");
        on_event(r, "s", "UserPromptSubmit", &json!({"prompt": "p"})).unwrap();
        assert_eq!(ids(r), ["wakeup"], "fired at last");
        on_event(r, "s", "SessionStart", &json!({"source": "resume"})).unwrap();
        assert!(read(r, "s").is_empty(), "a resumed session has no wake-up and nothing past");
        write(r, "u", &[json!({"id": "old", "kind": "cron", "recurring": true, "expires": now - 1})]).unwrap();
        tidy(r);
        assert!(sessions(r).is_empty());
    }
}
