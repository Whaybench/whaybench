//! The notifications panel (3.19): of everything in the memory, what the person should see now. Due dates, the person's
//! marked nodes, what changed since they last looked, AI suggestions, held nodes, and the trash (hidden until opened).
//! The person's choices (done, dropped, held, trash, reopen, mark) are written to that session's map as theirs.
use std::{cell::RefCell, rc::Rc};

use gtk::{gio, glib, prelude::*};
use serde_json::{Value, json};
use wb_core::{i18n::t, memory, paths::Data, status};

use crate::common::{confirm, ctl, every};

const SECTIONS: [(&str, &str); 7] = [("due", "notify-due"), ("marked", "notify-marked"), ("changed", "notify-changed"), ("check", "notify-check"), ("open", "notify-open"), ("held", "notify-held"), ("trash", "notify-trash")];

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
        // An empty section is left out; with all of them empty one line says so.
        if items.is_empty() { continue; }
        let closed = key == "trash" && !view.borrow().trash_open;
        let head = gtk::Label::new(Some(&format!("{}{} ({})", if key == "trash" { if closed { "▸ " } else { "▾ " } } else { "" }, t(title), items.len())));
        head.set_xalign(0.0);
        head.add_css_class("head");
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&head));
        row.set_widget_name(if key == "trash" { "__trash__" } else { "__head__" });
        list.append(&row);
        if closed { continue; }
        for item in items {
            let label = gtk::Label::new(Some(item["line"].as_str().unwrap_or("")));
            label.set_xalign(0.0);
            label.set_wrap(true);
            if let Some(t) = item["node"].get("detail").and_then(Value::as_str) { label.set_tooltip_text(Some(t)); }
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&label));
            row.set_widget_name(&format!("{}\t{}\t{}\t{}\t{}", item["session"].as_str().unwrap_or(""), item["node"]["id"].as_str().unwrap_or(""), item["node"].get("mark").is_some(), if key == "trash" { item["action"].as_str().unwrap_or("-") } else { "" }, if key == "check" { item["node"]["suggest"]["state"].as_str().unwrap_or("") } else { "" }));
            list.append(&row);
        }
    }
    if list.row_at_index(0).is_none() {
        let none = gtk::Label::new(Some(t("notify-none")));
        none.set_xalign(0.0);
        none.add_css_class("dim");
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&none));
        row.set_widget_name("__head__");
        list.append(&row);
    }
}

fn say_error(d: &Data, e: impl std::fmt::Display) { status::say(d, &wb_core::i18n::f("notify-not-written", &[("e", e.to_string())]), 8); }

/// Back from the trash as the action left it; nodes changed since stay where they are, and the person is told.
fn restore(d: &Data, action: &str) {
    match wb_core::map::restore(&d.root, action) {
        Ok(kept) if kept.is_empty() => status::say(d, t("notify-restored"), 5),
        Ok(kept) => status::say(d, &wb_core::i18n::f("notify-restored-kept", &[("n", kept.len().to_string())]), 8),
        Err(e) => say_error(d, e),
    }
}

/// Gone for good: the given deleted nodes, by number.
fn purge(d: &Data, nids: &[String]) {
    if let Err(e) = wb_core::map::purge(&d.root, nids) { say_error(d, e); }
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

/// Delete, as on the map panel: the branch or the node alone, in a submenu so it takes two presses. Here it deletes
/// the node itself (it waits in the Deleted section), where the map panel only takes it off its map.
fn delete_menu(menu: &gio::Menu, group: &gio::SimpleActionGroup, d: &Data, nid: &str, again: impl Fn() + Clone + 'static) {
    let sub = gio::Menu::new();
    let below = wb_core::map::below(&d.root, nid);
    let add = |name: &str, label: String, branch: bool| {
        sub.append(Some(&label), Some(&format!("a.{name}")));
        let act = gio::SimpleAction::new(name, None);
        let (d2, n2, again) = (d.clone(), nid.to_string(), again.clone());
        act.connect_activate(move |_, _| { if let Err(e) = wb_core::map::trash(&d2.root, &n2, branch) { say_error(&d2, e); } again(); });
        group.add_action(&act);
    };
    if below > 0 {
        add("delete-branch", wb_core::i18n::f("delete-whole-branch-including-below", &[("below", below.to_string())]), true);
        add("delete-node", t("delete-node-only-nodes-below-move").to_string(), false);
    } else {
        add("delete-node", t("delete-node").to_string(), false);
    }
    menu.append_submenu(Some(t("delete")), &sub);
}

/// Before a node goes for good: its name and how many history entries go with it.
fn confirm_purge(win: &gtk::ApplicationWindow, d: &Data, nid: &str, then: impl Fn() + 'static) {
    let shown = memory::show(&d.root, nid);
    let label = shown["nodes"][0]["label"].as_str().unwrap_or("").to_string();
    let events = shown["events"].as_array().cloned().unwrap_or_default();
    let n = events.len();
    // The node's history, oldest first: what it was and how it changed, all of which goes.
    let mut names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut lines: Vec<String> = crate::history::rows(&events, |s| names.entry(s.to_string()).or_insert_with(|| wb_core::sessions::session_name(d, s)).clone(), |nid| memory::label_of(&d.root, nid))
        .iter().map(|r| format!("{} {}", r.day, r.line)).collect();
    lines.reverse();
    let (d2, n2) = (d.clone(), nid.to_string());
    confirm(win, &wb_core::i18n::f("purge-confirm", &[("a1", label), ("n", n.to_string())]), &lines, t("notify-purge"), move || {
        // A node not deleted yet is deleted first (alone: what is under it moves up), then it goes for good.
        let deleted = memory::show(&d2.root, &n2)["nodes"][0]["state"] == "removed";
        if !deleted && let Err(e) = wb_core::map::trash(&d2.root, &n2, false) { say_error(&d2, e); }
        purge(&d2, std::slice::from_ref(&n2));
        then();
    });
}

/// The person's choice for one node (by its number), recorded as theirs.
fn choose(d: &Data, node: &str, change: Value) {
    let mut c = change;
    c["id"] = json!(node);
    if let Err(e) = wb_core::map::person(&d.root, json!([c])) { say_error(d, e); }
}

pub fn build(win: &gtk::ApplicationWindow, d: Data) {
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.set_activate_on_single_click(true);
    let seen = gtk::Button::with_label(t("notify-seen"));
    seen.set_tooltip_text(Some(t("notify-seen-tip")));
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    scroll.set_vexpand(true);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 2);
    // Under the sections: context, and the plan's 5-hour and weekly use with when they reset (from the status line).
    let usage = gtk::Label::new(None);
    usage.set_xalign(0.0);
    usage.set_wrap(true);
    usage.add_css_class("dim");
    // Two tabs: what to see now, and the history of every node, only to read.
    let now = gtk::Box::new(gtk::Orientation::Vertical, 2);
    now.append(&seen);
    now.append(&scroll);
    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    stack.add_titled(&now, Some("now"), t("notify-tab-now"));
    let (history_page, history_refresh) = history_tab(win, &d);
    stack.add_titled(&history_page, Some("history"), t("notify-tab-history"));
    let switcher = gtk::StackSwitcher::new();
    switcher.set_stack(Some(&stack));
    body.append(&switcher);
    body.append(&stack);
    body.append(&usage);
    win.set_child(Some(&body));
    {
        let (stack2, refresh) = (stack.clone(), history_refresh.clone());
        stack.connect_visible_child_name_notify(move |_| if stack2.visible_child_name().as_deref() == Some("history") { refresh(); });
    }
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
        let (d, list2, view, win) = (d.clone(), list.clone(), view.clone(), win.clone());
        click.connect_pressed(move |g, _, x, y| {
            let Some(r) = list2.row_at_y(y as i32) else { return };
            let name = r.widget_name().to_string();
            let menu = gio::Menu::new();
            let group = gio::SimpleActionGroup::new();
            let again = { let (d, l, v) = (d.clone(), list2.clone(), view.clone()); move || redraw(&d, &l, &v, true) };
            // The trash heading: empty it all (a menu, so it takes two presses).
            if name == "__trash__" {
                g.set_state(gtk::EventSequenceState::Claimed);
                let (d2, again, win2) = (d.clone(), again.clone(), win.clone());
                item(&menu, &group, "empty", "notify-empty-trash", move || {
                    let all: Vec<String> = memory::board(&d2.root, None)["trash"].as_array().into_iter().flatten()
                        .filter_map(|i| i["node"]["id"].as_str().map(String::from)).collect();
                    let (d3, again) = (d2.clone(), again.clone());
                    confirm(&win2, &wb_core::i18n::f("purge-all-confirm", &[("n", all.len().to_string())]), &[], t("notify-purge"), move || { purge(&d3, &all); again(); });
                });
                popup(&list2, &menu, &group, x, y);
                return;
            }
            let parts: Vec<String> = name.split('\t').map(String::from).collect();
            let [sid, node, marked, action, suggested] = parts.as_slice() else { return };
            g.set_state(gtk::EventSequenceState::Claimed);
            if !action.is_empty() {
                if action != "-" {
                    let (d2, a2, again) = (d.clone(), action.clone(), again.clone());
                    item(&menu, &group, "restore", "notify-restore", move || { restore(&d2, &a2); again(); });
                }
                let (d2, n2, again, win2) = (d.clone(), node.clone(), again.clone(), win.clone());
                item(&menu, &group, "purge", "notify-purge", move || confirm_purge(&win2, &d2, &n2, again.clone()));
                popup(&list2, &menu, &group, x, y);
                return;
            }
            // From To approve: the agent's suggestion first, approved as the person's own choice or declined.
            if !suggested.is_empty() {
                for (name, label, change) in [("approve", "notify-approve", json!({"state": suggested, "suggest": null})), ("decline", "notify-decline", json!({"suggest": null}))] {
                    let (d2, n2, again) = (d.clone(), node.clone(), again.clone());
                    item(&menu, &group, name, label, move || { choose(&d2, &n2, change.clone()); again(); });
                }
            }
            let items: [(&str, &str, Value); 5] = [
                ("done", "notify-done", json!({"state": "done"})),
                ("dropped", "notify-dropped", json!({"state": "dropped"})),
                ("held", "notify-hold", json!({"state": "postponed"})),
                ("reopen", "notify-reopen", json!({"state": null})),
                ("mark", if marked == "true" { "notify-unmark" } else { "notify-mark" }, json!({"mark": if marked == "true" { Value::Null } else { json!("important") }})),
            ];
            for (name, label, change) in items {
                let (d2, n2, again) = (d.clone(), node.clone(), again.clone());
                item(&menu, &group, name, label, move || { choose(&d2, &n2, change.clone()); again(); });
            }
            delete_menu(&menu, &group, &d, node, again.clone());
            let (d2, s2) = (d.clone(), sid.clone());
            item(&menu, &group, "open", "notify-open-session", move || ctl(&d2, vec!["--data-dir".into(), d2.root.to_string_lossy().to_string(), "session".into(), "show".into(), s2.clone()]));
            popup(&list2, &menu, &group, x, y);
        });
    }
    list.add_controller(click);
    redraw(&d, &list, &view, true);
    let show_usage = { let d = d.clone(); move || {
        let text = wb_core::sessions::target(&d).and_then(|t| wb_core::usage::latest(&d.root, Some(&t))).map(|u| wb_core::usage::line(&u)).unwrap_or_default();
        if usage.text() != text { usage.set_text(&text); }
        usage.set_visible(!text.is_empty());
    } };
    show_usage();
    let (d2, l2, v2) = (d.clone(), list.clone(), view.clone());
    every(2000, move || {
        redraw(&d2, &l2, &v2, false);
        show_usage();
        if stack.visible_child_name().as_deref() == Some("history") { history_refresh(); }
    });
}

const HISTORY_STEP: usize = 200;

/// The History tab and what brings it up to date: the memory is read again only when its files changed.
fn history_tab(win: &gtk::ApplicationWindow, d: &Data) -> (gtk::Box, Rc<dyn Fn()>) {
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    scroll.set_vexpand(true);
    let more = gtk::Button::new();
    more.add_css_class("flat");
    let page = gtk::Box::new(gtk::Orientation::Vertical, 2);
    page.append(&scroll);
    page.append(&more);
    struct Hist { sig: String, rows: Vec<crate::history::Row>, shown: usize }
    let hist = Rc::new(RefCell::new(Hist { sig: String::new(), rows: Vec::new(), shown: HISTORY_STEP }));
    let draw = { let (list, more, scroll, hist) = (list.clone(), more.clone(), scroll.clone(), hist.clone()); move || {
        let h = hist.borrow();
        let top = scroll.vadjustment().value();
        while let Some(r) = list.row_at_index(0) { list.remove(&r); }
        let mut day = "";
        let add = |text: &str, class: Option<&str>, tip: &str, node: &str| {
            let label = gtk::Label::new(Some(text));
            // The row's node and its name, for the right-click menu.
            label.set_widget_name(node);
            label.set_xalign(0.0);
            label.set_wrap(true);
            label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            if let Some(c) = class { label.add_css_class(c); }
            if !tip.is_empty() { label.set_tooltip_text(Some(tip)); }
            list.append(&label);
        };
        if h.rows.is_empty() { add(t("notify-none"), Some("dim"), "", ""); }
        for r in h.rows.iter().take(h.shown) {
            if r.day != day { day = &r.day; add(day, Some("head"), "", ""); }
            add(&r.line, None, &r.tip, &format!("{}\t{}", r.nid, r.label));
        }
        let left = h.rows.len().saturating_sub(h.shown);
        more.set_visible(left > 0);
        more.set_label(&wb_core::i18n::f("history-more", &[("n", left.min(HISTORY_STEP).to_string())]));
        scroll.vadjustment().set_value(top);
    } };
    let draw: Rc<dyn Fn()> = Rc::new(draw);
    {
        let (hist, draw) = (hist.clone(), draw.clone());
        more.connect_clicked(move |_| { hist.borrow_mut().shown += HISTORY_STEP; draw(); });
    }
    let d0 = d.clone();
    let d = d.clone();
    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        let sig = memory::events_mark(&d.root);
        if hist.borrow().sig == sig { return; }
        let mut names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let rows = crate::history::rows(&memory::events(&d.root), |s| names.entry(s.to_string()).or_insert_with(|| wb_core::sessions::session_name(&d, s)).clone(), |nid| memory::label_of(&d.root, nid));
        { let mut h = hist.borrow_mut(); h.sig = sig; h.rows = rows; }
        draw();
    });
    // A row's node: put it on the conversation's map, delete it, or delete it for good.
    let click = gtk::GestureClick::new();
    click.set_button(3);
    {
        let (d, list2, refresh, win) = (d0, list.clone(), refresh.clone(), win.clone());
        click.connect_pressed(move |g, _, x, y| {
            let Some(name) = list2.row_at_y(y as i32).and_then(|r| r.child()).map(|c| c.widget_name().to_string()) else { return };
            let Some((nid, label)) = name.split_once('\t').filter(|p| !p.0.is_empty()) else { return };
            if memory::label_of(&d.root, nid).is_none() { return; }
            g.set_state(gtk::EventSequenceState::Claimed);
            let menu = gio::Menu::new();
            let group = gio::SimpleActionGroup::new();
            let (d2, n2, l2) = (d.clone(), nid.to_string(), label.to_string());
            item(&menu, &group, "link", "history-link", move || match wb_core::sessions::target(&d2) {
                None => status::say(&d2, t("history-link-none"), 6),
                Some(s) => match wb_core::map::link(&d2.root, &s, &n2) {
                    Ok(_) => status::say(&d2, &wb_core::i18n::f("history-linked", &[("a1", l2.chars().take(24).collect())]), 5),
                    Err(e) => say_error(&d2, e),
                },
            });
            let again = { let refresh = refresh.clone(); move || refresh() };
            delete_menu(&menu, &group, &d, nid, again.clone());
            let (d2, n2, win2) = (d.clone(), nid.to_string(), win.clone());
            item(&menu, &group, "purge", "notify-purge", move || confirm_purge(&win2, &d2, &n2, again.clone()));
            popup(&list2, &menu, &group, x, y);
        });
    }
    list.add_controller(click);
    (page, refresh)
}
