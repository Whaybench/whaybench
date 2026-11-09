//! Ways of drawing the conversation map. `default`, the tree, is the map panel itself. Every other way
//! is an add-on, a folder in `addons/map-styles/`: a declaration of a frame the panel draws and the node data it needs.
//! The format for people who make them is in addons/map-styles/README.md.
use std::{collections::{HashMap, HashSet}, fs, path::PathBuf};

use serde_json::{Value, json};
use wb_core::paths::Data;

use crate::{Store, arr, read, s, write_json};

pub const DEFAULT: &str = "default";
const FRAMES: [&str; 5] = ["tree", "columns", "route", "time", "questions"];
/// Fields a side worker can write.
pub const FILLED: [&str; 3] = ["kind", "reason", "links"];

/// How a side worker is asked for a field.
pub fn filled_hint(field: &str) -> &'static str {
    match field {
        "kind" => wb_core::i18n::t("one-of-question-option-pro-con"),
        "reason" => wb_core::i18n::t("one-line-about-40-characters-why"),
        _ => wb_core::i18n::t("relations-to-nodes-of-other-branches"),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Style { pub id: String, pub name: String, pub raw: Value, pub path: PathBuf }

#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem { pub label: String, pub action: String, pub arg: String }

impl Style {
    /// The frame the panel draws, if it knows it. Otherwise the AI draws from the instruction.
    pub fn frame(&self) -> Option<&str> { self.raw.get("frame").and_then(Value::as_str).filter(|f| FRAMES.contains(f)) }
    pub fn instruction(&self) -> &str { s(&self.raw, "instruction") }
    pub fn in_menu(&self) -> bool { self.raw.get("menu") != Some(&json!(false)) }
    pub fn node_items(&self) -> Vec<MenuItem> {
        arr(&self.raw, "node_menu").iter().filter_map(|i| {
            let (label, action, arg) = (s(i, "label"), s(i, "do"), s(i, "arg"));
            (!label.is_empty() && !arg.is_empty() && ["request", "style", "view"].contains(&action)).then(|| MenuItem { label: label.into(), action: action.into(), arg: arg.into() })
        }).collect()
    }
    /// Everything of this style that goes to the AI as written.
    pub fn free_text(&self) -> String {
        let mut parts = vec![self.instruction().to_string()];
        parts.extend(self.node_items().into_iter().filter(|i| i.action == "request").map(|i| i.arg));
        parts.join("\n").trim().to_string()
    }
}

fn valid_id(id: &str) -> bool { !id.is_empty() && id != DEFAULT && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') }

pub fn find(d: &Data, id: &str) -> Option<Style> {
    if !valid_id(id) { return None; }
    let path = d.root.join("addons/map-styles").join(id).join("style.json");
    let raw = read(&path)?;
    // In the language in use if its add-on has it, then the file's own (English), then the folder's.
    let name = wb_core::i18n::name_in_language(&format!("map-style-{id}")).unwrap_or_else(|| if s(&raw, "name").is_empty() { id.to_string() } else { s(&raw, "name").to_string() });
    Some(Style { id: id.to_string(), name, raw, path })
}

/// The add-ons there are now, by name.
pub fn list(d: &Data) -> Vec<Style> {
    let mut v: Vec<Style> = fs::read_dir(d.root.join("addons/map-styles")).into_iter().flatten().flatten().filter(|e| e.path().is_dir())
        .filter_map(|e| find(d, &e.file_name().to_string_lossy())).collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

fn choice_file(d: &Data) -> PathBuf { d.root.join("map-style-current.json") }

/// The style of a panel (`map`, or a branch panel's node): `default` when none was chosen or its folder is gone.
pub fn current(d: &Data, panel: &str) -> String {
    let id = read(&choice_file(d)).and_then(|v| v.get(panel)?.as_str().map(String::from)).unwrap_or_default();
    if find(d, &id).is_some() { id } else { DEFAULT.to_string() }
}

pub fn set_current(d: &Data, panel: &str, id: &str) {
    let mut v = read(&choice_file(d)).unwrap_or_else(|| json!({}));
    v[panel] = json!(id);
    let _ = write_json(&choice_file(d), &v);
}

/// The nodes under a root, the root included, or all. Parents outside are dropped by the frames.
pub fn scoped(data: &Value, root: Option<&str>) -> Vec<Value> {
    let nodes = arr(data, "nodes");
    let Some(root) = root else { return nodes.to_vec() };
    let mut keep: HashSet<&str> = HashSet::from([root]);
    loop {
        let before = keep.len();
        for n in nodes { if keep.contains(s(n, "parent")) { keep.insert(s(n, "id")); } }
        if keep.len() == before { break; }
    }
    nodes.iter().filter(|n| keep.contains(s(n, "id"))).cloned().collect()
}

/// The path from the top down to a node.
pub fn path_to(nodes: &[Value], target: &str) -> Vec<Value> {
    let by: HashMap<&str, &Value> = nodes.iter().map(|n| (s(n, "id"), n)).collect();
    let (mut chain, mut at, mut seen) = (Vec::new(), target, HashSet::new());
    while let Some(n) = by.get(at) {
        if !seen.insert(at) { break; }
        chain.push((*n).clone());
        at = s(n, "parent");
    }
    chain.reverse();
    chain
}

/// For each field this style needs that a worker can write, the nodes lacking it.
pub fn missing(style: &Style, nodes: &[Value], current: &str) -> Vec<(String, Vec<String>)> {
    let empty = |n: &Value, f: &str| n.get(f).is_none_or(|v| v.is_null() || v.as_str() == Some("") || v.as_array().is_some_and(Vec::is_empty));
    let mut out = Vec::new();
    for f in arr(&style.raw, "needs").iter().filter_map(Value::as_str) {
        if !FILLED.contains(&f) { continue; }
        let mut lacking: Vec<String> = nodes.iter().filter(|n| empty(n, f)).map(|n| s(n, "id").to_string()).collect();
        if f == "reason" {
            // Only the stops on the way here need a reason.
            let visited: HashSet<String> = nodes.iter().filter(|n| !empty(n, "visits")).map(|n| s(n, "id").to_string()).collect();
            let on_way = if visited.is_empty() { path_to(nodes, current).iter().map(|n| s(n, "id").to_string()).collect() } else { visited };
            lacking.retain(|i| on_way.contains(i));
        }
        if !lacking.is_empty() { out.push((f.to_string(), lacking)); }
    }
    out
}

/// What the status line says about those.
pub fn missing_text(need: &[(String, Vec<String>)]) -> String {
    need.iter().map(|(f, ids)| wb_core::i18n::f("text-2", &[("a1", (match f.as_str() { "kind" => wb_core::i18n::t("kind"), "reason" => wb_core::i18n::t("reason"), _ => wb_core::i18n::t("link-2") }).to_string()), ("a2", (ids.len()).to_string())])).collect::<Vec<_>>().join(", ")
}

pub fn fill_item(template: &str, label: &str, path: &str, id: &str) -> String {
    template.replace("{node}", label).replace("{branch}", path).replace("{id}", id)
}

/// The request that has the AI draw the map in a style with no frame, in a view of its own.
pub fn draw_request(style: &Style, d: &Data, session: &str, scope: Option<(&str, &str)>) -> String {
    let title = wb_core::i18n::f("map-style", &[("a1", (scope.map(|(_, l)| format!(" · {}", l.chars().take(12).collect::<String>())).unwrap_or_default()).to_string())]);
    let target = scope.map_or_else(|| wb_core::i18n::t("the-conversation-map-of-this-session").to_string(), |(i, l)| wb_core::i18n::f("branch-node-and-below", &[("l", (l).to_string()), ("i", (i).to_string())]));
    let map = d.root.join(format!("{session}.json"));
    wb_core::i18n::f("map-style-show-in-the-style", &[("a1", (style.name).to_string()), ("target", (target).to_string()), ("a2", (map.display()).to_string()), ("a3", (style.instruction().trim()).to_string()), ("title", (title).to_string())])
}

impl Store {
    /// A style's free text may be used: it is the one the person confirmed. One with none has nothing to confirm.
    pub fn style_allowed(&self, style: &Style) -> bool {
        let text = style.free_text();
        text.is_empty() || self.trusted(&style.path.to_string_lossy(), &text)
    }
    pub fn allow_style(&self, style: &Style) { let _ = self.trust(&style.path.to_string_lossy(), &style.free_text()); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch;
    fn put(st: &Store, id: &str, text: &str) {
        let dir = st.data.root.join("addons/map-styles").join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("style.json"), text).unwrap();
    }
    #[test]
    fn a_folder_is_a_style_and_deleting_it_goes_back_to_default() {
        let st = scratch("ms");
        put(&st, "kanban", r#"{"format":1,"name":"Kanban","frame":"columns","later":1}"#);
        put(&st, "broken", "{ not json");
        put(&st, "default", r#"{"name":"fake"}"#);
        assert_eq!(list(&st.data).iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["kanban"]);
        assert_eq!(current(&st.data, "map"), DEFAULT);
        set_current(&st.data, "map", "kanban");
        set_current(&st.data, "n1", "kanban");
        assert_eq!(current(&st.data, "map"), "kanban");
        fs::remove_dir_all(st.data.root.join("addons/map-styles/kanban")).unwrap();
        assert_eq!(current(&st.data, "map"), DEFAULT);
        assert!(find(&st.data, "../kanban").is_none());
    }
    #[test]
    fn only_what_a_worker_can_write_is_asked_and_reasons_only_on_the_way() {
        let st = scratch("mm");
        put(&st, "x", r#"{"frame":"route","needs":["kind","reason","state","visits"]}"#);
        let style = find(&st.data, "x").unwrap();
        let nodes = vec![json!({"id":"a","label":"a","kind":"note"}), json!({"id":"b","parent":"a","label":"b"}), json!({"id":"c","parent":"a","label":"c"})];
        let need = missing(&style, &nodes, "b");
        assert_eq!(need, vec![("kind".to_string(), vec!["b".to_string(), "c".to_string()]), ("reason".to_string(), vec!["a".to_string(), "b".to_string()])]);
        assert_eq!(missing_text(&need), "Kind 2, Reason 2");
    }
    #[test]
    fn branches_paths_and_menu_items() {
        let st = scratch("mb");
        let data = json!({"nodes":[{"id":"a"},{"id":"b","parent":"a"},{"id":"c","parent":"b"},{"id":"z"}]});
        assert_eq!(scoped(&data, Some("b")).len(), 2);
        assert_eq!(path_to(arr(&data, "nodes"), "c").iter().map(|n| s(n, "id").to_string()).collect::<Vec<_>>(), vec!["a", "b", "c"]);
        put(&st, "m", r#"{"node_menu":[{"label":"summary","do":"request","arg":"{branch} {id}"},{"label":"bad","do":"erase","arg":"x"},{"label":"board","do":"style","arg":"kanban"}]}"#);
        let m = find(&st.data, "m").unwrap();
        assert_eq!(m.node_items().len(), 2);
        assert_eq!(m.frame(), None);
        assert_eq!(fill_item("{branch} {id}", "n", "top › n", "i"), "top › n i");
        assert!(!st.style_allowed(&m), "free text from elsewhere waits for the person");
        st.allow_style(&m);
        assert!(st.style_allowed(&m));
        fs::write(&m.path, r#"{"node_menu":[{"label":"summary","do":"request","arg":"other text"}]}"#).unwrap();
        assert!(!st.style_allowed(&find(&st.data, "m").unwrap()), "changed text is confirmed again");
    }
}
