//! The project panel: a project is its map and its recent changes; documents are add-ons, added one at a
//! time. There is no edit box: only the AI writes, and a change is a request to the chosen session. Pages are drawn as
//! HTML; their links are not followed but handled here.
use std::{cell::RefCell, collections::HashMap, path::Path, rc::Rc, time::{Duration, UNIX_EPOCH}};

use gtk::{glib, prelude::*};
use regex::Regex;
use serde_json::{json, Value};
use wb_core::{log as record, paths::Data, sessions, status};
use crate::web::Page;
use wb_store::{Store, docaddons, history, markdown as md};
use webkit6::prelude::*;

mod briefsafe;
mod pages;
use briefsafe::*;
use pages::*;

const MAP: &str = "map";
const RECENT: &str = "recent";
/// A Briefsafe project's conversations.
const SESSIONS: &str = "sessions";

/// The project shown now, as links name it.
fn shown_key(p: &P) -> String { p.bs.as_ref().map(|b| bs_key(b)).or(p.pid.clone()).unwrap_or_default() }
/// The project and the page shown now.
fn shown_at(p: &P) -> String { format!("{}\u{1}{}\u{1}{}", shown_key(p), p.tab, p.over.is_some()) }
/// A Briefsafe project's page key: never equal to a host project's, even one of the same id.
fn bs_key(id: &str) -> String { format!("bs:{id}") }

struct P {
    d: Data,
    st: Store,
    pid: Option<String>,
    rows: Vec<(String, String, usize)>,
    /// The tab on show: MAP, RECENT or a document's id.
    tab: String,
    anchors: HashMap<String, HashMap<String, String>>,
    /// The tabs as they are drawn, to draw them anew only when they change.
    tab_keys: Vec<(String, String)>,
    /// A conversion of a Word or Excel template is running.
    converting: Rc<std::cell::Cell<bool>>,
    /// A page over the tabs: a change's detail, as its target and event.
    over: Option<(String, String)>,
    range: Option<i64>,
    stamp: String,
    shown: String,
    scroll: HashMap<String, f64>,
    /// The page key the view shows now: tab of a project.
    key_now: String,
    failed: String,
    web: webkit6::WebView,
    pick: gtk::DropDown,
    path: gtk::Label,
    /// An open Briefsafe project's lines (set_lines): what can be done with it, its state, the agents to start.
    actions: gtk::Label,
    note: gtk::Label,
    agents: gtk::Label,
    /// Its conversations (the Sessions panel's list), shown in place of the page on the Sessions tab.
    talks: gtk::ScrolledWindow,
    list_talks: Rc<dyn Fn(&str)>,
    tabs: gtk::FlowBox,
    tab_buttons: Vec<(String, gtk::Button)>,
    win: gtk::ApplicationWindow,
    /// The host's own projects (`st` is a Briefsafe project's store while one is shown and open).
    host: Store,
    /// The record of the project on show cannot be read.
    unreadable: bool,
    /// A step waiting for the keyring's password, and one running again after it.
    ask_keyring: Option<Asking>,
    after_unlock: Option<Asking>,
    keyring_anew: bool,
    /// The Briefsafe project on show (spec 3.22), by id.
    bs: Option<String>,
    /// A `whaybench-project` step running (log in, log out, snapshot), for one project.
    job: Option<Job>,
    /// A log out asked for while another step ran (a snapshot): done once it ends.
    close_after: Vec<String>,
    /// A dialog open over a project's page (a request): closed when that project locks.
    dialog: Option<gtk::glib::WeakRef<gtk::Window>>,
    /// What the last step of a project said when it did not work: its project, the kind of step and the reason.
    bs_msg: HashMap<String, (String, String)>,
}
type Shared = Rc<RefCell<P>>;

fn stamp_of(st: &Store, pid: &str) -> String {
    let base = st.pdir(pid);
    ["events", "map.json", "project.json"].iter().map(|f| wb_core::paths::meta(&base.join(f)).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |t| t.as_nanos()).to_string()).collect::<Vec<_>>().join(",")
}

/// Briefsafe projects are listed by this key, apart from the host's projects' ids.
const BS: &str = "bs:";

/// Shows the project of this list key: a host project, or a Briefsafe one (its store only once it is open).
fn choose(p: &mut P, key: Option<String>) {
    match key.as_deref().and_then(|k| k.strip_prefix(BS)) {
        // The host's store until the next look finds it open: never another project's pages under this name.
        Some(id) => { p.bs = Some(id.to_string()); p.pid = Some(id.to_string()); p.st = p.host.clone(); p.tab = SESSIONS.to_string(); }
        None => { p.bs = None; p.pid = key; p.st = p.host.clone(); if p.tab == SESSIONS { p.tab = MAP.to_string(); } }
    }
    p.stamp.clear();
    p.over = None;
}

/// A request for the chosen session's AI; only to one that is running.
fn request(p: &P, ask: &str) {
    let Some(pid) = &p.pid else { return };
    let Some(session) = sessions::focus(&p.d) else { status::say(&p.d, wb_core::i18n::t("request-not-sent-no-session-selected"), 8); return };
    // A Briefsafe project's page asks only that project's own conversations: its text never goes to a host agent.
    if let Some(id) = &p.bs && wb_core::roots::Roots::new(p.d.clone()).project_of(&session).as_deref() != Some(id.as_str()) {
        status::say(&p.d, wb_core::i18n::t("briefsafe-request-own-only"), 8); return;
    }
    let text = wb_core::i18n::ai_f("project-this-session-edits-the-project", &[("pid", (pid).to_string()), ("ask", (ask).to_string())]);
    if sessions::queue_prompt(&p.d, &session, &text) { status::say(&p.d, wb_core::i18n::t("request-sent-selected-session"), 5); }
    else { status::say(&p.d, wb_core::i18n::t("request-not-sent-session-is-off-2"), 8); }
}

/// A short question in a window of its own, over the panel. `done` gets the text if there is any.
fn ask_text(win: &gtk::ApplicationWindow, title: &str, label: &str, done: impl Fn(String) + 'static) -> gtk::Window {
    let dlg = gtk::Window::builder().title(title).transient_for(win).modal(true).decorated(false).build();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    body.set_margin_top(10); body.set_margin_bottom(10); body.set_margin_start(10); body.set_margin_end(10);
    let lab = gtk::Label::new(Some(label));
    lab.set_xalign(0.0);
    lab.set_wrap(true);
    lab.set_max_width_chars(20);
    body.append(&lab);
    let buffer = gtk::TextBuffer::new(None);
    let view = gtk::TextView::with_buffer(&buffer);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_vexpand(true);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&view));
    scroll.set_vexpand(true);
    body.append(&scroll);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_halign(gtk::Align::End);
    let (cancel, ok) = (gtk::Button::with_label(wb_core::i18n::t("cancel")), gtk::Button::with_label(wb_core::i18n::t("send")));
    row.append(&cancel);
    row.append(&ok);
    body.append(&row);
    dlg.set_child(Some(&body));
    { let d = dlg.clone(); cancel.connect_clicked(move |_| d.close()); }
    {
        let d = dlg.clone();
        ok.connect_clicked(move |_| {
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).trim().to_string();
            d.close();
            if !text.is_empty() { done(text); }
        });
    }
    dlg.present();
    view.grab_focus();
    dlg
}

/// The folder name of a new project: from its name, or from the time when the name has no latin letters; never one in use.
fn new_project_id(st: &Store, name: &str) -> String {
    let slug: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>().split('-').filter(|x| !x.is_empty()).collect::<Vec<_>>().join("-");
    // Short enough for a file name with an ending.
    let slug = slug.chars().take(60).collect::<String>().trim_end_matches('-').to_string();
    let base = if slug.is_empty() { format!("p-{}", (wb_core::clock::now_ms() / 1000)) } else { slug };
    // One id for one project: a Briefsafe project's is taken too.
    let stubs: Vec<String> = wb_core::roots::stubs(&st.data).into_iter().map(|s| s.id).collect();
    let free = |x: &str| st.get_project(x).is_none() && !stubs.iter().any(|s| s == x);
    if free(&base) { return base; }
    (2..).map(|n| format!("{base}-{n}")).find(|x| free(x)).unwrap()
}

/// The documents to show: those added as add-ons. Older ones without a template stay on file, out of sight.
/// Every document of the project gets a tab: it is Markdown whatever made it (a template add-on, or a format the
/// program no longer has).
/// An archived one (a finished change proposal) has none; it is opened from the + menu.
fn shown_docs(project: &Value) -> Vec<Value> { list(project, "docs").iter().filter(|d| !archived(d)).cloned().collect() }
fn archived(d: &Value) -> bool { d["archived"] == true }

/// The map, then the documents, then recent changes, then the way to add one.
fn set_tabs(s_: &Shared, keys: Vec<(String, String)>) {
    let mut p = s_.borrow_mut();
    // The way to add a document depends on whether a Briefsafe project is shown, not only on the tabs.
    let mut keys = keys;
    if p.bs.is_some() && !keys.is_empty() { keys.push((BS.to_string(), String::new())); }
    if p.tab_keys != keys {
        p.tab_keys = keys.clone();
        keys.retain(|k| k.0 != BS);
        // No tabs (a locked project): not even the way to add a document. Nor for an open Briefsafe project: its
        // documents are its agents' to add, in its container, not the host's to write there.
        let any = !keys.is_empty() && p.bs.is_none();
        while let Some(c) = p.tabs.first_child() { p.tabs.remove(&c); }
        p.tab_buttons.clear();
        for (key, label) in keys {
            let b = gtk::Button::with_label(&label);
            b.add_css_class("flat");
            let (s2, k) = (s_.clone(), key.clone());
            b.connect_clicked(move |_| {
                { let mut p = s2.borrow_mut(); p.tab = k.clone(); p.over = None; }
                redraw(&s2);
            });
            p.tabs.insert(&b, -1);
            p.tab_buttons.push((key, b));
        }
        if any {
        let plus = gtk::Button::with_label(wb_core::i18n::t("document"));
        plus.add_css_class("flat");
        plus.set_tooltip_text(Some(wb_core::i18n::t("add-a-document-template-add-on")));
        let s2 = s_.clone();
        plus.connect_clicked(move |b| add_doc_menu(&s2, b));
        p.tabs.insert(&plus, -1);
        }
    }
    for (key, b) in &p.tab_buttons {
        if *key == p.tab && p.over.is_none() { b.add_css_class("chosen"); } else { b.remove_css_class("chosen"); }
    }
}

fn add_doc_menu(s_: &Shared, on: &gtk::Button) {
    // A Briefsafe project's documents are its agents' to add (the button is not there; nor is this way in).
    if s_.borrow().bs.is_some() { return; }
    let p = s_.borrow();
    let Some(pid) = p.pid.clone() else { p.path.set_text(wb_core::i18n::t("no-project-create-a-new-project")); return };
    let project = p.st.get_project(&pid).unwrap_or_default();
    let free = docaddons::list(&p.d);
    let old: Vec<Value> = list(&project, "docs").iter().filter(|d| archived(d)).cloned().collect();
    if free.is_empty() && old.is_empty() { p.path.set_text(wb_core::i18n::t("no-document-template-to-add-install")); return; }
    let menu = gtk::gio::Menu::new();
    let group = gtk::gio::SimpleActionGroup::new();
    for (i, a) in free.into_iter().enumerate() {
        let name = format!("a{i}");
        let act = gtk::gio::SimpleAction::new(&name, None);
        menu.append(Some(&a.name), Some(&format!("d.{name}")));
        let (s2, pid2) = (s_.clone(), pid.clone());
        act.connect_activate(move |_, _| {
            let (s3, pid3, a2) = (s2.clone(), pid2.clone(), a.clone());
            glib::idle_add_local_once(move || {
                let result = s3.borrow().st.add_doc(&pid3, &a2);
                match result {
                    Ok(id) => {
                        record::decide("project", "document added", "the person chose a template", &[("project", pid3.clone()), ("template", a2.id.clone()), ("document", id.clone())]);
                        { let mut p = s3.borrow_mut(); p.tab = id; p.over = None; p.stamp.clear(); }
                        tick(&s3);
                    }
                    Err(e) => s3.borrow().path.set_text(&wb_core::i18n::f("adding-the-document-failed", &[("e", (e).to_string())])),
                }
            });
        });
        group.add_action(&act);
    }
    // Archived documents in a submenu at the end: there only when there are some.
    if !old.is_empty() {
        let sub = gtk::gio::Menu::new();
        for (i, d) in old.iter().enumerate() {
            let name = format!("o{i}");
            let act = gtk::gio::SimpleAction::new(&name, None);
            sub.append(Some(s(d, "name")), Some(&format!("d.{name}")));
            let (s2, did) = (s_.clone(), s(d, "id").to_string());
            act.connect_activate(move |_, _| {
                let (s3, did) = (s2.clone(), did.clone());
                glib::idle_add_local_once(move || { { let mut p = s3.borrow_mut(); p.tab = did; p.over = None; } redraw(&s3); });
            });
            group.add_action(&act);
        }
        menu.append_submenu(Some(wb_core::i18n::t("archived-documents")), &sub);
    }
    let pop = gtk::PopoverMenu::from_model_full(&menu, gtk::PopoverMenuFlags::NESTED);
    pop.set_parent(on);
    pop.insert_action_group("d", Some(&group));
    pop.set_has_arrow(false);
    pop.connect_closed(|p| { let p = p.clone(); glib::idle_add_local_once(move || p.unparent()); });
    pop.popup();
}

/// Word and Excel templates become Markdown as soon as they are found, away from the window: it takes seconds.
fn convert_waiting(p: &P) {
    if p.converting.get() { return; }
    let found = docaddons::waiting(&p.d);
    if found.is_empty() { return; }
    p.converting.set(true);
    let (d, flag) = (p.d.clone(), p.converting.clone());
    let (send, take) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        for f in found {
            let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            status::say(&d, &wb_core::i18n::f("converting-document-template", &[("name", (name).to_string())]), 20);
            match docaddons::convert(&d, &f) {
                Ok(0) => { record::decide("project", "template converted", "a template with only its original", &[("file", name.clone())]); status::say(&d, &wb_core::i18n::f("template-converted", &[("name", name.clone())]), 8); }
                Ok(n) => { record::warn("project", "template converted", "a table whose cell counts differ", &[("file", name.clone()), ("rows", n.to_string())]); status::say(&d, &wb_core::i18n::f("template-converted-uneven", &[("name", name.clone()), ("n", n.to_string())]), 12); }
                Err(e) => { record::warn("project", "template conversion failed", &e, &[("file", name.clone())]); docaddons::note_failure(&f, &e); status::say(&d, &wb_core::i18n::f("template-conversion-failed", &[("name", name.clone()), ("e", e.to_string())]), 12); }
            }
        }
        let _ = send.send(());
    });
    glib::timeout_add_local(Duration::from_millis(500), move || {
        if take.try_recv().is_ok() { flag.set(false); glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
    });
}

/// Draws what is on show. The place in the page stays when the same page is drawn again.
fn redraw(s_: &Shared) {
    let (page, key, web, tabs) = {
        let mut p = s_.borrow_mut();
        let Some(pid) = p.pid.clone() else { return };
        p.unreadable = false;
        let Some(project) = p.st.get_project(&pid) else {
            // Its record is missing or unreadable (a Briefsafe project's is its agents' to change): said, nothing else shown.
            // (Log out and the rest stay for an open Briefsafe project: its lines are the panel's own.)
            let page = format!("<!doctype html><meta charset='utf-8'><style>{}</style><body><p class='warn'>{}</p></body>", style(&p.d), e(wb_core::i18n::t("briefsafe-unreadable")));
            if p.shown != page { p.web.load_html(&page, Some("about:blank")); p.shown = page; }
            // A Briefsafe project's conversations stay to be opened (one of them can put it right).
            let open = p.bs.is_some();
            if open { p.unreadable = true; p.tab = SESSIONS.to_string(); p.over = None; }
            drop(p);
            set_tabs(s_, if open { vec![(SESSIONS.to_string(), wb_core::i18n::t("sessions").to_string())] } else { Vec::new() });
            show_talks(&s_.borrow());
            return;
        };
        let up: Vec<String> = p.st.ancestors(&pid).iter().map(|a| s(a, "name").to_string()).chain(std::iter::once(s(&project, "name").to_string())).collect();
        let kids = p.st.children(&pid).len();
        if p.bs.is_none() { p.path.set_text(&format!("{}{}", up.join(" › "), if kids > 0 { wb_core::i18n::f("below", &[("kids", (kids).to_string())]) } else { String::new() })); }
        let body = page_for(&mut p);
        let key = format!("{pid}/{}", p.over.as_ref().map(|o| o.1.clone()).unwrap_or_else(|| p.tab.clone()));
        let html = format!("<!doctype html><meta charset='utf-8'><style>{}</style><body>{body}</body>", style(&p.d));
        let mut tabs = Vec::new();
        if p.bs.is_some() { tabs.push((SESSIONS.to_string(), wb_core::i18n::t("sessions").to_string())); }
        tabs.push((MAP.to_string(), wb_core::i18n::t("project-ideas").to_string()));
        tabs.extend(shown_docs(&project).iter().map(|d| (s(d, "id").to_string(), s(d, "name").to_string())));
        tabs.push((RECENT.to_string(), wb_core::i18n::t("recent-changes").to_string()));
        (html, key, p.web.clone(), tabs)
    };
    set_tabs(s_, tabs);
    show_talks(&s_.borrow());
    if s_.borrow().shown == page { return; }
    // Where the page stands now, then the new page at the place its key last stood.
    let s2 = s_.clone();
    let drawn_for = { let p = s_.borrow(); (p.pid.clone(), p.bs.clone()) };
    web.evaluate_javascript("window.scrollY", None, None, None::<&gtk::gio::Cancellable>, move |r| {
        let mut p = s2.borrow_mut();
        // Another tab or page since: that one's own drawing follows.
        let now = format!("{}/{}", p.pid.clone().unwrap_or_default(), p.over.as_ref().map(|o| o.1.clone()).unwrap_or_else(|| p.tab.clone()));
        if now != key { return; }
        // Another project, or this one locked, since it was drawn: its page is not put back over that.
        if (p.pid.clone(), p.bs.clone()) != drawn_for || p.bs.as_ref().and_then(|id| stub_of(&p.d, id)).is_some_and(|st| !st.open()) { return; }
        let was = p.key_now.clone();
        if let Ok(v) = r { p.scroll.insert(was, v.to_double()); }
        let top = p.scroll.get(&key).copied().unwrap_or(0.0);
        p.shown = page.clone();
        p.key_now = key.clone();
        p.web.load_html(&format!("{page}<script>window.scrollTo(0,{top})</script>"), Some("about:blank"));
    });
}

/// The conversations in place of the page, on an open Briefsafe project's Sessions tab; the lines over both.
fn show_talks(p: &P) {
    let open = p.bs.as_ref().and_then(|id| stub_of(&p.d, id)).filter(|st| st.open() && !opening(p, &st.id));
    let on = open.is_some() && p.tab == SESSIONS && p.over.is_none() && !p.tab_buttons.is_empty();
    set_lines(p, open.as_ref());
    (p.list_talks)(open.as_ref().filter(|_| on).map_or("", |st| st.id.as_str()));
    p.talks.set_visible(on);
    p.web.set_visible(!on);
}

/// A new project: a name, and an empty map or one brought in from elsewhere.
fn new_project(s_: &Shared) {
    use crate::mapimport as mi;
    let (d, win) = { let p = s_.borrow(); (p.d.clone(), p.win.clone()) };
    let mut sources: Vec<Option<mi::Source>> = vec![None];
    let mut names = vec![wb_core::i18n::t("empty-map").to_string()];
    for c in mi::exported(&d) { names.push(c.label); sources.push(Some(c.source)); }
    for g in mi::other_sessions(&d) { names.push(g.name.clone()); sources.push(Some(g.whole.source)); }
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let dlg = gtk::Window::builder().title(wb_core::i18n::t("new-project-2")).transient_for(&win).modal(true).decorated(false).build();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    body.set_margin_top(10); body.set_margin_bottom(10); body.set_margin_start(10); body.set_margin_end(10);
    let small = |t: &str| { let l = gtk::Label::new(Some(t)); l.set_xalign(0.0); l };
    body.append(&small(wb_core::i18n::t("project-name")));
    let entry = gtk::Entry::new();
    body.append(&entry);
    body.append(&small(wb_core::i18n::t("project-map-source")));
    let pick = gtk::DropDown::from_strings(&refs);
    body.append(&pick);
    let spring = gtk::Box::new(gtk::Orientation::Vertical, 0);
    spring.set_vexpand(true);
    body.append(&spring);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_halign(gtk::Align::End);
    let (cancel, ok) = (gtk::Button::with_label(wb_core::i18n::t("cancel")), gtk::Button::with_label(wb_core::i18n::t("create")));
    row.append(&cancel); row.append(&ok);
    body.append(&row);
    dlg.set_child(Some(&body));
    { let w = dlg.clone(); cancel.connect_clicked(move |_| w.close()); }
    let make = {
        let (w, s2, e2, p2, d2) = (dlg.clone(), s_.clone(), entry.clone(), pick.clone(), d.clone());
        Rc::new(move || {
            let name = e2.text().trim().to_string();
            if name.is_empty() { return; }
            let source = sources.get(p2.selected() as usize).cloned().flatten();
            // Another step runs: what was typed stays.
            if let Some(j) = &s2.borrow().job { status::say(&d2, &wb_core::i18n::f("briefsafe-busy", &[("what", j.what.clone())]), 5); return; }
            w.close();
            // A Briefsafe project (spec 3.22), made by `whaybench-project new` as a step of its own: its folder, a vault
            // where one can be made, the recovery key shown in the view; the map chosen goes in as a kept map file.
            let result = (|| -> Result<(String, Option<String>), String> {
                let pid = new_project_id(&s2.borrow().host, &name);
                let map = match &source {
                    Some(src) => {
                        // Whole, and only until the project has its own copy.
                        let (nodes, title, _) = mi::read(&d2, src).ok_or_else(|| wb_core::i18n::t("import-failed-no-map").to_string())?;
                        let file = new_map_file(&d2, &pid);
                        let _ = std::fs::create_dir_all(d2.cache().join("project-jobs"));
                        wb_core::paths::write_atomic(&file, json!({"format": "whaybench-map", "title": title, "nodes": nodes}).to_string()).map_err(|e| e.to_string())?;
                        Some(file.to_string_lossy().to_string())
                    }
                    None => None,
                };
                Ok((pid, map))
            })();
            match result {
                Ok((pid, map)) => {
                    record::decide("project", "new project", "the person named it", &[("project", pid.clone()), ("imported_map", source.is_some().to_string())]);
                    let mut args = vec!["new".to_string()];
                    if let Some(m) = map { args.extend(["--map".to_string(), m]); }
                    // The name is the person's, whatever it looks like.
                    args.extend(["--".to_string(), pid.clone(), name.clone()]);
                    let args: Vec<&str> = args.iter().map(String::as_str).collect();
                    status::say(&d2, wb_core::i18n::t("briefsafe-creating"), 60);
                    start_job(&mut s2.borrow_mut(), &pid, "new", wb_core::i18n::t("briefsafe-creating"), &args, None);
                }
                Err(e) => status::say(&d2, &wb_core::i18n::f("creation-failed", &[("e", e)]), 12),
            }
        })
    };
    { let m = make.clone(); ok.connect_clicked(move |_| m()); }
    entry.connect_activate(move |_| make());
    dlg.present();
    entry.grab_focus();
}

fn fill_list(s_: &Shared) {
    let mut p = s_.borrow_mut();
    let mut rows: Vec<(String, String, usize)> = p.host.tree().into_iter().map(|(v, depth)| (s(&v, "id").to_string(), s(&v, "name").to_string(), depth)).collect();
    // Briefsafe projects after the host's: their name and whether they are open, nothing of their content.
    rows.extend(wb_core::roots::stubs(&p.d).into_iter().map(|st| (format!("{BS}{}", st.id), format!("{} · {}", st.name, wb_core::i18n::t(if st.open() { "briefsafe-open" } else { "briefsafe-locked" })), 0)));
    if rows == p.rows { return; }
    p.rows = rows;
    let names: Vec<String> = if p.rows.is_empty() { vec![wb_core::i18n::t("no-project").to_string()] } else { p.rows.iter().map(|(_, n, depth)| format!("{}{}{n}", "   ".repeat(*depth), if *depth > 0 { "└ " } else { "" })).collect() };
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let model = gtk::StringList::new(&refs);
    // The project of the chosen session's folder, or the one already on show, or the first.
    let of_session = sessions::target(&p.d).and_then(|t| sessions::record_of(&p.d, &t)).and_then(|r| p.host.project_for_cwd(s(&r, "cwd"))).map(|v| s(&v, "id").to_string());
    let shown = p.bs.as_ref().map(|id| format!("{BS}{id}")).or(p.pid.clone());
    let want = shown.or(of_session);
    let at_row = want.and_then(|w| p.rows.iter().position(|r| r.0 == w)).unwrap_or(0);
    let key = p.rows.get(at_row).map(|r| r.0.clone());
    // The same project stays as it is (its change page, its message); only another is chosen anew.
    let now = p.bs.as_ref().map(|id| format!("{BS}{id}")).or(p.pid.clone());
    if key != now { choose(&mut p, key); } else { p.stamp.clear(); }
    // Kept borrowed while the list is set: the dropdown's own handler (try_borrow_mut) then leaves the choice alone
    // (setting a list selects its first row on its way).
    let pick = p.pick.clone();
    pick.set_model(Some(&model));
    pick.set_selected(at_row as u32);
    drop(p);
}

fn tick(s_: &Shared) {
    if s_.borrow().bs.is_none() { convert_waiting(&s_.borrow()); }
    fill_list(s_);
    let locked = {
        let mut p = s_.borrow_mut();
        poll_job(&mut p);
        let mut locked = None;
        if let Some(id) = p.bs.clone() {
            match stub_of(&p.d, &id) {
                // Not open (or a step running): its own page, nothing of its content, not even its tabs.
                Some(st) if opening(&p, &st.id) || !st.open() => {
                    p.st = p.host.clone();
                    p.path.set_text("");
                    locked = Some(closed_page(&p, &st));
                }
                // Open: its own store, read in its data the safe way.
                Some(st) => {
                    if p.st.data.root != st.root().root { p.st = Store::new(st.root()); p.stamp.clear(); }
                }
                None => { choose(&mut p, None); }
            }
        }
        locked
    };
    if s_.borrow().ask_keyring.is_some() {
        let s2 = s_.clone();
        glib::idle_add_local_once(move || ask_keyring(&s2));
    }
    s_.borrow().path.set_visible(locked.is_none());
    if let Some(page) = locked {
        set_tabs(s_, Vec::new());
        show_talks(&s_.borrow());
        // A request dialog left over its page goes too: it names the project's sections.
        if let Some(w) = s_.borrow_mut().dialog.take().and_then(|w| w.upgrade()) { w.close(); }
        let mut p = s_.borrow_mut();
        if p.shown != page { p.web.load_html(&page, Some("about:blank")); p.shown = page; }
        return;
    }
    let due = {
        let mut p = s_.borrow_mut();
        match p.pid.clone() {
            None => {
                p.path.set_text(wb_core::i18n::t("no-project"));
                let page = wb_core::i18n::f("no-projectcreate-new-project-add-content", &[("a1", (style(&p.d)).to_string())]);
                if p.shown != page { p.web.load_html(&page, Some("about:blank")); p.shown = page; }
                false
            }
            Some(pid) => {
                // The theme is in the page too: a change of theme draws it again.
                let now = format!("{}{}", stamp_of(&p.st, &pid), style(&p.d));
                if now == p.stamp { false } else { p.stamp = now; true }
            }
        }
    };
    if due { redraw(s_); }
    // Its conversations come and go, and a step's state: shown as they are now.
    show_talks(&s_.borrow());
}

fn open_source(p: &P, o: &Value) {
    // A Briefsafe project's sources are what its agents wrote: only its own files and conversations open.
    if let Some(id) = &p.bs {
        let own_file = s(o, "imported").is_empty() || wb_core::beneath::split(Path::new(s(o, "imported"))).is_some_and(|(r, _)| r == p.st.data.root);
        let sid = s(o, "session");
        let own_session = sid.is_empty() || (wb_core::map::safe_id(sid).is_ok() && wb_core::roots::Roots::new(p.d.clone()).project_of(sid).as_deref() == Some(id.as_str()));
        if !own_file || !own_session { status::say(&p.d, wb_core::i18n::t("briefsafe-outside-project"), 6); return; }
    }
    if !s(o, "imported").is_empty() {
        let path = Path::new(s(o, "imported"));
        if path.is_file() { crate::mapimport::show(&p.d, path, s(o, "title"), Some(s(o, "node"))); }
        else { status::say(&p.d, &wb_core::i18n::f("source-not-opened-no-file", &[("path", path.display().to_string())]), 8); }
        return;
    }
    let (session, node) = (s(o, "session"), s(o, "node"));
    if session.is_empty() { return; }
    // Opening a source is looking at its map, which is a file: the chosen session stays, and no session is woken.
    let sid = sessions::current_id(&p.d, session);
    let path = wb_core::roots::for_session(&p.d, &sid).root.join(format!("{sid}.json"));
    if path.is_file() { crate::mapimport::show(&p.d, &path, &sessions::session_name(&p.d, &sid), Some(node)); }
    else { status::say(&p.d, &wb_core::i18n::f("source-not-opened-no-file", &[("path", path.display().to_string())]), 8); }
}

fn clicked(s_: &Shared, uri: &str) {
    let Some(rest) = uri.strip_prefix("wb:") else { return };
    let (kind, value) = rest.split_once(':').unwrap_or((rest, ""));
    let value = glib::Uri::unescape_string(value, None::<&str>).map(|v| v.to_string()).unwrap_or_else(|| value.to_string());
    // Pressed on a page drawn for another project than the one shown now: nothing.
    let Some((page, value)) = value.split_once('\u{1}') else { return };
    let value = value.to_string();
    if page != shown_key(&s_.borrow()) { return; }
    let (pid, win) = { let p = s_.borrow(); (p.pid.clone().unwrap_or_default(), p.win.clone()) };
    let over = |s_: &Shared, o: Option<(String, String)>| { s_.borrow_mut().over = o; redraw(s_); };
    if kind == "bs" { return briefsafe_clicked(s_, &value); }
    match kind {
        "close" => over(s_, None),
        "range" => { s_.borrow_mut().range = value.parse::<i64>().ok().filter(|d| *d > 0); redraw(s_); }
        "src" => { if let Ok(o) = serde_json::from_str::<Value>(&value) { open_source(&s_.borrow(), &o); } }
        "chg" => {
            if let Some((did, id)) = value.split_once(':') { over(s_, Some((did.to_string(), id.to_string()))); }
        }
        "go" => {
            let Some((did, path)) = value.split_once(':') else { return };
            let (did, path) = (did.to_string(), path.to_string());
            { let mut p = s_.borrow_mut(); p.over = None; p.tab = did.clone(); }
            redraw(s_);
            let s2 = s_.clone();
            // The page has to be there first.
            glib::timeout_add_local_once(Duration::from_millis(350), move || {
                let p = s2.borrow();
                if let Some(a) = p.anchors.get(&did).and_then(|m| m.get(&path)) {
                    p.web.evaluate_javascript(&format!("var e=document.getElementById('{a}'); if(e) e.scrollIntoView();"), None, None, None::<&gtk::gio::Cancellable>, |_| {});
                }
            });
        }
        "req" | "dreq" => {
            let (did, line) = value.split_once(':').unwrap_or((value.as_str(), ""));
            let (did, line) = (did.to_string(), line.to_string());
            let (where_, original) = {
                let p = s_.borrow();
                let project = p.st.get_project(&pid).unwrap_or_default();
                let d = list(&project, "docs").iter().find(|x| s(x, "id") == did).cloned().unwrap_or_default();
                let name = if s(&d, "name").is_empty() { did.clone() } else { s(&d, "name").to_string() };
                let where_ = if line.is_empty() { format!("{name}({did})") } else {
                    let text = p.st.document(&pid, &did);
                    format!("{name}({did}) > {}", md::path_at(&md::headings(&text), line.parse().unwrap_or(0)).join(" > "))
                };
                (where_, s(&d, "original").to_string())
            };
            // The project the dialog was opened for: sent only if it is still the one shown.
            let bs_then = s_.borrow().bs.clone();
            let (s2, pid2, w2) = (s_.clone(), pid.clone(), where_.clone());
            let dlg = ask_text(&win, wb_core::i18n::t("request-change-2"), &wb_core::i18n::f("request", &[("where_", (where_).to_string())]), move |t| {
                let source = if original.is_empty() { String::new() } else { wb_core::i18n::ai_f("original-of-the-template", &[("original", (original).to_string())]) };
                if s2.borrow().bs != bs_then { return; }
                request(&s2.borrow(), &wb_core::i18n::ai_f("change-as-the-user-requests-check", &[("w2", (w2).to_string()), ("t", (t).to_string()), ("pid2", (pid2).to_string()), ("did", (did).to_string()), ("source", (source).to_string())]));
            });
            s_.borrow_mut().dialog = Some(gtk::prelude::ObjectExt::downgrade(&dlg));
        }
        "dback" | "dredo" => {
            let mut bits = value.splitn(3, ':');
            let (Some(did), Some(id), Some(path)) = (bits.next(), bits.next(), bits.next()) else { return };
            let where_ = format!("{did} > {path}");
            let cmd = format!("`whaybench-project change {pid} {did} {id}`");
            if kind == "dback" {
                request(&s_.borrow(), &wb_core::i18n::ai_f("put-section-back-as-it-was", &[("where_", (where_).to_string()), ("id", (id).to_string()), ("cmd", (cmd).to_string())]));
                over(s_, None);
            } else {
                let (s2, id2) = (s_.clone(), id.to_string());
                let w2 = where_.clone();
                let bs_then = s_.borrow().bs.clone();
                let dlg = ask_text(&win, wb_core::i18n::t("request-rework"), &wb_core::i18n::f("request-based-on-the-earlier-version", &[("where_", (where_).to_string())]), move |t| { let _ = &id2; if s2.borrow().bs != bs_then { return; } request(&s2.borrow(), &wb_core::i18n::ai_f("rework-section-user-request-the-earlier", &[("w2", (w2).to_string()), ("t", (t).to_string()), ("cmd", (cmd).to_string())])); });
                s_.borrow_mut().dialog = Some(gtk::prelude::ObjectExt::downgrade(&dlg));
            }
        }
        "iback" | "iredo" => {
            let mut bits = value.splitn(3, ':');
            let (Some(target), Some(id), Some(k)) = (bits.next(), bits.next(), bits.next()) else { return };
            let it = s_.borrow().st.item_change_detail(&pid, id).unwrap_or_default();
            let Some(c) = list(&it, "changes").get(k.parse::<usize>().unwrap_or(usize::MAX)).cloned() else { return };
            let label = history::label(&c).chars().take(60).collect::<String>();
            let where_ = wb_core::i18n::f("text-4", &[("a1", (history::target_name(target)).to_string()), ("a2", (if s(&c, "kind") == "item" { wb_core::i18n::t("item") } else { s(&c, "kind") }).to_string()), ("a3", (s(&c, "id")).to_string()), ("a4", label.clone())]);
            // The agent reads it in English; the dialog shows it in the language in use.
            let place = wb_core::i18n::ai_f("project-map-place", &[("kind", s(&c, "kind").to_string()), ("id", s(&c, "id").to_string()), ("label", label)]);
            let cmd = format!("`whaybench-project change-items {pid} {id}`");
            if kind == "iback" {
                request(&s_.borrow(), &wb_core::i18n::ai_f("put-back-as-it-was-before", &[("where_", place), ("id", (id).to_string()), ("cmd", (cmd).to_string())]));
                over(s_, None);
            } else {
                let (s2, w2) = (s_.clone(), place);
                let bs_then = s_.borrow().bs.clone();
                let dlg = ask_text(&win, wb_core::i18n::t("request-rework"), &wb_core::i18n::f("request-based-on-the-earlier-version", &[("where_", (where_).to_string())]), move |t| { if s2.borrow().bs != bs_then { return; } request(&s2.borrow(), &wb_core::i18n::ai_f("rework-user-request-the-earlier-state", &[("w2", (w2).to_string()), ("t", (t).to_string()), ("cmd", (cmd).to_string())])); });
                s_.borrow_mut().dialog = Some(gtk::prelude::ObjectExt::downgrade(&dlg));
            }
        }
        _ => {}
    }
}

pub fn build(win: &gtk::ApplicationWindow, d: Data) {
    let st = Store::new(d.clone());
    let host = st.clone();
    let web = webkit6::WebView::new();
    crate::common::set_up_web(&web, &d);
    web.set_vexpand(true);
    web.set_hexpand(true);
    if let Some(settings) = WebViewExt::settings(&web) { settings.set_default_charset("utf-8"); }

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let pick = gtk::DropDown::from_strings(&[wb_core::i18n::t("no-project")]);
    pick.set_hexpand(true);
    let new = gtk::Button::with_label(wb_core::i18n::t("new-project-2"));
    new.set_tooltip_text(Some(wb_core::i18n::t("create-project")));
    top.append(&pick); top.append(&new);
    let path = gtk::Label::new(None);
    path.set_xalign(0.0);
    path.set_wrap(true);
    // A long message wraps in the panel's width; left alone it would ask for its whole length and push the panel wider.
    path.set_max_width_chars(20);
    path.add_css_class("small");
    // Wraps in a narrow panel; a scrollbar here would lie over the tabs and take their presses.
    let tabs = gtk::FlowBox::new();
    tabs.set_selection_mode(gtk::SelectionMode::None);
    tabs.set_column_spacing(2);
    tabs.set_row_spacing(0);
    tabs.set_max_children_per_line(12);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 2);
    body.set_margin_start(4); body.set_margin_end(4); body.set_margin_top(4);
    // An open Briefsafe project's own lines: their links are the panel's, pressed here and nowhere else.
    let line = |small: bool| {
        let l = gtk::Label::new(None);
        l.set_xalign(0.0);
        l.set_wrap(true);
        l.set_max_width_chars(20);
        l.set_visible(false);
        if small { l.add_css_class("small"); }
        l
    };
    let (actions, note, agents) = (line(false), line(true), line(false));
    // What can be done stands at the right, apart from the name and the tabs.
    actions.set_xalign(1.0);
    actions.set_justify(gtk::Justification::Right);
    let (list, list_talks) = crate::sessions::of_project(&d);
    let talks = gtk::ScrolledWindow::new();
    talks.set_child(Some(&list));
    talks.set_vexpand(true);
    talks.set_visible(false);
    body.append(&top); body.append(&path); body.append(&actions); body.append(&note); body.append(&tabs); body.append(&agents); body.append(&talks); body.append(&web);
    win.set_child(Some(&body));

    let s_: Shared = Rc::new(RefCell::new(P {
        d: d.clone(), st, pid: None, rows: vec![(String::new(), String::new(), usize::MAX)], tab: MAP.to_string(), over: None,
        range: Some(7), stamp: String::new(), shown: String::new(), scroll: HashMap::new(), key_now: String::new(), failed: String::new(), web: web.clone(), pick: pick.clone(), path, actions: actions.clone(), note, agents: agents.clone(), talks, list_talks: Rc::new(list_talks), tabs, tab_buttons: Vec::new(), anchors: HashMap::new(), tab_keys: Vec::new(), converting: Rc::default(), win: win.clone(),
        host, unreadable: false, ask_keyring: None, after_unlock: None, keyring_anew: false, bs: None, job: None, bs_msg: HashMap::new(), dialog: None, close_after: Vec::new(),
    }));
    for l in [&actions, &agents] {
        let s2 = s_.clone();
        l.connect_activate_link(move |_, what| {
            let (s3, what) = (s2.clone(), what.to_string());
            glib::idle_add_local_once(move || briefsafe_clicked(&s3, &what));
            glib::Propagation::Stop
        });
    }
    // Links are handled here; only the page's own anchors and the drawn page itself pass.
    {
        let s2 = s_.clone();
        web.connect_decide_policy(move |_, decision, kind| {
            let Some(uri) = crate::web::pressed_link(decision, kind) else { return false };
            // A Briefsafe project's page is its agents' text: what it asks for by itself (a script, a refresh, a frame)
            // does nothing, only the person's press does (spec 3.22).
            let project_page = s2.borrow().bs.is_some();
            if project_page && !uri.starts_with("about:") && !crate::web::by_person(decision) { decision.ignore(); return true; }
            if uri.starts_with("wb:") {
                decision.ignore();
                let s3 = s2.clone();
                glib::idle_add_local_once(move || clicked(&s3, &uri));
                return true;
            }
            if uri.starts_with("http") { decision.ignore(); let _ = wb_core::view::open(&s2.borrow().d, &uri, None); return true; }
            // A Briefsafe project's page goes nowhere else (a file, another scheme): its anchors stay on the page.
            if s2.borrow().bs.is_some() && !uri.starts_with("about:") { decision.ignore(); return true; }
            false
        });
    }
    {
        let s2 = s_.clone();
        pick.connect_selected_notify(move |dd| {
            let Ok(mut p) = s2.try_borrow_mut() else { return };
            let chosen = p.rows.get(dd.selected() as usize).map(|r| r.0.clone()).filter(|x| !x.is_empty());
            let shown = p.bs.as_ref().map(|id| format!("{BS}{id}")).or(p.pid.clone());
            if chosen != shown {
                choose(&mut p, chosen);
                // Its page at once: the one before must not stay to be pressed under the new name.
                drop(p);
                let s3 = s2.clone();
                glib::idle_add_local_once(move || tick(&s3));
            }
        });
    }
    { let s2 = s_.clone(); new.connect_clicked(move |_| new_project(&s2)); }
    let run = move || {
        // A bad file must not stop the panel for good: the next tick tries again, and the reason is on the path line.
        let s3 = s_.clone();
        let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tick(&s3)));
        if let Err(cause) = done {
            let msg = cause.downcast_ref::<String>().cloned().or_else(|| cause.downcast_ref::<&str>().map(|x| x.to_string())).unwrap_or_else(|| wb_core::i18n::t("unknown-error").into());
            if let Ok(mut p) = s_.try_borrow_mut() {
                if p.failed != msg { p.failed = msg.clone(); record::error("project", "project view not refreshed", &[("error", msg.clone())]); }
                p.path.set_text(&wb_core::i18n::f("project-view-update-failed-retrying-in", &[("a1", (msg.chars().take(80).collect::<String>()).to_string())]));
            }
        }
    };
    crate::common::every(2000, run);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_new_project_gets_a_folder_name_not_in_use() {
        let dir = std::env::temp_dir().join(format!("wb-pp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let st = Store::new(Data::new(dir.clone()));
        assert_eq!(new_project_id(&st, "My App 2"), "my-app-2");
        assert!(new_project_id(&st, "ÄÖ").starts_with("p-"));
        st.create_project("my-app", "My App", "", "", Vec::new(), false).unwrap();
        assert_eq!(new_project_id(&st, "My App"), "my-app-2", "taken names go on");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
