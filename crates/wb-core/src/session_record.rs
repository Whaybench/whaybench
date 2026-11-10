//! What a session leaves for the panels besides its map: its record, its state, its
//! heartbeat and its answers. Every agent writes them through here (`whaybench record`, or the Codex hook
//! directly), so the files keep one form whichever agent wrote them.
use std::{fs::{self, OpenOptions}, io::Write, path::Path};

use serde_json::{Value, json};

use crate::map::{Result, now_ms, read_json, safe_id, write_json};

fn record_path(data: &Path, sid: &str) -> Result<std::path::PathBuf> {
    Ok(data.join("sessions").join(format!("{}.json", safe_id(sid)?)))
}

pub fn read(data: &Path, sid: &str) -> Result<Value> {
    read_json(&record_path(data, sid)?, Value::Null)
}

/// What a touch may set; what it leaves out keeps its value.
#[derive(Default)]
pub struct Touch<'a> { pub prompt: Option<&'a str>, pub tab: Option<&'a str>, pub cwd: Option<&'a str>, pub agent: Option<&'a str> }

/// The session is active now: the first prompt stays, the last one is this, and the tab it is in. Fields that
/// only described a past state (ended, a carried empty session) go.
pub fn touch(data: &Path, sid: &str, t: Touch) -> Result<Value> {
    let path = record_path(data, sid)?;
    let old = read_json(&path, Value::Null)?;
    if !old.is_null() && !old.is_object() { return Err("session record must be an object".into()); }
    let keep = |k: &str| old.get(k).filter(|v| !v.is_null()).cloned();
    let mut r = json!({"id": sid});
    let cwd = keep("cwd").or(t.cwd.map(|c| json!(c)));
    let first = keep("firstPrompt").or(t.prompt.map(|p| json!(p)));
    let last = t.prompt.map(|p| json!(p)).or(keep("lastPrompt"));
    let tab = match t.tab { Some(x) => Some(json!(safe_id(x)?)), None => keep("tab") };
    // Only a new record starts now; one from before starts were kept leaves it to its answers.
    let started = keep("started").or(old.is_null().then(|| json!(now_ms())));
    for (k, v) in [("cwd", cwd), ("firstPrompt", first), ("lastPrompt", last), ("started", started), ("lastActive", Some(json!(now_ms()))), ("tab", tab)] {
        if let Some(v) = v { r[k] = v; }
    }
    if let Some(a) = t.agent { r["agent"] = json!(a); }
    for k in ["agent", "movedTo", "lastCompletedTurn"] {
        if r.get(k).is_none() { if let Some(v) = keep(k) { r[k] = v; } }
    }
    write_json(&path, &r)?;
    Ok(r)
}

/// working, waiting (on the person) or done; a note is written even when the state is the same.
pub fn state(data: &Path, sid: &str, state: &str, note: Option<&str>) -> Result<()> {
    if !["working", "waiting", "done"].contains(&state) { return Err("state is working, waiting or done".into()); }
    let mut v = json!({"state": state, "at": now_ms()});
    if let Some(n) = note { v["note"] = json!(n); }
    write_json(&data.join("cache/state").join(format!("{}.json", safe_id(sid)?)), &v)
}

/// The heartbeat: fresh while a process holds the session, "0" once none does.
pub fn alive(data: &Path, sid: &str, on: bool) -> Result<()> {
    let path = data.join("cache/alive").join(safe_id(sid)?);
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(path, if on { now_ms().to_string() } else { "0".into() })?;
    Ok(())
}

/// A turn's answer: the latest for the answer panel, and one line per turn kept. With a turn id the same
/// turn is kept once however often it is sent.
pub fn answer(data: &Path, sid: &str, input: &Value) -> Result<bool> {
    let text = input.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty()).ok_or("answer needs text")?;
    let turn = input.get("turnId").and_then(Value::as_str);
    let path = data.join("answers").join(format!("{}.json", safe_id(sid)?));
    if turn.is_some() && read_json(&path, json!({}))?.get("turnId").and_then(Value::as_str) == turn { return Ok(false); }
    let prompt = match input.get("prompt").and_then(Value::as_str) {
        Some(p) => p.to_string(),
        None => read(data, sid)?.get("lastPrompt").and_then(Value::as_str).unwrap_or("").to_string(),
    };
    let at = now_ms();
    let mut a = json!({"sessionId": sid, "prompt": prompt, "text": text, "at": at});
    if let Some(t) = turn { a["turnId"] = json!(t); }
    write_json(&path, &a)?;
    let log = data.join("answer-log").join(format!("{sid}.jsonl"));
    fs::create_dir_all(log.parent().unwrap())?;
    writeln!(OpenOptions::new().create(true).append(true).open(log)?, "{}", json!({"at": at, "prompt": prompt, "text": text}))?;
    Ok(true)
}

/// This number's conversation goes on under `to` (/resume elsewhere): lists show it there. `empty` names a
/// background session Claude Code carried this one into that holds no conversation, for the program to stop.
pub fn moved(data: &Path, sid: &str, to: &str, empty: Option<&str>) -> Result<()> {
    // A session does not move to itself; such a record would hide it from every list.
    if to == sid { return Ok(()); }
    let path = record_path(data, sid)?;
    let mut r = read_json(&path, Value::Null)?;
    if r.is_null() { r = json!({"id": sid, "lastActive": now_ms()}); }
    r["movedTo"] = json!(safe_id(to)?);
    if let Some(e) = empty { r["emptyCarried"] = json!(safe_id(e)?); }
    write_json(&path, &r)?;
    alive(data, sid, false)
}

/// This new number carries on `from`'s conversation (/background): it takes the record, without the tab,
/// and the old one points here.
pub fn carry_from(data: &Path, sid: &str, from: &str) -> Result<()> {
    if from == sid { return Ok(()); }
    let old_path = record_path(data, from)?;
    let mut old = read_json(&old_path, Value::Null)?;
    if !old.is_object() { return Ok(()); }
    let mut new = old.clone();
    new["id"] = json!(sid);
    new["lastActive"] = json!(now_ms());
    new.as_object_mut().unwrap().shift_remove("tab");
    write_json(&record_path(data, sid)?, &new)?;
    old["movedTo"] = json!(sid);
    write_json(&old_path, &old)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_new_record_takes_now_as_its_start() -> Result<()> {
        let d = std::env::temp_dir().join(format!("wb-started-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("sessions"))?;
        let none = Touch { prompt: Some("q"), tab: None, cwd: None, agent: None };
        assert!(touch(&d, "new", none)?["started"].is_i64());
        write_json(&d.join("sessions/old.json"), &json!({"id":"old","lastActive":1}))?;
        let none = Touch { prompt: Some("q"), tab: None, cwd: None, agent: None };
        assert!(touch(&d, "old", none)?.get("started").is_none(), "a record from before keeps falling back to its answers");
        Ok(())
    }
    #[test]
    fn touch_keeps_the_first_prompt_and_the_move_and_drops_past_states() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let d = dir.path();
        write_json(&d.join("sessions/s1.json"), &json!({"id":"s1","cwd":"/w","firstPrompt":"first","movedTo":"s2","ended":true,"emptyCarried":"x"}))?;
        let r = touch(d, "s1", Touch { prompt: Some("next"), tab: Some("t1"), cwd: Some("/other"), agent: None })?;
        assert_eq!(r["firstPrompt"], "first");
        assert_eq!(r["lastPrompt"], "next");
        assert_eq!(r["cwd"], "/w");
        assert_eq!(r["tab"], "t1");
        assert_eq!(r["movedTo"], "s2");
        assert!(r.get("ended").is_none() && r.get("emptyCarried").is_none());
        let r = touch(d, "s1", Touch::default())?;
        assert_eq!(r["lastPrompt"], "next");
        assert_eq!(r["tab"], "t1");
        assert!(touch(d, "s1", Touch { tab: Some("../x"), ..Default::default() }).is_err());
        Ok(())
    }

    #[test]
    fn an_answer_with_a_turn_id_is_kept_once() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let d = dir.path();
        touch(d, "s1", Touch { prompt: Some("question"), ..Default::default() })?;
        assert!(answer(d, "s1", &json!({"text":"answer","turnId":"t1"}))?);
        assert!(!answer(d, "s1", &json!({"text":"answer","turnId":"t1"}))?);
        assert!(answer(d, "s1", &json!({"text":"second"}))?);
        assert_eq!(read_json(&d.join("answers/s1.json"), Value::Null)?["prompt"], "question");
        assert_eq!(fs::read_to_string(d.join("answer-log/s1.jsonl"))?.lines().count(), 2);
        assert!(answer(d, "s1", &json!({"text":"  "})).is_err());
        Ok(())
    }

    #[test]
    fn a_carried_conversation_takes_the_record_and_the_old_points_to_it() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let d = dir.path();
        touch(d, "old", Touch { prompt: Some("question"), tab: Some("t1"), cwd: Some("/w"), ..Default::default() })?;
        carry_from(d, "new", "old")?;
        let new = read(d, "new")?;
        assert_eq!(new["firstPrompt"], "question");
        assert!(new.get("tab").is_none());
        assert_eq!(read(d, "old")?["movedTo"], "new");
        moved(d, "new", "other", Some("spare"))?;
        assert_eq!(read(d, "new")?["emptyCarried"], "spare");
        assert_eq!(fs::read_to_string(d.join("cache/alive/new"))?, "0");
        assert!(state(d, "new", "busy", None).is_err());
        Ok(())
    }
    #[test]
    fn a_session_never_moves_to_itself() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let d = dir.path();
        touch(d, "s1", Touch { prompt: Some("question"), ..Default::default() })?;
        moved(d, "s1", "s1", None)?;
        carry_from(d, "s1", "s1")?;
        assert!(read_json(&d.join("sessions/s1.json"), Value::Null)?.get("movedTo").is_none());
        Ok(())
    }

}
