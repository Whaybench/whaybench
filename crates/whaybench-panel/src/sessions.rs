//! The session panel: the list, states and search.
use std::{cell::RefCell, rc::Rc};

use gtk::{gio, glib, prelude::*};
use serde_json::Value;
use wb_core::{log as record, paths::Data, sessions, status};

use crate::common::{ago, ctl, cut};

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
    let read = |p: std::path::PathBuf| wb_core::paths::read_text(&p).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
    if let Some(a) = read(wb_core::roots::for_session(d, id).root.join("answers").join(format!("{id}.json"))) { out.push((wb_core::i18n::t("answer"), text(&a, "text").to_string())); }
    out
}

/// What runs for the session besides it, as one line under the session, counted by agent: `Codex (2), Claude Code (1)`.
fn subagents_note(d: &Data, id: &str) -> Option<String> {
    let running = sessions::subagents_running(d, id);
    if running.is_empty() { return None; }
    let own = wb_core::agents::name_of(&d.root, sessions::record_of(d, id).as_ref().and_then(|r| r["agent"].as_str()));
    Some(wb_core::i18n::f("subagents-running", &[("a1", counted(&running, &own, &d.root))]))
}

/// A subagent is the session's own agent's; a task a program reported goes by its name, an agent's when it names one.
fn counted(running: &[Value], own: &str, root: &std::path::Path) -> String {
    let mut by: Vec<(String, usize)> = Vec::new();
    for a in running {
        let name = if a.get("source").is_none() { own.to_string() } else {
            let t = text(a, "type");
            wb_core::agents::list(root).into_iter().find(|(i, _)| i == t).map_or(t.to_string(), |x| x.1)
        };
        match by.iter_mut().find(|x| x.0 == name) { Some(x) => x.1 += 1, None => by.push((name, 1)) }
    }
    by.iter().map(|(n, c)| format!("{n} ({c})")).collect::<Vec<_>>().join(", ")
}

/// Sessions being restarted on a new Claude Code now: one restart at a time each.
static RESPAWNING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Why a session on an older Claude Code cannot be restarted now, as a word for its menu; None when it can.
fn respawn_block(d: &Data, id: &str) -> Option<&'static str> {
    if RESPAWNING.lock().unwrap().iter().any(|r| r == id) { return Some("respawn-busy"); }
    match sessions::state_of(d, id).as_deref() {
        Some("working") => return Some("respawn-working"),
        Some("waiting") => return Some("respawn-waiting"),
        _ => {}
    }
    // A self-paced /loop's wake-up is not brought back by a restart.
    if sessions::schedules::live(&wb_core::roots::for_session(d, id).root, id).iter().any(|j| j["kind"] == "wakeup") { return Some("respawn-loop"); }
    None
}

/// Restarts the session on the installed Claude Code off the panel's loop; what goes wrong goes to the log.
fn respawn(d: &Data, id: &str) {
    if respawn_block(d, id).is_some() { return; }
    RESPAWNING.lock().unwrap().push(id.to_string());
    let (d, id) = (d.clone(), id.to_string());
    std::thread::spawn(move || {
        if let Err(e) = sessions::respawn(&id) { record::warn("session", "not restarted on the new Claude Code", &e, &[("session", id.clone())]); }
        sessions::refresh_agents();
        // The supervisor may still be on the old one and start the session there again: the mark stays.
        if let Some((now, installed)) = sessions::old_version(&d, &id) {
            record::warn("session", "still on the old Claude Code", "its supervisor has not moved to the new one yet", &[("session", id.clone()), ("running", now), ("installed", installed)]);
        }
        RESPAWNING.lock().unwrap().retain(|r| r != &id);
    });
}

/// The sessions with a card waiting in the answer panel, read once for the whole list.
fn asking(d: &Data) -> std::collections::BTreeSet<String> {
    wb_core::ask::pending_all(d).iter().filter_map(|a| a["session"].as_str().map(String::from)).collect()
}

fn describe(d: &Data, info: &Value, asking: &std::collections::BTreeSet<String>) -> (String, String) {
    let id = sid(info);
    let alive = sessions::alive(d, id);
    // A card waiting in the answer panel is a wait for the person, also a held-back Codex question whose turn ended.
    let asking = alive && asking.contains(id);
    let state = if asking { Some("waiting".to_string()) } else if alive { sessions::state_of(d, id) } else { None };
    let mut parts = vec![wb_core::agents::name_of(&d.root, info.get("agent").and_then(Value::as_str)), match (alive, state.as_deref()) {
        (false, _) => wb_core::i18n::t("off").to_string(),
        (_, Some("working")) => wb_core::i18n::t("working").into(),
        (_, Some("waiting")) => wb_core::i18n::t("input-needed").into(),
        (_, Some("done")) => wb_core::i18n::t("answered").into(),
        _ => wb_core::i18n::t("running").into(),
    }, ago(info.get("lastActive").and_then(Value::as_i64).unwrap_or(0))];
    let jobs = if alive { sessions::schedules::live(&wb_core::roots::for_session(d, id).root, id).len() } else { 0 };
    if jobs > 0 { parts.push(wb_core::i18n::f("schedules-count", &[("n", jobs.to_string())])); }
    if let Some(m) = sessions::map_of(d, id) {
        let nodes = m.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
        let here = nodes.iter().find(|n| n.get("id") == m.get("current")).map(|n| text(n, "label").to_string());
        parts.push(here.map(|h| format!("● {}", cut(&h, 22))).unwrap_or_else(|| wb_core::i18n::t("map-2").into()));
        parts.push(wb_core::i18n::f("open-5", &[("a1", (nodes.iter().filter(|n| wb_core::map::is_open(n)).count()).to_string())]));
    }
    let name = sessions::session_name(d, id);
    let title = if name.is_empty() { wb_core::i18n::t("empty-session").into() } else { cut(&name, 34) };
    // On an older agent than the one installed: a small mark on the title. Claude Code's restart is in its right-click menu.
    let title = if alive && sessions::old_version(d, id).is_some() { format!("⟳ {title}") } else { title };
    (title, parts.join(" · "))
}

/// `only`: one Briefsafe project's conversations alone, as the project panel lists them ("" lists none).
struct View { query: String, show_hidden: bool, signature: String, first: Option<String>, only: Option<String> }

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

/// How long a conversation is looked at every two seconds after its last activity: half a year. An older one stays in the
/// list as it was last drawn; its next start makes it recent again.
const LOOKED_AT_MS: i64 = 183 * 86_400_000;

fn recent(info: &Value, now: i64) -> bool { now - info.get("lastActive").and_then(Value::as_i64).unwrap_or(0) < LOOKED_AT_MS }

fn redraw(d: &Data, list: &gtk::ListBox, view: &Rc<RefCell<View>>, force: bool) {
    // One project's list reads that project's conversations, not every one; with none to show, nothing.
    let one = view.borrow().only.clone();
    let root = match one.as_deref() {
        None => None,
        Some(id) => match wb_core::roots::stubs(d).into_iter().find(|s| s.id == id && s.open()) {
            Some(st) => Some(st.root()),
            None => {
                if view.borrow().signature != "-" { view.borrow_mut().signature = "-".into(); while let Some(r) = list.row_at_index(0) { list.remove(&r); } }
                return;
            }
        },
    };
    let all = sessions::all(root.as_ref().unwrap_or(d));
    let chosen = sessions::focus(d);
    let latest = sessions::latest(d);
    let open = sessions::open_tabs(d);
    let (query, show_hidden, only) = { let v = view.borrow(); (v.query.clone(), v.show_hidden, v.only.clone()) };
    let waiting = asking(d);
    let stubs = wb_core::roots::stubs(d);
    let now = wb_core::clock::now_ms();
    let signature = format!("{}{only:?}{chosen:?}{show_hidden}{query}{latest:?}{waiting:?}{:?}{:?}{:?}", all.len(), stubs.iter().map(|s| (&s.id, &s.name, s.open())).collect::<Vec<_>>(), open.iter().map(|t| (&t.tab, &t.session)).collect::<Vec<_>>(), all.iter().filter(|s| recent(s, now)).map(|s| {
        let id = sid(s);
        (id.to_string(), s.get("lastActive").cloned(), sessions::alive(d, id), sessions::state_of(d, id), s.get("ended").cloned(), sessions::map_changed(d, id), subagents_note(d, id), sessions::schedules::live(&wb_core::roots::for_session(d, id).root, id).len(), sessions::old_version(d, id))
    }).collect::<Vec<_>>());
    if !force && view.borrow().signature == signature { return; }
    view.borrow_mut().signature = signature;
    let rows = || {
        // Rows only, by index: a menu opened on the list is its child too, and GTK's own remove_all loops on it forever.
        while let Some(r) = list.row_at_index(0) { list.remove(&r); }
        let mut first = None;
        let add = |info: &Value, faded: bool, note: Option<String>| {
            let (title, sub) = describe(d, info, &waiting);
            let r = row(&title, &sub, note.as_deref(), faded);
            r.set_widget_name(sid(info));
            // An add-on agent has no restart in the menu that names the versions.
            if info.get("agent").is_some() && let Some((running, installed)) = sessions::old_version(d, sid(info)) { r.set_tooltip_text(Some(&format!("{running} → {installed}"))); }
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
            if only.is_none() {
                let follow = row(wb_core::i18n::t("follow-latest-session"), &wb_core::i18n::f("current-2", &[("a1", (latest.as_deref().map(|l| cut(l, 8)).unwrap_or_else(|| wb_core::i18n::t("none").into())).to_string())]), None, false);
                follow.set_widget_name("__follow__");
                if chosen.is_none() { follow.add_css_class("chosen"); }
                list.append(&follow);
            }
            let held = |s: &Value| open.iter().any(|t| t.tab == text(s, "tab") || t.session.as_deref() == Some(sid(s)));
            let mut hidden = Vec::new();
            // The host's conversations, then each Briefsafe project's under its name (spec 3.22): a locked one is its name
            // only, nothing of its conversations.
            let roots = wb_core::roots::Roots::new(d.clone());
            let of: Vec<Option<String>> = all.iter().map(|i| roots.project_of(sid(i))).collect();
            let groups = only.is_none().then_some(None).into_iter().chain(stubs.iter().filter(|st| only.as_ref().is_none_or(|o| *o == st.id)).map(Some));
            for group in groups {
                if only.is_some() && group.is_some_and(|st| !st.open()) { continue; }
                if let Some(st) = group.filter(|_| only.is_none()) {
                    let head = row(&format!("▣ {}", st.name), if st.open() { wb_core::i18n::t("briefsafe-open") } else { wb_core::i18n::t("briefsafe-locked") }, None, !st.open());
                    head.set_widget_name("__project__");
                    head.set_activatable(false);
                    list.append(&head);
                    if !st.open() { continue; }
                }
                let want = group.map(|st| st.id.as_str());
                for (info, p) in all.iter().zip(&of) {
                    if p.as_deref() != want { continue; }
                    let spoken = !text(info, "firstPrompt").is_empty();
                    // Moved on, or stopped without ever being spoken in: gone. Never spoken in and in no tab: a spare process.
                    if moved(info) || (!spoken && !sessions::alive(d, sid(info))) || (!spoken && !held(info)) { continue; }
                    // A number left behind by a move that did not hold: its conversation is listed under the number it went on in.
                    if sessions::ghost(d, &all, sid(info)) { continue; }
                    if info.get("ended") == Some(&Value::Bool(true)) { hidden.push(info); continue; }
                    add(info, false, subagents_note(d, sid(info)));
                }
            }
            if only.as_ref().is_some_and(|o| !o.is_empty()) && list.row_at_index(0).is_none() && hidden.is_empty() {
                let r = row(wb_core::i18n::t("briefsafe-no-sessions"), "", None, true);
                r.set_selectable(false);
                r.set_activatable(false);
                list.append(&r);
            }
            if !hidden.is_empty() {
                let head = row(&wb_core::i18n::f("hidden-sessions", &[("a1", (if show_hidden { "▾" } else { "▸" }).to_string()), ("a2", (hidden.len()).to_string())]), "", None, false);
                head.set_widget_name("__hidden__");
                list.append(&head);
                if show_hidden { for info in hidden { add(info, true, None); } }
            }
        }
        view.borrow_mut().first = first;
    };
    // At another search or a choice in the list it is drawn anew; a conversation that changed leaves the list where it was scrolled.
    if force { rows() } else { crate::common::keep_scroll(list, rows) }
}

pub fn build(win: &gtk::ApplicationWindow, d: Data) {
    crate::common::follow_agents();
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
    let view = Rc::new(RefCell::new(View { query: String::new(), show_hidden: false, signature: String::new(), first: None, only: None }));

    list_presses(&list, &d, &view);
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
    list_menu(&list, &d, &view);
    // Session → Search sessions in the menu bar (a message to this panel), and the sessions themselves: read when they
    // change; "n minutes ago" moves on with the clock, once a minute.
    // A session of an add-on agent shown running ends when its heartbeat runs out: looked at again then.
    let d0 = d.clone();
    let (d2, list2, view2) = (d.clone(), list.clone(), view.clone());
    let again: crate::common::Wake = Rc::default();
    let a2 = again.clone();
    let f: Rc<dyn Fn()> = Rc::new(move || {
        if status::take(&d, "sessions").is_some() {
            search.grab_focus();
            search.select_region(0, -1);
        }
        redraw(&d, &list, &view, false);
        if let Some(wait) = sessions::next_stale(&d) { crate::common::wake_at(&a2, wait + std::time::Duration::from_millis(100)); }
    });
    again.borrow_mut().0 = Some(f.clone());
    // Changes taken together for two seconds, as it read before; the menu bar's search at once.
    crate::common::on_change("sessions-message", &[(d0.cache().join("panels"), &["sessions.json"])], f.clone());
    crate::common::follow_data("sessions", &d0, 2000, |_| Vec::new(), f);
    glib::timeout_add_seconds_local(60, move || { redraw(&d2, &list2, &view2, false); glib::ControlFlow::Continue });
}

fn data_arg(d: &Data) -> Vec<String> { vec!["--data-dir".to_string(), d.root.to_string_lossy().to_string()] }

/// One Briefsafe project's conversations as this panel lists them (the same rows, presses and menu), for the Projects
/// panel: the list, and what to call to choose the project ("" for none) and draw it anew.
pub fn of_project(d: &Data) -> (gtk::ListBox, impl Fn(&str) + Clone + 'static) {
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.set_activate_on_single_click(true);
    let view = Rc::new(RefCell::new(View { query: String::new(), show_hidden: false, signature: String::new(), first: None, only: Some(String::new()) }));
    list_presses(&list, d, &view);
    list_menu(&list, d, &view);
    let (d, l) = (d.clone(), list.clone());
    (list, move |id: &str| {
        { let mut v = view.borrow_mut(); if v.only.as_deref() != Some(id) { v.only = Some(id.to_string()); v.show_hidden = false; v.signature.clear(); } }
        redraw(&d, &l, &view, false);
    })
}

fn list_presses(list: &gtk::ListBox, d: &Data, view: &Rc<RefCell<View>>) {
    {
        let (d, list2, view) = (d.clone(), list.clone(), view.clone());
        list.connect_row_activated(move |_, r| {
            let name = r.widget_name().to_string();
            match name.as_str() {
                "__hidden__" => { let v = view.borrow().show_hidden; view.borrow_mut().show_hidden = !v; }
                "__follow__" => sessions::clear_focus(&d),
                "" => return,
                other if other.starts_with("__") => return,
                id => {
                    sessions::set_focus(&d, id, "person's click");
                    let mut a = data_arg(&d);
                    // A project's conversation opens as that project's, refused if it has closed since the row was drawn.
                    match wb_core::roots::Roots::new(d.clone()).project_of(id) {
                        Some(p) => a.extend(["session".into(), "show".into(), "--project".into(), p, id.to_string()]),
                        None => a.extend(["session".into(), "show".into(), id.to_string()]),
                    }
                    ctl(&d, a);
                }
            }
            redraw(&d, &list2, &view, true);
        });
    }
}

fn list_menu(list: &gtk::ListBox, d: &Data, view: &Rc<RefCell<View>>) {
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
            // A background session on an older Claude Code: restart it on the new one, or say why not now (an item
            // with no action shows dimmed).
            let old = sessions::old_version(&d, &id).filter(|_| running && sessions::background(&id));
            let block = old.as_ref().and_then(|_| respawn_block(&d, &id));
            if let Some((_, installed)) = &old {
                match block {
                    None => menu.append(Some(&wb_core::i18n::f("session-respawn", &[("version", installed.clone())])), Some("s.respawn")),
                    Some(why) => menu.append(Some(&wb_core::i18n::f("session-respawn-blocked", &[("why", wb_core::i18n::t(why).to_string())])), Some("s.respawn-blocked")),
                }
            }
            let group = gio::SimpleActionGroup::new();
            if old.is_some() && block.is_none() {
                let act = gio::SimpleAction::new("respawn", None);
                let (d2, id2) = (d.clone(), id.clone());
                // Checked again when chosen: a turn may have started since the menu opened.
                act.connect_activate(move |_, _| respawn(&d2, &id2));
                group.add_action(&act);
            }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_conversation_is_looked_at_for_half_a_year_after_its_last_activity() {
        let day = 86_400_000;
        let last = |days: i64| serde_json::json!({"id": "s", "lastActive": 1000 * day - days * day});
        assert!(recent(&last(0), 1000 * day) && recent(&last(182), 1000 * day));
        assert!(!recent(&last(184), 1000 * day));
    }
    #[test]
    fn helpers_are_counted_by_agent() {
        let root = std::env::temp_dir().join(format!("wb-helpers-{}", std::process::id()));
        let r = |v: Value| v;
        let running = [r(serde_json::json!({"type": "codex", "source": "codex-ask"})), r(serde_json::json!({"type": "general-purpose", "tool": "Bash"})),
            r(serde_json::json!({"type": "codex", "source": "codex-ask"})), r(serde_json::json!({"type": "build", "source": "script"}))];
        assert_eq!(counted(&running, "Claude Code", &root), "codex (2), Claude Code (1), build (1)", "a name that is no agent's stays as given");
    }
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
