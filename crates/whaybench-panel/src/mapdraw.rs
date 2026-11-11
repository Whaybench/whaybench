//! Draws the conversation map in a style's frame, from the map data alone: no model is involved. Every node
//! element carries its id, and a press on it comes back as a link the panel handles, the same as on the tree.
use std::collections::{HashMap, HashSet};

use chrono::TimeZone;
use serde_json::Value;
use wb_core::{paths::Data, theme};
use wb_store::{arr, mapstyles::{self, Style}, s};

const LANES: [&str; 10] = ["#1e66f5", "#d20f39", "#8839ef", "#40a02b", "#fe640b", "#179299", "#df8e1d", "#ea76cb", "#7287fd", "#9ca0b0"];

fn e(t: &str) -> String { t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\'', "&#39;").replace('"', "&quot;") }
fn cut(t: &str, n: usize) -> String { t.chars().take(n).collect() }
fn is_open(n: &Value) -> bool { n.get("isOpen") != Some(&Value::Bool(false)) && s(n, "state") != "done" }

fn kind_of(k: &str) -> Option<(&'static str, &'static str)> {
    Some(match k { "question" => (wb_core::i18n::t("question"), "#8839ef"), "option" => (wb_core::i18n::t("option"), "#7c7f93"), "pro" => ("+", "#40a02b"), "con" => ("−", "#d20f39"), "conclusion" => (wb_core::i18n::t("conclusion"), "#1e66f5"), "note" => (wb_core::i18n::t("note"), "#9ca0b0"), "idea" => (wb_core::i18n::t("idea"), "#ea76cb"), "task" => (wb_core::i18n::t("kind-task"), "#fe640b"), "appointment" => (wb_core::i18n::t("kind-appointment"), "#179299"), "problem" => (wb_core::i18n::t("kind-problem"), "#e64553"), "fact" => (wb_core::i18n::t("kind-fact"), "#5c5f77"), "rule" => (wb_core::i18n::t("kind-rule"), "#df8e1d"), _ => return None })
}

fn glyph(n: &Value, current: &str) -> &'static str {
    if s(n, "id") == current { return "●"; }
    match s(n, "state") { "plan" => "□", "doing" => "◐", "done" => "✓", "changed" => "✎", _ => if n.get("isOpen") != Some(&Value::Bool(false)) { "◇" } else { "✓" } }
}

struct Tree<'a> { kids: HashMap<&'a str, Vec<&'a Value>>, by: HashMap<&'a str, &'a Value> }

fn tree_of(nodes: &[Value]) -> Tree<'_> {
    let by: HashMap<&str, &Value> = nodes.iter().map(|n| (s(n, "id"), n)).collect();
    let mut kids: HashMap<&str, Vec<&Value>> = HashMap::new();
    for n in nodes { kids.entry(if by.contains_key(s(n, "parent")) { s(n, "parent") } else { "" }).or_default().push(n); }
    Tree { kids, by }
}

fn label_line(n: &Value, current: &str, with_kind: bool) -> (&'static str, String) {
    let badge = kind_of(s(n, "kind")).filter(|_| with_kind).map(|(t, c)| format!("<span class='k' style='background:{c}'>{t}</span>")).unwrap_or_default();
    let class = if s(n, "id") == current { "here" } else if is_open(n) { "open" } else { "shut" };
    (class, format!("<span class='g'>{}</span>{badge}<span class='t' data-n='{}'>{}</span>", glyph(n, current), e(s(n, "id")), e(s(n, "label"))))
}

fn frame_tree(nodes: &[Value], current: &str, style: &Style) -> String {
    if s(&style.raw, "filter") == "path" { return frame_focus(nodes, current); }
    let show = style.raw.get("show_kind") == Some(&Value::Bool(true));
    let t = tree_of(nodes);
    fn item(n: &Value, t: &Tree, current: &str, show: bool) -> String {
        let (class, line) = label_line(n, current, show);
        let inner: String = t.kids.get(s(n, "id")).into_iter().flatten().map(|c| item(c, t, current, show)).collect();
        format!("<li class='{class}'>{line}{}</li>", if inner.is_empty() { String::new() } else { format!("<ul>{inner}</ul>") })
    }
    format!("<ul class='tree'>{}</ul>", t.kids.get("").into_iter().flatten().map(|n| item(n, &t, current, show)).collect::<String>())
}

/// The way to where the conversation is: the path down to the current node, the open nodes under each step, every other
/// top branch as one line.
fn frame_focus(nodes: &[Value], current: &str) -> String {
    let t = tree_of(nodes);
    let on_path: HashSet<String> = mapstyles::path_to(nodes, current).iter().map(|n| s(n, "id").to_string()).collect();
    fn tally(n: &Value, t: &Tree) -> usize {
        t.kids.get(s(n, "id")).into_iter().flatten().map(|c| usize::from(is_open(c)) + tally(c, t)).sum()
    }
    fn step(n: &Value, t: &Tree, current: &str, on_path: &HashSet<String>) -> String {
        let (class, line) = label_line(n, current, true);
        let sub = t.kids.get(s(n, "id")).cloned().unwrap_or_default();
        let mut inner = String::new();
        for c in &sub {
            if on_path.contains(s(c, "id")) { inner += &step(c, t, current, on_path); }
            else if is_open(c) {
                let (cc, cl) = label_line(c, current, true);
                let o = tally(c, t);
                inner += &format!("<li class='{cc}'>{cl}{}</li>", if o > 0 { wb_core::i18n::f("open-below", &[("o", (o).to_string())]) } else { String::new() });
            }
        }
        let hidden = sub.iter().filter(|c| !on_path.contains(s(c, "id")) && !is_open(c)).count();
        if hidden > 0 { inner += &wb_core::i18n::f("done-folded", &[("hidden", (hidden).to_string())]); }
        format!("<li class='{class}'>{line}{}</li>", if inner.is_empty() { String::new() } else { format!("<ul>{inner}</ul>") })
    }
    let mut rows = String::new();
    for top in t.kids.get("").into_iter().flatten() {
        if on_path.contains(s(top, "id")) { rows += &step(top, &t, current, &on_path); continue; }
        let o = tally(top, &t) + usize::from(is_open(top));
        rows += &wb_core::i18n::f("open-2", &[("a1", (if o > 0 { "open" } else { "shut" }).to_string()), ("a2", (if o > 0 { "◇" } else { "✓" }).to_string()), ("a3", (e(s(top, "id"))).to_string()), ("a4", (e(&cut(s(top, "label"), 40))).to_string()), ("o", (o).to_string())]);
    }
    format!("{}<ul class='tree'>{rows}</ul>", if on_path.is_empty() { wb_core::i18n::t("no-current-position") } else { "" })
}

/// Open questions first with their options and arguments, settled ones as one line, the rest folded.
fn frame_questions(nodes: &[Value], current: &str) -> String {
    let t = tree_of(nodes);
    let discuss = |n: &Value| ["question", "option", "pro", "con", "conclusion"].contains(&s(n, "kind"));
    fn branch(n: &Value, t: &Tree, current: &str, discuss: &dyn Fn(&Value) -> bool) -> String {
        let (class, line) = label_line(n, current, true);
        let kids = t.kids.get(s(n, "id")).cloned().unwrap_or_default();
        let mut inner: String = kids.iter().filter(|c| discuss(c)).map(|c| branch(c, t, current, discuss)).collect();
        let others = kids.iter().filter(|c| !discuss(c)).count();
        if others > 0 { inner += &wb_core::i18n::f("notes-and-tasks", &[("others", (others).to_string())]); }
        format!("<li class='{class}'>{line}{}</li>", if inner.is_empty() { String::new() } else { format!("<ul>{inner}</ul>") })
    }
    let open_q: HashSet<&str> = nodes.iter().filter(|n| s(n, "kind") == "question" && is_open(n)).map(|n| s(n, "id")).collect();
    let under_open = |n: &Value| {
        let (mut p, mut seen) = (s(n, "parent"), HashSet::new());
        while let Some(u) = t.by.get(p) {
            if open_q.contains(p) { return true; }
            if !seen.insert(p) { break; }
            p = s(u, "parent");
        }
        false
    };
    let groups: Vec<&Value> = nodes.iter().filter(|n| open_q.contains(s(n, "id")) && !under_open(n)).collect();
    let settled: Vec<&Value> = nodes.iter().filter(|n| !is_open(n) && !under_open(n) && (s(n, "kind") == "question" || (s(n, "kind") == "conclusion" && t.by.get(s(n, "parent")).is_none_or(|p| s(p, "kind") != "question")))).collect();
    let rest: Vec<&Value> = nodes.iter().filter(|n| !discuss(n)).collect();
    let settled_line = |n: &&Value| {
        let answers: Vec<String> = t.kids.get(s(n, "id")).into_iter().flatten().filter(|c| s(c, "kind") == "conclusion").map(|c| e(s(c, "label"))).collect();
        let tail = if s(n, "kind") == "question" && !answers.is_empty() { format!(" → {}", answers.join(" / ")) } else { String::new() };
        format!("<li class='shut'><span class='g'>✓</span><span class='t' data-n='{}'>{}{tail}</span></li>", e(s(n, "id")), e(s(n, "label")))
    };
    wb_core::i18n::f("open-questions-settled-notes-tasks-no", &[("a1", (groups.len()).to_string()), ("a2", (groups.iter().map(|n| branch(n, &t, current, &discuss)).collect::<String>()).to_string()), ("a3", (settled.len()).to_string()), ("a4", (settled.iter().map(settled_line).collect::<String>()).to_string()), ("a5", (rest.len()).to_string()), ("a6", (rest.iter().map(|n| { let (c, l) = label_line(n, current, true); format!("<li class='{c}'>{l}</li>") }).collect::<String>()).to_string())])
}

fn state_column(n: &Value) -> &'static str {
    let open = n.get("isOpen") != Some(&Value::Bool(false));
    match s(n, "state") { "doing" => "doing", "plan" if open => "plan", "done" | "changed" => "done", _ if !open => "done", _ => "idea" }
}

fn column_name(id: &str) -> &str { match id { "idea" => wb_core::i18n::t("idea"), "plan" => wb_core::i18n::t("planned"), "doing" => wb_core::i18n::t("in-progress"), "done" => wb_core::i18n::t("done"), o => o } }

fn frame_columns(nodes: &[Value], current: &str, style: &Style) -> String {
    let t = tree_of(nodes);
    let by = if s(&style.raw, "group").is_empty() { "state" } else { s(&style.raw, "group") };
    let top = |n: &Value| {
        let (mut at, mut seen) = (s(n, "id"), HashSet::new());
        while let Some(p) = t.by.get(at).map(|x| s(x, "parent")).filter(|p| t.by.contains_key(p)) { if !seen.insert(at) { break; } at = p; }
        s(t.by[at], "label").to_string()
    };
    let mut cols: Vec<(String, Vec<&Value>)> = arr(&style.raw, "columns").iter().filter_map(Value::as_str).map(|c| (c.to_string(), Vec::new())).collect();
    for n in nodes {
        let col = match by { "kind" => kind_of(s(n, "kind")).map_or(wb_core::i18n::t("no-kind"), |k| k.0).to_string(), "top" => top(n), _ => state_column(n).to_string() };
        match cols.iter_mut().find(|c| c.0 == col) { Some(c) => c.1.push(n), None => cols.push((col, vec![n])) }
    }
    let recent = style.raw.get("recent_done").and_then(Value::as_u64).unwrap_or(0) as usize;
    if recent > 0 && by == "state" {
        if let Some(c) = cols.iter_mut().find(|c| c.0 == "done") {
            let stamp = |n: &Value| n.get("closed").or_else(|| n.get("changed")).and_then(|x| x.get("turn")).and_then(Value::as_i64).unwrap_or(-1);
            c.1.sort_by_key(|n| stamp(n));
            let extra = c.1.len().saturating_sub(recent);
            c.1.drain(..extra);
        }
    }
    let body: String = cols.iter().map(|(name, cards)| {
        let rows: String = cards.iter().map(|n| {
            let path = t.by.get(s(n, "parent")).map_or(wb_core::i18n::t("top-level").to_string(), |p| format!("{} ›", e(&cut(s(p, "label"), 28))));
            format!("<div class='card{}' data-n='{}'><div class='path'>{path}</div>{}</div>", if s(n, "id") == current { " here" } else { "" }, e(s(n, "id")), e(s(n, "label")))
        }).collect();
        format!("<section class='col'><h3>{} <span class='n'>{}</span></h3>{rows}</section>", e(if by == "state" { column_name(name) } else { name }), cards.len())
    }).collect();
    format!("<div class='cols' style='grid-template-columns:repeat({},minmax(130px,1fr))'>{body}</div>", cols.len().max(1))
}

fn numbers(n: &Value, k: &str) -> Vec<i64> { arr(n, k).iter().filter_map(Value::as_i64).collect() }

fn frame_route(nodes: &[Value], current: &str) -> String {
    let t = tree_of(nodes);
    let mut visits: Vec<(i64, &str)> = nodes.iter().flat_map(|n| numbers(n, "visits").into_iter().map(move |v| (v, s(n, "id")))).collect();
    visits.sort();
    let mut stops: Vec<String> = Vec::new();
    for (_, id) in visits { if stops.last().map(String::as_str) != Some(id) { stops.push(id.to_string()); } }
    let mut note = String::new();
    if t.by.contains_key(current) && stops.last().map(String::as_str) != Some(current) {
        if stops.is_empty() {
            note = wb_core::i18n::t("no-visit-record-showing-the-tree").to_string();
            stops = mapstyles::path_to(nodes, current).iter().map(|n| s(n, "id").to_string()).collect();
        } else { stops.push(current.to_string()); }
    }
    let last = stops.len().saturating_sub(1);
    let mut rows = String::new();
    for (i, id) in stops.iter().enumerate() {
        let Some(n) = t.by.get(id.as_str()) else { continue };
        let dest = id == current && i == last;
        let role = if i == 0 { wb_core::i18n::t("start") } else if dest { wb_core::i18n::t("destination-current") } else { wb_core::i18n::t("via") };
        if i > 0 { rows += &format!("<div class='why'>↓{}</div>", if s(n, "reason").is_empty() { String::new() } else { format!(" {}", e(s(n, "reason"))) }); }
        rows += &format!("<div class='stop {}' data-n='{}'><small>{role}</small>{}</div>", if i == 0 { "s" } else if dest { "d" } else { "" }, e(id), e(s(n, "label")));
    }
    format!("{note}<div class='route'>{rows}</div>")
}

#[derive(Default)]
struct Moment { decided: bool, at: Option<i64>, labels: Vec<String> }

fn frame_time(nodes: &[Value], current: &str, style: &Style) -> String {
    let t = tree_of(nodes);
    let roots: Vec<&Value> = t.kids.get("").cloned().unwrap_or_default();
    let tops: Vec<&Value> = if roots.len() == 1 { t.kids.get(s(roots[0], "id")).cloned().unwrap_or_else(|| roots.clone()) } else { roots.clone() };
    fn subtree<'a>(n: &'a Value, t: &Tree<'a>, out: &mut Vec<&'a Value>) { out.push(n); for c in t.kids.get(s(n, "id")).into_iter().flatten() { subtree(c, t, out); } }
    let mut lanes: Vec<(&Value, HashMap<i64, Moment>, bool)> = Vec::new();
    for n in tops {
        let mut under = Vec::new();
        subtree(n, &t, &mut under);
        let mut events: HashMap<i64, Moment> = HashMap::new();
        for m in &under {
            for key in ["opened", "changed", "closed"] {
                let Some(stamp) = m.get(key).filter(|v| v.is_object()) else { continue };
                let Some(turn) = stamp.get("turn").and_then(Value::as_i64) else { continue };
                let ev = events.entry(turn).or_default();
                if key == "closed" && (s(m, "kind") == "conclusion" || s(m, "kind").is_empty()) { ev.decided = true; }
                if ev.at.is_none() { ev.at = stamp.get("at").and_then(Value::as_i64); }
                ev.labels.push(s(m, "label").to_string());
            }
            for v in numbers(m, "visits") { events.entry(v).or_default(); }
        }
        let here = under.iter().any(|m| s(m, "id") == current);
        lanes.push((n, events, here));
    }
    let mut turns: Vec<i64> = lanes.iter().flat_map(|l| l.1.keys().copied()).collect();
    turns.sort();
    turns.dedup();
    let Some(first) = turns.first().copied() else { return wb_core::i18n::t("no-time-record-recorded-from-when").to_string() };
    let topic = lanes.iter().filter(|l| l.2).flat_map(|l| l.1.keys().copied()).min().unwrap_or(first);
    let decision = lanes.iter().flat_map(|l| l.1.iter().filter(|(_, v)| v.decided).map(|(k, _)| *k)).max().unwrap_or(first);
    let ranges = [("all", wb_core::i18n::t("all"), first), ("topic", wb_core::i18n::t("from-the-start-of-the-current"), topic), ("decision", wb_core::i18n::t("from-the-last-decision"), decision)];
    let default = match s(&style.raw, "range") { "all" => "all", "decision" => "decision", _ => "topic" };
    let table = |start: i64| {
        let cols: Vec<i64> = turns.iter().copied().filter(|x| *x >= start).collect();
        let head: String = cols.iter().map(|c| {
            let at = lanes.iter().find_map(|l| l.1.get(c).and_then(|m| m.at));
            let clock = at.and_then(|ms| chrono::Local.timestamp_millis_opt(ms).single()).map(|x| x.format("%H:%M").to_string()).unwrap_or_default();
            wb_core::i18n::f("turn", &[("clock", (clock).to_string()), ("c", (c).to_string())])
        }).collect();
        let mut rows = String::new();
        for (i, (n, ev, _)) in lanes.iter().enumerate() {
            if !cols.iter().any(|c| ev.contains_key(c)) { continue; }
            let colour = LANES[i % LANES.len()];
            let (mut cells, mut prev) = (String::new(), None::<usize>);
            for (at, c) in cols.iter().enumerate() {
                let Some(m) = ev.get(c) else { cells += "<td></td>"; continue };
                let back = prev.is_some_and(|p| at - p > 1);
                let mut seen = HashSet::new();
                let label: Vec<&str> = m.labels.iter().map(String::as_str).filter(|l| seen.insert(*l)).collect();
                cells += &format!("<td><div class='seg{}' style='background:{colour}'>{}{}</div><div class='note'>{}</div></td>", if m.decided { " dec" } else { "" }, if back { "↩" } else { "" }, if m.decided { "◆" } else { "" }, e(&cut(&label.join("; "), 60)));
                prev = Some(at);
            }
            rows += &format!("<tr><td class='lab' data-n='{}'><i style='background:{colour}'></i>{}</td>{cells}</tr>", e(s(n, "id")), e(&cut(s(n, "label"), 24)));
        }
        format!("<table><tr><th></th>{head}</tr>{rows}</table>")
    };
    let radios: String = ranges.iter().map(|(k, _, _)| format!("<input type='radio' name='r' id='{k}'{}>", if *k == default { " checked" } else { "" })).collect();
    let labels: String = ranges.iter().map(|(k, n, _)| format!("<label for='{k}'>{n}</label>")).collect();
    let views: String = ranges.iter().map(|(k, _, st)| format!("<div class='view v-{k}'>{}</div>", table(*st))).collect();
    let show: String = ranges.iter().map(|(k, _, _)| format!("#{k}:checked ~ .views .v-{k}{{display:block}} #{k}:checked ~ .bar label[for={k}]{{background:var(--accent);color:var(--bg)}} ")).collect();
    wb_core::i18n::f("range-decision-return", &[("show", (show).to_string()), ("radios", (radios).to_string()), ("labels", (labels).to_string()), ("views", (views).to_string())])
}

/// The whole page. `picked` is the node the file panel is filtered on, marked as on the tree.
pub fn page(d: &Data, data: &Value, style: &Style, root: Option<&str>, picked: Option<&str>) -> String {
    let nodes = mapstyles::scoped(data, root);
    let current = s(data, "current");
    let body = match style.frame() {
        Some("columns") => frame_columns(&nodes, current, style),
        Some("route") => frame_route(&nodes, current),
        Some("time") => frame_time(&nodes, current, style),
        Some("questions") => frame_questions(&nodes, current),
        _ => frame_tree(&nodes, current, style),
    };
    let t = theme::load_active(d);
    let hex = |c: [f32; 4]| format!("#{:02x}{:02x}{:02x}", (c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8);
    let (bg, panel, ink, accent) = (hex(t.panel), hex(t.bar_bottom), format!("#{:02x}{:02x}{:02x}", t.ink[0], t.ink[1], t.ink[2]), hex(t.accent));
    let font = t.font.replace('\'', "");
    let mark = picked.map(|p| format!("[data-n=\"{}\"]{{background:color-mix(in srgb,var(--accent) 22%,transparent);border-radius:4px}}", e(p))).unwrap_or_default();
    format!(r#"<!doctype html><html lang="ko"><head><meta charset="utf-8"><style>
:root{{--bg:{bg};--panel:{panel};--ink:{ink};--accent:{accent}}}
body{{margin:0;padding:8px 10px;background:var(--bg);color:var(--ink);font-family:'{font}',sans-serif;font-size:13px;line-height:1.45}}
.sub{{opacity:.6;font-size:11.5px;margin:0 0 6px}}
ul.tree,ul.tree ul{{list-style:none;margin:0;padding-left:16px}} ul.tree{{padding-left:0}} li{{margin:2px 0}}
.g{{display:inline-block;width:1.3em}} li.shut>.g,li.shut>.t,li.shut>.k{{opacity:.5}} li.here{{font-weight:700}} li.fold{{opacity:.5;font-size:11.5px}}
.k{{font-size:10px;font-weight:700;border-radius:4px;padding:0 5px;margin-right:4px;color:#fff}}
.cols{{display:grid;gap:8px;align-items:start}} .col{{background:var(--panel);border-radius:8px;padding:6px}} .col h3{{font-size:12.5px;margin:2px 4px 6px}}
.n{{float:right;opacity:.5;font-weight:400}} .card{{background:var(--bg);border-radius:6px;padding:4px 7px;margin-bottom:5px}} .card.here{{outline:2px solid var(--accent)}}
.path{{opacity:.5;font-size:10.5px}}
.route{{display:flex;flex-direction:column;gap:3px}} .stop{{background:var(--panel);border-radius:7px;padding:5px 9px;border-left:4px solid #df8e1d}}
.stop.s{{border-color:#40a02b}} .stop.d{{border-color:var(--accent);font-weight:600}} .stop small{{display:block;opacity:.5;font-size:10px}}
.why{{opacity:.6;font-size:11.5px;padding-left:14px}}
input{{display:none}} .bar label{{display:inline-block;background:var(--panel);border-radius:5px;padding:2px 9px;margin:0 3px 6px 0;cursor:pointer;font-size:12px}}
.view{{display:none;overflow-x:auto}} table{{border-collapse:collapse}} th{{font-weight:400;opacity:.6;font-size:10.5px;padding:0 3px 3px;min-width:56px;text-align:left}}
th span{{font-size:9.5px}} td{{padding:3px;vertical-align:top;border-top:1px solid var(--panel)}} td.lab{{white-space:nowrap;font-weight:600;padding-right:8px}}
td.lab i{{display:inline-block;width:8px;height:8px;border-radius:2px;margin-right:5px}}
.seg{{height:13px;border-radius:3px;color:#fff;font-size:10px;font-weight:700;line-height:13px;padding-left:3px}} .seg.dec{{outline:2px solid var(--ink);outline-offset:-2px}}
.note{{font-size:10px;opacity:.75;max-width:110px}}
h4{{font-size:12px;margin:2px 0 4px;opacity:.8}} details{{margin-top:8px}} summary{{font-size:12px;font-weight:600;opacity:.8;cursor:pointer;margin-bottom:3px}}
.cnt{{opacity:.5;font-size:11px}} li.one{{margin:3px 0}}
[data-n]{{cursor:pointer}} [data-n]:hover{{text-decoration:underline}} {mark}
</style></head><body>{body}<script>
function send(act, ev, node) {{ location.href = 'wb:' + act + ':' + Math.round(ev.clientX) + ':' + Math.round(ev.clientY) + ':' + encodeURIComponent(node); }}
document.addEventListener('click', ev => {{ const t = ev.target.closest('[data-n]'); if (t) send('click', ev, t.dataset.n); }});
document.addEventListener('contextmenu', ev => {{ ev.preventDefault(); const t = ev.target.closest('[data-n]'); send('menu', ev, t ? t.dataset.n : ''); }});
</script></body></html>"#)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn style(raw: Value) -> Style { Style { id: "x".into(), name: "x".into(), raw, path: "/x".into() } }
    fn nodes() -> Vec<Value> {
        vec![
            json!({"id":"a","label":"root","isOpen":true,"kind":"question"}),
            json!({"id":"b","parent":"a","label":"choice <1>","isOpen":true,"kind":"option","state":"doing","visits":[2],"reason":"why it moved"}),
            json!({"id":"c","parent":"a","label":"conclusion","isOpen":false,"kind":"conclusion","closed":{"turn":3,"at":1793900000000i64},"visits":[1]}),
            json!({"id":"d","label":"memo","isOpen":true,"kind":"note","state":"plan"}),
        ]
    }
    #[test]
    fn every_node_carries_its_id_and_text_is_escaped() {
        for raw in [json!({"frame":"tree"}), json!({"frame":"tree","filter":"path"}), json!({"frame":"questions"})] {
            let html = match raw["frame"].as_str() { Some("questions") => frame_questions(&nodes(), "b"), _ => frame_tree(&nodes(), "b", &style(raw.clone())) };
            assert!(html.contains("data-n='b'") && html.contains("choice &lt;1&gt;") && !html.contains("choice <1>"), "{html}");
        }
    }
    #[test]
    fn columns_follow_state_and_keep_only_recent_done() {
        let html = frame_columns(&nodes(), "b", &style(json!({"frame":"columns","columns":["idea","plan","doing","done"],"recent_done":1})));
        let order: Vec<usize> = ["Idea", "Planned", "In progress", "Done"].iter().map(|c| html.find(c).unwrap()).collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]));
        assert!(html.contains("card here"));
        assert_eq!(state_column(&json!({"isOpen":false})), "done");
        assert_eq!(state_column(&json!({"isOpen":true})), "idea");
    }
    #[test]
    fn route_goes_by_visits_and_falls_back_to_the_path() {
        let html = frame_route(&nodes(), "b");
        assert!(html.find("conclusion").unwrap() < html.find("choice").unwrap() && html.contains("why it moved") && html.contains("Destination · Current"));
        let plain = vec![json!({"id":"a","label":"top"}), json!({"id":"z","parent":"a","label":"bottom"})];
        assert!(frame_route(&plain, "z").contains("No visit record"));
    }
    #[test]
    fn time_has_ranges_or_says_there_is_nothing() {
        let html = frame_time(&nodes(), "b", &style(json!({"frame":"time"})));
        assert!(html.contains("Range:") && html.contains("id='topic' checked") && html.contains("Turn 3"));
        assert!(frame_time(&[json!({"id":"a","label":"x"})], "a", &style(json!({}))).contains("No time record"));
    }
    #[test]
    fn questions_group_open_and_settled() {
        let html = frame_questions(&nodes(), "b");
        assert!(html.contains("Open questions 1") && html.contains("Notes, tasks, no kind 1"));
    }
}
