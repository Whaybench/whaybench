//! The view panel: what the AI makes and what the person opens, without a browser's own bars. It goes with
//! a session: choosing another session brings that session's views back, so two conversations can share one view.
use std::{cell::RefCell, collections::HashMap, path::Path, rc::Rc, time::UNIX_EPOCH};

use gtk::{gio, glib, prelude::*};
use pulldown_cmark::{html, Options, Parser};
use serde_json::{json, Value};
use wb_core::{log as record, paths::Data, sessions, status, theme, view::{self, History, Link}};
use webkit6::prelude::*;

/// Sent to the page for `whaybench-view read`. `ALL`: the whole page, otherwise only the text inside the window.
const READ_SCRIPT: &str = r#"(function(all){
  var sel = String(window.getSelection ? window.getSelection() : '').trim();
  var lines = [];
  if (all) { lines = (document.body ? document.body.innerText : '').split('\n'); }
  else if (document.body) {
    var walk = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT), n, range = document.createRange();
    while ((n = walk.nextNode())) {
      if (!n.textContent.trim()) continue;
      var tag = n.parentElement ? n.parentElement.tagName : '';
      if (tag === 'SCRIPT' || tag === 'STYLE') continue;
      range.selectNodeContents(n);
      var r = range.getBoundingClientRect();
      if (r.height > 0 && r.bottom > 0 && r.top < window.innerHeight && r.right > 0 && r.left < window.innerWidth) lines.push(n.textContent.trim());
    }
  }
  var out = {title: document.title, selected: sel ? sel.split('\n') : [], text: lines.map(function(l){return l.trim();}).filter(Boolean)};
  return JSON.stringify(out);
})(ALL)"#;

struct V {
    d: Data,
    name: String,
    link: Link,
    /// The session whose views are here; none for a view held on one thing, or when no session is chosen.
    followed: Option<String>,
    hist: History,
    /// The page a PDF was asked to open at, for the next display.
    page: Option<u32>,
    /// The theme the pages drawn here were drawn in.
    style: String,
    /// When each picture file was last taken, to see a new one.
    seen: HashMap<String, u128>,
    web: webkit6::WebView,
    place: gtk::Label,
    tie: gtk::Button,
    back: gtk::Button,
    forward: gtk::Button,
    drop_it: gtk::Button,
    win: gtk::ApplicationWindow,
}
type Shared = Rc<RefCell<V>>;

fn modified(p: &Path) -> Option<u128> { Some(std::fs::metadata(p).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos()) }

fn escape(t: &str) -> String { t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;") }

fn hex(c: [f32; 4]) -> String { format!("#{:02x}{:02x}{:02x}", (c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8) }

fn page_style(d: &Data) -> String {
    let t = theme::load_active(d);
    let ink = format!("#{:02x}{:02x}{:02x}", t.ink[0], t.ink[1], t.ink[2]);
    format!("html,body{{margin:0;background:{};color:{ink};font:15px/1.6 '{f}',sans-serif}}\
             main{{padding:14px 18px}} .path{{font-size:12px;opacity:.65;padding:8px 18px 0}}\
             pre{{font:13px/1.5 '{m}',monospace;white-space:pre-wrap;margin:0}}\
             code{{font-family:'{m}',monospace}} table{{border-collapse:collapse}} td,th{{border:1px solid {};padding:3px 8px}}\
             img{{max-width:100%}}", hex(t.panel), hex(t.border), f = t.font.replace('\'', ""), m = t.terminal_font.replace('\'', ""))
}

fn wrapped(d: &Data, path: &str, body: &str) -> String {
    format!("<!doctype html><meta charset='utf-8'><title>{0}</title><style>{1}</style><div class='path'>{0}</div><main>{body}</main>", escape(path), page_style(d))
}

/// A page for what a browser engine would only offer as a download: Markdown drawn, other text as it is.
fn text_page(d: &Data, path: &str) -> Option<String> {
    let low = path.to_lowercase();
    let shown_as_is = [".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".html", ".htm", ".mp4", ".webm", ".pdf"];
    if view::is_web(path) || shown_as_is.iter().any(|e| low.ends_with(e)) { return None; }
    let bytes = match std::fs::read(path) { Ok(b) => b, Err(e) => return Some(wrapped(d, path, &wb_core::i18n::f("read-failed-3", &[("a1", (escape(&e.to_string())).to_string())]))) };
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(2_000_000)]).to_string();
    if low.ends_with(".md") || low.ends_with(".markdown") {
        let mut out = String::new();
        let text = crate::common::table_pipes(&text);
        html::push_html(&mut out, Parser::new_ext(&text, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS));
        return Some(wrapped(d, path, &out));
    }
    Some(wrapped(d, path, &format!("<pre>{}</pre>", escape(&text))))
}

fn file_uri(path: &str) -> String { if view::is_web(path) { path.to_string() } else { gio::File::for_path(path).uri().to_string() } }

fn title_for(path: &str) -> String {
    if view::is_web(path) {
        let host = path.split("://").nth(1).unwrap_or(path).split('/').next().unwrap_or(path);
        return wb_core::i18n::f("web", &[("host", (host).to_string())]);
    }
    wb_core::i18n::f("document-2", &[("a1", (Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).to_string())])
}

fn short(id: Option<&str>) -> String { id.map(|s| s.chars().take(8).collect()).unwrap_or_else(|| wb_core::i18n::t("none").to_string()) }

fn session_label(d: &Data, id: &str, keep: usize) -> String {
    let n = sessions::session_name(d, id);
    let n = if n.is_empty() { id.to_string() } else { n };
    n.chars().take(keep).collect()
}

fn session_now(v: &V) -> Option<String> {
    match &v.link {
        Link::Follow => sessions::target(&v.d),
        Link::Session(s) => Some(sessions::current_id(&v.d, s)),
        Link::Target(..) => None,
    }
}

fn own_picture(d: &Data, session: &str) -> std::path::PathBuf { d.root.join("show").join(session).join("latest.html") }

/// What this session's history has already taken of its own picture.
fn read_taken(v: &V) -> HashMap<String, u128> {
    std::fs::read_to_string(v.hist.dir.join("taken.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write_taken(v: &V) {
    let Some(s) = &v.followed else { return };
    let own = own_picture(&v.d, s).to_string_lossy().to_string();
    let kept: HashMap<&String, &u128> = v.seen.iter().filter(|(k, _)| **k == own).collect();
    let _ = std::fs::create_dir_all(&v.hist.dir);
    let _ = std::fs::write(v.hist.dir.join("taken.json"), json!(kept).to_string());
}

/// Keeps a copy of a picture in the history, so the next one does not overwrite it; relative links still lead to where
/// it was written.
fn keep_picture(v: &V, path: &Path) -> Option<String> {
    let page = std::fs::read_to_string(path).ok()?;
    let base = format!("<base href=\"{}/\">", file_uri(&path.parent()?.to_string_lossy()));
    let page = if page.contains("<head>") { page.replacen("<head>", &format!("<head>{base}"), 1) } else { format!("{base}{page}") };
    std::fs::create_dir_all(&v.hist.dir).ok()?;
    let name = std::time::SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_nanos();
    let copy = v.hist.dir.join(format!("{name}.html"));
    std::fs::write(&copy, page).ok()?;
    Some(copy.to_string_lossy().to_string())
}

/// A file in a temporary folder will be gone one day, and with it what this session showed: the history keeps a copy.
/// Elsewhere the file itself is shown, as it is now.
fn lasting(v: &V, path: &str) -> String {
    let p = Path::new(path);
    if view::is_web(path) || !p.is_file() || !(p.starts_with("/tmp") || p.starts_with(std::env::temp_dir())) { return path.to_string(); }
    let low = path.to_lowercase();
    if low.ends_with(".html") || low.ends_with(".htm") { return keep_picture(v, p).unwrap_or_else(|| path.to_string()); }
    let name = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map(|t| t.as_nanos()).unwrap_or(0);
    let copy = v.hist.dir.join(format!("{name}-{}", p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
    if std::fs::create_dir_all(&v.hist.dir).is_ok() && std::fs::copy(p, &copy).is_ok() { copy.to_string_lossy().to_string() } else { path.to_string() }
}

fn display(v: &V) {
    match v.hist.current() {
        Some((path, title)) => {
            match text_page(&v.d, path) {
                Some(page) => v.web.load_html(&page, Some(&file_uri(&Path::new(path).parent().map(|p| format!("{}/", p.display())).unwrap_or_default()))),
                None => {
                    let at = v.page.filter(|_| path.to_lowercase().ends_with(".pdf")).map(|n| format!("#page={n}")).unwrap_or_default();
                    v.web.load_uri(&format!("{}{at}", file_uri(path)));
                }
            }
            v.place.set_text(&format!("{title} · {}/{}", v.hist.pos + 1, v.hist.items.len()));
        }
        None => {
            let id = short(v.followed.as_deref());
            let t = wb_core::i18n::f("session-no-view", &[("id", (id).to_string())]);
            v.web.load_html(&format!("<!doctype html><meta charset='utf-8'><style>{}</style>", page_style(&v.d)), None);
            v.place.set_text(&t);
        }
    }
    v.back.set_sensitive(v.hist.pos > 0 && !v.hist.items.is_empty());
    v.forward.set_sensitive(v.hist.pos + 1 < v.hist.items.len());
    v.drop_it.set_sensitive(!v.hist.items.is_empty() && !matches!(v.link, Link::Target(..)));
}

fn retitle(v: &V) {
    let name = view::panel_title(&v.name);
    let (tie, what) = match &v.link {
        Link::Follow => (wb_core::i18n::t("linked-to-session-tab").to_string(), String::new()),
        Link::Session(s) => { let n = session_label(&v.d, s, 18); (n.clone(), n) }
        Link::Target(_, t) => { let n: String = t.chars().take(20).collect(); (wb_core::i18n::f("pinned-view-2", &[("n", (n).to_string())]), n) }
    };
    v.tie.set_label(&wb_core::i18n::f("link", &[("tie", (tie).to_string())]));
    v.win.set_title(Some(&if what.is_empty() { name } else { format!("{name} · {what}") }));
}

/// New pictures and requests for the session on screen. True: something came in.
fn incoming(v: &mut V) -> bool {
    if matches!(v.link, Link::Target(..)) { return false; }
    let Some(session) = v.followed.clone() else { return false };
    let mut came = false;
    let path = own_picture(&v.d, &session);
    if let Some(when) = modified(&path) {
        let key = path.to_string_lossy().to_string();
        if v.seen.get(&key) != Some(&when) {
            v.seen.insert(key, when);
            write_taken(v);
            if let Some(copy) = keep_picture(v, &path) {
                v.hist.push(&copy, wb_core::i18n::t("picture"));
                came = true;
            }
        }
    }
    for r in view::take_requests(&v.d, &view::for_session(&session)) {
        if let Some(p) = r.get("path").and_then(Value::as_str) {
            let kept = lasting(v, p);
            v.hist.push(&kept, &title_for(p));
            v.page = page_of(&r);
            came = true;
        }
    }
    came
}

/// Another session, or another link: its own history, and what it made while another was on screen comes in now.
fn switch(v: &mut V) {
    v.followed = session_now(v);
    if let Link::Target(p, t) = &v.link {
        v.hist = History::only(p, t);
        display(v);
        return;
    }
    // The conversation may have moved to this number since it was last on screen: what it showed comes with it.
    if let Some(now) = v.followed.clone() {
        for old in sessions::moved_from(&v.d, &now) { view::session_moved(&v.d, &old, &now); }
    }
    v.hist = History::load(view::history_dir(&v.d, &v.name, v.followed.as_deref()));
    v.seen = read_taken(v);
    incoming(v);
    display(v);
}

fn bind(s: &Shared, link: Link) {
    let mut v = s.borrow_mut();
    record::decide("view", "link changed", "the person chose it in the menu", &[("view", v.name.clone()), ("link", link.to_json().to_string())]);
    view::set_view(&v.d, &v.name.clone(), &link);
    v.link = link;
    switch(&mut v);
    retitle(&v);
}

fn link_menu(s: &Shared) {
    let v = s.borrow();
    let menu = gio::Menu::new();
    let group = gio::SimpleActionGroup::new();
    let n = Rc::new(std::cell::Cell::new(0));
    let add = |m: &gio::Menu, label: &str, link: Link| {
        let name = format!("a{}", n.get());
        n.set(n.get() + 1);
        let act = gio::SimpleAction::new(&name, None);
        let s2 = s.clone();
        act.connect_activate(move |_, _| { let l = link.clone(); let s3 = s2.clone(); glib::idle_add_local_once(move || bind(&s3, l)); });
        group.add_action(&act);
        m.append(Some(label), Some(&format!("v.{name}")));
    };
    add(&menu, &wb_core::i18n::f("linked-to-session-tab-2", &[("a1", (if v.link == Link::Follow { "✓ " } else { "" }).to_string())]), Link::Follow);
    let here = if v.link == Link::Follow { sessions::target(&v.d) } else { v.followed.clone() };
    if let Some(h) = &here { add(&menu, &wb_core::i18n::f("current-session", &[("a1", (session_label(&v.d, h, 24)).to_string())]), Link::Session(h.clone())); }
    let others = gio::Menu::new();
    let text = |i: &Value, k: &str| i.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    for info in sessions::all(&v.d) {
        let id = text(&info, "id");
        if !text(&info, "movedTo").is_empty() || text(&info, "firstPrompt").is_empty() || Some(&id) == here.as_ref() { continue; }
        add(&others, &session_label(&v.d, &id, 30), Link::Session(id));
    }
    menu.append_submenu(Some(wb_core::i18n::t("other-session")), &others);
    if let Some((p, t)) = v.hist.current() {
        add(&menu, &wb_core::i18n::f("pinned-view-3", &[("a1", (t.chars().take(24).collect::<String>()).to_string())]), Link::Target(p.clone(), t.clone()));
    }
    let pop = gtk::PopoverMenu::from_model_full(&menu, gtk::PopoverMenuFlags::NESTED);
    pop.set_parent(&v.tie);
    pop.insert_action_group("v", Some(&group));
    pop.set_has_arrow(false);
    pop.connect_closed(|p| { let p = p.clone(); glib::idle_add_local_once(move || p.unparent()); });
    pop.popup();
}

/// `whaybench-view read`: the page's own text, without going through a screen picture.
fn answer_read(v: &V, all: bool, reply: String) {
    let script = READ_SCRIPT.replace("ALL", if all { "true" } else { "false" });
    let url = v.hist.current().map(|c| c.0.clone()).unwrap_or_default();
    // The engine's PDF viewer keeps its text to itself, so a PDF is read from the file.
    if !view::is_web(&url) && url.to_lowercase().ends_with(".pdf") {
        let text = std::process::Command::new("pdftotext").args(["-layout", &url, "-"]).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let out = json!({"url": url, "title": title_for(&url), "selected": [], "text": lines});
        let tmp = format!("{reply}.tmp");
        if std::fs::write(&tmp, out.to_string()).is_ok() { let _ = std::fs::rename(&tmp, &reply); }
        return;
    }
    v.web.evaluate_javascript(&script, None, None, None::<&gio::Cancellable>, move |result| {
        let mut out = match result {
            Ok(value) => serde_json::from_str::<Value>(&value.to_str()).unwrap_or_else(|_| json!({"error": wb_core::i18n::t("could-not-read-the-page-text")})),
            Err(e) => json!({"error": e.to_string()}),
        };
        out["url"] = json!(url);
        let tmp = format!("{reply}.tmp");
        if std::fs::write(&tmp, out.to_string()).is_ok() { let _ = std::fs::rename(&tmp, &reply); }
    });
}

fn page_of(r: &Value) -> Option<u32> { r.get("page").and_then(Value::as_u64).map(|n| n as u32) }

fn tick(s: &Shared) {
    // Own requests first: they do not depend on the link.
    let (d, name) = { let v = s.borrow(); (v.d.clone(), v.name.clone()) };
    let mut front = false;
    for r in view::take_requests(&d, &view::for_view(&name)) {
        match r.get("do").and_then(Value::as_str) {
            Some("show") => {
                let Some(p) = r.get("path").and_then(Value::as_str) else { continue };
                let mut v = s.borrow_mut();
                if matches!(v.link, Link::Target(..)) {
                    let link = Link::Target(p.to_string(), title_for(p));
                    view::set_view(&d, &name, &link);
                    v.link = link;
                    v.hist = History::only(p, &title_for(p));
                    retitle(&v);
                } else {
                    let kept = lasting(&v, p);
                    v.hist.push(&kept, &title_for(p));
                }
                v.page = page_of(&r);
                display(&v);
                v.page = None;
                front = true;
            }
            Some("read") => {
                let reply = r.get("reply").and_then(Value::as_str).unwrap_or("").to_string();
                if !reply.is_empty() { answer_read(&s.borrow(), r.get("all").and_then(Value::as_bool).unwrap_or(false), reply); }
            }
            Some("close") => { let win = s.borrow().win.clone(); win.close(); return; }
            _ => {}
        }
    }
    let mut v = s.borrow_mut();
    // Pages this panel draws itself follow the theme; pages from elsewhere have their own colours.
    let style = page_style(&d);
    if v.style != style {
        let first = v.style.is_empty();
        v.style = style;
        let own = v.hist.current().is_none_or(|(p, _)| text_page(&d, p).is_some());
        if !first && own { display(&v); }
    }
    let now = session_now(&v);
    if !matches!(v.link, Link::Target(..)) && now != v.followed {
        record::detail("view", "session changed", &[("view", name.clone()), ("session", short(now.as_deref()))]);
        switch(&mut v);
    } else if incoming(&mut v) {
        display(&v);
        v.page = None;
        front = true;
    }
    if front { status::front_app(&d, &view::app_of(&name)); }
}

pub fn build(win: &gtk::ApplicationWindow, d: Data, name: String) {
    let store = d.root.join("view/webkit");
    let session = webkit6::NetworkSession::new(Some(&store.join("data").to_string_lossy()), Some(&store.join("cache").to_string_lossy()));
    // Sign-ins stay over restarts: without a file the cookies live only as long as the panel.
    if let Some(cookies) = session.cookie_manager() {
        cookies.set_persistent_storage(&store.join("cookies.sqlite").to_string_lossy(), webkit6::CookiePersistentStorage::Sqlite);
    }
    let web = webkit6::WebView::builder().network_session(&session).build();
    crate::common::web_menu_in_language(&web);
    crate::common::follow_zoom(&web, &d);
    web.set_vexpand(true);
    web.set_hexpand(true);
    if let Some(settings) = webkit6::prelude::WebViewExt::settings(&web) { settings.set_allow_file_access_from_file_urls(true); settings.set_default_charset("utf-8"); }
    // A link that wants a new window stays in this view.
    web.connect_create(|w, action| {
        let action = action.clone();
        if let Some(uri) = action.request().and_then(|r| r.uri()) { w.load_uri(&uri); }
        None
    });


    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    bar.set_margin_start(4); bar.set_margin_end(4); bar.set_margin_top(2); bar.set_margin_bottom(2);
    let tie = gtk::Button::new();
    tie.add_css_class("flat");
    let place = gtk::Label::new(None);
    place.set_hexpand(true);
    place.set_xalign(1.0);
    place.set_ellipsize(gtk::pango::EllipsizeMode::Start);
    place.add_css_class("small");
    let (back, forward, drop_it) = (gtk::Button::with_label("◀"), gtk::Button::with_label("▶"), gtk::Button::with_label("✕"));
    drop_it.set_tooltip_text(Some(wb_core::i18n::t("remove-from-history-the-file-stays")));
    bar.append(&tie); bar.append(&place); bar.append(&back); bar.append(&forward); bar.append(&drop_it);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&bar);
    body.append(&web);
    win.set_child(Some(&body));

    let link = view::views(&d).into_iter().find(|(n, _)| *n == name).map(|(_, l)| l).unwrap_or_default();
    let s: Shared = Rc::new(RefCell::new(V { d: d.clone(), name: name.clone(), link, followed: None, hist: History::default(), page: None, style: String::new(), seen: HashMap::new(), web, place, tie: tie.clone(), back: back.clone(), forward: forward.clone(), drop_it: drop_it.clone(), win: win.clone() }));
    { let mut v = s.borrow_mut(); switch(&mut v); retitle(&v); }

    { let s = s.clone(); tie.connect_clicked(move |_| link_menu(&s)); }
    for (button, step) in [(&back, -1), (&forward, 1)] {
        let s = s.clone();
        button.connect_clicked(move |_| { let mut v = s.borrow_mut(); if v.hist.go(step) { display(&v); } });
    }
    { let s = s.clone(); drop_it.connect_clicked(move |_| { let mut v = s.borrow_mut(); v.hist.remove_current(); display(&v); }); }
    // The first view cannot be closed; another, once closed, does not come back at the next start.
    {
        let (d, name) = (d.clone(), name.clone());
        win.connect_close_request(move |_| {
            if name == view::FIRST_VIEW { return glib::Propagation::Stop; }
            view::drop_view(&d, &name);
            glib::Propagation::Proceed
        });
    }
    glib::timeout_add_local(std::time::Duration::from_millis(500), move || { tick(&s); glib::ControlFlow::Continue });
}
