//! Project documents: read, check out, put back, and the before and after of a recorded change.
use std::{fs, path::PathBuf};

use serde_json::{Value, json};

use crate::{Res, Store, arr, markdown::{changed_paths, without_mark, removed_paths, section_text}, new_id, read, s, sha, text, write_json, write_text};

impl Store {
    fn doc_dir(&self, pid: &str, did: &str) -> PathBuf { self.pdir(pid).join("docs").join(did) }
    pub fn doc_path(&self, pid: &str, did: &str) -> PathBuf { self.doc_dir(pid, did).join("document.md") }
    pub fn document(&self, pid: &str, did: &str) -> String { text(&self.doc_path(pid, did)) }
    fn meta_path(&self, pid: &str, did: &str) -> PathBuf { self.doc_dir(pid, did).join("meta.json") }
    pub fn meta(&self, pid: &str, did: &str) -> Value { read(&self.meta_path(pid, did)).unwrap_or_else(|| json!({"sha":"","sections":{}})) }
    fn work(&self, pid: &str, did: &str, session: &str) -> PathBuf {
        self.pdir(pid).join("work").join(if session.is_empty() { "user" } else { session }).join(format!("{did}.md"))
    }

    fn base_path(work: &PathBuf) -> PathBuf {
        let mut o = work.as_os_str().to_os_string();
        o.push(".base");
        o.into()
    }

    fn has_doc(&self, pid: &str, did: &str) -> bool {
        self.get_project(pid).is_some_and(|p| arr(&p, "docs").iter().any(|d| s(d, "id") == did))
    }

    fn base_now(&self, pid: &str, did: &str) -> String { s(&self.meta(pid, did), "sha").to_string() }

    /// A working copy for the session; remembers the version it started from.
    pub fn checkout(&self, pid: &str, did: &str, session: &str) -> Res<PathBuf> {
        if !self.has_doc(pid, did) { return Err(format!("no document {pid}/{did}")); }
        let path = self.work(pid, did, session);
        write_text(&path, &self.document(pid, did))?;
        write_text(&Self::base_path(&path), &self.base_now(pid, did))?;
        Ok(path)
    }

    /// Puts a working copy back. Fails if the original changed since checkout.
    pub fn put(&self, pid: &str, did: &str, session: &str, what: &str, source: Option<Value>, note: &str) -> Res<Value> {
        let path = self.work(pid, did, session);
        if !path.exists() { return Err("no working copy: checkout first".into()); }
        let base = text(&Self::base_path(&path)).trim().to_string();
        if base != self.base_now(pid, did) { return Err("it changed since checkout: checkout again and redo the edit on the new version".into()); }
        let new = text(&path);
        if what.is_empty() { return Err("--what is required (one-line summary of the change)".into()); }
        let old = self.document(pid, did);
        if new == old { return Ok(json!({"unchanged": true})); }
        let mut entry = json!({"doc": did, "what": what, "text": new});
        let (_, renamed) = removed_paths(&old, &new);
        if !renamed.is_empty() { entry["renamed"] = json!(renamed); }
        let ev = self.commit_docs(pid, vec![entry], session, source, note)?;
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(Self::base_path(&path));
        Ok(json!({"committed": ev["id"], "paths": ev["docs"][0]["paths"]}))
    }

    pub fn commit_docs(&self, pid: &str, entries: Vec<Value>, session: &str, source: Option<Value>, note: &str) -> Res<Value> {
        let at = self.next_at(pid);
        let mut done = Vec::new();
        for mut e in entries {
            let did = s(&e, "doc").to_string();
            let old = self.document(pid, &did);
            let mut m = self.meta(pid, &did);
            let full = s(&e, "text").to_string();
            let paths = changed_paths(&old, &full);
            write_text(&self.doc_path(pid, &did), &full)?;
            m["sha"] = json!(sha(&full));
            if m.get("sections").is_none() { m["sections"] = json!({}); }
            for p in &paths { m["sections"][without_mark(p)] = json!(at); }
            write_json(&self.meta_path(pid, &did), &m)?;
            e["paths"] = json!(paths);
            done.push(e);
        }
        let id = new_id(10);
        let ev = json!({"id": id, "at": at, "session": session, "source": source.unwrap_or_else(|| json!({})), "note": note, "docs": done});
        write_json(&self.event_path(pid, at, session, &id), &ev)?;
        self.prune(pid);
        Ok(ev)
    }

    /// Before and after of every section a recorded change touched, or None when the copies are gone.
    pub fn change_detail(&self, pid: &str, event: &str, did: &str) -> Option<Vec<Value>> {
        let evs = self.events(pid);
        let idx = evs.iter().position(|e| s(e, "id") == event)?;
        let entry = arr(&evs[idx], "docs").iter().find(|d| s(d, "doc") == did).filter(|e| e.get("text").is_some())?;
        let mut before = String::new();
        for e in evs[..idx].iter().rev() {
            let prev = arr(e, "docs").iter().find(|d| s(d, "doc") == did);
            if let Some(p) = prev {
                before = p.get("text")?.as_str()?.to_string();
                break;
            }
        }
        let after = s(entry, "text");
        let find = section_text;
        Some(arr(entry, "paths").iter().filter_map(Value::as_str).map(|p| {
            let clean = without_mark(p);
            json!({"path": clean, "before": find(&before, &clean), "after": find(after, &clean)})
        }).collect())
    }
}
