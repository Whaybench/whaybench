//! What every panel shares: the data directory, the theme as a style sheet, the program's own commands.
use std::{cell::RefCell, path::PathBuf, process::{Command, Stdio}, rc::Rc, sync::OnceLock, time::Duration};

use gtk::{gdk, gio, glib, prelude::*};
use webkit6::prelude::WebViewExt;
use serde_json::{Value, json};
use wb_core::{paths::{self, Data}, theme::{self, Colour, Theme}};

/// This panel's app id, under which its zoom is kept. Unset for the menu bar and the status line: their height is fixed.
static PANEL: OnceLock<String> = OnceLock::new();

pub fn set_panel(app_id: &str) { let _ = PANEL.set(app_id.to_string()); }

fn zoom_file(d: &Data) -> PathBuf { d.root.join("panel-zoom.json") }

/// This panel's zoom, 1.0 when none was set. Ctrl+wheel changes it; it is kept per panel over restarts.
pub fn zoom(d: &Data) -> f64 {
    let Some(key) = PANEL.get() else { return 1.0 };
    std::fs::read_to_string(zoom_file(d)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok())
        // Up to 0.3.1 the notifications panel was called alerts.
        .and_then(|v| v.get(key).or_else(|| v.get(key.replace("dev.whaybench.notify", "dev.whaybench.alerts"))).and_then(Value::as_f64)).unwrap_or(1.0).clamp(0.5, 3.0)
}

fn set_zoom(d: &Data, z: f64) {
    let Some(key) = PANEL.get() else { return };
    let mut v = std::fs::read_to_string(zoom_file(d)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}));
    v[key.as_str()] = json!(z);
    let _ = wb_core::paths::write_atomic(&zoom_file(d), v.to_string());
}

/// Ctrl+wheel anywhere in the window makes this panel larger or smaller, in steps of 10%.
pub fn zoom_by_wheel(win: &gtk::ApplicationWindow, d: &Data) {
    if PANEL.get().is_none() { return; }
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
    let d = d.clone();
    scroll.connect_scroll(move |c, _, dy| {
        if !c.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) || dy == 0.0 { return glib::Propagation::Proceed; }
        let step = if dy < 0.0 { 0.1 } else { -0.1 };
        set_zoom(&d, ((zoom(&d) + step) * 10.0).round() / 10.0);
        glib::Propagation::Stop
    });
    win.add_controller(scroll);
}

/// A panel's web view: its menu in the chosen language, no background of its own (the page's body paints it, or leaves
/// it to a see-through panel), and the panel's zoom.
pub fn set_up_web(web: &webkit6::WebView, d: &Data) {
    web_menu_in_language(web);
    web.set_background_color(&gdk::RGBA::TRANSPARENT);
    follow_zoom(web, d);
}

thread_local! {
    /// The folders watched for this panel, by what they are watched for: a new watch of the same key replaces the old.
    static WATCHES: RefCell<std::collections::HashMap<String, Vec<gio::FileMonitor>>> = RefCell::default();
}

/// Calls `f` when a file in one of `folders` is written, made, removed or moved there, as the system tells (inotify):
/// only the names given, or any with none. Kept under `key`; a new call with the same key replaces the watch (a panel
/// that now follows another session watches that one's folder). Nothing is looked for on a clock.
pub fn on_change(key: &str, folders: &[(std::path::PathBuf, &[&str])], f: Rc<dyn Fn()>) { watch(key, folders, 40, true, f) }

/// `on_change` for folders that are someone else's (a program's settings): one not there is not made.
pub fn on_change_existing(key: &str, folders: &[(std::path::PathBuf, &[&str])], f: Rc<dyn Fn()>) { watch(key, folders, 40, false, f) }

/// `on_change`, with the changes of `ms` after the first taken as one. A folder not there (and not made, unless `make`)
/// is waited for: its nearest folder that is there is watched for that one name only, and once it appears the watch is
/// made again, on it alone. Nothing more is watched than what is read.
fn watch(key: &str, folders: &[(std::path::PathBuf, &[&str])], ms: u64, make: bool, f: Rc<dyn Fn()>) {
    // Several changes at once (a write and a rename, a burst from an agent) are one call, a moment after the first.
    let run = once_soon(ms, f.clone());
    let owned: Rc<Vec<(std::path::PathBuf, Vec<String>)>> = Rc::new(folders.iter().map(|(d, n)| (d.clone(), n.iter().map(|x| x.to_string()).collect())).collect());
    let again = { let (key, owned, f) = (key.to_string(), owned.clone(), f.clone()); move || {
        let folders: Vec<(std::path::PathBuf, Vec<&str>)> = owned.iter().map(|(d, n)| (d.clone(), n.iter().map(String::as_str).collect())).collect();
        let refs: Vec<(std::path::PathBuf, &[&str])> = folders.iter().map(|(d, n)| (d.clone(), n.as_slice())).collect();
        watch(&key, &refs, ms, make, f.clone());
    } };
    let again: Rc<dyn Fn()> = Rc::new(again);
    let mut monitors = Vec::new();
    for (dir, names) in folders {
        if make { let _ = std::fs::create_dir_all(dir); }
        if !dir.is_dir() {
            // Waited for at the nearest folder that is there, by the one name on the way to it.
            let Some((at, name)) = dir.ancestors().skip(1).find(|a| a.is_dir()).and_then(|a| Some((a.to_path_buf(), dir.strip_prefix(a).ok()?.components().next()?.as_os_str().to_os_string()))) else { continue };
            let Ok(m) = gio::File::for_path(&at).monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, None::<&gio::Cancellable>) else { continue };
            let (again, run) = (again.clone(), run.clone());
            m.connect_changed(move |_, file, other, _| {
                let named = |x: Option<&gio::File>| x.and_then(|x| x.basename()).is_some_and(|b| b.as_os_str() == name);
                if named(Some(file)) || named(other) {
                    // Made again outside this call: the monitor running it is replaced.
                    let again = again.clone();
                    glib::idle_add_local_once(move || again());
                    run();
                }
            });
            monitors.push(m);
            continue;
        }
        let Ok(m) = gio::File::for_path(dir).monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, None::<&gio::Cancellable>) else { continue };
        // A name given as "+name" is news only when written into (a database's log, which its readers open and close
        // for writing too, without writing): nothing else of it.
        let written: Vec<String> = names.iter().filter_map(|n| n.strip_prefix('+')).map(String::from).collect();
        if !written.is_empty() { m.set_rate_limit(100); }
        let (names, f) = (names.iter().filter(|n| !n.starts_with('+')).map(|n| n.to_string()).collect::<Vec<_>>(), run.clone());
        // The folder itself removed or replaced: watched again (waited for, if it is gone).
        let (gone_again, dir2) = (again.clone(), dir.clone());
        m.connect_changed(move |_, file, other, event| {
            use gio::FileMonitorEvent as E;
            if matches!(event, E::Deleted | E::MovedOut | E::Unmounted) && file.path().as_deref() == Some(dir2.as_path()) {
                let again = gone_again.clone();
                glib::idle_add_local_once(move || again());
                f();
                return;
            }
            if !written.is_empty() {
                if event == E::Changed && file.basename().is_some_and(|b| written.iter().any(|n| b.as_os_str() == n.as_str())) { f(); }
                return;
            }
            // A file is read once written (ChangesDoneHint), not as it is made and still empty; a folder or a link made
            // is news at once (a new theme's; a project opening makes a link in cache/open).
            let made_folder = event == E::Created && matches!(file.query_file_type(gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, None::<&gio::Cancellable>), gio::FileType::Directory | gio::FileType::SymbolicLink);
            if !made_folder && !matches!(event, E::ChangesDoneHint | E::Deleted | E::MovedIn | E::MovedOut | E::Renamed) { return; }
            let named = |x: Option<&gio::File>| x.and_then(|x| x.basename()).is_some_and(|b| names.is_empty() || names.iter().any(|n| b.as_os_str() == n.as_str()));
            if named(Some(file)) || named(other) { f(); }
        });
        monitors.push(m);
    }
    WATCHES.with(|w| { w.borrow_mut().insert(key.to_string(), monitors); });
}

/// The folders a panel's lists are made from: the add-ons, where projects open, and each root (the host's and each open
/// project's) with its own files, its cache, the agents' states, the session records, the asks, the usage and the
/// update. Changes anywhere there are few and small; a panel reads its lists again, and draws only what changed.
pub fn data_folders(d: &Data) -> Vec<(std::path::PathBuf, &'static [&'static str])> {
    let mut f: Vec<(std::path::PathBuf, &'static [&'static str])> = vec![(d.root.join("addons/themes"), &[]),
        (d.root.join("addons/map-styles"), &[]), (d.root.join("addons/languages"), &[]), (d.root.join("addons/agents"), &[]),
        (d.root.join("addons"), &["themes-default-settings.json", "addon-trust.json"]), (d.cache().join("open"), &[]),
        // Briefsafe projects' names and places.
        (d.root.join("projects"), &[])];
    for r in wb_core::roots::all_of(d) {
        // Each root's own files (maps, names, focus, settings), and in its cache only the files read there: the status
        // message, the running program, the release found, the browser chosen. Not the add-on agents' heartbeats
        // (cache/alive, written every two seconds): a session's start shows in its tab and state, its end by the time
        // its heartbeat runs out.
        f.push((r.root.clone(), &[]));
        f.push((r.cache(), &["status.json", "instance.json", "latest-release.json", "chosen-browser"]));
        for p in [r.cache().join("state"), r.root.join("sessions"), r.cache().join("ask"), r.cache().join("panels"),
                  r.cache().join("update"), r.root.join("files"), r.cache().join("term"), r.cache().join("subagents"), r.root.join("node-files"),
                  r.root.join("forks"), r.cache().join("schedules"), r.root.join("answer-log"), r.root.join("answers")] { f.push((p, &[])); }
        // The memory: only a write into its database's log is a change (reading it opens and closes files there).
        f.push((r.root.join("memory"), &["+memory.db-wal"]));
        f.extend(wb_core::paths::list_dir(&r.cache().join("subagents")).into_iter().filter(|p| p.is_dir()).map(|p| (p, &[][..])));
    }
    // Each theme's, language's and agent's own folder: what is in them is read (a theme's settings, a language's
    // messages, an agent's description).
    for kind in ["themes", "languages", "agents"] {
        f.extend(paths::list_dir(&d.root.join("addons").join(kind)).into_iter().filter(|p| p.is_dir()).map(|p| (p, &[][..])));
    }
    f
}

/// `f` once, `ms` after the first of the calls that come meanwhile: never twice in waiting.
pub fn once_soon(ms: u64, f: Rc<dyn Fn()>) -> Rc<dyn Fn()> {
    let due = Rc::new(std::cell::Cell::new(false));
    Rc::new(move || {
        if due.replace(true) { return; }
        let (due, f) = (due.clone(), f.clone());
        glib::timeout_add_local_once(Duration::from_millis(ms), move || { due.set(false); f(); });
    })
}

thread_local! {
    /// The asks whose end is already waited for (`follow_asks`), and what to call when one ends.
    static ASKS_WAITED: RefCell<std::collections::HashSet<String>> = RefCell::default();
    static ASK_ENDED: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

/// An ask's card goes when its hook's process ends (a tab closed, the agent stopped) or its time runs out, though no
/// file changes then: `f` is called at each. The process's end is told by the system (a pidfd), the time by a timer set
/// once; a project's hook runs where its process cannot be seen here, so its time alone.
pub fn follow_asks(d: &Data, f: &Rc<dyn Fn()>) {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    for a in wb_core::ask::pending_all(d) {
        let Some(id) = a["id"].as_str().map(String::from) else { continue };
        // A question held back for its answer by prompt has no process waiting, nor a time.
        if a["via"] == "prompt" || !ASKS_WAITED.with(|w| w.borrow_mut().insert(id)) { continue; }
        if let Some(at) = a["at"].as_i64() {
            let left = (at + wb_core::ask::WAIT_MS + 10_000 - wb_core::clock::now_ms()).max(0) as u64 + 100;
            let f = f.clone();
            glib::timeout_add_local_once(Duration::from_millis(left), move || f());
        }
        // A project's hook may run in a container: its number means nothing here, its time alone ends it.
        if wb_core::ask::root_of(d, a["id"].as_str().unwrap_or("")).is_some_and(|r| r != d.root) { continue; }
        let Some(pid) = a["pid"].as_i64().filter(|p| std::path::Path::new(&format!("/proc/{p}")).exists()) else { continue };
        // Safety: pidfd_open on a process number; the descriptor it gives is ours alone.
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
        if raw < 0 { continue; }
        // Safety: a descriptor just made, owned here.
        let fd = unsafe { OwnedFd::from_raw_fd(raw as i32) };
        ASK_ENDED.with(|e| *e.borrow_mut() = Some(f.clone()));
        // Waited for off this thread, asleep in poll until the process ends (no clock), then told here.
        std::thread::spawn(move || {
            let mut p = libc::pollfd { fd: fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            // Safety: one pollfd of our own; -1 waits without a time.
            while unsafe { libc::poll(&mut p, 1, -1) } < 0 {}
            glib::MainContext::default().invoke(|| { if let Some(f) = ASK_ENDED.with(|e| e.borrow().clone()) { f(); } });
        });
    }
}

/// `f`, and after it the asks it shows waited for (`follow_asks`), calling this again when one ends.
pub fn with_asks(d: &Data, f: Rc<dyn Fn()>) -> Rc<dyn Fn()> {
    let me: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::default();
    let (me2, d) = (me.clone(), d.clone());
    let g: Rc<dyn Fn()> = Rc::new(move || {
        f();
        if let Some(g) = me2.borrow().clone() { follow_asks(&d, &g); }
    });
    *me.borrow_mut() = Some(g.clone());
    g
}

/// A look again at a time: what to call, and the one timer waiting for it.
pub type Wake = Rc<RefCell<(Option<Rc<dyn Fn()>>, Option<glib::SourceId>)>>;

/// Calls the wake's function `after` from now, in place of a call already waiting: one timer, never a pile of them.
pub fn wake_at(w: &Wake, after: Duration) {
    let mut b = w.borrow_mut();
    let Some(f) = b.0.clone() else { return };
    if let Some(old) = b.1.take() { old.remove(); }
    let w2 = w.clone();
    b.1 = Some(glib::timeout_add_local_once(after, move || { w2.borrow_mut().1 = None; f(); }));
}

thread_local! {
    /// What each panel of this process draws again when Claude Code's list of running sessions changes.
    static ON_AGENTS: RefCell<Vec<Rc<dyn Fn()>>> = const { RefCell::new(Vec::new()) };
}

/// Claude Code's running sessions looked at off this thread (`watch_agents`), and the panels told when they change.
pub fn follow_agents() {
    wb_core::sessions::watch_agents(|| { glib::MainContext::default().invoke(|| ON_AGENTS.with(|o| for f in o.borrow().iter() { f() })); });
}

/// Runs `f` now and whenever the data a panel shows changes (`data_folders` and the panel's own `extra`, asked again
/// each time), as the system tells; watched again when projects open or close or a watched folder appears or goes.
pub type Extra = fn(&Data) -> Vec<(std::path::PathBuf, &'static [&'static str])>;
/// Changes within `ms` of the first are taken as one: 250 ms, or less for what the person waits on.
pub fn follow_data(key: &str, d: &Data, ms: u64, extra: Extra, f: Rc<dyn Fn()>) {
    // Watched first, then read: a change while it reads is not missed. Watched again after: the first read may name
    // more to watch (the files panel's files).
    ON_AGENTS.with(|o| o.borrow_mut().push(once_soon(ms, f.clone())));
    arm_data(key.to_string(), d.clone(), extra, ms, f.clone());
    f();
    arm_data(key.to_string(), d.clone(), extra, ms, f);
}

fn arm_data(key: String, d: Data, extra: Extra, ms: u64, f: Rc<dyn Fn()>) {
    let all = move |d: &Data| { let mut v = data_folders(d); v.extend(extra(d)); v };
    let seen_of = |fs: &[(std::path::PathBuf, &[&str])]| fs.iter().map(|(p, _)| (p.clone(), p.is_dir())).collect::<Vec<_>>();
    let folders = all(&d);
    let seen = seen_of(&folders);
    let (k2, d2) = (key.clone(), d.clone());
    // The folders compared after `f` ran: what it drew may name new ones (the files panel's). Folders not there are not
    // made (a project closing meanwhile would find one in its way): a parent is watched.
    let run = once_soon(ms, Rc::new(move || {
        f();
        // New folders watched, then read once more: what was written in them before the watch is not missed.
        if seen_of(&all(&d2)) != seen { arm_data(k2.clone(), d2.clone(), extra, ms, f.clone()); f(); }
    }));
    watch(&key, &folders, 0, false, run);
}

/// The files a panel's look comes from: the file naming the theme in force, the panels' zoom, the default theme's
/// settings, and the theme's own folder (theme.json, settings.json).
pub fn theme_files(d: &Data, theme: &str) -> Vec<(std::path::PathBuf, &'static [&'static str])> {
    let mut f: Vec<(std::path::PathBuf, &'static [&'static str])> = vec![(d.root.clone(), &["theme.json", "panel-zoom.json"]), (d.root.join("addons"), &["themes-default-settings.json"])];
    // The default theme has no folder of its own (its settings are the file above): none is made to watch.
    if theme != theme::DEFAULT { f.push((d.root.join("addons/themes").join(theme), &[])); }
    f
}

/// A web view in the panel follows the panel's zoom, until it is gone: set again when the zoom file changes.
pub fn follow_zoom(web: &webkit6::WebView, d: &Data) {
    let (weak, root, d) = (web.downgrade(), d.root.clone(), d.clone());
    let key = format!("zoom {:p}", web.as_ptr());
    let apply = move || {
        let Some(web) = weak.upgrade() else { return };
        let z = zoom(&d);
        if (web.zoom_level() - z).abs() > 0.001 { web.set_zoom_level(z); }
    };
    apply();
    on_change(&key, &[(root, &["panel-zoom.json"])], Rc::new(apply));
}

pub fn data() -> Data {
    let root = paths::resolve(None, |k| std::env::var(k).ok()).unwrap_or_else(|| PathBuf::from("."));
    Data::new(root)
}

fn css_colour(c: Colour) -> String { format!("rgba({},{},{},{})", (c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, c[3]) }
fn ink(c: [u8; 3], a: f32) -> String { format!("rgba({},{},{},{a})", c[0], c[1], c[2]) }

pub fn style_sheet(t: &Theme) -> String {
    let (bg, text, soft, accent, line) = (css_colour(t.panel), ink(t.ink, 1.0), ink(t.ink, 0.47), css_colour(t.accent), css_colour(t.border));
    let bar = css_colour(t.bar_bottom);
    // See-through panels: the window and its lists leave the background to the panel's fill, so only that fades.
    let fill = if t.see_through() { "transparent".to_string() } else { bg.clone() };
    let field = if t.see_through() { "transparent".to_string() } else { bar.clone() };
    let (font, size, mono) = (t.font.replace('\'', ""), t.font_size, t.terminal_font.replace('\'', ""));
    let (small, h1, h2, h3) = (size - 1.0, size + 5.0, size + 3.0, size + 1.0);
    format!("
window, box, list, row, scrolledwindow, viewport, label, overlay {{ background: transparent; background-color: transparent; }}
list {{ color: {text}; }}
window {{ background: {fill}; color: {text}; font-family: '{font}'; font-size: {size}pt; }}
label {{ color: {text}; }}
.dim {{ color: {soft}; }}
.now {{ font-weight: bold; }}
.small {{ font-size: {small}pt; color: {soft}; }}
row {{ padding: 2px 6px; border-radius: 6px; }}
row:hover {{ background: alpha({accent}, 0.10); }}
row:selected, row.chosen {{ background: alpha({accent}, 0.22); color: {text}; }}
entry {{ background: {bar}; color: {text}; caret-color: {text}; }}
textview, textview text {{ background: {field}; color: {text}; caret-color: {text}; }}
entry {{ border: 1px solid {line}; border-radius: 6px; padding: 2px 6px; min-height: 22px; }}
spinbutton {{ background: {bar}; color: {text}; border: 1px solid {line}; border-radius: 6px; }}
spinbutton text {{ background: transparent; color: {text}; caret-color: {text}; }}
spinbutton button {{ background: transparent; border: none; color: {text}; min-height: 0; padding: 0 6px; }}
button {{ background: {bar}; color: {text}; border: 1px solid {line}; border-radius: 6px; padding: 2px 10px; min-height: 22px; }}
button:hover {{ background: alpha({accent}, 0.18); }}
button:disabled {{ color: {soft}; }}
button.flat {{ background: transparent; border: none; padding: 0 4px; }}
button.flat.chosen {{ background: alpha({accent}, 0.22); font-weight: bold; }}
/* The chosen one of a row of tabs (a stack switcher), drawn like a chosen flat button. */
stackswitcher > button:checked {{ background: alpha({accent}, 0.22); font-weight: bold; }}
flowboxchild {{ padding: 0; min-height: 0; }}
.code {{ font-family: '{mono}', monospace; background: {bar}; border-radius: 6px; padding: 6px 8px; }}
.cell {{ padding: 3px 8px; border: 1px solid {line}; }}
.head {{ font-weight: bold; background: {bar}; }}
.h1 {{ font-size: {h1}pt; font-weight: bold; }} .h2 {{ font-size: {h2}pt; font-weight: bold; }} .h3 {{ font-size: {h3}pt; font-weight: bold; }}
.status {{ font-size: {small}pt; padding: 0 8px; background-color: rgba(0,0,0,0.01); }}
listview, gridview, columnview, treeview, .view {{ background: {fill}; color: {text}; }}
window.dialog, dialog {{ background: {bg}; color: {text}; }}
listview > row, gridview > child {{ color: {text}; }}
headerbar {{ background: {bar}; color: {text}; border-bottom: 1px solid {line}; box-shadow: none; }}
headerbar label, headerbar .title {{ color: {text}; }}
popover contents {{ background: {bg}; color: {text}; }}
/* GTK's own tooltip is dark; the label rule above gave it the theme's text, which only a dark theme could read. */
tooltip, tooltip.background {{ background: {bar}; color: {text}; border: 1px solid {line}; }} tooltip label {{ color: {text}; }}
menubar {{ background: {bar}; color: {text}; min-height: 28px; font-size: {size}pt; }}
menubar > item {{ padding: 2px 10px; color: {text}; }}
menubar > item:hover, menubar > item:selected {{ background: alpha({accent}, 0.22); color: {text}; }}
check {{ background: {bar}; color: {text}; border: 1px solid {line}; }}
check:checked {{ background: alpha({accent}, 0.30); color: {text}; }}
popover scrollbar {{ opacity: 0; }}
/* A menu's scroll area is never shorter than its scroll bar: a one-item menu got a strip of nothing under the item. */
popover scrollbar, popover scrollbar range, popover scrollbar trough, popover scrollbar slider {{ min-height: 0; min-width: 0; }}
modelbutton {{ color: {text}; }} modelbutton:hover {{ background: alpha({accent}, 0.18); }}
")
}

/// Keeps the panel in the chosen theme: a change to its files takes effect as soon as the system tells of it.
pub fn follow_theme(d: &Data) {
    let provider = gtk::CssProvider::new();
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    let shown: Rc<RefCell<String>> = Rc::default();
    let d = d.clone();
    // The theme's own folder is watched as well: when another theme comes in force, its folder instead.
    let watching: Rc<RefCell<String>> = Rc::default();
    let again: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::default();
    let again2 = again.clone();
    let apply = move || {
        let mut t = theme::load_active(&d);
        if *watching.borrow() != t.name {
            *watching.borrow_mut() = t.name.clone();
            if let Some(f) = again2.borrow().clone() { on_change("theme", &theme_files(&d, &t.name), f); }
        }
        t.font_size *= zoom(&d);
        // GTK's own widgets, a font chooser for one, start from its light or dark look: the one that fits the theme.
        let dark = 0.2126 * t.panel[0] + 0.7152 * t.panel[1] + 0.0722 * t.panel[2] < 0.5;
        if let Some(s) = gtk::Settings::default() {
            // GTK 4.20 and later: set by name, so the program still builds and runs with an older GTK.
            const SCHEME: &str = "gtk-interface-color-scheme";
            let want = if dark { "dark" } else { "light" };
            if let Some(class) = s.find_property(SCHEME).and_then(|p| glib::EnumClass::with_type(p.value_type())) {
                let now = s.property_value(SCHEME);
                if glib::EnumValue::from_value(&now).map(|(_, v)| v.nick()) != Some(want)
                    && let Some(v) = class.to_value_by_nick(want) { s.set_property_from_value(SCHEME, &v); }
            }
            // The desktop's settings.ini may still ask for the dark look the old way; both have to agree, or GTK's own
            // parts (a menu now and then) come out dark.
            #[allow(deprecated)]
            if s.is_gtk_application_prefer_dark_theme() != dark { s.set_gtk_application_prefer_dark_theme(dark); }
            // GTK's own theme, never the desktop's: a desktop GTK theme (Breeze on KDE) styles menus its own way, and a
            // menu now and then came out in its colours with a frame around it.
            if s.gtk_theme_name().as_deref() != Some("Default") { s.set_gtk_theme_name(Some("Default")); }
            // Glyphs fitted to the pixel grid: left to GTK, small Hangul lost the bottom stroke of a final consonant
            // (rieul at 10 pt) while a round one (ieung) kept it. GTK 4.16 and later: set by name.
            const RENDERING: &str = "gtk-font-rendering";
            if let Some(class) = s.find_property(RENDERING).and_then(|p| glib::EnumClass::with_type(p.value_type()))
                && glib::EnumValue::from_value(&s.property_value(RENDERING)).map(|(_, v)| v.nick()) != Some("manual")
                && let Some(v) = class.to_value_by_nick("manual") { s.set_property_from_value(RENDERING, &v); }
            if s.gtk_xft_antialias() != 1 { s.set_gtk_xft_antialias(1); }
            if s.gtk_xft_hinting() != 1 { s.set_gtk_xft_hinting(1); }
            if s.gtk_xft_hintstyle().as_deref() != Some("hintslight") { s.set_gtk_xft_hintstyle(Some("hintslight")); }
        }
        let css = style_sheet(&t);
        if *shown.borrow() != css {
            provider.load_from_string(&css);
            *shown.borrow_mut() = css;
        }
    };
    let apply: Rc<dyn Fn()> = Rc::new(apply);
    *again.borrow_mut() = Some(apply.clone());
    // Read when the theme's files change (on_change), not on a clock.
    apply();
}

fn sibling(name: &str) -> PathBuf {
    std::env::current_exe().ok().map(|e| e.with_file_name(name)).filter(|p| p.exists()).unwrap_or_else(|| name.into())
}

/// Runs `whaybench ctl ...` away from the window, because it can take seconds; what it reports goes to the status line.
pub fn ctl(d: &Data, args: Vec<String>) {
    let d = d.clone();
    std::thread::spawn(move || {
        let out = Command::new(sibling("whaybench")).arg("ctl").args(&args).stdin(Stdio::null()).output();
        if let Ok(o) = out {
            let said = String::from_utf8_lossy(if o.status.success() { &o.stdout } else { &o.stderr }).trim().trim_start_matches("whaybench: ").to_string();
            if !said.is_empty() { wb_core::status::say(&d, &said, 8); }
        }
    });
}

/// In a Markdown table a `|` inside backticks would end the cell: the documents write commands with them.
pub fn table_pipes(text: &str) -> String {
    text.split('\n').map(|line| {
        if !line.trim_start().starts_with('|') { return line.to_string(); }
        let (mut out, mut code, mut last) = (String::with_capacity(line.len()), false, '\0');
        for c in line.chars() {
            if c == '`' { code = !code; }
            if c == '|' && code && last != '\\' { out.push('\\'); }
            out.push(c);
            last = c;
        }
        out
    }).collect::<Vec<_>>().join("\n")
}

pub fn ago(ms: i64) -> String {
    let minutes = (wb_core::clock::now_ms() - ms) / 60_000;
    if minutes < 1 { wb_core::i18n::t("just-now").into() } else if minutes < 60 { wb_core::i18n::f("min-ago", &[("minutes", (minutes).to_string())]) } else if minutes < 1440 { wb_core::i18n::f("h-ago", &[("a1", (minutes / 60).to_string())]) } else { wb_core::i18n::f("d-ago", &[("a1", (minutes / 1440).to_string())]) }
}

pub fn cut(t: &str, n: usize) -> String { t.chars().take(n).collect() }

/// WebKit's own right-click menu in the program's language: its Korean leaves Back, Forward and Stop in English.
pub fn web_menu_in_language(web: &webkit6::WebView) {
    use webkit6::{ContextMenuAction as A, ContextMenuItem, prelude::*};
    web.connect_context_menu(|_, menu, _| {
        for (i, item) in menu.items().into_iter().enumerate() {
            let key = match item.stock_action() {
                A::GoBack => "web-back", A::GoForward => "web-forward", A::Stop => "web-stop", A::Reload => "web-reload",
                A::Copy => "web-copy", A::Cut => "web-cut", A::Paste => "web-paste", A::SelectAll => "web-select-all",
                A::OpenLink => "web-open-link", A::CopyLinkToClipboard => "web-copy-link",
                A::CopyImageToClipboard => "web-copy-image", A::CopyImageUrlToClipboard => "web-copy-image-address",
                _ => continue,
            };
            menu.remove(&item);
            menu.insert(&ContextMenuItem::from_stock_action_with_label(item.stock_action(), wb_core::i18n::t(key)), i as i32);
        }
        false
    });
}


/// Asks once before a step: the text, Cancel, and the button `ok` that takes it.
/// `lines` are shown under the text, to read before deciding: what goes with it.
pub fn confirm(win: &gtk::ApplicationWindow, text: &str, lines: &[String], ok: &str, then: impl Fn() + 'static) {
    let dlg = gtk::Window::builder().title(ok).transient_for(win).modal(true).decorated(false).build();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    body.set_margin_top(10); body.set_margin_bottom(10); body.set_margin_start(10); body.set_margin_end(10);
    let say = gtk::Label::new(Some(text));
    say.set_xalign(0.0); say.set_wrap(true); say.set_max_width_chars(28);
    body.append(&say);
    if !lines.is_empty() {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        for l in lines {
            let line = gtk::Label::new(Some(l));
            line.set_xalign(0.0); line.set_wrap(true); line.set_wrap_mode(gtk::pango::WrapMode::WordChar); line.add_css_class("dim");
            list.append(&line);
        }
        body.append(&page_scroll(&list));
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_halign(gtk::Align::End);
    let (cancel, ok) = (gtk::Button::with_label(wb_core::i18n::t("cancel")), gtk::Button::with_label(ok));
    row.append(&cancel); row.append(&ok);
    body.append(&row);
    dlg.set_child(Some(&body));
    { let w = dlg.clone(); cancel.connect_clicked(move |_| w.close()); }
    let w = dlg.clone();
    ok.connect_clicked(move |_| { w.close(); then(); });
    dlg.present();
}

/// What a dialog or settings window holds: as big as its content, up to a limit, then scrolled. The window opens
/// at that size; when the person resizes it, the content takes the room it is given.
pub fn page_scroll(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).propagate_natural_width(true).propagate_natural_height(true)
        .max_content_height(640).vexpand(true).hexpand(true).child(child).build()
}

/// Rebuilds a scrolled list where the person had scrolled to. Left alone, GTK goes back to the top (the focused row
/// goes with the old rows), so the place is set again now and once more when the new rows are laid out.
pub fn keep_scroll(inside: &impl IsA<gtk::Widget>, rebuild: impl FnOnce()) {
    let adj = inside.ancestor(gtk::ScrolledWindow::static_type()).and_downcast::<gtk::ScrolledWindow>().map(|s| s.vadjustment());
    let top = adj.as_ref().map_or(0.0, |a| a.value());
    rebuild();
    if let Some(a) = adj { a.set_value(top); glib::idle_add_local_once(move || a.set_value(top)); }
}

/// Asks to install language support (`note` under the question), then installs it in the background, said on the
/// status line and in the record. `then` runs on that thread once it is installed.
pub fn install_engine(win: &gtk::ApplicationWindow, d: &Data, package: String, note: Vec<String>, then: impl FnOnce() + Send + 'static) {
    let ask = wb_core::i18n::f("input-install-ask", &[("package", package.clone())]);
    let (d, then) = (d.clone(), RefCell::new(Some(then)));
    confirm(win, &ask, &note, wb_core::i18n::t("input-install"), move || {
        let Some(then) = then.borrow_mut().take() else { return };
        let (d, package) = (d.clone(), package.clone());
        if wb_core::input::install_commands(&package).is_none() {
            wb_core::log::warn("input", "engine not installed", "no PackageKit and not WSL", &[("package", package.clone())]);
            wb_core::status::say(&d, &wb_core::i18n::f("input-install-by-hand", &[("package", package)]), 20);
            return;
        }
        wb_core::status::say(&d, &wb_core::i18n::f("input-installing", &[("package", package.clone())]), 120);
        std::thread::spawn(move || {
            let done = wb_core::input::install(&package);
            match &done {
                Ok(()) => wb_core::log::decide("input", "engine installed", "the person agreed", &[("package", package.clone())]),
                Err(why) => wb_core::log::warn("input", "engine not installed", why, &[("package", package.clone())]),
            }
            wb_core::status::say(&d, &wb_core::i18n::f(if done.is_ok() { "input-installed" } else { "input-install-failed" }, &[("package", package)]), 12);
            if done.is_ok() { then(); }
        });
    });
}

/// Another of Whaybench's panels, started by the compositor on its own display: started here, it would take this
/// panel's display (the desktop's, when this one is out there) and be neither placed nor ended with Whaybench.
pub fn open_panel(d: &Data, args: &[&str]) {
    let exe = std::env::current_exe().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
    let line = std::iter::once(exe.as_str()).chain(args.iter().copied()).map(|a| format!("'{}'", a.replace('\'', "'\\''"))).collect::<Vec<_>>().join(" ");
    let _ = wb_core::status::post(d, &json!({"cmd": "spawn", "command": format!("exec {line}")}));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pipes_in_code_stay_in_their_cell() {
        assert_eq!(table_pipes("| `a [b | c]` | d |"), "| `a [b \\| c]` | d |");
        assert_eq!(table_pipes("| a | b |"), "| a | b |");
        assert_eq!(table_pipes("text `a | b`"), "text `a | b`", "only in a table");
        assert_eq!(table_pipes("| `a \\| b` |"), "| `a \\| b` |", "already escaped");
    }
}
