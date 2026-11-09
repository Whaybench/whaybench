//! Work queue: every request about a project is one item, done once. A request reaching a second session
//! (resumed, replayed, sent twice) is told "already" and does nothing. A take with no done for 30 minutes (the
//! session died mid-work) can be taken again. The lock is made with O_EXCL: only one take succeeds.
use std::{fs, path::PathBuf};

use serde_json::{Value, json};

use crate::{Res, Store, new_id, now_ms, read, s, write_json};

pub const TAKE_STALE_MIN: i64 = 30;

impl Store {
    fn q_path(&self, pid: &str, id: &str) -> PathBuf { self.pdir(pid).join("queue").join(format!("{id}.json")) }
    fn q_lock(&self, pid: &str, id: &str) -> PathBuf { self.pdir(pid).join("queue").join(format!("{id}.lock")) }

    pub fn queue_add(&self, pid: &str, kind: &str, what: &str, session: &str) -> Res<String> {
        let id = new_id(8);
        let short: String = what.chars().take(120).collect();
        write_json(&self.q_path(pid, &id), &json!({"id": id, "at": now_ms(), "kind": kind, "what": short, "session": session, "status": "waiting"}))?;
        Ok(id)
    }

    /// An item whose request could not be sent.
    pub fn queue_drop(&self, pid: &str, id: &str) {
        let _ = fs::remove_file(self.q_path(pid, id));
        let _ = fs::remove_file(self.q_lock(pid, id));
    }

    pub fn queue(&self, pid: &str, status: Option<&str>) -> Vec<Value> {
        let mut f: Vec<PathBuf> = fs::read_dir(self.pdir(pid).join("queue")).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
        f.sort();
        let mut items: Vec<Value> = f.iter().filter_map(|p| read(p)).filter(|x| status.is_none_or(|t| s(x, "status") == t)).collect();
        items.sort_by_key(|x| x.get("at").and_then(Value::as_i64).unwrap_or(0));
        items
    }

    /// 'taken' for the one session that takes it, 'already' when taken by another or done, 'missing'.
    pub fn queue_take(&self, pid: &str, id: &str, session: &str) -> Res<&'static str> {
        let Some(mut item) = read(&self.q_path(pid, id)) else { return Ok("missing") };
        if s(&item, "status") == "done" { return Ok("already"); }
        let lock = self.q_lock(pid, id);
        if s(&item, "status") == "taken" {
            if s(&item, "by") == session { return Ok("taken"); }
            if now_ms() - item.get("takenAt").and_then(Value::as_i64).unwrap_or(0) < TAKE_STALE_MIN * 60_000 { return Ok("already"); }
            let _ = fs::remove_file(&lock);
        }
        if fs::OpenOptions::new().create_new(true).write(true).open(&lock).is_err() { return Ok("already"); }
        item["status"] = json!("taken");
        item["by"] = json!(session);
        item["takenAt"] = json!(now_ms());
        write_json(&self.q_path(pid, id), &item)?;
        Ok("taken")
    }

    pub fn queue_done(&self, pid: &str, id: &str, session: &str, note: &str) -> Res<Value> {
        let mut item = read(&self.q_path(pid, id)).ok_or_else(|| format!("no work item {id}"))?;
        if s(&item, "status") == "taken" && !s(&item, "by").is_empty() && s(&item, "by") != session { return Err(format!("work item {id} was taken by another session")); }
        item["status"] = json!("done");
        if !session.is_empty() { item["by"] = json!(session); }
        item["doneAt"] = json!(now_ms());
        item["note"] = json!(note.chars().take(200).collect::<String>());
        write_json(&self.q_path(pid, id), &item)?;
        Ok(item)
    }
}
