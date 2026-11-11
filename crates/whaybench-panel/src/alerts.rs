//! The alert panel (3.19): of everything in the memory, what the person should see now. Due dates, the person's
//! marked nodes, what changed since they last looked, AI suggestions, held nodes, and the trash (hidden until opened).
//! The person's choices (done, dropped, held, trash, reopen, mark) are written to that session's map as theirs.
use std::{cell::RefCell, rc::Rc};

use gtk::{gio, glib, prelude::*};
use serde_json::{Value, json};
use wb_core::{i18n::t, memory, paths::Data, status};

use crate::common::{ctl, every};

const SECTIONS: [(&str, &str); 7] = [("due", "alerts-due"), ("marked", "alerts-marked"), ("changed", "alerts-changed"), ("check", "alerts-check"), ("open", "alerts-open"), ("held", "alerts-held"), ("trash", "alerts-trash")];

struct View { shown: String, trash_open: bool }

fn redraw(d: &Data, list: &gtk::ListBox, view: &Rc<RefCell<View>>, force: bool) {
    let board = memory::board(&d.root, wb_core::sessions::target(d).as_deref());
    let sig = format!("{}{}", board, view.borrow().trash_open);
    if !force && view.borrow().shown == sig { return; }
    view.borrow_mut().shown = sig;
    // Rows only: an open right-click menu is the list's child too, and removing it loops forever.
    while let Some(r) = list.row_at_index(0) { list.remove(&r); }
    for (key, title) in SECTIONS {
        let items = board[key].as_array().cloned().unwrap_or_default();
        let closed = key == "trash" && !view.borrow().trash_open;
        let head = gtk::Label::new(Some(&format!("{}{} ({})", if key == "trash" { if closed { "▸ " } else { "▾ " } } else { "" }, t(title), items.len())));
        head.set_xalign(0.0);
        head.add_css_class("head");
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&head));
        row.set_widget_name(if key == "trash" { "__trash__" } else { "__head__" });
        list.append(&row);
        if closed { continue; }
        if items.is_empty() {
            let none = gtk::Label::new(Some(t("alerts-none")));
            none.set_xalign(0.0);
            none.add_css_class("dim");
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&none));
            row.set_widget_name("__head__");
            list.append(&row);
        }
        for item in items {
            let label = gtk::Label::new(Some(item["line"].as_str().unwrap_or("")));
            label.set_xalign(0.0);
            label.set_wrap(true);
            if let Some(t) = item["node"].get("detail").and_then(Value::as_str) { label.set_tooltip_text(Some(t)); }
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&label));
            row.set_widget_name(&format!("{}\t{}\t{}\t{}", item["session"].as_str().unwrap_or(""), item["node"]["id"].as_str().unwrap_or(""), item["node"].get("mark").is_some(), if key == "trash" { item["action"].as_str().unwrap_or("-") } else { "" }));
            list.append(&row);
        }
    }
}

fn say_error(d: &Data, e: impl std::fmt::Display) { status::say(d, &wb_core::i18n::f("alerts-not-written", &[("e", e.to_string())]), 8); }

/// Back from the trash as the action left it; nodes changed since stay where they are, and the person is told.
fn restore(d: &Data, action: &str) {
    match wb_core::map::restore(&d.root, action) {
        Ok(kept) if kept.is_empty() => status::say(d, t("alerts-restored"), 5),
        Ok(kept) => status::say(d, &wb_core::i18n::f("alerts-restored-kept", &[("n", kept.len().to_string())]), 8),
        Err(e) => say_error(d, e),
    }
}

/// Gone for good: the given trash items (session, node id).
fn purge(d: &Data, items: &[(String, String)]) {
    let mut by: std::collections::BTreeMap<&str, Vec<String>> = Default::default();
    for (s, n) in items { by.entry(s).or_default().push(n.clone()); }
    for (s, ids) in by { if let Err(e) = wb_core::map::purge(&d.root, s, &ids) { say_error(d, e); } }
}

fn popup(list: &gtk::ListBox, menu: &gio::Menu, group: &gio::SimpleActionGroup, x: f64, y: f64) {
    let pop = gtk::PopoverMenu::from_model_full(menu, gtk::PopoverMenuFlags::NESTED);
    pop.set_parent(list);
    pop.insert_action_group("a", Some(group));
    pop.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    pop.set_has_arrow(false);
    pop.connect_closed(|p| { let p = p.clone(); glib::idle_add_local_once(move || p.unparent()); });
    pop.popup();
}

fn item(menu: &gio::Menu, group: &gio::SimpleActionGroup, name: &str, label: &str, f: impl Fn() + 'static) {
    menu.append(Some(t(label)), Some(&format!("a.{name}")));
    let act = gio::SimpleAction::new(name, None);
    act.connect_activate(move |_, _| f());
    group.add_action(&act);
}

/// The person's choice for one node, written to its session's map as theirs.
fn choose(d: &Data, sid: &str, node: &str, change: Value) {
    let mut c = change;
    c["id"] = json!(node);
    if let Err(e) = wb_core::map::person(&d.root, sid, json!([c])) { say_error(d, e); }
}

pub fn build(win: &gtk::ApplicationWindow, d: Data) {
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.set_activate_on_single_click(true);
    let seen = gtk::Button::with_label(t("alerts-seen"));
    seen.set_tooltip_text(Some(t("alerts-seen-tip")));
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    scroll.set_vexpand(true);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 2);
    // Under the sections: context, and the plan's 5-hour and weekly use with when they reset (from the status line).
    let usage = gtk::Label::new(None);
    usage.set_xalign(0.0);
    usage.set_wrap(true);
    usage.add_css_class("dim");
    body.append(&seen);
    body.append(&scroll);
    body.append(&usage);
    win.set_child(Some(&body));
    let view = Rc::new(RefCell::new(View { shown: String::new(), trash_open: false }));
    {
        let (d, list2, view) = (d.clone(), list.clone(), view.clone());
        seen.connect_clicked(move |_| { memory::mark_seen(&d.root, "person"); redraw(&d, &list2, &view, true); });
    }
    {
        let (d, list2, view) = (d.clone(), list.clone(), view.clone());
        list.connect_row_activated(move |_, r| {
            if r.widget_name() == "__trash__" { let o = view.borrow().trash_open; view.borrow_mut().trash_open = !o; redraw(&d, &list2, &view, true); }
        });
    }
    let click = gtk::GestureClick::new();
    click.set_button(3);
    {
        let (d, list2, view) = (d.clone(), list.clone(), view.clone());
        click.connect_pressed(move |g, _, x, y| {
            let Some(r) = list2.row_at_y(y as i32) else { return };
            let name = r.widget_name().to_string();
            let menu = gio::Menu::new();
            let group = gio::SimpleActionGroup::new();
            let again = { let (d, l, v) = (d.clone(), list2.clone(), view.clone()); move || redraw(&d, &l, &v, true) };
            // The trash heading: empty it all (a menu, so it takes two presses).
            if name == "__trash__" {
                g.set_state(gtk::EventSequenceState::Claimed);
                let (d2, again) = (d.clone(), again.clone());
                item(&menu, &group, "empty", "alerts-empty-trash", move || {
                    let all: Vec<(String, String)> = memory::board(&d2.root, None)["trash"].as_array().into_iter().flatten()
                        .map(|i| (i["session"].as_str().unwrap_or("").to_string(), i["node"]["id"].as_str().unwrap_or("").to_string())).collect();
                    purge(&d2, &all);
                    again();
                });
                popup(&list2, &menu, &group, x, y);
                return;
            }
            let parts: Vec<String> = name.split('\t').map(String::from).collect();
            let [sid, node, marked, action] = parts.as_slice() else { return };
            g.set_state(gtk::EventSequenceState::Claimed);
            if !action.is_empty() {
                if action != "-" {
                    let (d2, a2, again) = (d.clone(), action.clone(), again.clone());
                    item(&menu, &group, "restore", "alerts-restore", move || { restore(&d2, &a2); again(); });
                }
                let (d2, s2, n2, again) = (d.clone(), sid.clone(), node.clone(), again.clone());
                item(&menu, &group, "purge", "alerts-purge", move || { purge(&d2, &[(s2.clone(), n2.clone())]); again(); });
                popup(&list2, &menu, &group, x, y);
                return;
            }
            let items: [(&str, &str, Value); 5] = [
                ("done", "alerts-done", json!({"state": "done"})),
                ("dropped", "alerts-dropped", json!({"state": "dropped"})),
                ("held", "alerts-hold", json!({"state": "postponed"})),
                ("reopen", "alerts-reopen", json!({"state": null})),
                ("mark", if marked == "true" { "alerts-unmark" } else { "alerts-mark" }, json!({"mark": if marked == "true" { Value::Null } else { json!("important") }})),
            ];
            for (name, label, change) in items {
                let (d2, s2, n2, again) = (d.clone(), sid.clone(), node.clone(), again.clone());
                item(&menu, &group, name, label, move || { choose(&d2, &s2, &n2, change.clone()); again(); });
            }
            // To the trash alone: what is under it moves up, as the map panel's "node only".
            let (d2, s2, n2, again) = (d.clone(), sid.clone(), node.clone(), again.clone());
            item(&menu, &group, "trash", "alerts-to-trash", move || { if let Err(e) = wb_core::map::trash(&d2.root, &s2, &n2, false) { say_error(&d2, e); } again(); });
            let (d2, s2) = (d.clone(), sid.clone());
            item(&menu, &group, "open", "alerts-open-session", move || ctl(&d2, vec!["--data-dir".into(), d2.root.to_string_lossy().to_string(), "session".into(), "show".into(), s2.clone()]));
            popup(&list2, &menu, &group, x, y);
        });
    }
    list.add_controller(click);
    redraw(&d, &list, &view, true);
    let show_usage = { let d = d.clone(); move || {
        let text = wb_core::usage::latest(&d.root, wb_core::sessions::target(&d).as_deref()).map(|u| wb_core::usage::line(&u)).unwrap_or_default();
        if usage.text() != text { usage.set_text(&text); }
        usage.set_visible(!text.is_empty());
    } };
    show_usage();
    let (d2, l2, v2) = (d.clone(), list.clone(), view.clone());
    every(2000, move || { redraw(&d2, &l2, &v2, false); show_usage(); });
}
