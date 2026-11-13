//! The menu bar. Its menus are made again when what they show changes: the chosen session, the panels that
//! are open, the theme in force.
use std::{cell::{Cell, RefCell}, rc::Rc};

use gtk::{gio, glib, prelude::*};
use serde_json::{json, Value};
use wb_core::{paths::Data, sessions, status, theme, view::{self, Link}};

use crate::common::ctl;

fn panel_program() -> std::path::PathBuf {
    std::env::current_exe().map(|p| std::path::PathBuf::from(p.to_string_lossy().trim_end_matches(" (deleted)"))).unwrap_or_else(|_| "whaybench-panel".into())
}

fn panel_command(kind: &str) -> String { format!("exec '{}' {kind}", panel_program().display()) }

/// The tabs that are open, from the saved layout (written within a frame of any change).
fn open_keys(d: &Data) -> Vec<String> {
    let text = std::fs::read_to_string(d.layout()).unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or_default();
    v.get("panels").and_then(Value::as_array).into_iter().flatten()
        .flat_map(|p| p.get("tabs").and_then(Value::as_array).cloned().unwrap_or_default())
        .filter_map(|t| t.as_str().map(String::from)).collect()
}

struct Maker { group: gio::SimpleActionGroup, n: Cell<u32> }

impl Maker {
    fn name(&self) -> String { let n = self.n.get(); self.n.set(n + 1); format!("a{n}") }
    fn add(&self, m: &gio::Menu, label: &str, f: impl Fn() + 'static) {
        let name = self.name();
        let act = gio::SimpleAction::new(&name, None);
        act.connect_activate(move |_, _| f());
        self.group.add_action(&act);
        m.append(Some(label), Some(&format!("m.{name}")));
    }
    /// With a tick; the menu shows the state it had when it was made.
    fn check(&self, m: &gio::Menu, label: &str, on: bool, f: impl Fn() + 'static) {
        let name = self.name();
        let act = gio::SimpleAction::new_stateful(&name, None, &on.to_variant());
        act.connect_activate(move |_, _| f());
        self.group.add_action(&act);
        m.append(Some(label), Some(&format!("m.{name}")));
    }
}

/// Where people can support the project; it opens in the view panel's browser.
const SUPPORT: &str = "https://ko-fi.com/whaybench";

fn whaybench_menu(d: &Data, k: &Maker) -> gio::Menu {
    let m = gio::Menu::new();
    let d2 = d.clone();
    k.add(&m, wb_core::i18n::t("restart-tabs-kept"), move || {
        status::say(&d2, wb_core::i18n::t("restarting-tabs-come-back-shortly"), 8);
        let _ = status::post(&d2, &json!({"cmd": "restart"}));
    });
    // Ends the program as closing the window does: the tabs go to the background and come back at the next start.
    let d3 = d.clone();
    k.add(&m, wb_core::i18n::t("quit"), move || { let _ = status::post(&d3, &json!({"cmd": "quit"})); });
    // The version, shown only: an item without an action cannot be chosen.
    let about = gio::Menu::new();
    about.append(Some(&format!("Whaybench {}", env!("CARGO_PKG_VERSION"))), None);
    if let Some(v) = wb_core::update::ready(d) {
        let d4 = d.clone();
        k.add(&about, &wb_core::i18n::f("restart-to-update", &[("v", v)]), move || { let _ = status::post(&d4, &json!({"cmd": "restart"})); });
    } else if let Some(v) = wb_core::update::newer(d) {
        let d4 = d.clone();
        k.add(&about, &wb_core::i18n::f("update-to", &[("v", v.clone())]), move || {
            status::say(&d4, &wb_core::i18n::f("updating", &[("v", v.clone())]), 120);
            let d5 = d4.clone();
            // Off the screen's thread: the download takes a while. Done, Whaybench restarts with the tabs kept.
            std::thread::spawn(move || match wb_core::update::run(&d5) {
                Ok(()) => { let _ = status::post(&d5, &json!({"cmd": "restart"})); }
                Err(log) => status::say(&d5, &wb_core::i18n::f("update-failed", &[("log", log)]), 15),
            });
        });
    }
    let d6 = d.clone();
    k.add(&about, wb_core::i18n::t("support"), move || { let _ = wb_core::view::open(&d6, SUPPORT, None); });
    m.append_submenu(Some(wb_core::i18n::t("about")), &about);
    m
}

fn session_args(d: &Data, rest: &[&str]) -> Vec<String> {
    let mut a = vec!["--data-dir".to_string(), d.root.to_string_lossy().to_string(), "session".to_string()];
    a.extend(rest.iter().map(|s| s.to_string()));
    a
}

fn new_tab_with_map(d: &Data) {
    match sessions::target(d) {
        Some(s) => ctl(d, session_args(d, &["new", &s])),
        None => status::say(d, wb_core::i18n::t("new-tab-keep-map-not-opened"), 6),
    }
}

fn session_menu(d: &Data, k: &Maker) -> gio::Menu {
    let m = gio::Menu::new();
    {
        let d2 = d.clone();
        k.add(&m, wb_core::i18n::t("search-sessions"), move || {
            status::focus_app(&d2, "dev.whaybench.sessions");
            status::send(&d2, "sessions", json!({"search": true}));
        });
    }
    for (id, name) in wb_core::agents::list(&d.root) {
        let d2 = d.clone();
        k.add(&m, &wb_core::i18n::f("new-conversation-with", &[("name", name)]), move || ctl(&d2, session_args(&d2, &["new", "--agent", &id])));
    }
    {
        let d2 = d.clone();
        k.add(&m, wb_core::i18n::t("end-this-tab-s-session"), move || {
            if let Some(s) = sessions::target(&d2) { ctl(&d2, session_args(&d2, &["end", &s])); }
        });
    }
    m
}

// Tidying and the map style are on the map panel itself: there can be two map panels, and the bar would not say which.
fn map_menu(d: &Data, k: &Maker) -> gio::Menu {
    let m = gio::Menu::new();
    let sec = gio::Menu::new();
    // The map panel asks for the file: the bar is one line high and a dialog fits inside its panel.
    { let d2 = d.clone(); k.add(&sec, wb_core::i18n::t("open-map"), move || status::send(&d2, "map", json!({"do": "open"}))); }
    { let d2 = d.clone(); k.add(&sec, wb_core::i18n::t("save-map"), move || status::send(&d2, "map", json!({"do": "export"}))); }
    m.append_section(None, &sec);
    let sec = gio::Menu::new();
    { let d2 = d.clone(); k.add(&sec, wb_core::i18n::t("new-claude-tab-keep-map"), move || new_tab_with_map(&d2)); }
    m.append_section(None, &sec);
    let sec = gio::Menu::new();
    {
        let d2 = d.clone();
        k.add(&sec, wb_core::i18n::t("mapstyleset-open"), move || {
            if open_keys(&d2).iter().any(|t| t.starts_with("dev.whaybench.mapstyle")) { status::focus_app(&d2, "dev.whaybench.mapstyle"); }
            else { let _ = status::post(&d2, &json!({"cmd": "togglepanel", "key": "dev.whaybench.mapstyle", "command": panel_command("mapstyle")})); }
        });
    }
    m.append_section(None, &sec);
    m
}

fn panel_menu(d: &Data, k: &Maker) -> gio::Menu {
    let m = gio::Menu::new();
    let open = open_keys(d);
    { let d2 = d.clone(); k.add(&m, wb_core::i18n::t("reset-layout"), move || { let _ = status::post(&d2, &json!({"cmd": "resetlayout"})); }); }
    {
        let d2 = d.clone();
        k.add(&m, wb_core::i18n::t("new-view"), move || { let _ = view::new_view(&d2, &panel_program(), &Link::Follow); });
    }
    let sec = gio::Menu::new();
    let toggle = |label: &str, key: &str, kind: Option<String>| {
        let on = open.iter().any(|t| t == key || t.starts_with(&format!("{key}#")));
        let (d2, key2) = (d.clone(), key.to_string());
        k.check(&sec, label, on, move || {
            let mut r = json!({"cmd": "togglepanel", "key": key2});
            if let Some(c) = &kind { r["command"] = json!(c); }
            let _ = status::post(&d2, &r);
        });
    };
    toggle(wb_core::i18n::t("conversation-map"), "dev.whaybench.map#1", Some(panel_command("map")));
    toggle(wb_core::i18n::t("sessions"), "dev.whaybench.sessions", Some(panel_command("sessions")));
    toggle(wb_core::i18n::t("files"), "dev.whaybench.files", Some(panel_command("files")));
    toggle(wb_core::i18n::t("answer"), "dev.whaybench.answer", Some(panel_command("answer")));
    toggle(wb_core::i18n::t("input"), "dev.whaybench.input", Some(panel_command("input")));
    toggle(wb_core::i18n::t("project"), "dev.whaybench.project", Some(panel_command("project")));
    toggle(wb_core::i18n::t("notify"), "dev.whaybench.notify", Some(panel_command("notify")));
    // The first view cannot be closed: its line only brings it up.
    { let d2 = d.clone(); k.check(&sec, wb_core::i18n::t("view-2"), true, move || status::focus_app(&d2, view::VIEW_APP)); }
    toggle(wb_core::i18n::t("browser"), view::FIREFOX_APP, Some(view::firefox_command(d, "")));
    // What else is open: extra views and map branches, which only close from here.
    for key in &open {
        if let Some(rest) = key.strip_prefix("dev.whaybench.view.") {
            let name = rest.split('#').next().unwrap_or(rest).replace('_', "-");
            toggle(&view::panel_title(&name), key, None);
        } else if key.starts_with("dev.whaybench.map.") {
            let title = crate::mapimport::panels(d).into_iter().find(|a| a.first().and_then(|p| p.strip_prefix('@')).is_some_and(|p| key.starts_with(&crate::mapimport::app_id(p)))).and_then(|a| a.get(1).cloned()).unwrap_or_default();
            toggle(&wb_core::i18n::f("imported-title", &[("title", title.chars().take(14).collect::<String>())]), key, None);
        } else if key.starts_with("dev.whaybench.map#") && !key.ends_with("#1") {
            toggle(&wb_core::i18n::f("map-branch", &[("a1", (key.rsplit('#').next().unwrap_or("")).to_string())]), key, None);
        }
    }
    m.append_section(None, &sec);
    m
}

fn settings_menu(d: &Data, k: &Maker) -> gio::Menu {
    let m = gio::Menu::new();
    let themes = gio::Menu::new();
    let now = theme::load_active(d).name;
    for (id, name) in theme::list(d) {
        let (d2, id2) = (d.clone(), id.clone());
        k.check(&themes, &name, id == now, move || { let _ = status::post(&d2, &json!({"cmd": "theme", "name": id2})); });
    }
    m.append_submenu(Some(wb_core::i18n::t("theme")), &themes);
    {
        let d2 = d.clone();
        k.add(&m, wb_core::i18n::t("themeset-open"), move || {
            if open_keys(&d2).iter().any(|t| t.starts_with("dev.whaybench.theme")) { status::focus_app(&d2, "dev.whaybench.theme"); }
            else { let _ = status::post(&d2, &json!({"cmd": "togglepanel", "key": "dev.whaybench.theme", "command": panel_command("theme")})); }
        });
    }
    // Under the theme: which language. The program's texts are read once, so a change applies after a restart.
    let languages = gio::Menu::new();
    let now = wb_core::i18n::lang().to_string();
    for (id, name) in wb_core::i18n::list(d) {
        let (d2, id2) = (d.clone(), id.clone());
        k.check(&languages, &name, id == now, move || {
            wb_core::i18n::choose(&d2, &id2);
            status::say(&d2, wb_core::i18n::t("language-changed-applied-after-restart"), 8);
        });
    }
    m.append_submenu(Some(wb_core::i18n::t("language")), &languages);
    m
}

fn make(d: &Data) -> (gio::Menu, gio::SimpleActionGroup) {
    let k = Maker { group: gio::SimpleActionGroup::new(), n: Cell::new(0) };
    let bar = gio::Menu::new();
    bar.append_submenu(Some("Whaybench"), &whaybench_menu(d, &k));
    bar.append_submenu(Some(wb_core::i18n::t("sessions")), &session_menu(d, &k));
    bar.append_submenu(Some(wb_core::i18n::t("conversation-map")), &map_menu(d, &k));
    bar.append_submenu(Some(wb_core::i18n::t("panels")), &panel_menu(d, &k));
    bar.append_submenu(Some(wb_core::i18n::t("settings")), &settings_menu(d, &k));
    (bar, k.group)
}

/// What the menus depend on; they are made again only when it changes.
fn signature(d: &Data) -> String { format!("{:?} {:?} {:?} {:?} {} {:?} {:?} {} {:?}", wb_core::update::newer(d), wb_core::update::ready(d), open_keys(d), sessions::target(d), theme::load_active(d).name, theme::list(d), wb_core::i18n::list(d), crate::mapstyles::current(d, "map"), crate::mapstyles::list(d).iter().map(|x| x.id.clone()).collect::<Vec<_>>()) }

/// Whether one of the bar's menus is showing.
fn menu_open(w: &gtk::Widget) -> bool {
    if w.downcast_ref::<gtk::Popover>().is_some_and(|p| p.is_visible()) { return true; }
    let mut child = w.first_child();
    while let Some(c) = child {
        if menu_open(&c) { return true; }
        child = c.next_sibling();
    }
    false
}

pub fn build(win: &gtk::ApplicationWindow, d: Data) {
    let (model, group) = make(&d);
    let bar = gtk::PopoverMenuBar::from_model(Some(&model));
    win.insert_action_group("m", Some(&group));
    win.set_child(Some(&bar));
    let inside = Rc::new(Cell::new(false));
    let hover = gtk::EventControllerMotion::new();
    { let i = inside.clone(); hover.connect_enter(move |_, _, _| i.set(true)); }
    { let i = inside.clone(); hover.connect_leave(move |_| i.set(false)); }
    bar.add_controller(hover);
    // Never under the pointer or with a menu showing: replacing the menus then pulls the widgets out from under the
    // event being handled, and the app falls over.
    // The latest release, looked up at start and then every six hours (the check itself skips when it is too soon).
    if wb_core::update::installed_release() {
        let d2 = d.clone();
        std::thread::spawn(move || {
            let look = |at_start: bool| {
                wb_core::update::check(&d2, at_start);
                if let Some(v) = wb_core::update::newer(&d2).filter(|_| wb_core::update::automatic(&d2)) {
                    if let Err(e) = wb_core::update::fetch(&d2, &v) { wb_core::log::warn("update", "not fetched", &e, &[]); }
                }
            };
            look(true);
            loop { std::thread::sleep(std::time::Duration::from_secs(3600)); look(false); }
        });
    }
    let (w, last) = (win.clone(), RefCell::new(signature(&d)));
    glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
        if inside.get() || menu_open(bar.upcast_ref()) { return glib::ControlFlow::Continue; }
        let now = signature(&d);
        if *last.borrow() != now {
            let (model, group) = make(&d);
            w.insert_action_group("m", Some(&group));
            bar.set_menu_model(Some(&model));
            *last.borrow_mut() = now;
        }
        glib::ControlFlow::Continue
    });
}
