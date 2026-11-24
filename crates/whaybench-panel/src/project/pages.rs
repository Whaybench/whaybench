//! The project panel's pages as HTML: a document, the project map, recent changes and one change.
use super::*;

pub(super) fn e(t: &str) -> String { t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\'', "&#39;") }
pub(super) fn s<'a>(v: &'a Value, k: &str) -> &'a str { v.get(k).and_then(Value::as_str).unwrap_or("") }
pub(super) fn list<'a>(v: &'a Value, k: &str) -> &'a [Value] { v.get(k).and_then(Value::as_array).map_or(&[], Vec::as_slice) }
thread_local! {
    /// The project the page being drawn is of: every link on it names it, and a press on a page left from another
    /// project does nothing.
    pub(super) static PAGE_OF: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}
/// A link handled by the panel. Its parts travel escaped, so a section name cannot break it.
pub(super) fn link(kind: &str, rest: &str, label: &str) -> String {
    let page = PAGE_OF.with(|p| p.borrow().clone());
    format!("<a href='wb:{kind}:{}'>{label}</a>", glib::Uri::escape_string(&format!("{page}\u{1}{rest}"), None, false))
}

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

pub(super) fn style(d: &Data) -> String {
    let p = Page::of(d);
    format!("{}body{{padding:6px 12px}}\
      h1{{font-size:16pt;margin:4px 0}} h2{{font-size:14pt;margin:18px 0 6px}} h3{{font-size:12pt;margin:14px 0 4px}} h4,h5,h6{{font-size:11pt;margin:12px 0 3px}}\
      a{{color:{};text-decoration:none}} .m{{opacity:.6;font-size:small}} .ok{{color:#2e7d32;font-size:small}} .warn{{color:#c0392b}} .toc{{font-size:small}}\
      table{{margin:4px 0}} td,th{{padding:3px 6px}} pre,code{{font-size:9.5pt}}", p.base("14px/1.55"), p.accent)
}

pub(super) fn page_for(p: &mut P) -> String {
    PROJECT_PAGE.with(|c| c.set(p.bs.is_some()));
    PAGE_OF.with(|k| *k.borrow_mut() = shown_key(p));
    let Some(pid) = p.pid.clone() else { return String::new() };
    if let Some((target, event)) = &p.over { return change_page(&p.st, &pid, target, event); }
    if p.tab == RECENT { return recent_page(&p.st, &pid, p.range); }
    if p.tab == SESSIONS && p.bs.is_some() { return String::new(); }
    let project = p.st.get_project(&pid).unwrap_or_default();
    match list(&project, "docs").iter().find(|d| s(d, "id") == p.tab).cloned() {
        Some(d) => { let st = p.st.clone(); doc_page(&st, &pid, &d, &mut p.anchors) }
        None => { p.tab = MAP.to_string(); map_page(&p.st, &pid) }
    }
}
