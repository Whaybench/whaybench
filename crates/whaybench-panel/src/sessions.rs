//! The session panel: the list, states and search.
use std::{cell::RefCell, rc::Rc};

use gtk::{gio, glib, prelude::*};
use serde_json::Value;
use wb_core::{paths::Data, sessions, status};

use crate::common::{ago, ctl, cut, every};

fn sid(v: &Value) -> &str { v.get("id").and_then(Value::as_str).unwrap_or("") }
fn text<'a>(v: &'a Value, k: &str) -> &'a str { v.get(k).and_then(Value::as_str).unwrap_or("") }

/// What matched every search word, as a short line, or None. Words may match in different places.
pub fn search_note(query: &str, pairs: &[(&str, String)]) -> Option<String> {
    let lower: Vec<(&str, String)> = pairs.iter().filter(|p| !p.1.is_empty()).map(|(k, t)| (*k, t.to_lowercase())).collect();
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(String::from).collect();
    if words.is_empty() || !words.iter().all(|w| lower.iter().any(|(_, t)| t.contains(w))) { return None; }
    for (kind, t) in &lower {
        if let Some(byte) = t.find(&words[0]) {
            let at = t[..byte].chars().count();
            let start = at.saturating_sub(12);
            let piece: String = t.chars().skip(start).take(40).collect();
            let flat = piece.split_whitespace().collect::<Vec<_>>().join(" ");
            return Some(format!("{kind}: {}{flat}", if start > 0 { "…" } else { "" }));
        }
    }
    None
}

fn pairs(d: &Data, info: &Value) -> Vec<(&'static str, String)> {
    let id = sid(info);
    let mut out = vec![(wb_core::i18n::t("name"), sessions::session_name(d, id)), (wb_core::i18n::t("first-question"), text(info, "firstPrompt").to_string()), (wb_core::i18n::t("last-question"), text(info, "lastPrompt").to_string())];
    if let Some(m) = sessions::map_of(d, id) {
        for n in m.get("nodes").and_then(Value::as_array).into_iter().flatten() { out.push((wb_core::i18n::t("map"), text(n, "label").to_string())); }
    }
    let read = |p: std::path::PathBuf| std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
    if let Some(a) = read(d.root.join("answers").join(format!("{id}.json"))) { out.push((wb_core::i18n::t("answer"), text(&a, "text").to_string())); }
    out
}

fn describe(d: &Data, info: &Value) -> (String, String) {
    let id = sid(info);
    let alive = sessions::alive(d, id);
    let state = if alive { sessions::state_of(d, id) } else { None };
    let mut parts = vec![wb_core::agents::name_of(&d.root, info.get("agent").and_then(Value::as_str)), match (alive, state.as_deref()) {
        (false, _) => wb_core::i18n::t("off").to_string(),
        (_, Some("working")) => wb_core::i18n::t("working").into(),
        (_, Some("waiting")) => wb_core::i18n::t("input-needed").into(),
        (_, Some("done")) => wb_core::i18n::t("answered").into(),
        _ => wb_core::i18n::t("running").into(),
    }, ago(info.get("lastActive").and_then(Value::as_i64).unwrap_or(0))];
    if let Some(m) = sessions::map_of(d, id) {
        let nodes = m.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
        let here = nodes.iter().find(|n| n.get("id") == m.get("current")).map(|n| text(n, "label").to_string());
        parts.push(here.map(|h| format!("● {}", cut(&h, 22))).unwrap_or_else(|| wb_core::i18n::t("map-2").into()));
        parts.push(wb_core::i18n::f("open-5", &[("a1", (nodes.iter().filter(|n| wb_core::map::is_open(n)).count()).to_string())]));
    }
    let name = sessions::session_name(d, id);
    (if name.is_empty() { wb_core::i18n::t("empty-session").into() } else { cut(&name, 34) }, parts.join(" · "))
}

struct View { query: String, show_hidden: bool, signature: String, first: Option<String> }

fn row(title: &str, sub: &str, note: Option<&str>, faded: bool) -> gtk::ListBoxRow {
    let area = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let top = gtk::Label::new(Some(title));
    top.set_xalign(0.0);
    top.set_ellipsize(gtk::pango::EllipsizeMode::End);
    if faded { top.add_css_class("dim"); }
    area.append(&top);
    for line in [Some(sub), note].into_iter().flatten().filter(|l| !l.is_empty()) {
        let l = gtk::Label::new(Some(line));
        l.set_xalign(0.0);
        l.add_css_class("small");
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        area.append(&l);
    }
    let r = gtk::ListBoxRow::new();
    r.set_child(Some(&area));
    r
}

fn redraw(d: &Data, list: &gtk::ListBox, view: &Rc<RefCell<View>>, force: bool) {
    let all = sessions::all(d);
    let chosen = sessions::focus(d);
    let latest = sessions::latest(d);
    let open = sessions::open_tabs(d);
    let (query, show_hidden) = { let v = view.borrow(); (v.query.clone(), v.show_hidden) };
    let signature = format!("{chosen:?}{show_hidden}{query}{latest:?}{:?}{:?}", open.iter().map(|t| (&t.tab, &t.session)).collect::<Vec<_>>(), all.iter().map(|s| {
        let id = sid(s);
        (id.to_string(), s.get("lastActive").cloned(), sessions::alive(d, id), sessions::state_of(d, id), s.get("ended").cloned(), sessions::map_of(d, id).map(|m| cut(&m.to_string(), 200)))
    }).collect::<Vec<_>>());
    if !force && view.borrow().signature == signature { return; }
    view.borrow_mut().signature = signature;
    // Rows only, by index: a menu opened on the list is its child too, and GTK's own remove_all loops on it forever.
    while let Some(r) = list.row_at_index(0) { list.remove(&r); }
    let mut first = None;
    let add = |info: &Value, faded: bool, note: Option<String>| {
        let (title, sub) = describe(d, info);
        let r = row(&title, &sub, note.as_deref(), faded);
        r.set_widget_name(sid(info));
        if chosen.as_deref() == Some(sid(info)) { r.add_css_class("chosen"); }
        list.append(&r);
    };
    let moved = |s: &Value| !text(s, "movedTo").is_empty();
    if !query.is_empty() {
        for info in all.iter().filter(|s| !moved(s) && !text(s, "firstPrompt").is_empty() && !sessions::ghost(d, &all, sid(s))) {
            if let Some(note) = search_note(&query, &pairs(d, info)) {
                if first.is_none() { first = Some(sid(info).to_string()); }
                add(info, info.get("ended") == Some(&Value::Bool(true)), Some(note));
            }
        }
        if first.is_none() {
            let r = row(wb_core::i18n::t("no-matching-session"), "", None, true);
            r.set_selectable(false);
            r.set_activatable(false);
            list.append(&r);
        }
    } else {
        let follow = row(wb_core::i18n::t("follow-latest-session"), &wb_core::i18n::f("current-2", &[("a1", (latest.as_deref().map(|l| cut(l, 8)).unwrap_or_else(|| wb_core::i18n::t("none").into())).to_string())]), None, false);
        follow.set_widget_name("__follow__");
        if chosen.is_none() { follow.add_css_class("chosen"); }
        list.append(&follow);
        let held = |s: &Value| open.iter().any(|t| t.tab == text(s, "tab") || t.session.as_deref() == Some(sid(s)));
        let mut hidden = Vec::new();
        for info in &all {
            let spoken = !text(info, "firstPrompt").is_empty();
            // Moved on, or stopped without ever being spoken in: gone. Never spoken in and in no tab: a spare process.
            if moved(info) || (!spoken && !sessions::alive(d, sid(info))) || (!spoken && !held(info)) { continue; }
            // A number left behind by a move that did not hold: its conversation is listed under the number it went on in.
            if sessions::ghost(d, &all, sid(info)) { continue; }
            if info.get("ended") == Some(&Value::Bool(true)) { hidden.push(info); continue; }
            add(info, false, None);
        }
        if !hidden.is_empty() {
            let head = row(&wb_core::i18n::f("hidden-sessions", &[("a1", (if show_hidden { "▾" } else { "▸" }).to_string()), ("a2", (hidden.len()).to_string())]), "", None, false);
            head.set_widget_name("__hidden__");
            list.append(&head);
            if show_hidden { for info in hidden { add(info, true, None); } }
        }
    }
    view.borrow_mut().first = first;
}

pub fn build(win: &gtk::ApplicationWindow, d: Data) {
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(wb_core::i18n::t("search-sessions-names-questions-answers-maps")));
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.set_activate_on_single_click(true);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    scroll.set_vexpand(true);
    let area = gtk::Box::new(gtk::Orientation::Vertical, 2);
    area.append(&search);
    area.append(&scroll);
    win.set_child(Some(&area));
    let view = Rc::new(RefCell::new(View { query: String::new(), show_hidden: false, signature: String::new(), first: None }));
    let data_arg = |d: &Data| vec!["--data-dir".to_string(), d.root.to_string_lossy().to_string()];

    {
        let (d, list2, view) = (d.clone(), list.clone(), view.clone());
        list.connect_row_activated(move |_, r| {
            let name = r.widget_name().to_string();
            match name.as_str() {
                "__hidden__" => { let v = view.borrow().show_hidden; view.borrow_mut().show_hidden = !v; }
                "__follow__" => sessions::clear_focus(&d),
                "" => return,
                id => {
                    sessions::set_focus(&d, id, "person's click");
                    let mut a = data_arg(&d);
                    a.extend(["session".into(), "show".into(), id.to_string()]);
                    ctl(&d, a);
                }
            }
            redraw(&d, &list2, &view, true);
        });
    }
    {
        let (d, list2, view) = (d.clone(), list.clone(), view.clone());
        search.connect_search_changed(move |s| {
            view.borrow_mut().query = s.text().trim().to_string();
            redraw(&d, &list2, &view, true);
        });
    }
    {
        let (d, view) = (d.clone(), view.clone());
        search.connect_activate(move |_| {
            if let Some(id) = view.borrow().first.clone() {
                let mut a = data_arg(&d);
                a.extend(["session".into(), "show".into(), id]);
                ctl(&d, a);
            }
        });
    }
    // Right click: hide, or back to the list.
    let click = gtk::GestureClick::new();
    click.set_button(3);
    {
        let (d, list2, view) = (d.clone(), list.clone(), view.clone());
        click.connect_pressed(move |g, _, x, y| {
            let Some(r) = list2.row_at_y(y as i32) else { return };
            let id = r.widget_name().to_string();
            if id.is_empty() || id.starts_with("__") { return; }
            g.set_state(gtk::EventSequenceState::Claimed);
            let ended = sessions::record_of(&d, &id).is_some_and(|i| i.get("ended") == Some(&Value::Bool(true)));
            let menu = gio::Menu::new();
            menu.append(Some(if ended { wb_core::i18n::t("restore-to-list") } else { wb_core::i18n::t("hide") }), Some("s.toggle"));
            // Ending stops the session's process too; its conversation stays saved and can be opened again.
            let running = sessions::alive(&d, &id);
            if running { menu.append(Some(wb_core::i18n::t("end-session")), Some("s.end")); }
            let group = gio::SimpleActionGroup::new();
            if running {
                let end = gio::SimpleAction::new("end", None);
                let (d2, id2) = (d.clone(), id.clone());
                end.connect_activate(move |_, _| {
                    let mut a = data_arg(&d2);
                    a.extend(["session".into(), "end".into(), id2.clone()]);
                    ctl(&d2, a);
                });
                group.add_action(&end);
            }
            let act = gio::SimpleAction::new("toggle", None);
            let (d2, l3, v2) = (d.clone(), list2.clone(), view.clone());
            act.connect_activate(move |_, _| {
                if ended { sessions::unend(&d2, &id); } else { sessions::mark_ended(&d2, &id); status::say(&d2, wb_core::i18n::t("session-hidden-moved-to-hidden-sessions"), 5); }
                redraw(&d2, &l3, &v2, true);
            });
            group.add_action(&act);
            let pop = gtk::PopoverMenu::from_model_full(&menu, gtk::PopoverMenuFlags::NESTED);
            pop.set_parent(&list2);
            pop.insert_action_group("s", Some(&group));
            pop.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            pop.set_has_arrow(false);
            pop.connect_closed(|p| { let p = p.clone(); glib::idle_add_local_once(move || p.unparent()); });
            pop.popup();
        });
    }
    list.add_controller(click);
    {
        let (d, search) = (d.clone(), search.clone());
        every(300, move || {
            // Session → Search sessions in the menu bar: this panel takes the keyboard in its search line.
            if status::take(&d, "sessions").is_some() {
                search.grab_focus();
                search.select_region(0, -1);
            }
        });
    }
    every(2000, move || redraw(&d, &list, &view, false));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_word_must_be_somewhere() {
        let p = [("Name", "build the parser".to_string()), ("Answer", "tests pass on the server".to_string())];
        assert_eq!(search_note("parser SERVER", &p).unwrap(), "Name: build the parser");
        assert!(search_note("parser none", &p).is_none());
        assert!(search_note("", &p).is_none());
        assert_eq!(search_note("pass", &p).unwrap(), "Answer: tests pass on the server");
    }
    #[test]
    fn a_long_hit_shows_its_surroundings() {
        let long = format!("{}needle{}", "a".repeat(30), "b".repeat(60));
        let got = search_note("needle", &[("Answer", long)]).unwrap();
        assert!(got.starts_with("Answer: …"));
        assert!(got.contains("needle"));
    }
}
