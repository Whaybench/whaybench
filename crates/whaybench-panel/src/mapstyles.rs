//! Ways of drawing the conversation map. `default`, the tree, is the map panel itself. Every other way
//! is an add-on, a folder in `addons/map-styles/`: a declaration of a frame the panel draws and the node data it needs.
//! The format for people who make them is in addons/map-styles/README.md.
use std::{collections::{HashMap, HashSet}, fs, path::PathBuf};

use serde_json::{Value, json};
use wb_core::paths::Data;

use wb_store::{arr, read, s, sha, write_json};

pub const DEFAULT: &str = "default";
const FRAMES: [&str; 5] = ["tree", "columns", "route", "time", "questions"];
/// Fields a side worker can write.
pub const FILLED: [&str; 3] = ["kind", "reason", "links"];

/// How a side worker is asked for a field.
pub fn filled_hint(field: &str) -> &'static str {
    match field {
        "kind" => wb_core::i18n::ai("one-of-question-option-pro-con"),
        "reason" => wb_core::i18n::ai("one-line-about-40-characters-why"),
        _ => wb_core::i18n::ai("relations-to-nodes-of-other-branches"),
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

fn valid_id(id: &str) -> bool { id != DEFAULT && wb_core::addons::valid_id(id) }

/// How a frame's setting is changed in Conversation map → Display style settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Setting {
    /// One of these values; the first is what a style without the setting gets.
    Choice(&'static [&'static str]),
    Flag,
    Count(u64, u64),
    /// Words in order, such as the columns.
    List,
}

/// The settings a frame has (addons/map-styles/README.md). Only these are taken from a style's settings.json.
pub fn settings_of(frame: &str) -> &'static [(&'static str, Setting)] {
    match frame {
        "tree" => &[("filter", Setting::Choice(&["", "path"])), ("show_kind", Setting::Flag)],
        "columns" => &[("group", Setting::Choice(&["state", "kind", "top"])), ("columns", Setting::List), ("recent_done", Setting::Count(0, 50))],
        "time" => &[("range", Setting::Choice(&["topic", "all", "decision"]))],
        _ => &[],
    }
}

fn fits(setting: Setting, v: &Value) -> bool {
    match setting {
        Setting::Choice(options) => v.as_str().is_some_and(|x| options.contains(&x)),
        Setting::Flag => v.is_boolean(),
        Setting::Count(lo, hi) => v.as_u64().is_some_and(|n| (lo..=hi).contains(&n)),
        Setting::List => v.as_array().is_some_and(|a| a.iter().all(|x| x.as_str().is_some_and(|w| !w.is_empty()))),
    }
}

/// The person's changes to a style live beside it: deleting the folder takes them along, and the style a program
/// version brings is written again without losing them.
fn settings_file(d: &Data, id: &str) -> PathBuf { d.root.join("addons/map-styles").join(id).join("settings.json") }

/// Changes one setting of a style; `null` goes back to the style's own value.
pub fn set(d: &Data, id: &str, key: &str, value: Value) -> wb_store::Res<()> {
    if !valid_id(id) { return Err("no such style".into()); }
    let mut over = read(&settings_file(d, id)).filter(Value::is_object).unwrap_or_else(|| json!({}));
    if value.is_null() { over.as_object_mut().map(|o| o.remove(key)); } else { over[key] = value; }
    write_json(&settings_file(d, id), &over)
}

/// The tree, Default, is the program's own: its indent (letters per level) is in the data folder's settings.json.
pub const INDENT: (u64, u64, u64) = (1, 8, 4);

pub fn default_indent(d: &Data) -> u64 {
    read(&d.root.join("settings.json")).and_then(|v| v["map_indent"].as_u64()).filter(|n| (INDENT.0..=INDENT.1).contains(n)).unwrap_or(INDENT.2)
}

pub fn set_default_indent(d: &Data, letters: u64) -> wb_store::Res<()> {
    let path = d.root.join("settings.json");
    let mut v = read(&path).filter(Value::is_object).unwrap_or_else(|| json!({}));
    v["map_indent"] = json!(letters.clamp(INDENT.0, INDENT.1));
    write_json(&path, &v)
}

/// The style as it came, without the person's changes.
pub fn reset(d: &Data, id: &str) { if valid_id(id) { let _ = fs::remove_file(settings_file(d, id)); } }

pub fn find(d: &Data, id: &str) -> Option<Style> {
    if !valid_id(id) { return None; }
    let path = d.root.join("addons/map-styles").join(id).join("style.json");
    let mut raw = read(&path)?;
    // The person's settings over the style's own, only those its frame has and only valid ones: anything else in the
    // file (an instruction, say) never reaches the AI from here.
    if let (Some(frame), Some(over)) = (raw.get("frame").and_then(Value::as_str).map(String::from), read(&settings_file(d, id))) {
        for (key, setting) in settings_of(&frame) {
            if let Some(v) = over.get(*key).filter(|v| fits(*setting, v)) { raw[*key] = v.clone(); }
        }
    }
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

/// The style a new conversation from the map is given by: the one chosen for it, else the main map's.
pub fn carry_choice(d: &Data) -> String {
    let id = read(&choice_file(d)).and_then(|v| v.get("carry")?.as_str().map(String::from)).unwrap_or_default();
    if id == DEFAULT || find(d, &id).is_some() { id } else { current(d, "map") }
}

/// What a new conversation from the map takes when this style is chosen: a tree with the path filter (Focus) takes the
/// way to the current node and every open node, with the branches above them; every other style takes the whole map.
pub fn carried(style: Option<&Style>, nodes: &[Value], current: &str) -> Vec<Value> {
    let focus = style.is_some_and(|st| st.frame() == Some("tree") && s(&st.raw, "filter") == "path");
    if !focus { return nodes.to_vec(); }
    let mut keep: HashSet<String> = path_to(nodes, current).iter().map(|n| s(n, "id").to_string()).collect();
    for n in nodes.iter().filter(|n| wb_core::map::is_open(n)) {
        keep.extend(path_to(nodes, s(n, "id")).iter().map(|a| s(a, "id").to_string()));
    }
    // What a kept node requires or assumes comes too, with its branch: the map is not valid without the nodes named.
    let ids: HashSet<&str> = nodes.iter().map(|n| s(n, "id")).collect();
    loop {
        let named: Vec<String> = nodes.iter().filter(|n| keep.contains(s(n, "id")))
            .flat_map(|n| [s(n, "requires"), s(n, "assumes")])
            .flat_map(|e| e.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_')).filter(|w| ids.contains(w)).map(String::from).collect::<Vec<_>>())
            .filter(|id| !keep.contains(id)).collect();
        if named.is_empty() { break; }
        for id in named { keep.extend(path_to(nodes, &id).iter().map(|a| s(a, "id").to_string())); }
    }
    nodes.iter().filter(|n| keep.contains(s(n, "id"))).cloned().collect()
}

/// The session's map as `style` passes it on, written for `session new` to start from. None when the style takes the
/// whole map: the new conversation then takes it over from the session itself.
pub fn carry_file(d: &Data, session: &str, style: &str) -> Option<PathBuf> {
    let st = find(d, style)?;
    let data = wb_core::sessions::map_of(d, session)?;
    let nodes = arr(&data, "nodes");
    let kept = carried(Some(&st), nodes, s(&data, "current"));
    if kept.len() == nodes.len() { return None; }
    let dir = d.cache().join("carry");
    // Read when their conversation started; a day later they are of no use.
    for e in fs::read_dir(&dir).into_iter().flatten().flatten() {
        if e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|a| a.as_secs() > 86_400) { let _ = fs::remove_file(e.path()); }
    }
    let path = dir.join(format!("{session}-{}.json", chrono::Local::now().timestamp_millis()));
    let title = format!("{} · {}", wb_core::sessions::session_name(d, session), st.name);
    let body = json!({"format": "whaybench-map", "version": data.get("version"), "title": title, "from": session, "style": style,
        "current": data.get("current"), "nodes": wb_core::map::saved_nodes(&kept)});
    wb_core::paths::write_atomic(&path, body.to_string()).ok()?;
    Some(path)
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
            let on_way: HashSet<String> = path_to(nodes, current).iter().map(|n| s(n, "id").to_string()).collect();
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
pub fn draw_request(style: &Style, session: &str, scope: Option<(&str, &str)>) -> String {
    let title = wb_core::i18n::f("map-style", &[("a1", (scope.map(|(_, l)| format!(" · {}", l.chars().take(12).collect::<String>())).unwrap_or_default()).to_string())]);
    let target = scope.map_or_else(|| wb_core::i18n::ai("the-conversation-map-of-this-session").to_string(), |(i, l)| wb_core::i18n::ai_f("branch-node-and-below", &[("l", (l).to_string()), ("i", (i).to_string())]));
    // The map file only names the nodes: the agent reads the map through the command.
    let map = format!("whaybench map {session}");
    wb_core::i18n::ai_f("map-style-show-in-the-style", &[("a1", (style.name).to_string()), ("target", (target).to_string()), ("a2", map), ("a3", (style.instruction().trim()).to_string()), ("title", (title).to_string())])
}

// A style's free text goes into the AI as written, so it is used only after the person read and confirmed it;
// a confirmation holds for that exact text.
fn trust_file(d: &Data) -> PathBuf { d.root.join("addon-trust.json") }

/// A style's free text may be used: it is the one the person confirmed. One with none has nothing to confirm.
pub fn allowed(d: &Data, style: &Style) -> bool {
    let text = style.free_text();
    text.trim().is_empty() || read(&trust_file(d)).is_some_and(|k| s(&k, &style.path.to_string_lossy()) == sha(&text))
}

pub fn allow(d: &Data, style: &Style) {
    let mut k = read(&trust_file(d)).unwrap_or_else(|| json!({}));
    k[style.path.to_string_lossy().as_ref()] = Value::String(sha(&style.free_text()));
    let _ = write_json(&trust_file(d), &k);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn focus_carries_the_way_and_the_open_nodes_others_all() {
        let n = |id: &str, parent: &str, state: &str| json!({"id": id, "parent": parent, "state": state});
        let nodes = vec![n("a", "", "done"), n("a1", "a", "done"), n("a2", "a", "plan"), n("b", "", "done"), n("b1", "b", "done"), n("c", "", "doing")];
        let focus = Style { id: "focus".into(), name: "Focus".into(), raw: json!({"frame": "tree", "filter": "path"}), path: PathBuf::new() };
        let ids = |v: Vec<Value>| v.iter().map(|x| s(x, "id").to_string()).collect::<Vec<_>>();
        assert_eq!(ids(carried(Some(&focus), &nodes, "b1")), ["a", "a2", "b", "b1", "c"], "way to b1, open a2 with its branch, open c; a1 stays");
        assert_eq!(carried(None, &nodes, "b1").len(), 6, "the tree takes all");
        let mut needs = nodes.clone();
        needs[5]["requires"] = json!("a1 ∧ ¬b");
        assert_eq!(ids(carried(Some(&focus), &needs, "b1")), ["a", "a1", "a2", "b", "b1", "c"], "a closed node an open one requires comes along");
    }
    struct St { data: Data }
    fn scratch(name: &str) -> St {
        let d = std::env::temp_dir().join(format!("wb-panel-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        St { data: Data::new(d) }
    }
    fn put(st: &St, id: &str, text: &str) {
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
        put(&st, "x", r#"{"frame":"route","needs":["kind","reason","state"]}"#);
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
        assert!(!allowed(&st.data, &m), "free text from elsewhere waits for the person");
        allow(&st.data, &m);
        assert!(allowed(&st.data, &m));
        fs::write(&m.path, r#"{"node_menu":[{"label":"summary","do":"request","arg":"other text"}]}"#).unwrap();
        assert!(!allowed(&st.data, &find(&st.data, "m").unwrap()), "changed text is confirmed again");
    }
    #[test]
    fn a_persons_settings_go_over_the_style_but_only_its_frames_and_only_valid_ones() {
        let st = scratch("mset");
        put(&st, "k", r#"{"name":"K","frame":"columns","group":"state","instruction":"as written"}"#);
        set(&st.data, "k", "group", json!("kind")).unwrap();
        set(&st.data, "k", "recent_done", json!(500)).unwrap();
        set(&st.data, "k", "instruction", json!("do something else")).unwrap();
        let k = find(&st.data, "k").unwrap();
        assert_eq!(s(&k.raw, "group"), "kind");
        assert!(k.raw.get("recent_done").is_none(), "out of its range");
        assert_eq!(k.instruction(), "as written", "only the frame's settings come from the file");
        set(&st.data, "k", "group", Value::Null).unwrap();
        assert_eq!(s(&find(&st.data, "k").unwrap().raw, "group"), "state");
        set(&st.data, "k", "group", json!("top")).unwrap();
        reset(&st.data, "k");
        assert_eq!(s(&find(&st.data, "k").unwrap().raw, "group"), "state");
    }
}
