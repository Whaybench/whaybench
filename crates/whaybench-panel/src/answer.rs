//! The answer panel: the chosen session's last answer, kept answers with back and forward, and the answers of a
//! map node. The panel's own list holds the last 50 of all sessions, taken from every session's answer log, so answers
//! that come at once, or from a session not followed, are all in it.
use std::{cell::RefCell, rc::Rc};

use gtk::prelude::*;
use serde_json::{Value, json};
use wb_core::{paths::Data, records, sessions, status};
use webkit6::prelude::*;

use crate::{common::{cut, every}, web::{self, Page, RawHtml}};

const KEEP: usize = 50;

struct Panel {
    history: Vec<Value>,
    pos: usize,
    /// Kept answers while showing the answers of a node, and where in them.
    related: Option<(Vec<Value>, usize)>,
    followed: Option<String>,
    last_at: i64,
    /// The newest answer taken from the logs, and when they were last looked at.
    seen: i64,
    scanned: i64,
    /// What was shown before the newest answer was asked for: a node's answers, and the kept answer as (at, session).
    before: Option<(Option<(Vec<Value>, usize)>, i64, String)>,
}

/// Answers in the logs after `since`, oldest first, as (session, row). A log not written to since `scanned` is not read.
fn logged_after(d: &Data, since: i64, scanned: i64) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for f in wb_core::roots::all_of(d).iter().flat_map(|r| std::fs::read_dir(r.root.join("answer-log")).into_iter().flatten().flatten()) {
        let Some(sid) = f.file_name().to_str().and_then(|n| n.strip_suffix(".jsonl")).map(String::from) else { continue };
        let modified = f.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(i64::MAX, |t| t.as_millis() as i64);
        // A second of slack: the file's time and the clock read at the last look can differ a little.
        if modified < scanned - 1000 { continue; }
        let Ok(text) = std::fs::read_to_string(f.path()) else { continue };
        out.extend(text.lines().rev().filter_map(|l| serde_json::from_str::<Value>(l).ok()).take_while(|r| r["at"].as_i64().unwrap_or(0) > since).map(|r| (sid.clone(), r)));
    }
    out.sort_by_key(|(_, r)| r["at"].as_i64().unwrap_or(0));
    out
}

/// No answer of the followed session comes after the one shown.
fn shows_newest(history: &[Value], pos: usize, mine: &[String]) -> bool {
    !history.iter().skip(pos + 1).any(|r| r["session"].as_str().is_some_and(|s| mine.iter().any(|m| m == s)))
}

fn same(r: &Value, at: i64, session: &str) -> bool { r["at"].as_i64() == Some(at) && r["session"].as_str() == Some(session) }

fn time_of(ms: i64, format: &str) -> String {
    wb_core::clock::local(ms, format)
}

fn history_file(d: &Data) -> std::path::PathBuf { d.root.join("answer-history.json") }

fn save(d: &Data, h: &[Value]) {
    // A project's answers are not written to the host's root (spec 3.22): they stay in this panel while it runs.
    let roots = wb_core::roots::Roots::new(d.clone());
    let kept: Vec<Value> = h.iter().filter(|e| e["session"].as_str().is_none_or(|s| roots.project_of(s).is_none())).cloned().collect();
    let _ = wb_core::paths::write_atomic(&history_file(d), Value::Array(kept).to_string());
}

struct Widgets { web: webkit6::WebView, where_: gtk::Label, back: gtk::Button, forward: gtk::Button, drop: gtk::Button, newest: gtk::Button, ret: gtk::Button, shown: RefCell<String> }

fn style(d: &Data) -> String {
    let p = Page::of(d);
    format!("{}body{{padding:6px 12px 12px}} .asked{{opacity:.6;font-size:small;margin-bottom:6px}} p{{margin:6px 0}} ul,ol{{margin:6px 0;padding-left:22px}}\
      h1{{font-size:16pt;margin:12px 0 6px}} h2{{font-size:14pt;margin:12px 0 6px}} h3{{font-size:12pt;margin:10px 0 4px}} h4,h5,h6{{font-size:11pt;margin:10px 0 3px}}\
      a{{color:{}}} hr{{border:0;border-top:1px solid {line}}} blockquote{{margin:6px 0;padding-left:10px;border-left:3px solid {line}}}\
      table{{margin:6px 0}} td,th{{padding:3px 8px;vertical-align:top;text-align:left}} th{{background:{surface};word-break:keep-all}}\
      pre,code{{font-size:9.5pt}} pre{{background:{surface};padding:6px 8px;border-radius:4px}}", p.base("14px/1.55"), p.accent, line = p.line, surface = p.surface)
}

/// Loads the page unless it is the one shown: a reload would drop the selection and the scroll place.
fn show(w: &Widgets, page: String) {
    if *w.shown.borrow() == page { return; }
    w.web.load_html(&page, Some("about:blank"));
    *w.shown.borrow_mut() = page;
}

fn draw(p: &Panel, w: &Widgets, d: &Data) {
    let (row, place, can_back, can_forward) = if let Some((rows, i)) = &p.related {
        (rows.get(*i), wb_core::i18n::f("related-answers", &[("a1", (time_of(rows[*i]["at"].as_i64().unwrap_or(0), "%m-%d %H:%M")).to_string()), ("a2", (i + 1).to_string()), ("a3", (rows.len()).to_string())]), *i > 0, i + 1 < rows.len())
    } else if let Some(row) = p.history.get(p.pos) {
        let title = row.get("title").and_then(Value::as_str).unwrap_or("").to_string();
        (Some(row), format!("{title} · {}/{}", p.pos + 1, p.history.len()), p.pos > 0, p.pos + 1 < p.history.len())
    } else {
        (None, wb_core::i18n::f("session-no-answer", &[("a1", (sessions::target(d).map(|s| cut(&s, 8)).unwrap_or_else(|| wb_core::i18n::t("none").into())).to_string())]), false, false)
    };
    w.where_.set_text(&place);
    w.back.set_sensitive(can_back);
    w.forward.set_sensitive(can_forward);
    w.drop.set_sensitive(row.is_some());
    w.newest.set_sensitive(p.related.is_some() || p.pos + 1 < p.history.len());
    w.ret.set_sensitive(p.before.is_some());
    w.drop.set_tooltip_text(Some(if p.related.is_some() { wb_core::i18n::t("close-related-answers") } else { wb_core::i18n::t("remove-from-history") }));
    let top = format!("<!doctype html><meta charset='utf-8'><style>{}</style>", style(d));
    let Some(row) = row else { return show(w, top) };
    let prompt = row.get("prompt").and_then(Value::as_str).unwrap_or("");
    let asked = format!("{} · {}", time_of(row["at"].as_i64().unwrap_or(0), "%H:%M"), cut(&wb_core::prompt::label(prompt), 120));
    show(w, format!("{top}<div class='asked'>{}</div>{}", web::escape(&asked), web::markdown(row.get("text").and_then(Value::as_str).unwrap_or(""), RawHtml::AsText)));
}

/// Everything a permission ask would allow, field by field and whole: the person allows only what they saw.
/// Characters that show nothing or turn the text around (NUL, zero-width, direction marks) are shown as \u{...}.
fn about(input: &Value) -> String {
    let fields = input.as_object().map(|f| f.iter().map(|(k, v)| format!("{k}: {}", v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))).collect::<Vec<_>>().join("\n"));
    let text = fields.unwrap_or_else(|| input.to_string());
    let hidden = |c: char| (c.is_control() && c != '\n' && c != '\t') || matches!(c, '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}');
    text.chars().map(|c| if hidden(c) { format!("\\u{{{:x}}}", c as u32) } else { c.to_string() }).collect()
}

/// The answer written, or the card stays with the error on the status line, so it can be pressed again.
fn saved(d: &Data, r: wb_core::map::Result<()>) -> bool {
    r.map_err(|e| status::say(d, &wb_core::i18n::f("ask-not-saved", &[("a1", e.to_string())]), 8)).is_ok()
}

/// Fills the card with an agent's question or permission ask (wb_core::ask); a press answers it and hides the card.
fn fill_card(outer: &gtk::Box, d: &Data, ask: &Value) {
    while let Some(c) = outer.first_child() { outer.remove(&c); }
    // A new box each time, so its Enter key goes with it.
    let card = &gtk::Box::new(gtk::Orientation::Vertical, 4);
    outer.append(card);
    let id = ask["id"].as_str().unwrap_or("").to_string();
    let sid = ask["session"].as_str().unwrap_or("").to_string();
    let name = cut(&sessions::session_name(d, &sid), 24);
    let question = ask["kind"] == "question";
    let head = gtk::Label::new(Some(&if question { wb_core::i18n::f("ask-question", &[("a1", name)]) }
        else { wb_core::i18n::f("ask-permission", &[("a1", name), ("a2", ask["tool"].as_str().unwrap_or("").to_string())]) }));
    head.set_xalign(0.0);
    head.add_css_class("heading");
    card.append(&head);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let by_prompt = ask["via"] == "prompt";
    let done = {
        let outer = outer.clone();
        move || outer.set_visible(false)
    };
    if question {
        // Each question: its options (one or, with multiSelect, several) and a line for another answer.
        let mut asked: Vec<(String, Vec<(gtk::CheckButton, String)>, gtk::Entry)> = Vec::new();
        for q in ask["input"]["questions"].as_array().into_iter().flatten() {
            let text = q["question"].as_str().unwrap_or("").to_string();
            if let Some(h) = q["header"].as_str() { let l = gtk::Label::new(Some(h)); l.set_xalign(0.0); l.add_css_class("dim"); l.add_css_class("small"); card.append(&l); }
            let l = gtk::Label::new(Some(&text));
            l.set_xalign(0.0);
            l.set_wrap(true);
            card.append(&l);
            let mut options: Vec<(gtk::CheckButton, String)> = Vec::new();
            for o in q["options"].as_array().into_iter().flatten() {
                let label = o["label"].as_str().unwrap_or("").to_string();
                let b = gtk::CheckButton::with_label(&label);
                if let Some(t) = o["description"].as_str() { b.set_tooltip_text(Some(t)); }
                if q["multiSelect"] != true && let Some((first, _)) = options.first() { b.set_group(Some(first)); }
                card.append(&b);
                options.push((b, label));
            }
            let other = gtk::Entry::new();
            other.set_placeholder_text(Some(wb_core::i18n::t("ask-other")));
            card.append(&other);
            asked.push((text, options, other));
        }
        let send = gtk::Button::with_label(wb_core::i18n::t("ask-send"));
        {
            let (d, sid, id, done) = (d.clone(), sid.clone(), id.clone(), done.clone());
            send.connect_clicked(move |_| {
                let answers: Vec<(String, String)> = asked.iter().map(|(q, options, other)| {
                    let typed = other.text().trim().to_string();
                    let a = if typed.is_empty() { options.iter().filter(|(b, _)| b.is_active()).map(|(_, l)| l.as_str()).collect::<Vec<_>>().join(", ") } else { typed };
                    (q.clone(), a)
                }).collect();
                // Every question needs an answer.
                if answers.iter().any(|(_, a)| a.is_empty()) { return; }
                if by_prompt {
                    if !sessions::queue_prompt(&d, &sid, &wb_core::ask::reply_prompt(&answers)) { status::say(&d, wb_core::i18n::t("ask-not-sent"), 8); return; }
                    wb_core::ask::clear_in(&d, &id);
                } else {
                    let map: serde_json::Map<String, Value> = answers.into_iter().map(|(q, a)| (q, json!(a))).collect();
                    if !saved(&d, wb_core::ask::answer_in(&d, &id, "allow", &Value::Object(map))) { return; }
                }
                done();
            });
        }
        buttons.append(&send);
        // Enter on an option or in the line for another answer sends, as in the terminal; on a button it presses that button.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let (send, card) = (send.clone(), card.clone());
            keys.connect_key_pressed(move |_, key, _, _| {
                if !matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter) { return gtk::glib::Propagation::Proceed; }
                let Some(focus) = card.root().and_then(|r| r.focus()) else { return gtk::glib::Propagation::Proceed };
                if let Ok(b) = focus.clone().downcast::<gtk::CheckButton>() { b.set_active(true); }
                else if focus.ancestor(gtk::Entry::static_type()).is_none() { return gtk::glib::Propagation::Proceed; }
                send.emit_clicked();
                gtk::glib::Propagation::Stop
            });
        }
        card.add_controller(keys);
    } else {
        // A text view, not a label: a large input (a whole file to write) stays quick to lay out.
        let l = gtk::TextView::new();
        l.buffer().set_text(&about(&ask["input"]));
        l.set_editable(false);
        l.set_cursor_visible(false);
        l.set_wrap_mode(gtk::WrapMode::WordChar);
        l.set_monospace(true);
        // A long input scrolls in its place rather than being cut.
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_propagate_natural_height(true);
        scroll.set_max_content_height(240);
        scroll.set_child(Some(&l));
        card.append(&scroll);
        for (key, decision) in [("ask-allow", "allow"), ("ask-deny", "deny")] {
            let b = gtk::Button::with_label(wb_core::i18n::t(key));
            let (d, id, done) = (d.clone(), id.clone(), done.clone());
            b.connect_clicked(move |_| { if saved(&d, wb_core::ask::answer_in(&d, &id, decision, &json!({}))) { done(); } });
            buttons.append(&b);
        }
    }
    // A held-back question has no terminal ask to fall back on: it is only closed.
    let last = gtk::Button::with_label(wb_core::i18n::t(if by_prompt { "ask-close" } else { "ask-terminal" }));
    last.add_css_class("flat");
    {
        let d = d.clone();
        last.connect_clicked(move |_| {
            if by_prompt { wb_core::ask::clear_in(&d, &id); } else if !saved(&d, wb_core::ask::answer_in(&d, &id, "terminal", &json!({}))) { return; }
            done();
        });
    }
    buttons.append(&last);
    card.append(&buttons);
    outer.set_visible(true);
    // The keys go to the question at once; a permission waits for a press, so a stray Enter allows nothing.
    if question && let Some(first) = card.observe_children().into_iter().flatten().filter_map(|c| c.downcast::<gtk::CheckButton>().ok()).next() { first.grab_focus(); }
}

/// The nearest answer in the conversation: a session moved on has none of its own at first.
fn newest_answer(d: &Data, session: &str) -> Option<Value> {
    records::lineage(d, session).into_iter().find_map(|id| std::fs::read_to_string(wb_core::roots::for_session(d, &id).root.join("answers").join(format!("{id}.json"))).ok().and_then(|t| serde_json::from_str(&t).ok()))
}

pub fn build(win: &gtk::ApplicationWindow, d: Data) {
    let where_ = gtk::Label::new(None);
    where_.set_xalign(0.0);
    where_.set_hexpand(true);
    where_.set_ellipsize(gtk::pango::EllipsizeMode::End);
    where_.add_css_class("small");
    let (back, forward, drop) = (gtk::Button::with_label("◀"), gtk::Button::with_label("▶"), gtk::Button::with_label("✕"));
    let (ret, newest) = (gtk::Button::with_label("↶"), gtk::Button::with_label("⟳"));
    ret.set_tooltip_text(Some(wb_core::i18n::t("answer-shown-before")));
    newest.set_tooltip_text(Some(wb_core::i18n::t("answer-newest")));
    for b in [&back, &forward, &drop, &ret, &newest] { b.add_css_class("flat"); }
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    bar.set_margin_start(8);
    bar.set_margin_end(4);
    bar.append(&where_);
    bar.append(&ret);
    bar.append(&back);
    bar.append(&forward);
    bar.append(&newest);
    bar.append(&drop);
    let web = webkit6::WebView::new();
    crate::common::set_up_web(&web, &d);
    web.set_vexpand(true);
    // Never squeezed to nothing by the card: a web view left with no height stops showing new pages.
    web.set_size_request(-1, 80);
    web.set_hexpand(true);
    if let Some(settings) = WebViewExt::settings(&web) { settings.set_default_charset("utf-8"); settings.set_enable_javascript(false); }
    // A link in the answer opens in the view panel; the answer's own page stays.
    {
        let d = d.clone();
        web.connect_decide_policy(move |_, decision, kind| {
            let Some(uri) = web::pressed_link(decision, kind) else { return false };
            if uri.starts_with("about:") { return false; }
            decision.ignore();
            if uri.starts_with("http") { let _ = wb_core::view::open(&d, &uri, None); }
            true
        });
    }
    // An agent's question or permission ask, above the answer.
    let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
    card.set_margin_start(12);
    card.set_margin_end(12);
    card.set_margin_bottom(6);
    card.set_visible(false);
    let area = gtk::Box::new(gtk::Orientation::Vertical, 0);
    area.append(&bar);
    // A long card scrolls in its place, so the answer below keeps its room.
    let card_room = gtk::ScrolledWindow::new();
    card_room.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    card_room.set_propagate_natural_height(true);
    card_room.set_max_content_height(260);
    card_room.set_child(Some(&card));
    area.append(&card_room);
    area.append(&web);
    win.set_child(Some(&area));

    let history: Vec<Value> = wb_core::paths::read_kept(&history_file(&d));
    let pos = history.len().saturating_sub(1);
    // A first start takes the last answers already logged; after that, what came while the panel was closed.
    let seen = history.iter().filter_map(|r| r["at"].as_i64()).max().unwrap_or(0);
    let panel = Rc::new(RefCell::new(Panel { history, pos, related: None, followed: sessions::target(&d), last_at: 0, seen, scanned: 0, before: None }));
    let w = Rc::new(Widgets { web, where_, back: back.clone(), forward: forward.clone(), drop: drop.clone(), newest: newest.clone(), ret: ret.clone(), shown: RefCell::default() });
    draw(&panel.borrow(), &w, &d);

    let go = |step: i64, panel: Rc<RefCell<Panel>>, w: Rc<Widgets>, d: Data| move |_: &gtk::Button| {
        {
            let mut p = panel.borrow_mut();
            if let Some((rows, i)) = &mut p.related {
                let n = *i as i64 + step;
                if n >= 0 && (n as usize) < rows.len() { *i = n as usize; }
            } else {
                let n = p.pos as i64 + step;
                if n >= 0 && (n as usize) < p.history.len() { p.pos = n as usize; }
            }
        }
        draw(&panel.borrow(), &w, &d);
    };
    back.connect_clicked(go(-1, panel.clone(), w.clone(), d.clone()));
    {
        let (panel, w, d) = (panel.clone(), w.clone(), d.clone());
        newest.connect_clicked(move |_| {
            {
                let mut p = panel.borrow_mut();
                let key = p.history.get(p.pos).map(|r| (r["at"].as_i64().unwrap_or(0), r["session"].as_str().unwrap_or("").to_string()));
                if let Some((at, sid)) = key && (p.related.is_some() || p.pos + 1 < p.history.len()) {
                    let related = p.related.take();
                    p.before = Some((related, at, sid));
                    p.pos = p.history.len() - 1;
                }
            }
            draw(&panel.borrow(), &w, &d);
        });
    }
    {
        let (panel, w, d) = (panel.clone(), w.clone(), d.clone());
        ret.connect_clicked(move |_| {
            {
                let mut p = panel.borrow_mut();
                if let Some((related, at, sid)) = p.before.take() {
                    // The answer may have left the list meanwhile; the place shown then stays.
                    if let Some(i) = p.history.iter().position(|r| same(r, at, &sid)) { p.pos = i; }
                    p.related = related;
                }
            }
            draw(&panel.borrow(), &w, &d);
        });
    }
    forward.connect_clicked(go(1, panel.clone(), w.clone(), d.clone()));
    {
        let (panel, w, d) = (panel.clone(), w.clone(), d.clone());
        drop.connect_clicked(move |_| {
            {
                let mut p = panel.borrow_mut();
                if p.related.take().is_none() && p.pos < p.history.len() {
                    let at = p.pos;
                    p.history.remove(at);
                    p.pos = p.pos.min(p.history.len().saturating_sub(1));
                    save(&d, &p.history);
                }
            }
            draw(&panel.borrow(), &w, &d);
        });
    }
    let card_of: RefCell<Option<String>> = RefCell::default();
    every(1000, move || {
        // Each root's hooks look for the panel in their own root.
        for r in wb_core::roots::all_of(&d) { wb_core::ask::panel_alive(&r.root); }
        let asks = wb_core::ask::pending_all(&d);
        let id_of = |a: &Value| a["id"].as_str().unwrap_or("").to_string();
        // The card shown stays while its ask waits, so nothing typed in it is lost; then the followed
        // conversation's next ask, else the oldest one waiting.
        let shown = card_of.borrow().clone();
        if !asks.iter().any(|a| Some(id_of(a)) == shown) {
            let mine = if asks.is_empty() { Vec::new() } else { sessions::target(&d).map(|s| records::lineage(&d, &s)).unwrap_or_default() };
            let next = asks.iter().find(|a| a["session"].as_str().is_some_and(|s| mine.iter().any(|m| m == s))).or(asks.first());
            match next {
                Some(a) => {
                    fill_card(&card, &d, a);
                    // The whole card up to 260 px, so its options are never scrolled out of sight; the answer keeps 80 px.
                    let (_, natural, _, _) = card.measure(gtk::Orientation::Vertical, card_room.width().max(1));
                    card_room.set_min_content_height(natural.min(260));
                    status::focus_app(&d, "dev.whaybench.answer");
                }
                None => {
                    // Once a card goes (answered here or elsewhere), its room goes too and the answer is loaded
                    // again: what came while it showed is drawn for sure.
                    if shown.is_some() {
                        w.shown.borrow_mut().clear();
                        card_room.set_min_content_height(-1);
                    }
                    card.set_visible(false);
                }
            }
            *card_of.borrow_mut() = next.map(id_of);
        }
        {
            let mut p = panel.borrow_mut();
            // From the map's menu: {"session", "node", "at"}.
            if let Some(req) = status::take(&d, "answer") {
                let session = req["session"].as_str().unwrap_or("").to_string();
                let at = req["at"].as_i64().unwrap_or(0);
                let rows = records::answers_of(&d, &records::lineage(&d, &session));
                match records::related_index(&rows, at) {
                    Some(i) => {
                        wb_core::log::decide("answer", "related answers", "found", &[("session", session), ("kept_answers", rows.len().to_string())]);
                        // A node made while tidying the map has that turn's time, not the time it was talked about.
                        let tidied = rows[i]["prompt"].as_str().is_some_and(|t| t.starts_with(wb_core::i18n::ai("map-update")));
                        status::say(&d, &if tidied { wb_core::i18n::f("related-answers-node-created-at-map", &[("a1", (i + 1).to_string()), ("a2", (rows.len()).to_string())]) } else { wb_core::i18n::f("related-answers-2", &[("a1", (i + 1).to_string()), ("a2", (rows.len()).to_string())]) }, 8);
                        p.related = Some((rows, i));
                        status::focus_app(&d, "dev.whaybench.answer");
                    }
                    None => {
                        wb_core::log::decide("answer", "related answers", "no kept answer after it", &[("session", session), ("kept_answers", rows.len().to_string())]);
                        status::say(&d, wb_core::i18n::t("no-related-answers-no-answers-kept"), 8);
                    }
                }
            }
            let session = sessions::target(&d);
            let now = wb_core::clock::now_ms();
            let fresh = logged_after(&d, p.seen, p.scanned);
            p.scanned = now;
            let mine = session.as_deref().map(|s| records::lineage(&d, s)).unwrap_or_default();
            // Showing the followed session's newest answer, a new one of it is shown; while a node's answers or an
            // earlier answer are shown, it only joins the list. Choosing another session shows its newest; the same
            // conversation going on under a new number (/resume) is not another session.
            let moved = matches!((&p.followed, &session), (Some(f), Some(s)) if sessions::current_id(&d, f) == *s);
            let switched = session != p.followed && !moved;
            let on_newest = switched || (p.related.is_none() && shows_newest(&p.history, p.pos, &mine));
            if !fresh.is_empty() {
                let first = p.history.is_empty();
                for (sid, a) in fresh.iter().rev().take(KEEP).rev() {
                    let at = a["at"].as_i64().unwrap_or(0);
                    p.seen = p.seen.max(at);
                    if p.history.iter().any(|r| same(r, at, sid)) { continue; }
                    p.history.push(json!({"at": at, "session": sid, "prompt": a["prompt"], "text": a["text"], "title": cut(&sessions::session_name(&d, sid), 24)}));
                    // Another session's answer waits in the list.
                    if on_newest && mine.contains(sid) { p.pos = p.history.len() - 1; p.last_at = at; }
                }
                let skip = p.history.len().saturating_sub(KEEP);
                p.history.drain(..skip);
                p.pos = p.pos.saturating_sub(skip).min(p.history.len().saturating_sub(1));
                // With no session followed, a first list opens on its newest answer.
                if first && mine.is_empty() { p.pos = p.history.len().saturating_sub(1); }
                save(&d, &p.history);
            }
            if session != p.followed {
                p.followed = session.clone();
                if switched { p.related = None; p.last_at = 0; }
            }
            if let Some(a) = session.as_deref().and_then(|s| newest_answer(&d, s)) {
                let at = a["at"].as_i64().unwrap_or(0);
                // Only a newer one: the log line is written before this file, so for a moment this file can hold
                // the answer before the one just taken from the log (and bring back one the person dropped).
                if at > p.last_at {
                    p.last_at = at;
                    let sid = a["sessionId"].as_str().unwrap_or("");
                    let known = p.history.iter().any(|r| same(r, at, sid));
                    if !known {
                        let name = session.as_deref().map(|s| sessions::session_name(&d, s)).unwrap_or_default();
                        p.history.push(json!({"at": at, "session": a["sessionId"], "prompt": a["prompt"], "text": a["text"], "title": cut(&name, 24)}));
                        let skip = p.history.len().saturating_sub(KEEP);
                        p.history.drain(..skip);
                        p.pos = p.pos.saturating_sub(skip);
                        save(&d, &p.history);
                    }
                    if on_newest
                        && let Some(i) = p.history.iter().position(|r| same(r, at, sid)) { p.pos = i; }
                }
            }
        }
        // Drawn each time: the page holds the theme's colours, and the same page is not loaded again.
        draw(&panel.borrow(), &w, &d);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_permission_shows_its_whole_input_and_hidden_characters() {
        let shown = about(&json!({"command": "ls\u{202e}txt.exe\u{0}; rm -rf x", "n": 2}));
        assert!(shown.contains("\\u{202e}") && shown.contains("\\u{0}") && shown.ends_with("n: 2"), "{shown}");
        assert!(shown.contains("; rm -rf x"), "nothing after the NUL is lost");
    }

    #[test]
    fn only_the_newest_answer_shown_moves_on() {
        let h: Vec<Value> = ["a", "b", "a", "b"].iter().map(|s| json!({"session": s})).collect();
        let mine = vec!["a".to_string()];
        assert!(shows_newest(&h, 2, &mine), "only another session's answer after it");
        assert!(!shows_newest(&h, 0, &mine), "an earlier answer is shown");
        assert!(shows_newest(&[], 0, &mine));
    }

    #[test]
    fn answers_of_every_session_come_in_order_and_once() {
        let root = std::env::temp_dir().join(format!("wb-answers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = Data::new(root.clone());
        std::fs::create_dir_all(root.join("answer-log")).unwrap();
        let row = |at: i64, t: &str| json!({"at": at, "prompt": "q", "text": t}).to_string();
        std::fs::write(root.join("answer-log/claude.jsonl"), format!("{}\n{}\n", row(10, "old"), row(30, "c"))).unwrap();
        std::fs::write(root.join("answer-log/codex.jsonl"), format!("{}\n", row(30, "x"))).unwrap();
        let got: Vec<(String, String)> = logged_after(&d, 20, 0).into_iter().map(|(s, r)| (s, r["text"].as_str().unwrap().to_string())).collect();
        assert_eq!(got.len(), 2, "two answers at the same moment from two sessions are both kept");
        assert!(got.iter().any(|(s, t)| s == "codex" && t == "x") && !got.iter().any(|(_, t)| t == "old"));
        assert!(logged_after(&d, 30, 0).is_empty());
        assert!(logged_after(&d, 0, i64::MAX).is_empty(), "logs not written since the last look are not read");
        let _ = std::fs::remove_dir_all(&root);
    }
}
