//! The project panel: a project is its map and its recent changes; documents are add-ons, added one at a
//! time. There is no edit box: only the AI writes, and a change is a request to the chosen session. Pages are drawn as
//! HTML; their links are not followed but handled here.
use std::{cell::RefCell, collections::HashMap, path::Path, rc::Rc, time::{Duration, UNIX_EPOCH}};

use gtk::{glib, prelude::*};
use pulldown_cmark::{html, Options, Parser};
use regex::Regex;
use serde_json::{json, Value};
use wb_core::{log as record, paths::Data, sessions, status, theme};
use wb_store::{Store, docaddons, history, markdown as md};
use webkit6::prelude::*;

const MAP: &str = "map";
const RECENT: &str = "recent";

fn e(t: &str) -> String { t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\'', "&#39;") }
fn s<'a>(v: &'a Value, k: &str) -> &'a str { v.get(k).and_then(Value::as_str).unwrap_or("") }
fn list<'a>(v: &'a Value, k: &str) -> &'a [Value] { v.get(k).and_then(Value::as_array).map_or(&[], Vec::as_slice) }
/// A link handled by the panel. Its parts travel escaped, so a section name cannot break it.
fn link(kind: &str, rest: &str, label: &str) -> String { format!("<a href='wb:{kind}:{}'>{label}</a>", glib::Uri::escape_string(rest, None, false)) }

fn when(ms: i64) -> String {
    if ms == 0 { return String::new(); }
    chrono::DateTime::from_timestamp_millis(ms).map(|t| t.with_timezone(&chrono::Local).format("%m-%d %H:%M").to_string()).unwrap_or_default()
}
fn at(v: &Value, k: &str) -> i64 { v.get(k).and_then(Value::as_i64).unwrap_or(0) }

fn stage_name(st: &str) -> &'static str {
    match st { "idea" => wb_core::i18n::t("idea"), "decided" => wb_core::i18n::t("decided"), "doing" => wb_core::i18n::t("in-progress-2"), "unverified" => wb_core::i18n::t("unverified"), "done" => wb_core::i18n::t("done"), "resolved" => wb_core::i18n::t("resolved"), "dropped" => wb_core::i18n::t("dropped"), _ => "" }
}
fn stage_mark(st: &str) -> &'static str {
    match st { "idea" => "◇", "decided" => "□", "doing" | "unverified" => "◐", "done" | "resolved" => "✓", "dropped" => "✕", _ => "·" }
}
/// A scope value in the language in use.
fn scope_name(v: &str) -> &str {
    match v {
        "include" => wb_core::i18n::t("scope-include"),
        "hold" => wb_core::i18n::t("scope-hold"),
        "exclude" => wb_core::i18n::t("scope-exclude"),
        o => o,
    }
}

fn is_open(st: &str) -> bool { matches!(st, "idea" | "decided" | "doing" | "unverified") }
fn render(text: &str) -> String {
    let mut out = String::new();
    let text = crate::common::table_pipes(text);
    html::push_html(&mut out, Parser::new_ext(&text, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS));
    out
}

/// What a document's page leaves out: writing guides and source comments are for the AI.
struct Hidden { guide: Regex, source: Regex }
impl Hidden {
    fn new() -> Self {
        Hidden { guide: Regex::new(r"<!--\s*guide:.*?-->").unwrap(), source: Regex::new(r"<!--\s*source:.*?-->").unwrap() }
    }
}

/// One document, with its table of contents and request links.
fn doc_page(st: &Store, pid: &str, d: &Value, anchors: &mut HashMap<String, HashMap<String, String>>) -> String {
    let did = s(d, "id");
    let text = st.document(pid, did);
    let heads = md::headings(&text);
    let lines: Vec<&str> = text.split('\n').collect();
    let at_head: HashMap<usize, (usize, String)> = heads.iter().map(|h| (h.0, (h.1, h.2.clone()))).collect();
    let hide = Hidden::new();
    let (mut toc, mut out, mut mine) = (Vec::new(), Vec::new(), HashMap::new());
    for (n, line) in lines.iter().enumerate() {
        if let Some((level, title)) = at_head.get(&n) {
            let path = md::path_at(&heads, n).join(" > ");
            let anchor = format!("h{n}");
            mine.insert(path.clone(), anchor.clone());
            if *level > 1 { toc.push(format!("{}<a href='#{anchor}'>{}</a>", "&nbsp;".repeat(4 * (level - 2)), e(title))); }
            let ask = if *level > 1 { format!(" <small>{}</small>", link("req", &format!("{did}:{n}"), wb_core::i18n::t("request-change"))) } else { String::new() };
            out.push(format!("{} <a id='{anchor}'></a>{}{ask}", "#".repeat(*level), title));
            continue;
        }
        if hide.guide.is_match(line) || hide.source.is_match(line) { continue; }
        out.push(line.to_string());
    }
    anchors.insert(did.to_string(), mine);
    let mut head = wb_core::i18n::f("lines-written-by-ai", &[("a1", (lines.len()).to_string()), ("a2", (link("dreq", did, wb_core::i18n::t("request-change-to-this-document"))).to_string())]);
    if !s(d, "original").is_empty() { head += &wb_core::i18n::f("original", &[("a1", (e(s(d, "original"))).to_string())]); }
    head += &wb_core::i18n::f("contents", &[("a1", (toc.join("<br>")).to_string())]);
    format!("{head}<div id='body'></div>{}", render(&out.join("\n")))
}

fn map_page(st: &Store, pid: &str) -> String {
    let m = st.project_map(pid);
    let items = list(&m, "items");
    if items.is_empty() { return wb_core::i18n::t("no-project-map-to-add-conversation").into(); }
    let mut kids: HashMap<&str, Vec<&Value>> = HashMap::new();
    for i in items { kids.entry(s(i, "parent")).or_default().push(i); }
    let row = |i: &Value, depth: usize| {
        let stage = s(i, "stage");
        let closed = !is_open(stage);
        let scope = if !s(i, "include").is_empty() && !closed { wb_core::i18n::f("scope", &[("a1", scope_name(s(i, "include")).to_string())]) } else { String::new() };
        let bits: Vec<String> = [stage_name(stage).to_string(), scope, s(i, "reason").to_string()].into_iter().filter(|x| !x.is_empty()).collect();
        let origins: Vec<String> = list(i, "origin").iter().take(3).map(|o| {
            if !o.is_object() { return e(&o.as_str().map(String::from).unwrap_or_else(|| o.to_string()).chars().take(24).collect::<String>()); }
            let label = if s(o, "label").is_empty() { s(o, "node") } else { s(o, "label") };
            link("src", &o.to_string(), &e(&label.chars().take(24).collect::<String>()))
        }).collect();
        let origin = if origins.is_empty() { String::new() } else { wb_core::i18n::f("source-nodes", &[("a1", (origins.join(", ")).to_string())]) };
        let pad = "&nbsp;".repeat(6 * depth);
        let mut line = format!("{pad}<span{}>{} {}</span> <span class='ok'>{}</span>{origin}", if closed { " class='m'" } else { "" }, stage_mark(stage), e(s(i, "text")), e(&bits.join(" · ")));
        if !s(i, "note").is_empty() { line += &format!("<br>{pad}{}<span class='m'>{}</span>", "&nbsp;".repeat(6), e(s(i, "note"))); }
        line
    };
    fn walk(parent: &str, area: &str, depth: usize, kids: &HashMap<&str, Vec<&Value>>, rows: &mut Vec<String>, row: &dyn Fn(&Value, usize) -> String) {
        for i in kids.get(parent).into_iter().flatten() {
            if s(i, "area") == area { rows.push(row(i, depth)); walk(s(i, "id"), area, depth + 1, kids, rows, row); }
        }
    }
    let last = items.iter().map(|i| at(i, "at")).max().unwrap_or(0);
    let mut out = vec![wb_core::i18n::f("written-by-ai-last-update-idea", &[("a1", (when(last)).to_string())])];
    let mut summary = Vec::new();
    for a in list(&m, "areas") {
        if a.get("watch").is_some_and(|w| w != &json!(false) && !w.is_null()) { continue; }
        let open: Vec<&Value> = items.iter().filter(|i| s(i, "area") == s(a, "id") && is_open(s(i, "stage"))).collect();
        if open.is_empty() { continue; }
        let rows: Vec<String> = open.iter().map(|i| format!("&nbsp;&nbsp;{} {} <span class='m'>{}{}</span>", stage_mark(s(i, "stage")), e(&s(i, "text").chars().take(80).collect::<String>()), stage_name(s(i, "stage")), if s(i, "include").is_empty() { String::new() } else { wb_core::i18n::f("scope-2", &[("a1", e(scope_name(s(i, "include"))).to_string())]) })).collect();
        summary.push(format!("<b><a href='#area-{}'>{}</a></b> {}<br>{}", e(s(a, "id")), e(s(a, "title")), open.len(), rows.join("<br>")));
    }
    if !summary.is_empty() { out.push(wb_core::i18n::f("open-items", &[("a1", (summary.join("<br>")).to_string())])); }
    let ids: Vec<&str> = items.iter().map(|i| s(i, "id")).collect();
    for a in list(&m, "areas") {
        let mut rows = Vec::new();
        walk("", s(a, "id"), 0, &kids, &mut rows, &row);
        for i in items { if s(i, "area") == s(a, "id") && !s(i, "parent").is_empty() && !ids.contains(&s(i, "parent")) { rows.push(row(i, 0)); } }
        if rows.is_empty() { continue; }
        let note = if s(a, "note").is_empty() { String::new() } else { format!("<p class='m'>{}</p>", e(s(a, "note"))) };
        out.push(format!("<h3 id='area-{}'>{}</h3>{note}<p>{}</p>", e(s(a, "id")), e(s(a, "title")), rows.join("<br>")));
    }
    out.concat()
}

fn folder_size(dir: &Path) -> (usize, u64) {
    let (mut n, mut size) = (0, 0);
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() { n += 1; size += e.metadata().map(|m| m.len()).unwrap_or(0); }
    (n, size)
}

fn recent_page(st: &Store, pid: &str, range: Option<i64>) -> String {
    let choices: Vec<String> = [(Some(1), wb_core::i18n::t("today")), (Some(7), wb_core::i18n::t("n7-days")), (Some(30), wb_core::i18n::t("n30-days")), (None, wb_core::i18n::t("all"))].into_iter()
        .map(|(d, label)| if range == d { format!("<b>{label}</b>") } else { link("range", &d.unwrap_or(0).to_string(), label) }).collect();
    let (kept, size) = folder_size(&st.pdir(pid).join("events"));
    let mut out = vec![wb_core::i18n::f("period-records-mb-document-copies-days", &[("a1", (choices.join(" · ")).to_string()), ("kept", (kept).to_string()), ("a2", format!("{:.1}", size as f64 / 1_048_576.0)), ("a3", (history::FULL_DAYS).to_string()), ("a4", (history::META_DAYS).to_string()), ("a5", (history::MAX_MB).to_string())])];
    let changes = st.recent(pid, range);
    if changes.is_empty() { out.push(wb_core::i18n::t("no-changes").into()); }
    for c in changes {
        let paths: Vec<String> = list(&c, "paths").iter().filter_map(Value::as_str).filter(|p| !p.is_empty()).map(String::from).collect();
        let paths = if paths.is_empty() { vec![wb_core::i18n::t("start-of-document").to_string()] } else { paths };
        let look = if c.get("hasCopy").and_then(Value::as_bool).unwrap_or(false) { format!(" {}", link("chg", &format!("{}:{}", s(&c, "doc"), s(&c, "id")), wb_core::i18n::t("details"))) } else { wb_core::i18n::f("no-copy-past-the-day-retention", &[("a1", (history::FULL_DAYS).to_string())]) };
        let head = format!("<span class='m'>{}{}</span>{look}<br>", when(at(&c, "at")), "");
        let rows: Vec<String> = paths.iter().take(12).map(|p| {
            let clean = md::without_mark(p);
            let place = std::iter::once(s(&c, "docName").to_string()).chain(clean.split(" > ").map(String::from)).map(|x| e(&x)).collect::<Vec<_>>().join(" &gt; ");
            let shown = if s(&c, "doc") == MAP { place } else { link("go", &format!("{}:{clean}", s(&c, "doc")), &place) };
            format!("{shown}{}", if *p != clean { wb_core::i18n::t("deleted-mark") } else { "" })
        }).collect();
        let more = if paths.len() > 12 { wb_core::i18n::f("more", &[("a1", (paths.len() - 12).to_string())]) } else { String::new() };
        if rows.len() == 1 { out.push(format!("<p>{head}{} : {}</p>", rows[0], e(s(&c, "what")))); }
        else { out.push(format!("<p>{head}{}<br>{}{more}</p>", e(s(&c, "what")), rows.iter().map(|r| format!("· {r}")).collect::<Vec<_>>().join("<br>"))); }
    }
    out.concat()
}

/// A recorded change to a document, section by section, or to the map, item by item.
fn change_page(st: &Store, pid: &str, target: &str, event: &str) -> String {
    let ev = st.events(pid).into_iter().find(|x| s(x, "id") == event).unwrap_or_default();
    let back = format!("<p>{}</p>", link("close", "", wb_core::i18n::t("list")));
    let head = wb_core::i18n::f("change-3", &[("back", (back).to_string()), ("a1", (when(at(&ev, "at"))).to_string()), ("a2", (e(event)).to_string())]);
    if target == MAP {
        let it = st.item_change_detail(pid, event).unwrap_or_else(|| json!({"changes": [], "what": ""}));
        let mut out = vec![format!("{head}<p><b>{}</b></p>", e(s(&it, "what")))];
        let name = |k: &str| -> String { match k { "text" => wb_core::i18n::t("content"), "title" => wb_core::i18n::t("title-2"), "stage" => wb_core::i18n::t("stage"), "include" => wb_core::i18n::t("scope-3"), "reason" => wb_core::i18n::t("reason"), "note" => wb_core::i18n::t("description"), "area" => wb_core::i18n::t("area"), "parent" => wb_core::i18n::t("parent-item"), "status" => wb_core::i18n::t("status"), "goal" => wb_core::i18n::t("goal"), "steps" => wb_core::i18n::t("steps"), "items" => wb_core::i18n::t("map-items"), "size" => wb_core::i18n::t("size"), "origin" => wb_core::i18n::t("source-nodes-2"), "order" => wb_core::i18n::t("order"), o => o }.to_string() };
        let val = |k: &str, v: Option<&Value>| -> String {
            let Some(v) = v.filter(|v| !v.is_null() && v.as_str() != Some("")) else { return "<span class='m'>—</span>".into() };
            match (k, v) {
                ("stage", Value::String(x)) => e(stage_name(x)),
                (_, Value::String(x)) => e(x),
                (_, Value::Array(a)) => a.iter().map(|x| e(&x.as_str().map(String::from).unwrap_or_else(|| x.to_string()))).collect::<Vec<_>>().join("<br>"),
                (_, o) => e(&o.to_string()),
            }
        };
        for (k, c) in list(&it, "changes").iter().enumerate() {
            let op = match s(c, "op") { "add" => wb_core::i18n::t("added"), "update" => wb_core::i18n::t("modified"), _ => wb_core::i18n::t("delete") };
            out.push(format!("<hr><h3>{} · {op} · {}</h3>", history::target_name(target), e(&history::label(c).chars().take(80).collect::<String>())));
            if c.get("before").is_none() && c.get("after").is_none() { out.push(wb_core::i18n::f("no-copy-past-the-day-retention-2", &[("a1", (history::FULL_DAYS).to_string())])); continue; }
            let scope = format!("{target}:{event}:{k}");
            out.push(format!("<p>{} &nbsp; {}</p>", link("iback", &scope, wb_core::i18n::t("restore-item")), link("iredo", &scope, wb_core::i18n::t("request-rework-from-the-earlier-version"))));
            let (a, b) = (c.get("before").cloned().unwrap_or_default(), c.get("after").cloned().unwrap_or_default());
            let mut keys: Vec<String> = a.as_object().into_iter().flat_map(|o| o.keys().cloned()).collect();
            for x in b.as_object().into_iter().flat_map(|o| o.keys().cloned()) { if !keys.contains(&x) { keys.push(x); } }
            let rows: String = keys.iter().filter(|x| !["at", "by", "updatedAt", "id"].contains(&x.as_str()) && (a.get(x.as_str()) != b.get(x.as_str()) || s(c, "op") != "update"))
                .map(|x| format!("<tr><td>{}</td><td>{}</td><td>{}</td></tr>", name(x), val(x, a.get(x.as_str())), val(x, b.get(x.as_str())))).collect();
            out.push(wb_core::i18n::f("itembeforeafter", &[("rows", (rows).to_string())]));
        }
        return out.concat();
    }
    let doc = target;
    let what = list(&ev, "docs").iter().find(|d| s(d, "doc") == doc).map(|d| s(d, "what")).unwrap_or("");
    let mut out = vec![format!("{head}<p><b>{}</b></p>", e(what))];
    let Some(detail) = st.change_detail(pid, event, doc) else { out.push(wb_core::i18n::t("no-copy-past-retention").into()); return out.concat(); };
    let side = |t: &str, none: &str| if t.is_empty() { format!("<p class='m'>{none}</p>") } else { render(t) };
    for d in detail {
        let scope = format!("{doc}:{event}:{}", s(&d, "path"));
        out.push(wb_core::i18n::f("nbsp-beforeafter", &[("a1", (e(&s(&d, "path").replace(" > ", " › "))).to_string()), ("a2", (link("dback", &scope, wb_core::i18n::t("restore-section"))).to_string()), ("a3", (link("dredo", &scope, wb_core::i18n::t("request-rework-from-the-earlier-version"))).to_string()), ("a4", (side(s(&d, "before"), wb_core::i18n::t("new-section"))).to_string()), ("a5", (side(s(&d, "after"), wb_core::i18n::t("deleted"))).to_string())]));
    }
    out.concat()
}

fn style(d: &Data) -> String {
    let t = theme::load_active(d);
    let hex = |c: [f32; 4]| format!("#{:02x}{:02x}{:02x}", (c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8);
    let ink = format!("#{:02x}{:02x}{:02x}", t.ink[0], t.ink[1], t.ink[2]);
    format!("html,body{{margin:0;background:{};color:{ink};font:14px/1.55 '{f}',sans-serif}} body{{padding:6px 12px}}\
      h1{{font-size:16pt;margin:4px 0}} h2{{font-size:14pt;margin:18px 0 6px}} h3{{font-size:12pt;margin:14px 0 4px}} h4,h5,h6{{font-size:11pt;margin:12px 0 3px}}\
      a{{color:{};text-decoration:none}} .m{{opacity:.6;font-size:small}} .ok{{color:#2e7d32;font-size:small}} .warn{{color:#c0392b}} .toc{{font-size:small}}\
      table{{border-collapse:collapse;margin:4px 0}} td,th{{border:1px solid {};padding:3px 6px}} pre,code{{font-family:'{m}',monospace;font-size:9.5pt}} pre{{white-space:pre-wrap}}", hex(t.panel), hex(t.accent), hex(t.border), f = t.font.replace('\'', ""), m = t.terminal_font.replace('\'', ""))
}

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
    tabs: gtk::FlowBox,
    tab_buttons: Vec<(String, gtk::Button)>,
    win: gtk::ApplicationWindow,
}
type Shared = Rc<RefCell<P>>;

fn stamp_of(st: &Store, pid: &str) -> String {
    let base = st.pdir(pid);
    ["events", "map.json", "project.json"].iter().map(|f| std::fs::metadata(base.join(f)).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |t| t.as_nanos()).to_string()).collect::<Vec<_>>().join(",")
}

/// A request for the chosen session's AI; only to one that is running.
fn request(p: &P, ask: &str) {
    let Some(pid) = &p.pid else { return };
    let Some(session) = sessions::target(&p.d) else { p.path.set_text(wb_core::i18n::t("request-not-sent-no-session-selected")); return };
    let text = wb_core::i18n::ai_f("project-this-session-edits-the-project", &[("pid", (pid).to_string()), ("ask", (ask).to_string())]);
    if sessions::queue_prompt(&p.d, &session, &text) { status::say(&p.d, wb_core::i18n::t("request-sent-selected-session"), 5); }
    else { p.path.set_text(wb_core::i18n::t("request-not-sent-session-is-off-2")); }
}

/// A short question in a window of its own, over the panel. `done` gets the text if there is any.
fn ask_text(win: &gtk::ApplicationWindow, title: &str, label: &str, done: impl Fn(String) + 'static) {
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
}

/// The folder name of a new project: from its name, or from the time when the name has no latin letters; never one in use.
fn new_project_id(st: &Store, name: &str) -> String {
    let slug: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>().split('-').filter(|x| !x.is_empty()).collect::<Vec<_>>().join("-");
    let base = if slug.is_empty() { format!("p-{}", chrono::Utc::now().timestamp()) } else { slug };
    if st.get_project(&base).is_none() { return base; }
    (2..).map(|n| format!("{base}-{n}")).find(|x| st.get_project(x).is_none()).unwrap()
}

/// The documents to show: those added as add-ons. Older ones without a template stay on file, out of sight.
/// Every document of the project gets a tab: it is Markdown whatever made it (a template add-on, or a format the
/// program no longer has).
fn shown_docs(project: &Value) -> Vec<Value> { list(project, "docs").to_vec() }

fn page_for(p: &mut P) -> String {
    let Some(pid) = p.pid.clone() else { return String::new() };
    if let Some((target, event)) = &p.over { return change_page(&p.st, &pid, target, event); }
    if p.tab == RECENT { return recent_page(&p.st, &pid, p.range); }
    let project = p.st.get_project(&pid).unwrap_or_default();
    match shown_docs(&project).into_iter().find(|d| s(d, "id") == p.tab) {
        Some(d) => { let st = p.st.clone(); doc_page(&st, &pid, &d, &mut p.anchors) }
        None => { p.tab = MAP.to_string(); map_page(&p.st, &pid) }
    }
}

/// The map, then the documents, then recent changes, then the way to add one.
fn set_tabs(s_: &Shared, keys: Vec<(String, String)>) {
    let mut p = s_.borrow_mut();
    if p.tab_keys != keys {
        p.tab_keys = keys.clone();
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
        let plus = gtk::Button::with_label(wb_core::i18n::t("document"));
        plus.add_css_class("flat");
        plus.set_tooltip_text(Some(wb_core::i18n::t("add-a-document-template-add-on")));
        let s2 = s_.clone();
        plus.connect_clicked(move |b| add_doc_menu(&s2, b));
        p.tabs.insert(&plus, -1);
    }
    for (key, b) in &p.tab_buttons {
        if *key == p.tab && p.over.is_none() { b.add_css_class("chosen"); } else { b.remove_css_class("chosen"); }
    }
}

fn add_doc_menu(s_: &Shared, on: &gtk::Button) {
    let p = s_.borrow();
    let Some(pid) = p.pid.clone() else { p.path.set_text(wb_core::i18n::t("no-project-create-a-new-project")); return };
    let project = p.st.get_project(&pid).unwrap_or_default();
    let have: Vec<&str> = list(&project, "docs").iter().map(|d| s(d, "id")).collect();
    let free: Vec<docaddons::DocAddon> = docaddons::list(&p.d).into_iter().filter(|a| !have.contains(&a.id.as_str())).collect();
    if free.is_empty() { p.path.set_text(wb_core::i18n::t("no-document-template-to-add-install")); return; }
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
                    Ok(()) => {
                        record::decide("project", "document added", "the person chose a template", &[("project", pid3.clone()), ("template", a2.id.clone())]);
                        { let mut p = s3.borrow_mut(); p.tab = a2.id.clone(); p.over = None; p.stamp.clear(); }
                        tick(&s3);
                    }
                    Err(e) => s3.borrow().path.set_text(&wb_core::i18n::f("adding-the-document-failed", &[("e", (e).to_string())])),
                }
            });
        });
        group.add_action(&act);
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
        let Some(project) = p.st.get_project(&pid) else { return };
        let up: Vec<String> = p.st.ancestors(&pid).iter().map(|a| s(a, "name").to_string()).chain(std::iter::once(s(&project, "name").to_string())).collect();
        let kids = p.st.children(&pid).len();
        p.path.set_text(&format!("{}{}", up.join(" › "), if kids > 0 { wb_core::i18n::f("below", &[("kids", (kids).to_string())]) } else { String::new() }));
        let body = page_for(&mut p);
        let key = format!("{pid}/{}", p.over.as_ref().map(|o| o.1.clone()).unwrap_or_else(|| p.tab.clone()));
        let html = format!("<!doctype html><meta charset='utf-8'><style>{}</style><body>{body}</body>", style(&p.d));
        let mut tabs = vec![(MAP.to_string(), wb_core::i18n::t("map").to_string())];
        tabs.extend(shown_docs(&project).iter().map(|d| (s(d, "id").to_string(), s(d, "name").to_string())));
        tabs.push((RECENT.to_string(), wb_core::i18n::t("recent-changes").to_string()));
        (html, key, p.web.clone(), tabs)
    };
    set_tabs(s_, tabs);
    if s_.borrow().shown == page { return; }
    // Where the page stands now, then the new page at the place its key last stood.
    let s2 = s_.clone();
    web.evaluate_javascript("window.scrollY", None, None, None::<&gtk::gio::Cancellable>, move |r| {
        let mut p = s2.borrow_mut();
        let was = p.key_now.clone();
        if let Ok(v) = r { p.scroll.insert(was, v.to_double()); }
        let top = p.scroll.get(&key).copied().unwrap_or(0.0);
        p.shown = page.clone();
        p.key_now = key.clone();
        p.web.load_html(&format!("{page}<script>window.scrollTo(0,{top})</script>"), Some("about:blank"));
    });
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
            w.close();
            let result = (|| -> Result<String, String> {
                let p = s2.borrow();
                let pid = new_project_id(&p.st, &name);
                p.st.create_project(&pid, &name, "", "", Vec::new(), false)?;
                if let Some(src) = &source {
                    let (nodes, title, from) = mi::read(&d2, src).ok_or_else(|| wb_core::i18n::t("import-failed-no-map").to_string())?;
                    let path = mi::keep(&d2, &nodes, &title, &from).map_err(|e| e.to_string())?;
                    p.st.map_from_nodes(&pid, &nodes, &title, &path.to_string_lossy())?;
                }
                Ok(pid)
            })();
            match result {
                Ok(pid) => {
                    record::decide("project", "new project", "the person named it", &[("project", pid.clone()), ("imported_map", source.is_some().to_string())]);
                    { let mut p = s2.borrow_mut(); p.pid = Some(pid); p.tab = MAP.to_string(); p.over = None; p.stamp.clear(); p.rows.clear(); }
                    tick(&s2);
                }
                Err(e) => s2.borrow().path.set_text(&wb_core::i18n::f("creation-failed", &[("e", e)])),
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
    let rows: Vec<(String, String, usize)> = p.st.tree().into_iter().map(|(v, depth)| (s(&v, "id").to_string(), s(&v, "name").to_string(), depth)).collect();
    if rows == p.rows { return; }
    p.rows = rows;
    let names: Vec<String> = if p.rows.is_empty() { vec![wb_core::i18n::t("no-project").to_string()] } else { p.rows.iter().map(|(_, n, depth)| format!("{}{}{n}", "   ".repeat(*depth), if *depth > 0 { "└ " } else { "" })).collect() };
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let model = gtk::StringList::new(&refs);
    // The project of the chosen session's folder, or the one already on show, or the first.
    let of_session = sessions::target(&p.d).and_then(|t| sessions::record_of(&p.d, &t)).and_then(|r| p.st.project_for_cwd(s(&r, "cwd"))).map(|v| s(&v, "id").to_string());
    let want = p.pid.clone().or(of_session);
    let at_row = want.and_then(|w| p.rows.iter().position(|r| r.0 == w)).unwrap_or(0);
    p.pid = p.rows.get(at_row).map(|r| r.0.clone());
    p.stamp.clear();
    let pick = p.pick.clone();
    drop(p);
    pick.set_model(Some(&model));
    pick.set_selected(at_row as u32);
}

fn tick(s_: &Shared) {
    convert_waiting(&s_.borrow());
    fill_list(s_);
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
}

fn open_source(p: &P, o: &Value) {
    if !s(o, "imported").is_empty() {
        let path = Path::new(s(o, "imported"));
        if path.is_file() { crate::mapimport::show(&p.d, path, s(o, "title"), Some(s(o, "node"))); }
        else { status::say(&p.d, &wb_core::i18n::f("source-not-opened-no-file", &[("path", path.display().to_string())]), 8); }
        return;
    }
    let (session, node) = (s(o, "session"), s(o, "node"));
    if session.is_empty() { return; }
    sessions::set_focus(&p.d, &sessions::current_id(&p.d, session), wb_core::i18n::t("project"));
    status::focus_app(&p.d, "dev.whaybench.map");
    let (d, node) = (p.d.clone(), node.to_string());
    // The map first has to change to that session.
    glib::timeout_add_local_once(Duration::from_millis(1500), move || status::send(&d, "map", json!({"do": "pick", "node": node})));
}

fn clicked(s_: &Shared, uri: &str) {
    let Some(rest) = uri.strip_prefix("wb:") else { return };
    let (kind, value) = rest.split_once(':').unwrap_or((rest, ""));
    let value = glib::Uri::unescape_string(value, None::<&str>).map(|v| v.to_string()).unwrap_or_else(|| value.to_string());
    let (pid, win) = { let p = s_.borrow(); (p.pid.clone().unwrap_or_default(), p.win.clone()) };
    let over = |s_: &Shared, o: Option<(String, String)>| { s_.borrow_mut().over = o; redraw(s_); };
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
            let (s2, pid2, w2) = (s_.clone(), pid.clone(), where_.clone());
            ask_text(&win, wb_core::i18n::t("request-change-2"), &wb_core::i18n::f("request", &[("where_", (where_).to_string())]), move |t| {
                let source = if original.is_empty() { String::new() } else { wb_core::i18n::ai_f("original-of-the-template", &[("original", (original).to_string())]) };
                request(&s2.borrow(), &wb_core::i18n::ai_f("change-as-the-user-requests-check", &[("w2", (w2).to_string()), ("t", (t).to_string()), ("pid2", (pid2).to_string()), ("did", (did).to_string()), ("source", (source).to_string())]));
            });
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
                ask_text(&win, wb_core::i18n::t("request-rework"), &wb_core::i18n::f("request-based-on-the-earlier-version", &[("where_", (where_).to_string())]), move |t| { let _ = &id2; request(&s2.borrow(), &wb_core::i18n::ai_f("rework-section-user-request-the-earlier", &[("w2", (w2).to_string()), ("t", (t).to_string()), ("cmd", (cmd).to_string())])); });
            }
        }
        "iback" | "iredo" => {
            let mut bits = value.splitn(3, ':');
            let (Some(target), Some(id), Some(k)) = (bits.next(), bits.next(), bits.next()) else { return };
            let it = s_.borrow().st.item_change_detail(&pid, id).unwrap_or_default();
            let Some(c) = list(&it, "changes").get(k.parse::<usize>().unwrap_or(usize::MAX)).cloned() else { return };
            let where_ = wb_core::i18n::f("text-4", &[("a1", (history::target_name(target)).to_string()), ("a2", (if s(&c, "kind") == "item" { wb_core::i18n::t("item") } else { s(&c, "kind") }).to_string()), ("a3", (s(&c, "id")).to_string()), ("a4", (history::label(&c).chars().take(60).collect::<String>()).to_string())]);
            let cmd = format!("`whaybench-project change-items {pid} {id}`");
            if kind == "iback" {
                request(&s_.borrow(), &wb_core::i18n::ai_f("put-back-as-it-was-before", &[("where_", (where_).to_string()), ("id", (id).to_string()), ("cmd", (cmd).to_string())]));
                over(s_, None);
            } else {
                let (s2, w2) = (s_.clone(), where_.clone());
                ask_text(&win, wb_core::i18n::t("request-rework"), &wb_core::i18n::f("request-based-on-the-earlier-version", &[("where_", (where_).to_string())]), move |t| request(&s2.borrow(), &wb_core::i18n::f("rework-user-request-the-earlier-state", &[("w2", (w2).to_string()), ("t", (t).to_string()), ("cmd", (cmd).to_string())])));
            }
        }
        _ => {}
    }
}

pub fn build(win: &gtk::ApplicationWindow, d: Data) {
    let st = Store::new(d.clone());
    let web = webkit6::WebView::new();
    crate::common::web_menu_in_language(&web);
    crate::common::follow_zoom(&web, &d);
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
    body.append(&top); body.append(&path); body.append(&tabs); body.append(&web);
    win.set_child(Some(&body));

    let s_: Shared = Rc::new(RefCell::new(P {
        d: d.clone(), st, pid: None, rows: vec![(String::new(), String::new(), usize::MAX)], tab: MAP.to_string(), over: None,
        range: Some(7), stamp: String::new(), shown: String::new(), scroll: HashMap::new(), key_now: String::new(), failed: String::new(), web: web.clone(), pick: pick.clone(), path, tabs, tab_buttons: Vec::new(), anchors: HashMap::new(), tab_keys: Vec::new(), converting: Rc::default(), win: win.clone(),
    }));
    // Links are handled here; only the page's own anchors and the drawn page itself pass.
    {
        let s2 = s_.clone();
        web.connect_decide_policy(move |_, decision, kind| {
            if kind != webkit6::PolicyDecisionType::NavigationAction && kind != webkit6::PolicyDecisionType::NewWindowAction { return false; }
            let Some(nav) = decision.downcast_ref::<webkit6::NavigationPolicyDecision>() else { return false };
            let uri = nav.navigation_action().and_then(|a| a.request()).and_then(|r| r.uri()).map(|u| u.to_string()).unwrap_or_default();
            if uri.starts_with("wb:") {
                decision.ignore();
                let s3 = s2.clone();
                glib::idle_add_local_once(move || clicked(&s3, &uri));
                return true;
            }
            if uri.starts_with("http") { decision.ignore(); let _ = wb_core::view::open(&s2.borrow().d, &uri, None); return true; }
            false
        });
    }
    {
        let s2 = s_.clone();
        pick.connect_selected_notify(move |dd| {
            let Ok(mut p) = s2.try_borrow_mut() else { return };
            let chosen = p.rows.get(dd.selected() as usize).map(|r| r.0.clone()).filter(|x| !x.is_empty());
            if chosen != p.pid { p.pid = chosen; p.stamp.clear(); p.over = None; }
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
