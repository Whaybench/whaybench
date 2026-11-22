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

const MAP: &str = "map";
const RECENT: &str = "recent";

fn e(t: &str) -> String { t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\'', "&#39;") }
fn s<'a>(v: &'a Value, k: &str) -> &'a str { v.get(k).and_then(Value::as_str).unwrap_or("") }
fn list<'a>(v: &'a Value, k: &str) -> &'a [Value] { v.get(k).and_then(Value::as_array).map_or(&[], Vec::as_slice) }
/// A link handled by the panel. Its parts travel escaped, so a section name cannot break it.
fn link(kind: &str, rest: &str, label: &str) -> String { format!("<a href='wb:{kind}:{}'>{label}</a>", glib::Uri::escape_string(rest, None, false)) }

fn when(ms: i64) -> String {
    if ms == 0 { return String::new(); }
    wb_core::clock::local(ms, "%m-%d %H:%M")
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
thread_local! {
    /// The page drawn now is a Briefsafe project's: its own HTML is shown as text and its links lead nowhere.
    static PROJECT_PAGE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
fn render(text: &str) -> String {
    if PROJECT_PAGE.with(|c| c.get()) {
        // A project's text is its agents' to write: no markup of its own, links only to web pages, no images fetched.
        return crate::web::markdown(text, crate::web::RawHtml::Untrusted);
    }
    crate::web::markdown(text, crate::web::RawHtml::Kept)
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
    let mut mine_marks: Vec<(String, String, String, String)> = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        if let Some((level, title)) = at_head.get(&n) {
            let path = md::path_at(&heads, n).join(" > ");
            let anchor = format!("h{n}");
            mine.insert(path.clone(), anchor.clone());
            if *level > 1 { toc.push(format!("{}<a href='#{anchor}'>{}</a>", "&nbsp;".repeat(4 * (level - 2)), e(title))); }
            let ask = if *level > 1 { format!(" <small>{}</small>", link("req", &format!("{did}:{n}"), wb_core::i18n::t("request-change"))) } else { String::new() };
            // The panel's own markup goes in after the Markdown is drawn: in the text it would be the document's (and
            // shown as text for a Briefsafe project). A marker of characters no Markdown makes stands for it.
            mine_marks.push((format!("\u{E000}{n}\u{E000}"), format!("<a id='{anchor}'></a>"), format!("\u{E001}{n}\u{E001}"), ask));
            out.push(format!("{} \u{E000}{n}\u{E000} {} \u{E001}{n}\u{E001}", "#".repeat(*level), title));
            continue;
        }
        if hide.guide.is_match(line) || hide.source.is_match(line) { continue; }
        out.push(line.to_string());
    }
    anchors.insert(did.to_string(), mine);
    let mut head = wb_core::i18n::f("lines-written-by-ai", &[("a1", (lines.len()).to_string()), ("a2", (link("dreq", did, wb_core::i18n::t("request-change-to-this-document"))).to_string())]);
    if !s(d, "original").is_empty() { head += &wb_core::i18n::f("original", &[("a1", (e(s(d, "original"))).to_string())]); }
    head += &wb_core::i18n::f("contents", &[("a1", (toc.join("<br>")).to_string())]);
    let mut body = render(&out.join("\n"));
    // One pass over the page (a document of many headings must not take long).
    let marks: HashMap<String, String> = mine_marks.into_iter().flat_map(|(m, h, am, a)| [(m, h), (am, a)]).collect();
    if !marks.is_empty() {
        let re = Regex::new("[\u{E000}\u{E001}][0-9]+[\u{E000}\u{E001}]").unwrap();
        body = re.replace_all(&body, |c: &regex::Captures| marks.get(&c[0]).cloned().unwrap_or_default()).into_owned();
    }
    format!("{head}<div id='body'></div>{body}")
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
    for f in wb_core::paths::list_dir(dir) { n += 1; size += wb_core::paths::meta(&f).map(|m| m.len()).unwrap_or(0); }
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
                .map(|x| format!("<tr><td>{}</td><td>{}</td><td>{}</td></tr>", e(&name(x)), val(x, a.get(x.as_str())), val(x, b.get(x.as_str())))).collect();
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
    let p = Page::of(d);
    format!("{}body{{padding:6px 12px}}\
      h1{{font-size:16pt;margin:4px 0}} h2{{font-size:14pt;margin:18px 0 6px}} h3{{font-size:12pt;margin:14px 0 4px}} h4,h5,h6{{font-size:11pt;margin:12px 0 3px}}\
      a{{color:{};text-decoration:none}} .m{{opacity:.6;font-size:small}} .ok{{color:#2e7d32;font-size:small}} .warn{{color:#c0392b}} .toc{{font-size:small}}\
      table{{margin:4px 0}} td,th{{padding:3px 6px}} pre,code{{font-size:9.5pt}}", p.base("14px/1.55"), p.accent)
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
    /// The host's own projects (`st` is a Briefsafe project's store while one is shown and open).
    host: Store,
    /// The Briefsafe project on show (spec 3.22), by id.
    bs: Option<String>,
    /// A `whaybench-project` step running (log in, log out, snapshot), for one project.
    job: Option<Job>,
    /// What the last step of a project said when it did not work: its project, the kind of step and the reason.
    bs_msg: HashMap<String, (String, String)>,
}
type Shared = Rc<RefCell<P>>;

fn stamp_of(st: &Store, pid: &str) -> String {
    let base = st.pdir(pid);
    ["events", "map.json", "project.json"].iter().map(|f| wb_core::paths::meta(&base.join(f)).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |t| t.as_nanos()).to_string()).collect::<Vec<_>>().join(",")
}

/// A step of `whaybench-project` running in the background for one Briefsafe project.
struct Job {
    id: String,
    /// open, recover, close or snapshot.
    kind: String,
    /// What the page says while it runs.
    what: String,
    child: std::process::Child,
    started: std::time::Instant,
    /// Its output, in files: a pipe filled up (podman's download progress) would stop it.
    out: std::path::PathBuf,
    err: std::path::PathBuf,
}

/// Briefsafe projects are listed by this key, apart from the host's projects' ids.
const BS: &str = "bs:";

fn stub_of(d: &Data, id: &str) -> Option<wb_core::roots::Stub> { wb_core::roots::stubs(d).into_iter().find(|s| s.id == id) }

/// Shows the project of this list key: a host project, or a Briefsafe one (its store only once it is open).
fn choose(p: &mut P, key: Option<String>) {
    match key.as_deref().and_then(|k| k.strip_prefix(BS)) {
        // The host's store until the next look finds it open: never another project's pages under this name.
        Some(id) => { p.bs = Some(id.to_string()); p.pid = Some(id.to_string()); p.st = p.host.clone(); }
        None => { p.bs = None; p.pid = key; p.st = p.host.clone(); }
    }
    p.stamp.clear();
    p.over = None;
}

/// Runs a step of `whaybench-project` for a Briefsafe project in the background (keyring and podman can take long);
/// `input` goes to its standard input (a recovery key), never on its command line. One at a time: another is said.
fn start_job(p: &mut P, id: &str, kind: &str, what: &str, args: &[&str], input: Option<String>) {
    if let Some(j) = &p.job { status::say(&p.d, &wb_core::i18n::f("briefsafe-busy", &[("what", j.what.clone())]), 5); return; }
    // Said when it cannot even start (no program, a full disk), not left as a press that did nothing.
    let fail = |p: &mut P, why: String| { p.bs_msg.insert(id.into(), (kind.into(), why)); };
    let Ok(exe) = std::env::current_exe() else { return fail(p, "whaybench-project not found".into()) };
    let dir = p.d.cache().join("project-jobs");
    let _ = std::fs::create_dir_all(&dir);
    let (out, err) = (dir.join(format!("{id}.out")), dir.join(format!("{id}.err")));
    let files = std::fs::File::create(&out).and_then(|o| std::fs::File::create(&err).map(|e| (o, e)));
    let (o, e) = match files { Ok(f) => f, Err(e) => return fail(p, e.to_string()) };
    let mut c = std::process::Command::new(exe.with_file_name("whaybench-project"));
    // Its own process group: stopped past its time, what it started (podman, gocryptfs) goes with it.
    std::os::unix::process::CommandExt::process_group(&mut c, 0);
    c.args(args).env(wb_core::paths::ENV_DATA_DIR, &p.d.root).stdin(std::process::Stdio::piped()).stdout(o).stderr(e);
    match c.spawn() {
        Ok(mut child) => {
            if let Some(mut i) = child.stdin.take() { let _ = std::io::Write::write_all(&mut i, input.unwrap_or_default().as_bytes()); }
            p.job = Some(Job { id: id.into(), kind: kind.into(), what: what.into(), child, started: std::time::Instant::now(), out, err });
            // Another project's message stays for it.
            p.bs_msg.remove(id);
            p.stamp.clear();
        }
        Err(e) => fail(p, e.to_string()),
    }
}

/// A finished step: what it said, kept for its project. A step past its time (a keyring that never answers, an image
/// that never comes) is stopped and said so. True when one finished.
fn poll_job(p: &mut P) -> bool {
    let Some(job) = p.job.as_mut() else { return false };
    let limit = if job.kind == "open" || job.kind == "recover" { 600 } else { 120 };
    let done = matches!(job.child.try_wait(), Ok(Some(_)));
    let late = !done && job.started.elapsed().as_secs() > limit;
    if !done && !late { return false; }
    let Some(mut job) = p.job.take() else { return false };
    if late {
        // Safety: kill on the process group the job leads (made with process_group(0)); its own id is that group's.
        unsafe { libc::kill(-(job.child.id() as i32), libc::SIGKILL) };
        // A snapshot stopped half way would leave the project's container paused.
        // Off the panel's thread: podman may be what is slow.
        if job.kind == "snapshot" && let Some(st) = stub_of(&p.d, &job.id) { std::thread::spawn(move || wb_core::container::unpause(&st)); }
        // A log in stopped half way may leave the vault open behind a locked page: closed again.
        if (job.kind == "open" || job.kind == "recover") && let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe.with_file_name("whaybench-project")).args(["close", &job.id]).env(wb_core::paths::ENV_DATA_DIR, &p.d.root)
                .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
        }
    }
    let status = job.child.wait();
    let ok = !late && status.is_ok_and(|st| st.success());
    let said = std::fs::read_to_string(&job.err).unwrap_or_default().trim().trim_start_matches("error: ").to_string();
    let out: Value = std::fs::read_to_string(&job.out).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let _ = (std::fs::remove_file(&job.out), std::fs::remove_file(&job.err));
    if late { p.bs_msg.insert(job.id.clone(), (job.kind.clone(), wb_core::i18n::t("briefsafe-too-long").to_string())); }
    else if !ok { p.bs_msg.insert(job.id.clone(), (job.kind.clone(), said)); }
    else if job.kind == "snapshot" {
        let warn = s(&out, "warning");
        status::say(&p.d, &if warn.is_empty() { wb_core::i18n::t("briefsafe-snapshot-taken").to_string() } else { wb_core::i18n::f("briefsafe-snapshot-warning", &[("why", warn.to_string())]) }, 8);
    }
    p.stamp.clear();
    true
}

/// What the last step of this project said, if it did not work.
fn msg_for<'a>(p: &'a P, id: &str) -> Option<(&'a str, &'a str)> { p.bs_msg.get(id).map(|m| (m.0.as_str(), m.1.as_str())) }

/// A step for this project is running, and it hides its content (logging in).
fn opening(p: &P, id: &str) -> bool { p.job.as_ref().is_some_and(|j| j.id == id && (j.kind == "open" || j.kind == "recover")) }

/// The page of a Briefsafe project that is not open: locked, a step running, or why the last one did not work.
fn closed_page(p: &P, st: &wb_core::roots::Stub) -> String {
    let t = |k: &str| wb_core::i18n::t(k).to_string();
    let name = e(&st.name);
    let body = if let Some(j) = p.job.as_ref().filter(|j| j.id == st.id) {
        format!("<h1>{name}</h1><p>{}</p>", e(&wb_core::i18n::f("briefsafe-working", &[("what", j.what.clone())])))
    } else {
        let mut b = format!("<h1>{name}</h1><p>{}</p>", e(&wb_core::i18n::f("briefsafe-locked-page", &[("name", st.name.clone())])));
        match msg_for(p, &st.id) {
            // No key: said in the person's words, with the way on (and still there after a recovery key that did not fit).
            Some((kind, why)) if st.vault && (why.contains(wb_core::vault::NO_KEY) || kind == "recover" || (kind == "open" && why.contains("gocryptfs"))) => {
                if kind == "recover" { b += &format!("<p class='warn'>{}</p>", e(&wb_core::i18n::f("briefsafe-failed", &[("why", why.to_string())]))); }
                b += &format!("<p>{}</p><p>{}</p>", e(&t("briefsafe-no-key-page")), link("bs", &format!("{}:recover", st.id), &e(&t("briefsafe-enter-recovery"))));
            }
            Some((_, why)) => b += &format!("<p class='warn'>{}</p>", e(&wb_core::i18n::f("briefsafe-failed", &[("why", why.to_string())]))),
            None => {}
        }
        b += &format!("<p>{}</p>", link("bs", &format!("{}:open", st.id), &e(&t("briefsafe-login"))));
        b
    };
    format!("<!doctype html><meta charset='utf-8'><style>{}</style><body>{body}</body>", style(&p.d))
}

/// Over an open Briefsafe project's pages: its conversations and what can be done with it.
/// The conversations of a Briefsafe project, newest first (read once per look).
fn project_sessions(d: &Data, id: &str) -> Vec<Value> {
    let roots = wb_core::roots::Roots::new(d.clone());
    let mut mine: Vec<Value> = sessions::all(d).into_iter().filter(|r| roots.project_of(s(r, "id")).as_deref() == Some(id)).collect();
    mine.sort_by_key(|r| std::cmp::Reverse(at(r, "lastActive")));
    mine
}

fn open_bar(p: &P, st: &wb_core::roots::Stub) -> String {
    let t = |k: &str| e(wb_core::i18n::t(k));
    let mine: Vec<Value> = project_sessions(&p.d, &st.id).into_iter().filter(|r| s(r, "movedTo").is_empty() && r["ended"] != true).collect();
    let rows: Vec<String> = mine.iter().take(12).map(|r| format!("<li>{}</li>", link("bs", &format!("{}:resume:{}", st.id, s(r, "id")), &e(&sessions::session_name(&p.d, s(r, "id")))))).collect();
    let list = if rows.is_empty() { format!("<p class='m'>{}</p>", t("briefsafe-no-sessions")) } else { format!("<ul>{}</ul>", rows.join("")) };
    // A step running for it (a snapshot, logging out), or why the last one did not work: said over its pages.
    let note = match (p.job.as_ref().filter(|j| j.id == st.id), msg_for(p, &st.id)) {
        (Some(j), _) => format!("<p class='m'>{}</p>", e(&wb_core::i18n::f("briefsafe-working", &[("what", j.what.clone())]))),
        (None, Some((_, why))) => format!("<p class='warn'>{}</p>", e(&wb_core::i18n::f("briefsafe-failed", &[("why", why.to_string())]))),
        _ => String::new(),
    };
    // Opened without a container (no podman or image for this system): said, its agents run as the person.
    let note = if wb_core::container::opened_isolated(st) { note } else { format!("{note}<p class='m'>{}</p>", t("briefsafe-not-isolated")) };
    format!("{note}<p>{} · {} · {}</p><h3>{}</h3>{list}<hr>", link("bs", &format!("{}:new", st.id), &t("briefsafe-new-conversation")), link("bs", &format!("{}:snapshot", st.id), &t("briefsafe-snapshot")), link("bs", &format!("{}:close", st.id), &t("briefsafe-logout")), t("briefsafe-sessions"))
}

/// A request for the chosen session's AI; only to one that is running.
fn request(p: &P, ask: &str) {
    let Some(pid) = &p.pid else { return };
    let Some(session) = sessions::focus(&p.d) else { p.path.set_text(wb_core::i18n::t("request-not-sent-no-session-selected")); return };
    // A Briefsafe project's page asks only that project's own conversations: its text never goes to a host agent.
    if let Some(id) = &p.bs && wb_core::roots::Roots::new(p.d.clone()).project_of(&session).as_deref() != Some(id.as_str()) {
        p.path.set_text(wb_core::i18n::t("briefsafe-request-own-only")); return;
    }
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
    let base = if slug.is_empty() { format!("p-{}", (wb_core::clock::now_ms() / 1000)) } else { slug };
    if st.get_project(&base).is_none() { return base; }
    (2..).map(|n| format!("{base}-{n}")).find(|x| st.get_project(x).is_none()).unwrap()
}

/// The documents to show: those added as add-ons. Older ones without a template stay on file, out of sight.
/// Every document of the project gets a tab: it is Markdown whatever made it (a template add-on, or a format the
/// program no longer has).
/// An archived one (a finished change proposal) has none; it is opened from the + menu.
fn shown_docs(project: &Value) -> Vec<Value> { list(project, "docs").iter().filter(|d| !archived(d)).cloned().collect() }
fn archived(d: &Value) -> bool { d["archived"] == true }

fn page_for(p: &mut P) -> String {
    PROJECT_PAGE.with(|c| c.set(p.bs.is_some()));
    let Some(pid) = p.pid.clone() else { return String::new() };
    if let Some((target, event)) = &p.over { return change_page(&p.st, &pid, target, event); }
    if p.tab == RECENT { return recent_page(&p.st, &pid, p.range); }
    let project = p.st.get_project(&pid).unwrap_or_default();
    match list(&project, "docs").iter().find(|d| s(d, "id") == p.tab).cloned() {
        Some(d) => { let st = p.st.clone(); doc_page(&st, &pid, &d, &mut p.anchors) }
        None => { p.tab = MAP.to_string(); map_page(&p.st, &pid) }
    }
}

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
        let Some(project) = p.st.get_project(&pid) else {
            // Its record is missing or unreadable (a Briefsafe project's is its agents' to change): said, nothing else shown.
            let page = format!("<!doctype html><meta charset='utf-8'><style>{}</style><body><p class='warn'>{}</p></body>", style(&p.d), e(wb_core::i18n::t("briefsafe-unreadable")));
            if p.shown != page { p.web.load_html(&page, Some("about:blank")); p.shown = page; }
            drop(p);
            set_tabs(s_, Vec::new());
            return;
        };
        let up: Vec<String> = p.st.ancestors(&pid).iter().map(|a| s(a, "name").to_string()).chain(std::iter::once(s(&project, "name").to_string())).collect();
        let kids = p.st.children(&pid).len();
        p.path.set_text(&format!("{}{}", up.join(" › "), if kids > 0 { wb_core::i18n::f("below", &[("kids", (kids).to_string())]) } else { String::new() }));
        let mut body = page_for(&mut p);
        if let Some(st) = p.bs.clone().and_then(|id| stub_of(&p.d, &id)) { body = open_bar(&p, &st) + &body; }
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
    let drawn_for = { let p = s_.borrow(); (p.pid.clone(), p.bs.clone()) };
    web.evaluate_javascript("window.scrollY", None, None, None::<&gtk::gio::Cancellable>, move |r| {
        let mut p = s2.borrow_mut();
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
                // Always the host's own (a Briefsafe project shown has its own store in `st`).
                let pid = new_project_id(&p.host, &name);
                p.host.create_project(&pid, &name, "", "", Vec::new(), false)?;
                if let Some(src) = &source {
                    let (nodes, title, from) = mi::read(&d2, src).ok_or_else(|| wb_core::i18n::t("import-failed-no-map").to_string())?;
                    let path = mi::keep(&d2, &nodes, &title, &from).map_err(|e| e.to_string())?;
                    p.host.map_from_nodes(&pid, &nodes, &title, &path.to_string_lossy())?;
                }
                Ok(pid)
            })();
            match result {
                Ok(pid) => {
                    record::decide("project", "new project", "the person named it", &[("project", pid.clone()), ("imported_map", source.is_some().to_string())]);
                    { let mut p = s2.borrow_mut(); choose(&mut p, Some(pid)); p.tab = MAP.to_string(); p.rows.clear(); }
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
                    p.path.set_text(&format!("{} · {}", st.name, wb_core::i18n::t("briefsafe-locked")));
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
    if let Some(page) = locked {
        set_tabs(s_, Vec::new());
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
                // Its conversations come and go too, and a step's state.
                let talk = if p.bs.is_some() {
                    let job = p.job.as_ref().map(|j| format!("{}{}", j.id, j.kind)).unwrap_or_default();
                    format!("{job}{:?}{}", p.bs_msg.get(&pid), project_sessions(&p.d, &pid).iter().map(|r| format!("{}{}", s(r, "id"), at(r, "lastActive"))).collect::<Vec<_>>().join(","))
                } else { String::new() };
                let now = format!("{}{}{talk}", stamp_of(&p.st, &pid), style(&p.d));
                if now == p.stamp { false } else { p.stamp = now; true }
            }
        }
    };
    if due { redraw(s_); }
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
            ask_text(&win, wb_core::i18n::t("request-change-2"), &wb_core::i18n::f("request", &[("where_", (where_).to_string())]), move |t| {
                let source = if original.is_empty() { String::new() } else { wb_core::i18n::ai_f("original-of-the-template", &[("original", (original).to_string())]) };
                if s2.borrow().bs != bs_then { return; }
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
                let bs_then = s_.borrow().bs.clone();
                ask_text(&win, wb_core::i18n::t("request-rework"), &wb_core::i18n::f("request-based-on-the-earlier-version", &[("where_", (where_).to_string())]), move |t| { let _ = &id2; if s2.borrow().bs != bs_then { return; } request(&s2.borrow(), &wb_core::i18n::ai_f("rework-section-user-request-the-earlier", &[("w2", (w2).to_string()), ("t", (t).to_string()), ("cmd", (cmd).to_string())])); });
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
                let bs_then = s_.borrow().bs.clone();
                ask_text(&win, wb_core::i18n::t("request-rework"), &wb_core::i18n::f("request-based-on-the-earlier-version", &[("where_", (where_).to_string())]), move |t| { if s2.borrow().bs != bs_then { return; } request(&s2.borrow(), &wb_core::i18n::f("rework-user-request-the-earlier-state", &[("w2", (w2).to_string()), ("t", (t).to_string()), ("cmd", (cmd).to_string())])); });
            }
        }
        _ => {}
    }
}

/// A Briefsafe project's step from its page: log in or out, a snapshot, a conversation (spec 3.22).
fn briefsafe_clicked(s_: &Shared, what: &str) {
    let (d, win, id) = { let p = s_.borrow(); (p.d.clone(), p.win.clone(), p.bs.clone()) };
    let Some(id) = id else { return };
    // A link names the project its page was drawn for: pressed on a page left from another project, it does nothing.
    let Some((for_id, what)) = what.split_once(':') else { return };
    if for_id != id { return; }
    let t = |k: &str| wb_core::i18n::t(k).to_string();
    match what {
        "open" => start_job(&mut s_.borrow_mut(), &id, "open", &t("briefsafe-opening"), &["open", &id], None),
        "recover" => {
            let s2 = s_.clone();
            let id2 = id.clone();
            ask_text(&win, &t("briefsafe-enter-recovery"), &t("briefsafe-recovery-label"), move |key| {
                start_job(&mut s2.borrow_mut(), &id2, "recover", wb_core::i18n::t("briefsafe-opening"), &["open", &id2, "--recovery"], Some(format!("{key}\n")));
            });
        }
        "snapshot" => {
            if s_.borrow().job.is_none() { status::say(&d, wb_core::i18n::t("briefsafe-snapshotting"), 6); }
            start_job(&mut s_.borrow_mut(), &id, "snapshot", &t("briefsafe-snapshotting"), &["snapshot", &id], None);
        }
        "close" => {
            // Its conversations close with it: said first when some are open.
            let tabs = sessions::open_tabs(&d).iter().filter(|tab| sessions::tab_project(&d, &tab.tab).as_deref() == Some(id.as_str())).count();
            let s2 = s_.clone();
            let id2 = id.clone();
            let go = move || start_job(&mut s2.borrow_mut(), &id2, "close", wb_core::i18n::t("briefsafe-closing"), &["close", &id2], None);
            if tabs == 0 { go(); } else { crate::common::confirm(&win, &wb_core::i18n::f("briefsafe-logout-confirm", &[("n", tabs.to_string())]), &[], &t("briefsafe-logout"), go); }
        }
        "new" => crate::common::ctl(&d, vec!["--data-dir".into(), d.root.to_string_lossy().to_string(), "session".into(), "new".into(), "--project".into(), id]),
        other => {
            // Its own conversation only, opened as the project's (refused there if it has closed meanwhile).
            if let Some(sid) = other.strip_prefix("resume:") && wb_core::map::safe_id(sid).is_ok()
                && wb_core::roots::Roots::new(d.clone()).project_of(sid).as_deref() == Some(id.as_str()) {
                crate::common::ctl(&d, vec!["--data-dir".into(), d.root.to_string_lossy().to_string(), "session".into(), "show".into(), "--project".into(), id, sid.to_string()]);
            }
        }
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
    body.append(&top); body.append(&path); body.append(&tabs); body.append(&web);
    win.set_child(Some(&body));

    let s_: Shared = Rc::new(RefCell::new(P {
        d: d.clone(), st, pid: None, rows: vec![(String::new(), String::new(), usize::MAX)], tab: MAP.to_string(), over: None,
        range: Some(7), stamp: String::new(), shown: String::new(), scroll: HashMap::new(), key_now: String::new(), failed: String::new(), web: web.clone(), pick: pick.clone(), path, tabs, tab_buttons: Vec::new(), anchors: HashMap::new(), tab_keys: Vec::new(), converting: Rc::default(), win: win.clone(),
        host, bs: None, job: None, bs_msg: HashMap::new(),
    }));
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
