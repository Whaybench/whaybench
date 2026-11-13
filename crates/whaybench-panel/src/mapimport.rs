//! Other maps brought in: an exported file or another session's map is kept as a file of its
//! own and shown in a read-only panel. The session's own map is never touched: it holds what this conversation said.
use std::{fs, path::{Path, PathBuf}};

use serde_json::{Value, json};
use wb_core::{i18n, paths::Data, sessions, status};
use wb_store::s;

/// A map that can be brought in.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    File(PathBuf),
    /// A session's map, whole or one top branch.
    Session(String, Option<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Choice { pub label: String, pub source: Source }

pub struct Sessions { pub name: String, pub whole: Choice }

fn maps_dir(d: &Data) -> PathBuf { d.root.join("maps") }
fn imported_dir(d: &Data) -> PathBuf { d.root.join("imported") }

/// The last exported files, newest first.
pub fn exported(d: &Data) -> Vec<Choice> {
    let mut files: Vec<PathBuf> = fs::read_dir(maps_dir(d)).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    files.reverse();
    files.into_iter().take(15).filter_map(|p| {
        let v: Value = serde_json::from_str(&fs::read_to_string(&p).ok()?).ok()?;
        let when = s(&v, "exported").to_string();
        let title = if s(&v, "title").is_empty() { p.file_stem()?.to_string_lossy().to_string() } else { s(&v, "title").to_string() };
        Some(Choice { label: format!("{title} · {when}"), source: Source::File(p) })
    }).collect()
}

/// The maps of other sessions, each as a whole.
pub fn other_sessions(d: &Data) -> Vec<Sessions> {
    let here = sessions::target(d);
    sessions::all(d).into_iter().filter(|i| s(i, "movedTo").is_empty() && Some(s(i, "id")) != here.as_deref()).filter_map(|i| {
        let id = s(&i, "id").to_string();
        let map = sessions::map_of(d, &id)?;
        let nodes = map.get("nodes")?.as_array()?;
        if nodes.is_empty() { return None; }
        Some(Sessions { name: sessions::session_name(d, &id).chars().take(30).collect(), whole: Choice { label: i18n::t("whole-map").to_string(), source: Source::Session(id, None) } })
    }).collect()
}

fn below(nodes: &[Value], root: &str) -> Vec<Value> {
    let mut keep = vec![root.to_string()];
    loop {
        let before = keep.len();
        for n in nodes { if keep.iter().any(|k| k == s(n, "parent")) && !keep.iter().any(|k| k == s(n, "id")) { keep.push(s(n, "id").to_string()); } }
        if keep.len() == before { break; }
    }
    nodes.iter().filter(|n| keep.iter().any(|k| k == s(n, "id"))).cloned().map(|mut n| {
        // The root of what is taken has no parent here.
        if s(&n, "id") == root { if let Some(o) = n.as_object_mut() { o.remove("parent"); } }
        n
    }).collect()
}

/// The nodes, title and session (the one it came from, or "") of a source.
pub fn read(d: &Data, source: &Source) -> Option<(Vec<Value>, String, String)> {
    match source {
        Source::File(p) => {
            let v = wb_core::map::view_file(&d.root, p).ok()?;
            let title = if s(&v, "title").is_empty() { p.file_stem()?.to_string_lossy().to_string() } else { s(&v, "title").to_string() };
            // A session's own map names its session, a saved one where it came from.
            let from = if s(&v, "from").is_empty() { s(&v, "sessionId") } else { s(&v, "from") };
            Some((v.get("nodes")?.as_array()?.clone(), title, from.to_string()))
        }
        Source::Session(id, root) => {
            let map = sessions::map_of(d, id)?;
            let nodes = map.get("nodes")?.as_array()?.clone();
            match root {
                None => Some((nodes, sessions::session_name(d, id), id.clone())),
                Some(r) => {
                    let title = nodes.iter().find(|n| s(n, "id") == r).map(|n| s(n, "label").to_string())?;
                    Some((below(&nodes, r), title, id.clone()))
                }
            }
        }
    }
}

/// Keeps a copy as a file of its own, with the session it came from (for its related answers). Its path.
pub fn keep(d: &Data, nodes: &[Value], title: &str, from: &str) -> std::io::Result<PathBuf> {
    fs::create_dir_all(imported_dir(d))?;
    let path = wb_core::map::free_path(&imported_dir(d), &wb_core::map::file_name(sessions::span(d, from, nodes), None, ""));
    let imported = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    wb_core::paths::write_atomic(&path, json!({"format": "whaybench-map", "title": title, "imported": imported, "from": from, "nodes": wb_core::map::saved_nodes(nodes)}).to_string())?;
    Ok(path)
}

/// The app id of an imported map's panel: its own, so it can be brought up and is known after a restart.
pub fn app_id(path: &str) -> String {
    let n = path.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
    format!("dev.whaybench.map.i{n:08x}")
}

pub fn message_key(path: &str) -> String { format!("map-{}", app_id(path)) }

fn program() -> PathBuf {
    wb_core::paths::own_exe().unwrap_or_else(|_| "whaybench-panel".into())
}

/// Opens the file's panel, or brings it up if it is open, at a node if one is given.
pub fn show(d: &Data, path: &Path, title: &str, node: Option<&str>) {
    let (p, id) = (path.to_string_lossy().to_string(), app_id(&path.to_string_lossy()));
    let open = fs::read_to_string(d.layout()).is_ok_and(|t| t.contains(&format!("\"{id}#")));
    if open { status::focus_app(d, &id); }
    else { let _ = std::process::Command::new(program()).args(["map", &format!("@{p}"), title]).spawn(); }
    if let Some(n) = node { let (d2, key, n2) = (d.clone(), message_key(&p), n.to_string()); gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(if open { 100 } else { 1500 }), move || status::send(&d2, &key, json!({"do": "pick", "node": n2}))); }
}

/// Brings a source in: kept as a file, shown in a read-only panel.
pub fn bring_in(d: &Data, source: &Source) {
    let Some((nodes, title, from)) = read(d, source) else { status::say(d, i18n::t("import-failed-no-map"), 6); return };
    match keep(d, &nodes, &title, &from) {
        Ok(path) => {
            show(d, &path, &title, None);
            status::say(d, &i18n::f("imported-map-opened", &[("title", title.chars().take(30).collect::<String>()), ("n", nodes.len().to_string())]), 8);
        }
        Err(e) => status::say(d, &i18n::f("import-failed", &[("e", e.to_string())]), 8),
    }
}

/// Panels besides the main ones, kept over a restart: `[args…]` of `whaybench-panel map`.
fn panels_file(d: &Data) -> PathBuf { d.root.join("map-panels.json") }

pub fn remember(d: &Data, args: &[String]) {
    let mut all = panels(d);
    if !all.iter().any(|a| a == args) { all.push(args.to_vec()); }
    let _ = wb_core::paths::write_atomic(&panels_file(d), json!(all).to_string());
}

pub fn forget(d: &Data, args: &[String]) {
    let all: Vec<Vec<String>> = panels(d).into_iter().filter(|a| a != args).collect();
    let _ = wb_core::paths::write_atomic(&panels_file(d), json!(all).to_string());
}

pub fn panels(d: &Data) -> Vec<Vec<String>> {
    wb_core::paths::read_kept(&panels_file(d))
}

/// The request that takes one node of an imported map into this conversation as a reference.
pub fn reference_request(path: &str, title: &str, node: &Value, flow: &[String], lines: &[String]) -> String {
    let source = json!({"file": path, "node": s(node, "id"), "title": title, "path": flow}).to_string();
    let upper = if flow.is_empty() { i18n::ai("none-top-branch").to_string() } else { flow.join(" › ") };
    i18n::ai_f("import-reference-request", &[("title", title.to_string()), ("head", s(node, "label").to_string()), ("flow", upper), ("lines", lines.iter().take(60).cloned().collect::<Vec<_>>().join("\n")), ("source", source)])
}

/// The request that hands a whole imported map to a session: its file, its size and its top branches; the session
/// reads the file for what it needs, since a whole map can be far too long to send.
pub fn whole_reference_request(path: &str, title: &str, data: &Value) -> String {
    let nodes = data.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
    let ids: std::collections::HashSet<&str> = nodes.iter().map(|n| s(n, "id")).collect();
    let open = nodes.iter().filter(|n| wb_core::map::is_open(n)).count();
    let tops: Vec<String> = nodes.iter().filter(|n| !ids.contains(s(n, "parent"))).take(60).map(|n| format!("- {}", s(n, "label"))).collect();
    let source = json!({"file": path, "node": "", "title": title}).to_string();
    i18n::ai_f("import-reference-whole-request", &[("title", title.to_string()), ("path", path.to_string()), ("count", nodes.len().to_string()), ("open", open.to_string()), ("lines", tops.join("\n")), ("source", source)])
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scratch(tag: &str) -> Data {
        let dir = std::env::temp_dir().join(format!("wb-mi-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Data::new(dir)
    }
    #[test]
    fn a_whole_map_is_sent_as_its_file_size_and_top_branches() {
        let data = json!({"nodes":[{"id":"a","label":"top"},{"id":"b","parent":"a","label":"under","state":"done"},{"id":"z","label":"other","state":"done"}]});
        let t = whole_reference_request("/m/x.json", "X", &data);
        assert!(t.contains("/m/x.json") && t.contains("3 nodes, 1 open"));
        assert!(t.contains("- top") && t.contains("- other") && !t.contains("- under"));
    }
    #[test]
    fn a_branch_comes_with_what_is_under_it_and_a_copy_is_kept() {
        let d = scratch("a");
        fs::create_dir_all(d.root.join("sessions")).unwrap();
        fs::write(d.root.join("sessions/s1.json"), r#"{"id":"s1","firstPrompt":"q"}"#).unwrap();
        wb_core::map::command(&d.root, "s1", Some(json!({"nodes":[{"id":"a","label":"top"},{"id":"b","parent":"a","label":"bottom"},{"id":"z","label":"other"}]}))).unwrap();
        // Another session is on show: this one is one of the others.
        fs::remove_file(d.root.join("latest.json")).unwrap();
        let (nodes, title, from) = read(&d, &Source::Session("s1".into(), Some("a".into()))).unwrap();
        assert_eq!((nodes.len(), title.as_str(), from.as_str()), (2, "top", "s1"));
        assert!(nodes[0].get("parent").is_none());
        let groups = other_sessions(&d);
        assert_eq!(groups.len(), 1);
        let path = keep(&d, &nodes, &title, &from).unwrap();
        assert!(path.file_name().unwrap().to_string_lossy().starts_with("map-"), "named by the rule, not by the title");
        let (again, t2, f2) = read(&d, &Source::File(path.clone())).unwrap();
        assert_eq!((again.len(), t2.as_str(), f2.as_str()), (2, "top", "s1"), "the copy keeps where it came from");
        assert_ne!(keep(&d, &nodes, &title, &from).unwrap(), path, "kept twice, two files");
        assert!(exported(&d).is_empty(), "kept copies are not exported files");
    }
    #[test]
    fn panels_are_remembered_once_and_forgotten() {
        let d = scratch("b");
        let a = vec!["@/x".to_string(), "title".to_string()];
        remember(&d, &a);
        remember(&d, &a);
        assert_eq!(panels(&d), vec![a.clone()]);
        forget(&d, &a);
        assert!(panels(&d).is_empty());
        assert_eq!(app_id("/x"), app_id("/x"));
        assert_ne!(app_id("/x"), app_id("/y"));
    }
}
