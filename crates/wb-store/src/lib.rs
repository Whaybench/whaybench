//! The project store. Only the AI writes, through `whaybench-project`; panels read,
//! request and decide. One file per part of a project:
//!
//! ```text
//! projects/<pid>/project.json             name, desc, parent, docs [{id, addon, name, original}], paths
//! projects/<pid>/docs/<doc>/document.md   the document; headings carry their numbers
//! projects/<pid>/docs/<doc>/meta.json     {sha, sections: {heading path: last change ms}}
//! projects/<pid>/work/<session>/...       a session's working copies
//! projects/<pid>/events/*.json            change records, kept within limits
//! projects/<pid>/map.json, queue/<id>.json
//! ```
pub mod docaddons;
pub mod documents;
pub mod history;
pub mod items;
pub mod mapstyles;
pub mod markdown;
pub mod projects;
pub mod queue;
pub mod trust;

use std::{
    fs,
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
    pub fn pdir(&self, pid: &str) -> PathBuf { self.root.join(pid) }
    pub fn get_project(&self, pid: &str) -> Option<Value> { read(&self.pdir(pid).join("project.json")) }
    pub fn list_projects(&self) -> Vec<Value> {
        let mut names: Vec<PathBuf> = fs::read_dir(&self.root).into_iter().flatten().flatten().map(|e| e.path().join("project.json")).collect();
        names.sort();
        names.iter().filter_map(|p| read(p)).collect()
    }
}

pub fn read(path: &Path) -> Option<Value> { serde_json::from_str(&fs::read_to_string(path).ok()?).ok() }
pub fn text(path: &Path) -> String { fs::read_to_string(path).unwrap_or_default() }

pub fn write_text(path: &Path, content: &str) -> Res<()> {
    if let Some(d) = path.parent() { fs::create_dir_all(d).map_err(|e| e.to_string())?; }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&tmp, content).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
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
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    Store::new(Data::new(d))
}
