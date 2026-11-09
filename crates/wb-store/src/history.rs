//! Change records (events/) and their limits, like a log: time, content and size.
//! Past the content limit a record drops its copies and keeps when, who, where and what changed; past the time
//! limit it goes. Over the size limit the oldest lose their copies first, then go. Documents and the map
//! themselves are never touched by this.
use std::{fs, path::PathBuf};

use serde_json::{Map, Value, json};

use crate::{Res, Store, arr, new_id, now_ms, read, s, write_json};

pub const FULL_DAYS: i64 = 30;
pub const META_DAYS: i64 = 365;
pub const MAX_MB: u64 = 50;
const SKIP: [&str; 3] = ["at", "by", "updatedAt"];

pub fn target_name(_: &str) -> &'static str { wb_core::i18n::t("project-map") }

fn has_copy(ev: &Value) -> bool {
    arr(ev, "docs").iter().any(|d| d.get("text").is_some())
        || ev.get("items").map(|i| arr(i, "changes").iter().any(|c| c.get("before").is_some() || c.get("after").is_some())).unwrap_or(false)
}

fn strip(ev: &mut Value) {
    if let Some(ds) = ev.get_mut("docs").and_then(Value::as_array_mut) {
        for d in ds { if let Some(o) = d.as_object_mut() { o.remove("text"); } }
    }
    if let Some(cs) = ev.get_mut("items").and_then(|i| i.get_mut("changes")).and_then(Value::as_array_mut) {
        for c in cs {
            let l = label(c);
            c["label"] = json!(l);
            if let Some(o) = c.as_object_mut() { o.remove("before"); o.remove("after"); }
        }
    }
    ev["trimmed"] = json!(true);
}

pub fn label(c: &Value) -> String {
    let x = [c.get("after"), c.get("before")].into_iter().flatten().find(|v| v.is_object()).cloned().unwrap_or_else(|| json!({}));
    for k in [s(&x, "text"), s(&x, "title"), s(c, "label"), s(c, "id")] { if !k.is_empty() { return k.to_string(); } }
    String::new()
}

/// One side's items that differ, ignoring the fields that change with every write.
pub fn diff_objs(kind: &str, old: &Map<String, Value>, new: &Map<String, Value>) -> Vec<Value> {
    let plain = |v: &Value| -> Map<String, Value> { v.as_object().map(|o| o.iter().filter(|(k, _)| !SKIP.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default() };
    let mut out = Vec::new();
    for k in old.keys().chain(new.keys().filter(|k| !old.contains_key(*k))) {
        let (a, b) = (old.get(k), new.get(k));
        if let (Some(x), Some(y)) = (a, b) { if plain(x) == plain(y) { continue; } }
        let op = if a.is_none() { "add" } else if b.is_none() { "remove" } else { "update" };
        out.push(json!({"kind": kind, "id": k, "op": op, "before": a, "after": b}));
    }
    out
}

impl Store {
    fn event_files(&self, pid: &str) -> Vec<PathBuf> {
        let mut f: Vec<PathBuf> = fs::read_dir(self.pdir(pid).join("events")).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
        f.sort();
        f
    }

    /// The time of a new record: now, but always after the last one, so records made in the same millisecond keep their order.
    pub fn next_at(&self, pid: &str) -> i64 {
        let last = self.event_files(pid).last().and_then(|f| f.file_name()?.to_str()?.split('-').next()?.parse::<i64>().ok()).unwrap_or(0);
        now_ms().max(last + 1)
    }

    pub fn events(&self, pid: &str) -> Vec<Value> {
        let mut out: Vec<Value> = self.event_files(pid).iter().filter_map(|f| read(f)).collect();
        out.sort_by_key(|e| e.get("at").and_then(Value::as_i64).unwrap_or(0));
        out
    }

    pub fn event_path(&self, pid: &str, at: i64, session: &str, id: &str) -> PathBuf {
        let who: String = if session.is_empty() { "user".into() } else { session.chars().take(8).collect() };
        self.pdir(pid).join("events").join(format!("{at}-{who}-{id}.json"))
    }

    pub fn prune(&self, pid: &str) { self.prune_with(pid, FULL_DAYS, META_DAYS, MAX_MB * 1024 * 1024); }

    pub fn prune_with(&self, pid: &str, full_days: i64, meta_days: i64, cap: u64) {
        let (now, day) = (now_ms(), 86_400_000);
        for f in self.event_files(pid) {
            let Some(mut ev) = read(&f) else { continue };
            let age = now - ev.get("at").and_then(Value::as_i64).unwrap_or(now);
            if age > meta_days * day { let _ = fs::remove_file(&f); }
            else if age > full_days * day && has_copy(&ev) { strip(&mut ev); let _ = write_json(&f, &ev); }
        }
        let size = |files: &[PathBuf]| files.iter().filter_map(|f| fs::metadata(f).ok()).map(|m| m.len()).sum::<u64>();
        let files = self.event_files(pid);
        for f in &files {
            if size(&files) <= cap { break; }
            if let Some(mut ev) = read(f).filter(has_copy) { strip(&mut ev); let _ = write_json(f, &ev); }
        }
        for f in &files {
            if size(&self.event_files(pid)) <= cap { break; }
            let _ = fs::remove_file(f);
        }
    }

    /// One record per write to the map: every item that changed with before and after.
    pub fn log_items(&self, pid: &str, target: &str, changes: Vec<Value>, session: &str, note: &str) -> Res<Option<Value>> {
        if changes.is_empty() { return Ok(None); }
        let at = self.next_at(pid);
        let mut counts: Vec<(String, usize)> = Vec::new();
        for c in &changes {
            let op = s(c, "op").to_string();
            match counts.iter_mut().find(|x| x.0 == op) { Some(x) => x.1 += 1, None => counts.push((op, 1)) }
        }
        let what = if !note.is_empty() { note.to_string() } else {
            let parts: Vec<String> = counts.iter().map(|(o, n)| format!("{} {n}", match o.as_str() { "add" => wb_core::i18n::t("added"), "update" => wb_core::i18n::t("modified"), _ => wb_core::i18n::t("delete") })).collect();
            format!("{}: {}", target_name(target), parts.join(", "))
        };
        let id = new_id(10);
        let ev = json!({"id": id, "at": at, "session": session, "source": {}, "note": note, "docs": [], "items": {"target": target, "what": what, "changes": changes}});
        write_json(&self.event_path(pid, at, session, &id), &ev)?;
        self.prune(pid);
        Ok(Some(ev))
    }

    pub fn item_change_detail(&self, pid: &str, event: &str) -> Option<Value> {
        self.events(pid).into_iter().find(|e| s(e, "id") == event)?.get("items").cloned()
    }

    /// Every recorded change, newest first, by place, within `days` (None = all kept), at most 500 records.
    pub fn recent(&self, pid: &str, days: Option<i64>) -> Vec<Value> {
        let Some(p) = self.get_project(pid) else { return Vec::new() };
        let names: Map<String, Value> = arr(&p, "docs").iter().map(|d| (s(d, "id").to_string(), json!(if s(d, "name").is_empty() { s(d, "id") } else { s(d, "name") }))).collect();
        let since = days.map(|d| now_ms() - d * 86_400_000).unwrap_or(0);
        let evs: Vec<Value> = self.events(pid).into_iter().filter(|e| e.get("at").and_then(Value::as_i64).unwrap_or(0) >= since).collect();
        let skip = evs.len().saturating_sub(500);
        let mut out = Vec::new();
        for ev in evs.iter().skip(skip).rev() {
            if let Some(it) = ev.get("items").filter(|i| i.is_object()) {
                let target = s(it, "target");
                let areas: Map<String, Value> = if target == "map" { arr(&self.project_map(pid), "areas").iter().map(|a| (s(a, "id").to_string(), json!(s(a, "title")))).collect() } else { Map::new() };
                let mut paths = Vec::new();
                for c in arr(it, "changes").iter().take(40) {
                    let x = c.get("after").filter(|v| v.is_object()).or(c.get("before")).cloned().unwrap_or(Value::Null);
                    let lead = if s(c, "kind") == "item" { areas.get(s(&x, "area")).and_then(Value::as_str).unwrap_or("") } else { "" };
                    let short: String = label(c).chars().take(50).collect();
                    let place = if lead.is_empty() { short } else { format!("{lead} > {short}") };
                    paths.push(if s(c, "op") == "remove" { format!("{place}{}", crate::markdown::DELETED) } else { place });
                }
                out.push(json!({"at": ev["at"], "id": ev["id"], "hasCopy": arr(it, "changes").iter().any(|c| c.get("before").is_some()), "doc": target, "docName": target_name(target), "paths": paths, "what": it["what"], "session": s(ev, "session"), "approved": null, "source": {}, "itemEvent": true}));
            }
            for e in arr(ev, "docs") {
                out.push(json!({"at": ev["at"], "id": ev["id"], "hasCopy": e.get("text").is_some(), "doc": e["doc"], "docName": names.get(s(e, "doc")).cloned().unwrap_or_else(|| e["doc"].clone()), "paths": e.get("paths").cloned().unwrap_or_else(|| json!([])), "what": s(e, "what"), "session": s(ev, "session"), "approved": ev.get("approved"), "source": ev.get("source").cloned().unwrap_or_else(|| json!({}))}));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(at: i64) -> Value { json!({"id":"e","at":at,"docs":[{"doc":"d","text":"body","paths":["a"]}],"items":{"target":"map","changes":[{"id":"i","op":"add","kind":"item","after":{"text":"text"}}]}}) }
    #[test]
    fn old_records_lose_copies_then_go() {
        let st = crate::scratch("prune");
        let day = 86_400_000;
        let dir = st.pdir("p").join("events");
        fs::create_dir_all(&dir).unwrap();
        let now = now_ms();
        write_json(&dir.join("1.json"), &event(now - 400 * day)).unwrap();
        write_json(&dir.join("2.json"), &event(now - 40 * day)).unwrap();
        write_json(&dir.join("3.json"), &event(now)).unwrap();
        st.prune("p");
        assert!(!dir.join("1.json").exists(), "past the time limit");
        let mid = read(&dir.join("2.json")).unwrap();
        assert_eq!(mid["trimmed"], json!(true));
        assert!(mid["docs"][0].get("text").is_none());
        assert_eq!(mid["items"]["changes"][0]["label"], json!("text"));
        assert!(read(&dir.join("3.json")).unwrap()["docs"][0].get("text").is_some());
    }
    #[test]
    fn over_the_size_limit_the_oldest_go_first() {
        let st = crate::scratch("cap");
        let dir = st.pdir("p").join("events");
        fs::create_dir_all(&dir).unwrap();
        let now = now_ms();
        let big = "é".repeat(2000);
        for i in 1..=3 { write_json(&dir.join(format!("{i}.json")), &json!({"id":"e","at":now,"docs":[{"doc":"d","text":big,"paths":[]}],"note":big})).unwrap(); }
        st.prune_with("p", 30, 365, 7000);
        assert!(!dir.join("1.json").exists() || read(&dir.join("1.json")).unwrap().get("trimmed").is_some());
        assert!(dir.join("3.json").exists());
        let total: u64 = fs::read_dir(&dir).unwrap().flatten().map(|e| e.metadata().unwrap().len()).sum();
        assert!(total <= 7000);
    }
    #[test]
    fn automatic_fields_alone_are_no_change() {
        let mut a = Map::new(); a.insert("x".into(), json!({"id":"x","text":"text","at":1,"by":"s"}));
        let mut b = Map::new(); b.insert("x".into(), json!({"id":"x","text":"text","at":2,"by":"t"}));
        assert!(diff_objs("item", &a, &b).is_empty());
        b.insert("x".into(), json!({"id":"x","text":"new text"}));
        b.insert("y".into(), json!({"id":"y"}));
        let d = diff_objs("item", &a, &b);
        assert_eq!(d.iter().map(|c| s(c, "op")).collect::<Vec<_>>(), ["update", "add"]);
    }
}
