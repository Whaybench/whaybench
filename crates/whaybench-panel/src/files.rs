//! The file panel: files the chosen session read or changed, by the map node it was on.
use std::{cell::RefCell, collections::{BTreeMap, HashMap, HashSet}, path::Path, rc::Rc};

use gtk::{gdk, gio, glib, prelude::*};
use serde_json::{Value, json};
use similar::{ChangeTag, TextDiff};
use wb_core::{paths::Data, records, sessions, status, view};

use crate::common::every;

#[derive(Default, Clone, Debug, PartialEq)]
pub struct Entry { pub read: u32, pub edit: u32, pub attached: u32, pub last: i64 }

fn s<'a>(v: &'a Value, k: &str) -> &'a str { v.get(k).and_then(Value::as_str).unwrap_or("") }

/// Node to path to counts.
pub fn group(touches: &[Value], hidden: &HashSet<String>) -> BTreeMap<String, BTreeMap<String, Entry>> {
    let mut groups: BTreeMap<String, BTreeMap<String, Entry>> = BTreeMap::new();
    for t in touches {
        let path = s(t, "path");
        if path.is_empty() || hidden.contains(path) { continue; }
        let e = groups.entry(s(t, "node").to_string()).or_default().entry(path.to_string()).or_default();
        match s(t, "op") { "edit" => e.edit += 1, "attached" => e.attached += 1, _ => e.read += 1 }
        e.last = e.last.max(t.get("at").and_then(Value::as_i64).unwrap_or(0));
    }
    groups
}

/// The mark of a file: changed, only read, only attached.
pub fn mark(e: &Entry) -> &'static str { if e.edit > 0 { "✎" } else if e.read > 0 { "👁" } else { "📌" } }

/// Open in vim, by its swap file; or changed on disk after the AI last handled it, not while its own command ran.
pub fn file_state(path: &str, last_ai: i64, mtime: Option<i64>, swap: bool, runs: &[(i64, i64)]) -> Option<&'static str> {
    let _ = path;
    if swap { return Some("vim"); }
    let m = mtime?;
    let own = runs.iter().any(|(a, b)| *a - 500 <= m && m <= *b + 1500);
    (m > last_ai + 1500 && !own).then_some("changed")
}

fn swap_exists(path: &str) -> bool {
    let p = Path::new(path);
    let (Some(dir), Some(name)) = (p.parent(), p.file_name()) else { return false };
    ["swp", "swo", "swn"].iter().any(|x| dir.join(format!(".{}.{x}", name.to_string_lossy())).exists())
}

fn mtime_ms(path: &str) -> Option<i64> {
    std::fs::metadata(path).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|t| t.as_millis() as i64)
}

fn hidden_file(d: &Data) -> std::path::PathBuf { d.root.join("files-hidden.json") }

fn hidden(d: &Data) -> HashSet<String> {
    std::fs::read_to_string(hidden_file(d)).ok().and_then(|t| serde_json::from_str::<HashSet<String>>(&t).ok()).unwrap_or_default()
}

fn escape_html(t: &str) -> String { t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;") }

/// The AI's edits of a file, newest first, as a coloured page.
pub fn diff_page(path: &str, edits: &[Value], bg: &str, fg: &str) -> String {
    let mut blocks = String::new();
    for e in edits.iter().rev() {
        let when = chrono::DateTime::from_timestamp_millis(e["at"].as_i64().unwrap_or(0)).map(|t| t.with_timezone(&chrono::Local).format("%m-%d %H:%M").to_string()).unwrap_or_default();
        let diff = TextDiff::from_lines(s(e, "before"), s(e, "after"));
        let mut rows = String::new();
        for group in diff.grouped_ops(3) {
            rows += "<div class=hunk>@@</div>";
            for op in group {
                for c in diff.iter_changes(&op) {
                    let (cls, sign) = match c.tag() { ChangeTag::Insert => ("add", "+"), ChangeTag::Delete => ("del", "-"), ChangeTag::Equal => ("", " ") };
                    rows += &format!("<div class='{cls}'>{sign}{}</div>", escape_html(c.value().trim_end_matches('\n')));
                }
            }
        }
        let kind = if e.get("whole") == Some(&Value::Bool(true)) { wb_core::i18n::t("whole-file-written") } else { wb_core::i18n::t("partial-edit") };
        blocks += &format!("<h3>{when} · {kind}</h3><pre>{}</pre>", if rows.is_empty() { wb_core::i18n::t("no-change").to_string() } else { rows });
    }
    format!("<!doctype html><meta charset=utf-8><title>{t}</title><style>body{{margin:0;padding:12px 16px;font:13px sans-serif;background:{bg};color:{fg}}}pre{{font:13px monospace;padding:8px;white-space:pre-wrap}}.add{{background:#40a02b26}}.del{{background:#d20f3926}}.hunk{{opacity:.6}}h3{{font-size:13px;margin:14px 0 4px}}</style><h2 style='font-size:15px'>{t}</h2>{blocks}", t = escape_html(path))
}

struct Panel { node: Option<String>, signature: String }

fn menu(d: &Data, list: &gtk::ListBox, path: &str, x: f64, y: f64, redraw: Rc<dyn Fn()>) {
    let group = gio::SimpleActionGroup::new();
    let top = gio::Menu::new();
    let n = Rc::new(std::cell::Cell::new(0));
    let add = |m: &gio::Menu, label: &str, f: Box<dyn Fn()>| {
        let name = format!("a{}", n.get());
        n.set(n.get() + 1);
        let act = gio::SimpleAction::new(&name, None);
        act.connect_activate(move |_, _| f());
        group.add_action(&act);
        m.append(Some(label), Some(&format!("f.{name}")));
    };
    { let (d2, p) = (d.clone(), path.to_string()); add(&top, wb_core::i18n::t("open-in-view-panel"), Box::new(move || { let _ = view::open(&d2, &p, None); })); }
    { let (d2, p) = (d.clone(), path.to_string()); add(&top, wb_core::i18n::t("open-in-vim"), Box::new(move || { let _ = view::vim(&d2, &p); })); }
    // Apps of the desktop that open this kind of file.
    let (kind, _) = gio::content_type_guess(Some(path), None::<&[u8]>);
    let default = gio::AppInfo::default_for_type(&kind, false).map(|a| a.id().map(|i| i.to_string()).unwrap_or_default());
    let apps = gio::AppInfo::all_for_type(&kind);
    if !apps.is_empty() {
        let sec = gio::Menu::new();
        for app in apps {
            let is_default = default.as_deref() == app.id().as_deref();
            let (p, a) = (path.to_string(), app.clone());
            add(&sec, &format!("{}{}", app.name(), if is_default { wb_core::i18n::t("default-2") } else { "" }), Box::new(move || {
                let _ = a.launch(&[gio::File::for_path(&p)], None::<&gio::AppLaunchContext>);
            }));
        }
        top.append_section(None, &sec);
    }
    let sec = gio::Menu::new();
    if let Some(session) = sessions::target(d) {
        let edits = records::file_edits(d, &session, path);
        if !edits.is_empty() {
            let (d2, p) = (d.clone(), path.to_string());
            add(&sec, wb_core::i18n::t("ai-edits"), Box::new(move || {
                let t = wb_core::theme::load_active(&d2);
                let hex = |c: [u8; 3]| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
                let bg = t.panel.map(|x| (x * 255.0) as u8);
                let html = diff_page(&p, &edits, &hex([bg[0], bg[1], bg[2]]), &hex(t.ink));
                let dir = d2.cache().join("diffs");
                let _ = std::fs::create_dir_all(&dir);
                let out = dir.join(format!("{}.html", chrono::Utc::now().timestamp_millis()));
                if std::fs::write(&out, html).is_ok() { let _ = view::open(&d2, &out.to_string_lossy(), None); }
            }));
        }
    }
    {
        let (d2, p, redraw) = (d.clone(), path.to_string(), redraw.clone());
        add(&sec, wb_core::i18n::t("remove-from-list"), Box::new(move || {
            let mut h: Vec<String> = hidden(&d2).into_iter().collect();
            h.push(p.clone());
            h.sort();
            let _ = std::fs::write(hidden_file(&d2), json!(h).to_string());
            status::say(&d2, &wb_core::i18n::f("removed-from-list", &[("a1", (Path::new(&p).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()).to_string())]), 5);
            redraw();
        }));
    }
    top.append_section(None, &sec);
    let pop = gtk::PopoverMenu::from_model_full(&top, gtk::PopoverMenuFlags::NESTED);
    pop.set_parent(list);
    pop.insert_action_group("f", Some(&group));
    pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    pop.set_has_arrow(false);
    pop.connect_closed(|p| { let p = p.clone(); glib::idle_add_local_once(move || p.unparent()); });
    pop.popup();
}

fn draw(d: &Data, list: &gtk::ListBox, panel: &RefCell<Panel>, unit: i32, force: bool) {
    let Some(session) = sessions::target(d) else { return };
    let mut touches = records::touches(d, &session);
    let map = sessions::map_of(d, &session).unwrap_or(Value::Null);
    let raw: Vec<Value> = map.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
    let names: HashMap<String, String> = raw.iter().map(|n| (s(n, "id").to_string(), s(n, "label").to_string())).collect();
    let picked = panel.borrow().node.clone();
    if let Some(node) = picked.as_ref().filter(|n| names.contains_key(*n)) {
        let mut branch: HashSet<String> = HashSet::from([node.clone()]);
        loop {
            let more: Vec<String> = raw.iter().filter(|n| branch.contains(s(n, "parent")) && !branch.contains(s(n, "id"))).map(|n| s(n, "id").to_string()).collect();
            if more.is_empty() { break; }
            branch.extend(more);
        }
        touches.retain(|t| branch.contains(s(t, "node")));
    }
    let hide = hidden(d);
    let mut last_ai: HashMap<String, i64> = HashMap::new();
    for t in touches.iter().filter(|t| matches!(s(t, "op"), "read" | "edit")) {
        let e = last_ai.entry(s(t, "path").to_string()).or_insert(0);
        *e = (*e).max(t.get("at").and_then(Value::as_i64).unwrap_or(0));
    }
    let runs = records::bash_runs(d, &session);
    let state: HashMap<String, &'static str> = last_ai.iter().filter_map(|(p, at)| file_state(p, *at, mtime_ms(p), swap_exists(p), &runs).map(|x| (p.clone(), x))).collect();
    let mut sorted_state: Vec<_> = state.iter().collect();
    sorted_state.sort();
    let signature = format!("{session}{picked:?}{}{:?}{}{}{sorted_state:?}", touches.len(), touches.last().map(|t| t["at"].clone()), hide.len(), names.len());
    if !force && panel.borrow().signature == signature { return; }
    panel.borrow_mut().signature = signature;
    // Rows only, by index: a menu opened on the list is its child too, and GTK's own remove_all loops on it forever.
    while let Some(r) = list.row_at_index(0) { list.remove(&r); }
    let groups = group(&touches, &hide);
    let mut order: Vec<_> = groups.iter().collect();
    order.sort_by_key(|(_, files)| std::cmp::Reverse(files.values().map(|e| e.last).max().unwrap_or(0)));
    for (node, files) in order {
        let name = names.get(node).cloned().unwrap_or_else(|| if node.is_empty() { wb_core::i18n::t("no-map-node").to_string() } else { wb_core::i18n::f("deleted-node", &[("node", (node).to_string())]) });
        let head = gtk::Label::new(Some(&format!("{name}  ·  {}", files.len())));
        head.set_xalign(0.0);
        head.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let r = gtk::ListBoxRow::new();
        r.set_child(Some(&head));
        r.set_activatable(false);
        list.append(&r);
        let mut rows: Vec<_> = files.iter().collect();
        rows.sort_by_key(|(_, e)| std::cmp::Reverse(e.last));
        for (path, e) in rows {
            let base = Path::new(path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| path.clone());
            let now = match state.get(path) { Some(&"vim") => " ✏", Some(&"changed") => " ✋", _ => "" };
            let label = gtk::Label::new(Some(&format!("{} {base}{now}", mark(e))));
            label.set_xalign(0.0);
            label.set_margin_start(unit);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let readable = std::fs::File::open(path).is_ok();
            if !readable { label.add_css_class("dim"); }
            let mut tip = wb_core::i18n::f("read-edited", &[("path", (path).to_string()), ("a1", (e.read).to_string()), ("a2", (e.edit).to_string()), ("a3", (if e.attached > 0 { wb_core::i18n::t("linked-to-node") } else { "" }).to_string())]);
            match state.get(path) { Some(&"vim") => tip += wb_core::i18n::t("being-edited-in-vim"), Some(&"changed") => tip += wb_core::i18n::t("changed-after-ai-work-saved-by"), _ => {} }
            if !readable { tip += wb_core::i18n::t("no-read-permission"); }
            label.set_tooltip_text(Some(&tip));
            let r = gtk::ListBoxRow::new();
            r.set_child(Some(&label));
            r.set_widget_name(path);
            list.append(&r);
        }
    }
}

pub fn build(win: &gtk::ApplicationWindow, d: Data) {
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.set_activate_on_single_click(true);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    win.set_child(Some(&scroll));
    let unit = { let (w, _) = win.create_pango_layout(Some("MMMM")).pixel_size(); w.max(16) };
    let panel = Rc::new(RefCell::new(Panel { node: None, signature: String::new() }));
    {
        let d = d.clone();
        list.connect_row_activated(move |_, r| {
            let path = r.widget_name();
            if path.starts_with('/') { let _ = view::open(&d, &path, None); }
        });
    }
    let click = gtk::GestureClick::new();
    click.set_button(3);
    {
        let (d, list2, panel) = (d.clone(), list.clone(), panel.clone());
        click.connect_pressed(move |g, _, x, y| {
            let Some(r) = list2.row_at_y(y as i32) else { return };
            let path = r.widget_name().to_string();
            if !path.starts_with('/') { return; }
            g.set_state(gtk::EventSequenceState::Claimed);
            let (d3, l3, p3) = (d.clone(), list2.clone(), panel.clone());
            menu(&d, &list2, &path, x, y, Rc::new(move || draw(&d3, &l3, &p3, unit, true)));
        });
    }
    list.add_controller(click);
    // A node pressed in the map filters; the panel does not come forward.
    every(500, {
        let (d, list, panel) = (d.clone(), list.clone(), panel.clone());
        move || {
            if let Some(v) = status::take(&d, "files") {
                panel.borrow_mut().node = v.get("node").and_then(Value::as_str).map(String::from);
                draw(&d, &list, &panel, unit, true);
            }
        }
    });
    every(3000, move || draw(&d, &list, &panel, unit, false));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn touches_group_by_node_and_hidden_ones_go() {
        let t = [json!({"path":"/a","op":"read","at":1,"node":"n"}),json!({"path":"/a","op":"edit","at":5,"node":"n"}),json!({"path":"/b","op":"attached","at":2,"node":"n"}),json!({"path":"/c","op":"read","at":3})];
        let g = group(&t, &HashSet::from(["/c".to_string()]));
        assert_eq!(g.len(), 1);
        assert_eq!(g["n"]["/a"], Entry { read: 1, edit: 1, attached: 0, last: 5 });
        assert_eq!(mark(&g["n"]["/a"]), "✎");
        assert_eq!(mark(&g["n"]["/b"]), "📌");
        assert_eq!(mark(&Entry { read: 1, ..Default::default() }), "👁");
    }
    #[test]
    fn who_changed_a_file() {
        assert_eq!(file_state("/a", 1000, Some(9000), false, &[]), Some("changed"));
        assert_eq!(file_state("/a", 1000, Some(2000), false, &[]), None, "within 1.5 seconds of the AI's touch");
        assert_eq!(file_state("/a", 1000, Some(9000), false, &[(8000, 8500)]), None, "during the AI's own command");
        assert_eq!(file_state("/a", 1000, Some(9000), true, &[]), Some("vim"));
        assert_eq!(file_state("/a", 1000, None, false, &[]), None);
    }
    #[test]
    fn a_diff_page_marks_lines() {
        let e = [json!({"at":1000,"before":"a\nb\n<x>\n","after":"a\nc\n<x>\n"})];
        let h = diff_page("/f<1>", &e, "#fff", "#000");
        assert!(h.contains("class='del'>-b") && h.contains("class='add'>+c") && h.contains("&lt;x&gt;") && h.contains("/f&lt;1&gt;"));
    }
}
