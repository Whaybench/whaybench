//! The conversation map panel: a tree the AI writes and the person reads, folds and
//! acts on. With a branch: only that branch. The tree is the style `default`; every other style is an add-on, drawn in a
//! page beside it (see `mapdraw`).
use std::{cell::RefCell, collections::{HashMap, HashSet}, rc::Rc};

use gtk::{gdk, gio, glib, prelude::*};
use serde_json::{Value, json};
use wb_core::{log as record, paths::Data, records, sessions, status};
use wb_store::{Store, mapstyles, s};
use webkit6::prelude::*;

use crate::{common::{cut, every}, mapimport};

/// The mark at a node.
pub fn glyph(n: &Value, current: &str) -> &'static str {
    if s(n, "id") == current { return "●"; }
    match s(n, "state") {
        "plan" => "□",
        "doing" => "◐",
        "done" => "✓",
        "changed" => "✎",
        "dropped" => "✕",
        "postponed" => "‖",
        _ => if is_open(n) { "◇" } else { "✓" },
    }
}

use wb_core::map::is_open;

pub struct Tree { pub kids: HashMap<String, Vec<Value>> }

pub fn tree_of(data: &Value) -> Tree {
    let nodes = data.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
    let ids: HashSet<String> = nodes.iter().map(|n| s(n, "id").to_string()).collect();
    let mut kids: HashMap<String, Vec<Value>> = HashMap::new();
    for n in nodes {
        let p = s(&n, "parent").to_string();
        kids.entry(if ids.contains(&p) { p } else { String::new() }).or_default().push(n);
    }
    Tree { kids }
}

impl Tree {
    pub fn below(&self, id: &str) -> Vec<Value> {
        let mut out = Vec::new();
        for c in self.kids.get(id).into_iter().flatten() {
            out.push(c.clone());
            out.extend(self.below(s(c, "id")));
        }
        out
    }
    /// Folded at first when it and everything below is closed and the present place is not in it. Top never.
    pub fn default_folded(&self, n: &Value, top: bool, current: &str) -> bool {
        if top { return false; }
        let sub = self.below(s(n, "id"));
        !sub.is_empty() && !is_open(n) && sub.iter().all(|k| !is_open(k)) && !sub.iter().any(|k| s(k, "id") == current)
    }
}

struct Panel {
    data: Option<Value>,
    signature: String,
    flipped: HashSet<String>,
    picked: Option<String>,
    /// The style on show, and the page drawn for it.
    style: String,
    page: String,
}

/// The request the map update sends. It is for the agent, not the person, so it is English whatever the interface
/// language: the conversation is put into the node fields the map tool describes, not summarized.
pub fn update_request(scope: Option<(&str, &str)>, extra: &str, style: Option<&mapstyles::Style>) -> String {
    let mut t = String::from("[Map update] Bring this session's map up to date with the whole conversation, answers included. \
Keep every node and its id; do not shorten or merge. Put what was said into the node fields: kind, a label by its kind's \
rule, detail for the rest, state, links, at/due/requires. Send only what changed; reply in two or three lines.");
    if let Some((id, label)) = scope { t += &format!("\nScope: branch '{label}' (node {id}) and below. Do not change other branches."); }
    if let Some(st) = style { t += &format!("\nThe map is shown in the style '{}'{}.", st.name, if st.frame().is_some() { "; fill what it needs (kind, reason, links)" } else { "" }); }
    if !extra.is_empty() { t += &format!("\nAdditional request: {extra}"); }
    t
}

/// The node and what is under it as lines, for Reflect in project.
fn outline(tree: &Tree, n: &Value, depth: usize, out: &mut Vec<String>) {
    let mut extra = String::new();
    for k in ["kind", "reason", "state"] { if !s(n, k).is_empty() { extra += &format!(" [{k}: {}]", s(n, k)); } }
    out.push(format!("{}- {}{}{extra}", "  ".repeat(depth), s(n, "label"), if is_open(n) { wb_core::i18n::t("open") } else { "" }));
    for c in tree.kids.get(s(n, "id")).into_iter().flatten() { outline(tree, c, depth + 1, out); }
}

pub fn flow_of(data: &Value, id: &str) -> Vec<String> {
    let nodes = data.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut chain = Vec::new();
    let mut at = nodes.iter().find(|n| s(n, "id") == id).map(|n| s(n, "parent").to_string()).unwrap_or_default();
    while let Some(n) = nodes.iter().find(|n| s(n, "id") == at) {
        if chain.len() >= 50 { break; }
        chain.push(s(n, "label").to_string());
        at = s(n, "parent").to_string();
    }
    chain.reverse();
    chain
}

pub fn reflect_request(pid: Option<&str>, work: Option<&str>, flow: &[String], lines: &[String]) -> String {
    let mut t = String::new();
    if let (Some(p), Some(q)) = (pid, work) {
        t += &wb_core::i18n::f("project-work-first-run-whaybench-project", &[("q", (q).to_string()), ("p", (p).to_string())]);
    }
    let target = pid.map(|p| wb_core::i18n::f("project-2", &[("p", (p).to_string())])).unwrap_or_else(|| wb_core::i18n::t("a-new-project-create-it-with").into());
    let map_target = pid.unwrap_or(wb_core::i18n::t("text-3"));
    t += &wb_core::i18n::f("reflect-in-project-the-user-sent", &[("target", (target).to_string()), ("a1", (if flow.is_empty() { wb_core::i18n::t("none-top-branch").to_string() } else { flow.join(" › ") }).to_string()), ("a2", (lines.iter().take(120).cloned().collect::<Vec<_>>().join("\n")).to_string())]);
    t += &wb_core::i18n::f("n1-first-compare-with-whaybench-project", &[("map_target", (map_target).to_string())]);
    t
}

fn maps_dir(d: &Data) -> std::path::PathBuf { d.maps() }

/// Writes the branch, or the whole map, as a file in the data directory; returns its path.
fn export(d: &Data, data: &Value, root: Option<&str>, title: &str, prefix: &str) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(maps_dir(d))?;
    let tree = tree_of(data);
    let nodes: Vec<Value> = match root {
        None => data.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default(),
        Some(r) => {
            let mut v: Vec<Value> = data["nodes"].as_array().into_iter().flatten().filter(|n| s(n, "id") == r).cloned().collect();
            if let Some(o) = v.first_mut().and_then(Value::as_object_mut) { o.shift_remove("parent"); }
            v.extend(tree.below(r));
            v
        }
    };
    let span = sessions::span(d, data.get("sessionId").and_then(Value::as_str).unwrap_or(""), &nodes);
    let path = wb_core::map::free_path(&maps_dir(d), &wb_core::map::file_name(span, root, prefix));
    let body = json!({"format": "whaybench-map", "version": data.get("version"), "title": title, "exported": chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(), "from": data.get("sessionId"), "nodes": nodes});
    std::fs::write(&path, serde_json::to_string_pretty(&body).unwrap_or_default())?;
    Ok(path)
}

/// A menu item that runs a closure: collected, then attached when the menu opens.
struct Items { menu: gio::Menu, group: gio::SimpleActionGroup, n: Rc<std::cell::Cell<u32>> }

impl Items {
    fn new() -> Self { Items { menu: gio::Menu::new(), group: gio::SimpleActionGroup::new(), n: Rc::default() } }
    fn sub(&self) -> Items { Items { menu: gio::Menu::new(), group: self.group.clone(), n: self.n.clone() } }
    fn add(&self, label: &str, enabled: bool, f: impl Fn() + 'static) {
        let name = format!("a{}", self.n.get());
        self.n.set(self.n.get() + 1);
        let act = gio::SimpleAction::new(&name, None);
        act.set_enabled(enabled);
        act.connect_activate(move |_, _| f());
        self.group.add_action(&act);
        self.menu.append(Some(label), Some(&format!("m.{name}")));
    }
}

fn popup(on: &impl IsA<gtk::Widget>, items: Items, x: f64, y: f64) {
    let pop = gtk::PopoverMenu::from_model_full(&items.menu, gtk::PopoverMenuFlags::NESTED);
    pop.set_parent(on);
    pop.insert_action_group("m", Some(&items.group));
    pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    pop.set_has_arrow(false);
    pop.connect_closed(|p| { let p = p.clone(); glib::idle_add_local_once(move || p.unparent()); });
    pop.popup();
}

/// The path of a panel's imported map. Such a panel's scope is `@<path>` and its title.
fn imported_of(scope: Option<&(String, String)>) -> Option<&str> { scope.and_then(|x| x.0.strip_prefix('@')) }
/// The node a branch panel shows.
fn branch_root(scope: Option<&(String, String)>) -> Option<&str> { scope.filter(|x| !x.0.starts_with('@')).map(|x| x.0.as_str()) }

/// Open map from the menu bar: a saved map file, chosen in the folder maps are saved to, opens as an imported map.
fn open_map_file(win: &gtk::ApplicationWindow, d: &Data) {
    let filter = gtk::FileFilter::new();
    filter.add_suffix("json");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder().title(wb_core::i18n::t("open-map")).modal(true).filters(&filters)
        .initial_folder(&gio::File::for_path(maps_dir(d))).build();
    let d = d.clone();
    dialog.open(Some(win), None::<&gio::Cancellable>, move |r| {
        if let Some(path) = r.ok().and_then(|f| f.path()) { mapimport::bring_in(&d, &mapimport::Source::File(path)); }
    });
}

/// Adds Open map.
fn import_menu(d: &Data, items: &Items) {
    // The same as the menu bar's: a saved map file, chosen in a file dialog this panel shows.
    let d2 = d.clone();
    items.add(wb_core::i18n::t("open-map"), true, move || status::send(&d2, "map", json!({"do": "open"})));
}

/// The menu of an imported map: read only, by purpose.
fn imported_menu(d: &Data, data: &Value, node: Option<&Value>, path: &str, title: &str) -> Items {
    let items = Items::new();
    let session = sessions::target(d);
    let tree = tree_of(data);
    if let Some(n) = node {
        let refer = items.sub();
        let (id, label) = (s(n, "id").to_string(), s(n, "label").to_string());
        let mut lines = Vec::new();
        outline(&tree, n, 0, &mut lines);
        let flow = flow_of(data, &id);
        // Related answers of the session the map was saved from: dimmed when its answers are not on this computer.
        {
            let from = s(data, "from").to_string();
            let at = records::node_time(n);
            let kept = !from.is_empty() && records::lineage(d, &from).iter().any(|n| d.root.join("answer-log").join(format!("{n}.jsonl")).exists());
            let (d2, id2) = (d.clone(), id.clone());
            items.add(wb_core::i18n::t("related-answers-3"), at.is_some() && kept, move || status::send(&d2, "answer", json!({"session": from, "node": id2, "at": at.unwrap_or(0)})));
        }
        {
            let (d2, s2, n2, p2, t2, f2, l2) = (d.clone(), session.clone(), n.clone(), path.to_string(), title.to_string(), flow.clone(), lines.clone());
            refer.add(wb_core::i18n::t("take-as-reference-this-node"), session.is_some(), move || {
                let Some(t) = &s2 else { return };
                let text = mapimport::reference_request(&p2, &t2, &n2, &f2, &l2);
                if sessions::queue_prompt(&d2, t, &text) { status::say(&d2, &wb_core::i18n::f("reference-request-sent", &[("title", cut(&t2, 20)), ("head", cut(s(&n2, "label"), 20)), ("session", cut(t, 8))]), 8); }
                else { status::say(&d2, &wb_core::i18n::f("request-not-sent-session-is-off", &[("a1", cut(t, 8))]), 8); }
            });
        }
        let sub = items.sub();
        let store = Store::new(d.clone());
        for p in store.list_projects() {
            let (pid, name) = (s(&p, "id").to_string(), s(&p, "name").to_string());
            let (st, d2, s2, l2, f2, label2) = (store.clone(), d.clone(), session.clone().unwrap_or_default(), lines.clone(), flow.clone(), label.clone());
            sub.add(&name, session.is_some(), move || {
                let Ok(q) = st.queue_add(&pid, "reflect", &label2, &s2) else { return };
                if sessions::queue_prompt(&d2, &s2, &reflect_request(Some(&pid), Some(&q), &f2, &l2)) { status::say(&d2, &wb_core::i18n::f("project-reflection-request-sent-session", &[("a1", cut(&label2, 20)), ("a2", cut(&s2, 8))]), 8); }
                else { st.queue_drop(&pid, &q); status::say(&d2, &wb_core::i18n::f("request-not-sent-session-is-off", &[("a1", cut(&s2, 8))]), 8); }
            });
        }
        refer.menu.append_submenu(Some(wb_core::i18n::t("reflect-in-project")), &sub.menu);
        items.menu.append_section(None, &refer.menu);
    }
    let whole = items.sub();
    {
        let (d2, s2, p2, t2, data2) = (d.clone(), session.clone(), path.to_string(), title.to_string(), data.clone());
        whole.add(wb_core::i18n::t("take-as-reference-whole-map"), session.is_some(), move || {
            let Some(t) = &s2 else { return };
            let text = mapimport::whole_reference_request(&p2, &t2, &data2);
            if sessions::queue_prompt(&d2, t, &text) { status::say(&d2, &wb_core::i18n::f("reference-request-sent", &[("title", cut(&t2, 20)), ("head", wb_core::i18n::t("whole-map").to_string()), ("session", cut(t, 8))]), 8); }
            else { status::say(&d2, &wb_core::i18n::f("request-not-sent-session-is-off", &[("a1", cut(t, 8))]), 8); }
        });
    }
    items.menu.append_section(None, &whole.menu);
    let keep = items.sub();
    {
        let (d2, p2, t2) = (d.clone(), path.to_string(), title.to_string());
        keep.add(wb_core::i18n::t("new-conversation-from-this-map"), true, move || {
            crate::common::ctl(&d2, vec!["--data-dir".into(), d2.root.to_string_lossy().to_string(), "session".into(), "new".into(), p2.clone()]);
            status::say(&d2, &wb_core::i18n::f("new-session-from-imported-map", &[("title", cut(&t2, 24))]), 8);
        });
    }
    items.menu.append_section(None, &keep.menu);
    let store = items.sub();
    {
        let (d2, data2) = (d.clone(), data.clone());
        let (root, name) = node.map_or((None, title.to_string()), |n| (Some(s(n, "id").to_string()), s(n, "label").to_string()));
        store.add(if root.is_some() { wb_core::i18n::t("export-branch") } else { wb_core::i18n::t("export-whole-map") }, true, move || match export(&d2, &data2, root.as_deref(), &name, "") {
            Ok(p) => status::say(&d2, &wb_core::i18n::f("map-exported", &[("a1", p.display().to_string())]), 8),
            Err(e) => status::say(&d2, &wb_core::i18n::f("map-export-failed", &[("e", e.to_string())]), 8),
        });
    }
    items.menu.append_section(None, &store.menu);
    items
}

fn node_menu(d: &Data, data: &Value, node: Option<&Value>, scope: Option<&(String, String)>) -> Items {
    if let Some(path) = imported_of(scope) { return imported_menu(d, data, node, path, scope.map_or("", |x| x.1.as_str())); }
    let items = Items::new();
    let session = sessions::target(d);
    let tree = tree_of(data);
    if let Some(n) = node {
        let (id, label) = (s(n, "id").to_string(), s(n, "label").to_string());
        // Where a node came from, if it came from an imported map.
        if let Some(src) = n.get("source").filter(|x| !s(x, "file").is_empty()).cloned() {
            let d2 = d.clone();
            items.add(wb_core::i18n::t("open-source-imported"), true, move || open_node_source(&d2, &src));
        }
        {
            let (id2, label2) = (id.clone(), label.clone());
            items.add(wb_core::i18n::t("open-branch-in-a-new-panel"), true, move || {
                if let Ok(exe) = std::env::current_exe() { let _ = std::process::Command::new(exe).args(["map", &id2, &label2]).spawn(); }
            });
        }
        // Related answers: dimmed when the node has no time.
        let at = records::node_time(n);
        {
            let (d2, s2, id2) = (d.clone(), session.clone().unwrap_or_default(), id.clone());
            items.add(wb_core::i18n::t("related-answers-3"), at.is_some(), move || status::send(&d2, "answer", json!({"session": s2, "node": id2, "at": at.unwrap_or(0)})));
        }
        // Reflect in project.
        let sub = items.sub();
        let store = Store::new(d.clone());
        let mut lines = Vec::new();
        outline(&tree, n, 0, &mut lines);
        let flow = flow_of(data, &id);
        for p in store.list_projects() {
            let (pid, name) = (s(&p, "id").to_string(), s(&p, "name").to_string());
            let (st, d2, s2, l2, f2, label2) = (store.clone(), d.clone(), session.clone().unwrap_or_default(), lines.clone(), flow.clone(), label.clone());
            sub.add(&name, session.is_some(), move || {
                let Ok(q) = st.queue_add(&pid, "reflect", &label2, &s2) else { return };
                if sessions::queue_prompt(&d2, &s2, &reflect_request(Some(&pid), Some(&q), &f2, &l2)) {
                    status::say(&d2, &wb_core::i18n::f("project-reflection-request-sent-session", &[("a1", (cut(&label2, 20)).to_string()), ("a2", (cut(&s2, 8)).to_string())]), 8);
                } else {
                    st.queue_drop(&pid, &q);
                    status::say(&d2, &wb_core::i18n::f("request-not-sent-session-is-off", &[("a1", (cut(&s2, 8)).to_string())]), 8);
                }
            });
        }
        {
            let (d2, s2, l2, f2) = (d.clone(), session.clone().unwrap_or_default(), lines.clone(), flow.clone());
            sub.add(wb_core::i18n::t("new-project"), session.is_some(), move || {
                if sessions::queue_prompt(&d2, &s2, &reflect_request(None, None, &f2, &l2)) { status::say(&d2, wb_core::i18n::t("new-project-request-sent"), 8); }
            });
        }
        items.menu.append_submenu(Some(wb_core::i18n::t("reflect-in-project")), &sub.menu);
    }
    let section = items.sub();
    {
        let (d2, data2) = (d.clone(), data.clone());
        let (root, title) = match node.map(|n| (s(n, "id").to_string(), s(n, "label").to_string())).or_else(|| scope.cloned()) { Some((i, l)) => (Some(i), l), None => (None, wb_core::i18n::t("whole-conversation-map").to_string()) };
        section.add(if root.is_some() { wb_core::i18n::t("export-branch") } else { wb_core::i18n::t("export-whole-map") }, true, move || {
            match export(&d2, &data2, root.as_deref(), &title, "") {
                Ok(p) => status::say(&d2, &wb_core::i18n::f("map-exported", &[("a1", (p.display()).to_string())]), 8),
                Err(e) => status::say(&d2, &wb_core::i18n::f("map-export-failed", &[("e", (e).to_string())]), 8),
            }
        });
    }
    items.menu.append_section(None, &section.menu);
    if session.is_some() { let more = items.sub(); import_menu(d, &more); items.menu.append_section(None, &more.menu); }
    // Deleting is a submenu, so it takes two presses; what goes waits in the trash, restorable from the alerts panel.
    if let (Some(n), Some(target)) = (node, session.as_ref()) {
        let (id, label) = (s(n, "id").to_string(), s(n, "label").to_string());
        let below = tree.below(&id).len();
        let drop = items.sub();
        let act = |branch: bool, what: String| {
            let (d2, t2, label2, id2) = (d.clone(), target.clone(), label.clone(), id.clone());
            let n = if branch { below + 1 } else { 1 };
            move || match wb_core::map::trash(&d2.root, &t2, &id2, branch) {
                Ok(_) => {
                    record::decide("map", "delete", &what, &[("session", t2.clone()), ("node", id2.clone())]);
                    status::say(&d2, &wb_core::i18n::f("moved-to-trash", &[("a1", cut(&label2, 24)), ("n", n.to_string())]), 8);
                }
                Err(e) => status::say(&d2, &wb_core::i18n::f("delete-failed", &[("e", e.to_string())]), 8),
            }
        };
        if below > 0 {
            drop.add(&wb_core::i18n::f("delete-whole-branch-including-below", &[("below", (below).to_string())]), true, act(true, "whole branch".into()));
            drop.add(wb_core::i18n::t("delete-node-only-nodes-below-move"), true, act(false, "node only".into()));
        } else {
            drop.add(wb_core::i18n::t("delete-node"), true, act(true, "node".into()));
        }
        let sec = items.sub();
        sec.menu.append_submenu(Some(wb_core::i18n::t("delete")), &drop.menu);
        items.menu.append_section(None, &sec.menu);
    }
    // After the standard items: what the panel's style adds. It cannot take any away.
    if let Some(style) = mapstyles::find(d, &mapstyles::current(d, panel_key(scope))) {
        let own = items.sub();
        let allowed = Store::new(d.clone()).style_allowed(&style);
        let (id, label) = node.map_or((String::new(), String::new()), |n| (s(n, "id").to_string(), s(n, "label").to_string()));
        let path = if node.is_some() { let mut f = flow_of(data, &id); f.push(label.clone()); f.join(" › ") } else { String::new() };
        let mut any = false;
        for item in style.node_items() {
            let arg = mapstyles::fill_item(&item.arg, &label, &path, &id);
            let (d2, key) = (d.clone(), panel_key(scope).to_string());
            match item.action.as_str() {
                "request" => {
                    let target = session.clone();
                    own.add(&item.label, allowed && target.is_some() && node.is_some(), move || {
                        let Some(t) = &target else { return };
                        if !sessions::queue_prompt(&d2, t, &arg) { status::say(&d2, &wb_core::i18n::f("request-not-sent-session-is-off", &[("a1", (cut(t, 8)).to_string())]), 8); }
                    });
                }
                "style" => own.add(&item.label, true, move || mapstyles::set_current(&d2, &key, &arg)),
                _ => own.add(&item.label, true, move || { let _ = wb_core::view::open(&d2, &arg, None); }),
            }
            any = true;
        }
        if any { items.menu.append_section(Some(&wb_core::i18n::f("style", &[("a1", (style.name).to_string())])), &own.menu); }
    }
    items
}

/// A node's source: the imported map's panel, at the original node.
fn open_node_source(d: &Data, src: &Value) {
    let path = std::path::Path::new(s(src, "file"));
    if !path.is_file() { status::say(d, &wb_core::i18n::f("source-not-opened-no-file", &[("path", path.display().to_string())]), 8); return; }
    mapimport::show(d, path, s(src, "title"), Some(s(src, "node")));
}

/// Each map panel keeps its own style: the main one under `map`, a branch panel under its node.
fn panel_key(scope: Option<&(String, String)>) -> &str { scope.map_or("map", |x| x.0.as_str()) }

/// What a style from elsewhere sends to the AI is shown before it is used for the first time.
fn confirm_style(win: &gtk::ApplicationWindow, d: &Data, style: mapstyles::Style, then: impl Fn() + 'static) {
    let dlg = gtk::Window::builder().title(wb_core::i18n::t("map-style-instructions")).transient_for(win).modal(true).decorated(false).build();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    body.set_margin_top(10); body.set_margin_bottom(10); body.set_margin_start(10); body.set_margin_end(10);
    let say = gtk::Label::new(Some(&wb_core::i18n::f("the-instructions-of-style-go-to", &[("a1", (style.name).to_string()), ("a2", (style.path.display()).to_string())])));
    say.set_xalign(0.0); say.set_wrap(true); say.set_max_width_chars(20);
    body.append(&say);
    let text = gtk::TextView::new();
    text.set_editable(false);
    text.set_wrap_mode(gtk::WrapMode::WordChar);
    text.buffer().set_text(&style.free_text());
    let box_ = gtk::ScrolledWindow::new();
    box_.set_child(Some(&text));
    box_.set_vexpand(true);
    body.append(&box_);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_halign(gtk::Align::End);
    let (cancel, ok) = (gtk::Button::with_label(wb_core::i18n::t("cancel")), gtk::Button::with_label(wb_core::i18n::t("confirm-and-use")));
    row.append(&cancel); row.append(&ok);
    body.append(&row);
    dlg.set_child(Some(&body));
    { let w = dlg.clone(); cancel.connect_clicked(move |_| w.close()); }
    { let (w, d2) = (dlg.clone(), d.clone()); ok.connect_clicked(move |_| { Store::new(d2.clone()).allow_style(&style); w.close(); then(); }); }
    dlg.present();
}

/// The style menu: the tree, then the styles there are.
fn style_menu(win: &gtk::ApplicationWindow, on: &gtk::Button, d: &Data, key: &str) {
    let items = Items::new();
    let now = mapstyles::current(d, key);
    let mark = |id: &str, name: &str| format!("{}{name}", if id == now { "✓ " } else { "" });
    { let (d2, k) = (d.clone(), key.to_string()); items.add(&mark(mapstyles::DEFAULT, wb_core::i18n::t("default")), true, move || mapstyles::set_current(&d2, &k, mapstyles::DEFAULT)); }
    for style in mapstyles::list(d).into_iter().filter(|x| x.in_menu()) {
        let label = mark(&style.id, &style.name);
        let (d2, k, w2) = (d.clone(), key.to_string(), win.clone());
        items.add(&label, true, move || {
            let (d3, k3, id) = (d2.clone(), k.clone(), style.id.clone());
            let choose = move || mapstyles::set_current(&d3, &k3, &id);
            if Store::new(d2.clone()).style_allowed(&style) { choose(); } else { confirm_style(&w2, &d2, style.clone(), choose); }
        });
    }
    let (x, y) = (0.0, on.height() as f64);
    popup(on, items, x, y);
}

/// The page of a style, or the tree. The place in the page stays when it is drawn again.
#[allow(clippy::too_many_arguments)]
fn show_style(stack: &gtk::Stack, web: &Rc<RefCell<Option<webkit6::WebView>>>, panel: &Rc<RefCell<Panel>>, d: &Data, scope: Option<&(String, String)>, on_web: &Rc<dyn Fn(&webkit6::WebView)>) {
    let (style_id, data, picked) = { let p = panel.borrow(); (p.style.clone(), p.data.clone(), p.picked.clone()) };
    let Some(style) = mapstyles::find(d, &style_id).filter(|x| x.frame().is_some()) else { stack.set_visible_child_name("tree"); return };
    let Some(data) = data else { stack.set_visible_child_name("tree"); return };
    if web.borrow().is_none() {
        let view = webkit6::WebView::new();
        crate::common::web_menu_in_language(&view);
        crate::common::follow_zoom(&view, d);
        if let Some(settings) = WebViewExt::settings(&view) { settings.set_default_charset("utf-8"); }
        on_web(&view);
        stack.add_named(&view, Some("page"));
        *web.borrow_mut() = Some(view);
    }
    stack.set_visible_child_name("page");
    let page = crate::mapdraw::page(d, &data, &style, branch_root(scope), picked.as_deref());
    if panel.borrow().page == page { return; }
    panel.borrow_mut().page = page.clone();
    let view = web.borrow().clone().unwrap();
    let v2 = view.clone();
    view.evaluate_javascript("window.scrollY", None, None, None::<&gio::Cancellable>, move |r| {
        let top = r.map(|x| x.to_double()).unwrap_or(0.0);
        v2.load_html(&page.replace("</body>", &format!("<script>window.scrollTo(0,{top})</script></body>")), Some("about:blank"));
    });
}

/// What a newly chosen style needs: the side worker for missing node data, or the AI for a style with no frame.
fn style_chosen(d: &Data, panel: &Rc<RefCell<Panel>>, scope: Option<&(String, String)>) {
    let id = panel.borrow().style.clone();
    let Some(style) = mapstyles::find(d, &id) else { status::say(d, &wb_core::i18n::f("map-style-2", &[("a1", (wb_core::i18n::t("default")).to_string())]), 5); return };
    // A read-only panel has no session to ask: what is there is drawn.
    if imported_of(scope).is_some() { status::say(d, &wb_core::i18n::f("map-style-2", &[("a1", style.name.clone())]), 5); return; }
    record::decide("map", "style changed", "the person chose", &[("panel", panel_key(scope).to_string()), ("style", id)]);
    let session = sessions::target(d);
    let Some(session) = session else {
        if style.frame().is_some() { status::say(d, &wb_core::i18n::f("map-style-2", &[("a1", (style.name).to_string())]), 5); } else { status::say(d, &wb_core::i18n::f("map-style-not-shown-no-session", &[("a1", (style.name).to_string())]), 6); }
        return;
    };
    if style.frame().is_some() {
        let Some(data) = panel.borrow().data.clone() else { status::say(d, &wb_core::i18n::f("map-style-2", &[("a1", (style.name).to_string())]), 5); return };
        let root = scope.map(|x| x.0.clone());
        let need = mapstyles::missing(&style, &mapstyles::scoped(&data, root.as_deref()), s(&data, "current"));
        if need.is_empty() || scope.is_some_and(|x| x.0.is_empty()) { status::say(d, &wb_core::i18n::f("map-style-2", &[("a1", (style.name).to_string())]), 5); return; }
        status::say(d, &wb_core::i18n::f("map-style-filling-missing-data-side", &[("a1", (style.name).to_string()), ("a2", (mapstyles::missing_text(&need)).to_string())]), 10);
        let d2 = d.clone();
        std::thread::spawn(move || crate::mapfill::run(&d2, &session, &style, root.as_deref()));
    } else {
        let text = mapstyles::draw_request(&style, d, &session, scope.map(|x| (x.0.as_str(), x.1.as_str())));
        if sessions::queue_prompt(d, &session, &text) { status::say(d, &wb_core::i18n::f("map-style-display-request-sent-session", &[("a1", (style.name).to_string()), ("a2", (cut(&session, 8)).to_string())]), 6); }
        else { status::say(d, &wb_core::i18n::f("request-not-sent-session-is-off", &[("a1", (cut(&session, 8)).to_string())]), 8); }
    }
}

fn escape(t: &str) -> String { glib::markup_escape_text(t).to_string() }

fn draw(list: &gtk::ListBox, panel: &Rc<RefCell<Panel>>, d: &Data, scope: Option<&(String, String)>, unit: i32) {
    let (data, flipped, picked) = {
        let p = panel.borrow();
        let Some(v) = p.data.clone() else { return };
        (v, p.flipped.clone(), p.picked.clone())
    };
    let scroll = list.parent().and_then(|v| v.parent()).and_then(|s| s.downcast::<gtk::ScrolledWindow>().ok());
    let keep = scroll.as_ref().map(|s| s.vadjustment().value());
    // Rows only, by index: a menu opened on the list is its child too, and GTK's own remove_all loops on it forever.
    while let Some(r) = list.row_at_index(0) { list.remove(&r); }
    let current = s(&data, "current").to_string();
    let tree = tree_of(&data);
    let counts = sessions::target(d).filter(|_| imported_of(scope).is_none()).map(|t| records::file_counts(d, &t)).unwrap_or_default();
    fn add(list: &gtk::ListBox, tree: &Tree, nodes: &[Value], depth: i32, top: bool, ctx: &(String, HashMap<String, usize>, HashSet<String>, Option<String>, i32, Rc<RefCell<Panel>>, Data, Option<(String, String)>, Value)) {
        let (current, counts, flipped, picked, unit, panel, d, scope, data) = ctx;
        for n in nodes {
            let id = s(n, "id").to_string();
            let here = id == *current;
            let kids = tree.kids.get(&id);
            let folded = kids.is_some() && (tree.default_folded(n, top, current) != flipped.contains(&id));
            let area = gtk::Box::new(gtk::Orientation::Horizontal, 2);
            area.set_margin_start(depth * unit);
            let arrow = gtk::Button::with_label(if kids.is_none() { " " } else if folded { "▸" } else { "▾" });
            arrow.add_css_class("flat");
            arrow.set_sensitive(kids.is_some());
            {
                let (id2, panel2, d2, scope2, list2, unit2) = (id.clone(), panel.clone(), d.clone(), scope.clone(), list.clone(), *unit);
                arrow.connect_clicked(move |_| {
                    { let mut p = panel2.borrow_mut(); if !p.flipped.remove(&id2) { p.flipped.insert(id2.clone()); } }
                    draw(&list2, &panel2, &d2, scope2.as_ref(), unit2);
                });
            }
            area.append(&arrow);
            let text = format!("{} {}{}", glyph(n, current), escape(s(n, "label")), if here { wb_core::i18n::t("current") } else { "" });
            let label = gtk::Label::new(None);
            label.set_markup(&text);
            label.set_xalign(0.0);
            label.set_wrap(true);
            label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            label.set_hexpand(true);
            if here { label.add_css_class("now"); } else if !is_open(n) { label.add_css_class("dim"); }
            if let Some(t) = n.get("detail").and_then(Value::as_str) { label.set_tooltip_text(Some(t)); }
            area.append(&label);
            // Came from an imported map: marked, a double press opens it where it was.
            if let Some(src) = n.get("source").filter(|x| !s(x, "file").is_empty()).cloned() {
                let mark = gtk::Label::new(Some(&wb_core::i18n::f("imported-mark", &[("title", cut(s(&src, "title"), 16))])));
                mark.add_css_class("small");
                mark.set_valign(gtk::Align::Start);
                let flow: Vec<&str> = src.get("path").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
                mark.set_tooltip_text(Some(&wb_core::i18n::f("imported-mark-tooltip", &[("title", s(&src, "title").to_string()), ("flow", flow.join(" › "))])));
                let press = gtk::GestureClick::new();
                press.set_button(1);
                let d2 = d.clone();
                press.connect_pressed(move |_, count, _, _| { if count == 2 { open_node_source(&d2, &src); } });
                mark.add_controller(press);
                area.append(&mark);
            }
            // The file icon, and only it, brings this node's files up: pressing nodes must leave the panels in place.
            if let Some(c) = counts.get(&id) {
                let icon = gtk::Label::new(Some(&format!("📄{c}")));
                icon.set_valign(gtk::Align::Start);
                icon.set_tooltip_text(Some(wb_core::i18n::t("show-this-node-s-files")));
                let press = gtk::GestureClick::new();
                press.set_button(1);
                {
                    let (d2, panel2, list2, id2, scope2, unit2) = (d.clone(), panel.clone(), list.clone(), id.clone(), scope.clone(), *unit);
                    press.connect_pressed(move |g, _, _, _| {
                        g.set_state(gtk::EventSequenceState::Claimed);
                        panel2.borrow_mut().picked = Some(id2.clone());
                        status::send(&d2, "files", json!({"node": id2}));
                        status::front_app(&d2, "dev.whaybench.files");
                        let (d3, panel3, list3, scope3) = (d2.clone(), panel2.clone(), list2.clone(), scope2.clone());
                        glib::idle_add_local_once(move || draw(&list3, &panel3, &d3, scope3.as_ref(), unit2));
                    });
                }
                icon.add_controller(press);
                area.append(&icon);
            }
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&area));
            row.set_widget_name(&id);
            if picked.as_deref() == Some(&id) { row.add_css_class("chosen"); }
            let click = gtk::GestureClick::new();
            click.set_button(3);
            {
                let (d2, data2, n2, scope2, row2) = (d.clone(), data.clone(), n.clone(), scope.clone(), row.clone());
                click.connect_pressed(move |g, _, x, y| {
                    g.set_state(gtk::EventSequenceState::Claimed);
                    popup(&row2, node_menu(&d2, &data2, Some(&n2), scope2.as_ref()), x, y);
                });
            }
            row.add_controller(click);
            list.append(&row);
            if let (Some(k), false) = (kids, folded) { add(list, tree, k, depth + 1, false, ctx); }
        }
    }
    let ctx = (current.clone(), counts, flipped, picked, unit, panel.clone(), d.clone(), scope.cloned(), data.clone());
    match scope.filter(|x| !x.0.starts_with('@')) {
        None => add(list, &tree, tree.kids.get("").map(Vec::as_slice).unwrap_or(&[]), 0, true, &ctx),
        Some((root, _)) => {
            let branch: Vec<Value> = data["nodes"].as_array().into_iter().flatten().filter(|n| s(n, "id") == root).cloned().collect();
            add(list, &tree, &branch, 0, true, &ctx);
        }
    }
    if let (Some(sc), Some(v)) = (scroll, keep) {
        glib::idle_add_local_once(move || sc.vadjustment().set_value(v));
    }
}

pub fn build(win: &gtk::ApplicationWindow, d: Data, scope: Option<(String, String)>) {
    if let Some((_, label)) = &scope { win.set_title(Some(&if imported_of(scope.as_ref()).is_some() { wb_core::i18n::f("imported-title", &[("title", cut(label, 14))]) } else { cut(label, 16) })); }
    // Kept over a restart; closing it for good forgets it. The program ending is not a close.
    if let Some((a, b)) = &scope {
        let args = vec![a.clone(), b.clone()];
        mapimport::remember(&d, &args);
        let d2 = d.clone();
        win.connect_close_request(move |_| { mapimport::forget(&d2, &args); glib::Propagation::Proceed });
    }
    let extra = gtk::Entry::new();
    extra.set_placeholder_text(Some(wb_core::i18n::t("additional-request-optional")));
    extra.set_hexpand(true);
    let tidy = gtk::Button::with_label(wb_core::i18n::t("tidy"));
    tidy.set_tooltip_text(Some(&wb_core::i18n::f("rebuild-the-map-from-the-whole", &[("a1", (if scope.is_some() { wb_core::i18n::t("branch-only") } else { "" }).to_string())])));
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    for m in [4] { bar.set_margin_start(m); bar.set_margin_end(m); bar.set_margin_top(m); bar.set_margin_bottom(2); }
    bar.append(&extra);
    bar.append(&tidy);
    if imported_of(scope.as_ref()).is_some() {
        extra.set_placeholder_text(Some(wb_core::i18n::t("imported-map-read-only")));
        extra.set_sensitive(false);
        tidy.set_sensitive(false);
    }
    let change = gtk::Button::with_label(wb_core::i18n::t("change"));
    change.set_tooltip_text(Some(wb_core::i18n::t("change-map-style")));
    bar.append(&change);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.set_activate_on_single_click(true);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    scroll.set_vexpand(true);
    let area = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let stack = gtk::Stack::new();
    stack.add_named(&scroll, Some("tree"));
    stack.set_vexpand(true);
    area.append(&bar);
    area.append(&stack);
    win.set_child(Some(&area));

    // One level of indent is four 'M' wide in the present font.
    let unit = { let (w, _) = win.create_pango_layout(Some("MMMM")).pixel_size(); w.max(16) };
    let panel = Rc::new(RefCell::new(Panel { data: None, signature: String::new(), flipped: HashSet::new(), picked: None, style: String::new(), page: String::new() }));
    panel.borrow_mut().style = mapstyles::current(&d, panel_key(scope.as_ref()));
    if let Some(x) = mapstyles::find(&d, &panel.borrow().style) { change.set_label(&wb_core::i18n::f("change-2", &[("a1", (cut(&x.name, 8)).to_string())])); }
    let web: Rc<RefCell<Option<webkit6::WebView>>> = Rc::default();
    { let (w2, d2, k) = (win.clone(), d.clone(), panel_key(scope.as_ref()).to_string()); change.connect_clicked(move |b| style_menu(&w2, b, &d2, &k)); }

    let ask = {
        let (d, extra, scope) = (d.clone(), extra.clone(), scope.clone());
        Rc::new(move || {
            let Some(session) = sessions::target(&d) else { status::say(&d, wb_core::i18n::t("map-tidy-not-run-no-session"), 6); return };
            let text = update_request(scope.as_ref().map(|(i, l)| (i.as_str(), l.as_str())), extra.text().trim(), mapstyles::find(&d, &mapstyles::current(&d, panel_key(scope.as_ref()))).as_ref());
            extra.set_text("");
            if sessions::queue_prompt(&d, &session, &text) {
                status::say(&d, &wb_core::i18n::f("map-update-request-sent-session", &[("a1", (scope.as_ref().map(|(_, l)| wb_core::i18n::f("branch", &[("a1", (cut(l, 12)).to_string())])).unwrap_or_default()).to_string()), ("a2", (cut(&session, 8)).to_string())]), 6);
            } else {
                status::say(&d, &wb_core::i18n::f("request-not-sent-session-is-off", &[("a1", (cut(&session, 8)).to_string())]), 8);
            }
        })
    };
    { let a = ask.clone(); tidy.connect_clicked(move |_| a()); }
    { let a = ask.clone(); extra.connect_activate(move |_| a()); }
    // One press: the files panel shows this branch's files. The same node again: not any more.
    {
        let (d, panel, scope, list2) = (d.clone(), panel.clone(), scope.clone(), list.clone());
        list.connect_row_activated(move |_, r| {
            let id = r.widget_name().to_string();
            {
                let mut p = panel.borrow_mut();
                p.picked = if p.picked.as_deref() == Some(&id) { None } else { Some(id) };
                if imported_of(scope.as_ref()).is_none() { status::send(&d, "files", json!({"node": p.picked})); }
            }
            draw(&list2, &panel, &d, scope.as_ref(), unit);
        });
    }
    // Empty space: the whole map's menu.
    let click = gtk::GestureClick::new();
    click.set_button(3);
    {
        let (d, panel, scope, list2) = (d.clone(), panel.clone(), scope.clone(), list.clone());
        click.connect_pressed(move |_, _, x, y| {
            if list2.row_at_y(y as i32).is_some() { return; }
            if let Some(data) = panel.borrow().data.clone() { popup(&list2, node_menu(&d, &data, None, scope.as_ref()), x, y); }
        });
    }
    list.add_controller(click);
    // A page's presses come as links: the same acts as on the tree.
    let on_web: Rc<dyn Fn(&webkit6::WebView)> = {
        let (d, panel, scope, list, stack, web) = (d.clone(), panel.clone(), scope.clone(), list.clone(), stack.clone(), web.clone());
        let this: Rc<RefCell<Option<Rc<dyn Fn(&webkit6::WebView)>>>> = Rc::default();
        let this2 = this.clone();
        let f: Rc<dyn Fn(&webkit6::WebView)> = Rc::new(move |view: &webkit6::WebView| {
            let (d, panel, scope, list, stack, web, this2) = (d.clone(), panel.clone(), scope.clone(), list.clone(), stack.clone(), web.clone(), this2.clone());
            view.connect_decide_policy(move |v, decision, kind| {
                if kind != webkit6::PolicyDecisionType::NavigationAction { return false; }
                let Some(nav) = decision.downcast_ref::<webkit6::NavigationPolicyDecision>() else { return false };
                let uri = nav.navigation_action().and_then(|a| a.request()).and_then(|r| r.uri()).map(|u| u.to_string()).unwrap_or_default();
                let Some(rest) = uri.strip_prefix("wb:") else { return false };
                decision.ignore();
                let mut bits = rest.splitn(4, ':');
                let (act, x, y, node) = (bits.next().unwrap_or(""), bits.next().and_then(|n| n.parse::<f64>().ok()).unwrap_or(0.0), bits.next().and_then(|n| n.parse::<f64>().ok()).unwrap_or(0.0), bits.next().unwrap_or(""));
                let node = glib::Uri::unescape_string(node, None::<&str>).map(|n| n.to_string()).unwrap_or_default();
                if act == "menu" {
                    let Some(data) = panel.borrow().data.clone() else { return true };
                    let found = data["nodes"].as_array().into_iter().flatten().find(|n| s(n, "id") == node).cloned();
                    popup(v, node_menu(&d, &data, found.as_ref(), scope.as_ref()), x, y);
                } else if act == "click" {
                    {
                        let mut p = panel.borrow_mut();
                        p.picked = if p.picked.as_deref() == Some(node.as_str()) { None } else { Some(node) };
                        status::send(&d, "files", json!({"node": p.picked}));
                    }
                    draw(&list, &panel, &d, scope.as_ref(), unit);
                    if let Some(me) = this2.borrow().clone() { show_style(&stack, &web, &panel, &d, scope.as_ref(), &me); }
                }
                true
            });
        });
        *this.borrow_mut() = Some(f.clone());
        f
    };
    show_style(&stack, &web, &panel, &d, scope.as_ref(), &on_web);
    let win2 = win.clone();
    every(1000, move || {
        // From the menu bar and the project panel; a branch panel takes none.
        if branch_root(scope.as_ref()).is_none() {
            let key = imported_of(scope.as_ref()).map_or("map".to_string(), mapimport::message_key);
            if let Some(m) = status::take(&d, &key) {
                match m.get("do").and_then(Value::as_str) {
                    Some("tidy") => ask(),
                    Some("open") => open_map_file(&win2, &d),
                    // From the project panel: an item's original node.
                    Some("pick") => {
                        if let Some(n) = m.get("node").and_then(Value::as_str) {
                            panel.borrow_mut().picked = Some(n.to_string());
                            if imported_of(scope.as_ref()).is_none() { status::send(&d, "files", json!({"node": n})); }
                            draw(&list, &panel, &d, scope.as_ref(), unit);
                            show_style(&stack, &web, &panel, &d, scope.as_ref(), &on_web);
                        }
                    }
                    Some("export") => {
                        let data = panel.borrow().data.clone();
                        match data.as_ref().map(|data| export(&d, data, None, "", "")) {
                            Some(Ok(p)) => status::say(&d, &wb_core::i18n::f("map-exported", &[("a1", (p.display()).to_string())]), 8),
                            Some(Err(e)) => status::say(&d, &wb_core::i18n::f("map-export-failed", &[("e", (e).to_string())]), 8),
                            None => status::say(&d, wb_core::i18n::t("map-export-failed-no-map"), 8),
                        }
                    }
                    _ => {}
                }
            }
        }
        // The style may have changed from a menu, or its folder may be gone.
        let now = mapstyles::current(&d, panel_key(scope.as_ref()));
        if panel.borrow().style != now {
            { let mut p = panel.borrow_mut(); p.style = now.clone(); p.page.clear(); }
            change.set_label(&mapstyles::find(&d, &now).map_or(wb_core::i18n::t("change").to_string(), |x| wb_core::i18n::f("change-2", &[("a1", (cut(&x.name, 8)).to_string())])));
            show_style(&stack, &web, &panel, &d, scope.as_ref(), &on_web);
            style_chosen(&d, &panel, scope.as_ref());
        }
        // No session chosen: the map that changed last. A session without a map: the screen stays as it is.
        let path = imported_of(scope.as_ref()).map(std::path::PathBuf::from).or_else(|| sessions::target(&d).map(|t| d.root.join(format!("{t}.json")))).unwrap_or_else(|| sessions::latest_map(&d));
        let Ok(text) = std::fs::read_to_string(&path) else { return };
        let counts = sessions::target(&d).map(|t| { let mut c: Vec<_> = records::file_counts(&d, &t).into_iter().collect(); c.sort(); format!("{c:?}") }).unwrap_or_default();
        let signature = format!("{}{text}{counts}{:?}", path.display(), wb_core::theme::load_active(&d));
        if panel.borrow().signature == signature { return; }
        panel.borrow_mut().signature = signature;
        // The trash is the alerts panel's; the map shows what is left.
        let mut data: Option<Value> = serde_json::from_str(&text).ok();
        if let Some(arr) = data.as_mut().and_then(|m| m["nodes"].as_array_mut()) { arr.retain(|n| n["state"] != "removed"); }
        panel.borrow_mut().data = data;
        draw(&list, &panel, &d, scope.as_ref(), unit);
        show_style(&stack, &web, &panel, &d, scope.as_ref(), &on_web);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_saved_map_is_named_by_kind_and_its_first_and_last_change() {
        let at = |y: i32, mo: u32, da: u32, h: u32, mi: u32| chrono::TimeZone::with_ymd_and_hms(&chrono::Local, y, mo, da, h, mi, 0).unwrap().timestamp_millis();
        let nodes = vec![json!({"id":"a","opened":{"at":at(2026,11,3,14,22)}}), json!({"id":"b","opened":{"at":at(2026,11,5,9,0)},"closed":{"at":at(2026,11,11,1,35)}})];
        assert_eq!(wb_core::map::file_name(wb_core::map::node_span(&nodes).unwrap(), None, ""), "map-20261103-1422-20261111-0135");
        assert_eq!(wb_core::map::file_name(wb_core::map::node_span(&nodes).unwrap(), Some("pub-front"), "deleted-"), "deleted-branch-pub-front-20261103-1422-20261111-0135");
        assert!(wb_core::map::file_name(wb_core::map::node_span(&nodes).unwrap(), Some("\u{d55c}"), "").starts_with("branch-node-"));
    }
    #[test]
    fn marks_by_state() {
        assert_eq!(glyph(&json!({"id":"a"}), "a"), "●");
        assert_eq!(glyph(&json!({"id":"a"}), ""), "◇");
        assert_eq!(glyph(&json!({"id":"a","state":"done"}), ""), "✓");
        assert_eq!(glyph(&json!({"id":"a","state":"plan"}), ""), "□");
        assert_eq!(glyph(&json!({"id":"a","state":"doing"}), ""), "◐");
        assert_eq!(glyph(&json!({"id":"a","state":"changed"}), ""), "✎");
        assert_eq!((glyph(&json!({"id":"a","state":"dropped"}), ""), glyph(&json!({"id":"a","state":"postponed"}), "")), ("✕", "‖"));
    }
    fn map() -> Value {
        json!({"current":"d","nodes":[{"id":"t","state":"done","label":"top"},{"id":"a","parent":"t","state":"done"},{"id":"b","parent":"a","state":"done"},{"id":"c","parent":"t","state":"done"},{"id":"d","parent":"c","state":"done"},{"id":"lost","parent":"none"}]})
    }
    #[test]
    fn what_folds_at_first() {
        let m = map();
        let tree = tree_of(&m);
        let get = |id: &str| m["nodes"].as_array().unwrap().iter().find(|n| s(n, "id") == id).unwrap().clone();
        assert!(!tree.default_folded(&get("t"), true, "d"), "top is always open");
        assert!(tree.default_folded(&get("a"), false, "d"), "all closed");
        assert!(!tree.default_folded(&get("c"), false, "d"), "holds the present place");
        assert!(!tree.default_folded(&get("b"), false, "d"), "nothing below");
        assert_eq!(tree.kids[""].len(), 2, "a node whose parent is gone sits at the top");
    }
    #[test]
    fn the_flow_above_a_node() { assert_eq!(flow_of(&json!({"nodes":[{"id":"t","label":"top"},{"id":"a","parent":"t","label":"middle"},{"id":"b","parent":"a","label":"bottom"}]}), "b"), ["top", "middle"]); }
    #[test]
    fn requests_are_built_in_order() {
        let t = update_request(None, "", None);
        assert!(t.starts_with("[Map update]") && t.contains("do not shorten or merge") && t.contains("detail for the rest") && !t.contains("Scope") && !t.contains("Additional request"));
        let t = update_request(Some(("n1", "branch")), "more", None);
        assert!(t.contains("Scope: branch 'branch' (node n1)") && t.ends_with("Additional request: more"));
        let r = reflect_request(Some("p"), Some("q1"), &["top".into()], &["- node".into()]);
        assert!(r.starts_with("[Project work q1] First run `whaybench-project take p q1`") && r.contains("Upper flow: top\nNode and contents below it:\n- node"));
        assert!(reflect_request(None, None, &[], &[]).starts_with("[Reflect in project]"));
    }
}
