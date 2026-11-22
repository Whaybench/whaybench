//! The project store. Only the AI writes, through `whaybench-project`; panels read,
//! request and decide. One file per part of a project:
//!
//! ```text
//! projects/<pid>/project.json             name, desc, parent, docs [{id, addon, name, original}], paths,
//!                                         folder, subvolume, kind "briefsafe" (the project folder, spec 3.22)
//! projects/<pid>/docs/<doc>/document.md   the document; headings carry their numbers
//! projects/<pid>/docs/<doc>/meta.json     {sha, sections: {heading path: last change ms}}
//! projects/<pid>/work/<session>/...       a session's working copies
//! projects/<pid>/events/*.json            change records, kept within limits
//! projects/<pid>/map.json, queue/<id>.json
//! ```
pub mod docaddons;
pub mod documents;
pub mod folder;
pub mod history;
pub mod items;
pub mod markdown;
pub mod projects;
pub mod queue;

use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Map, Value};
use sha1::{Digest, Sha1};
use wb_core::paths::Data;

pub type Res<T> = Result<T, String>;

#[derive(Clone)]
pub struct Store { pub data: Data, pub root: PathBuf }

impl Store {
    pub fn new(data: Data) -> Self { let root = data.root.join("projects"); Store { data, root } }
    pub fn pdir(&self, pid: &str) -> PathBuf { self.root.join(part(pid)) }
    pub fn get_project(&self, pid: &str) -> Option<Value> { read(&self.pdir(pid).join("project.json")) }
    pub fn list_projects(&self) -> Vec<Value> {
        // Through `paths`: a Briefsafe project's store is in its data, which its agents can change (no links, no pipes).
        let mut names: Vec<PathBuf> = wb_core::paths::list_dir(&self.root).into_iter().map(|p| p.join("project.json")).collect();
        names.sort();
        names.iter().filter_map(|p| read(p)).collect()
    }
}

/// An id as one plain name in a path. Ids come from project data too (a Briefsafe project's is its agents' to write):
/// an absolute one or one with `/` or `..` would lead out of the store, so it names nothing there.
pub(crate) fn part(id: &str) -> &str {
    if id.is_empty() || id == "." || id == ".." || id.contains(['/', '\0']) { "_not-a-name_" } else { id }
}

pub fn read(path: &Path) -> Option<Value> { serde_json::from_str(&wb_core::paths::read_text(path).ok()?).ok() }
pub fn text(path: &Path) -> String { wb_core::paths::read_text(path).unwrap_or_default() }

pub fn write_text(path: &Path, content: &str) -> Res<()> {
    wb_core::paths::write_atomic(path, content).map_err(|e| e.to_string())
}

pub fn write_json(path: &Path, v: &Value) -> Res<()> {
    write_text(path, &serde_json::to_string_pretty(v).map_err(|e| e.to_string())?)
}

pub fn now_ms() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0) }

pub fn sha(content: &str) -> String {
    let d = Sha1::digest(content.as_bytes());
    d.iter().take(6).map(|b| format!("{b:02x}")).collect()
}

/// A short id nobody else has: from the clock, the process and a counter.
pub fn new_id(n: usize) -> String {
    static C: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let d = Sha1::digest(format!("{nanos}-{}-{}", std::process::id(), C.fetch_add(1, Ordering::Relaxed)));
    d.iter().map(|b| format!("{b:02x}")).collect::<String>()[..n].to_string()
}

pub fn s<'a>(v: &'a Value, key: &str) -> &'a str { v.get(key).and_then(Value::as_str).unwrap_or("") }
pub fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] { v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) }
pub fn by_id(list: &[Value]) -> Map<String, Value> { list.iter().map(|x| (s(x, "id").to_string(), x.clone())).collect() }

#[cfg(test)]
pub(crate) fn scratch(name: &str) -> Store {
    static N: AtomicU64 = AtomicU64::new(0);
    let d = std::env::temp_dir().join(format!("wb-store-{name}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    Store::new(Data::new(d))
}
