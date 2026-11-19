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
}

/// Answers in the logs after `since`, oldest first, as (session, row). A log not written to since `scanned` is not read.
fn logged_after(d: &Data, since: i64, scanned: i64) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for f in std::fs::read_dir(d.root.join("answer-log")).into_iter().flatten().flatten() {
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

fn same(r: &Value, at: i64, session: &str) -> bool { r["at"].as_i64() == Some(at) && r["session"].as_str() == Some(session) }

fn time_of(ms: i64, format: &str) -> String {
    wb_core::clock::local(ms, format)
}

fn history_file(d: &Data) -> std::path::PathBuf { d.root.join("answer-history.json") }

fn save(d: &Data, h: &[Value]) {
    let _ = wb_core::paths::write_atomic(&history_file(d), Value::Array(h.to_vec()).to_string());
}

struct Widgets { web: webkit6::WebView, where_: gtk::Label, back: gtk::Button, forward: gtk::Button, drop: gtk::Button, shown: RefCell<String> }

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
    w.drop.set_tooltip_text(Some(if p.related.is_some() { wb_core::i18n::t("close-related-answers") } else { wb_core::i18n::t("remove-from-history") }));
    let top = format!("<!doctype html><meta charset='utf-8'><style>{}</style>", style(d));
    let Some(row) = row else { return show(w, top) };
    let prompt = row.get("prompt").and_then(Value::as_str).unwrap_or("");
    let asked = format!("{} · {}", time_of(row["at"].as_i64().unwrap_or(0), "%H:%M"), cut(prompt, 120));
    show(w, format!("{top}<div class='asked'>{}</div>{}", web::escape(&asked), web::markdown(row.get("text").and_then(Value::as_str).unwrap_or(""), RawHtml::AsText)));
}

/// What a permission ask is about, in one place: the command, file or address, else the whole input.
fn about(input: &Value) -> String {
    let one = ["command", "file_path", "path", "url", "pattern"].iter().find_map(|k| input[*k].as_str().map(String::from));
    cut(&one.unwrap_or_else(|| input.to_string()), 600)
}

fn button(label: &str) -> gtk::Button { gtk::Button::with_label(label) }

/// Fills the card with an agent's question or permission ask (wb_core::ask); a press answers it and hides the card.
fn fill_card(outer: &gtk::Box, d: &Data, ask: &Value) {
    while let Some(c) = outer.first_child() { outer.remove(&c); }
    // A new box each time, so its Enter key goes with it.
    let card = &gtk::Box::new(gtk::Orientation::Vertical, 4);
    outer.append(card);
    let sid = ask["session"].as_str().unwrap_or("").to_string();
    let at = ask["at"].as_i64().unwrap_or(0);
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
        let send = button(wb_core::i18n::t("ask-send"));
        {
            let (d, sid, done) = (d.clone(), sid.clone(), done.clone());
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
                    wb_core::ask::clear(&d.root, &sid);
                } else {
                    let map: serde_json::Map<String, Value> = answers.into_iter().map(|(q, a)| (q, json!(a))).collect();
                    let _ = wb_core::ask::answer(&d.root, &sid, at, "allow", &Value::Object(map));
                }
                done();
            });
        }
        buttons.append(&send);
        // Enter sends, as in the terminal: on an option or in the line for another answer (after the input method commits).
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let (send, card) = (send.clone(), card.clone());
            keys.connect_key_pressed(move |_, key, _, _| {
                if !matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter) { return gtk::glib::Propagation::Proceed; }
                // As in the terminal, Enter takes the option the keys are on.
                if let Some(b) = card.root().and_then(|r| r.focus()).and_then(|w| w.downcast::<gtk::CheckButton>().ok()) { b.set_active(true); }
                send.emit_clicked();
                gtk::glib::Propagation::Stop
            });
        }
        card.add_controller(keys);
    } else {
        let l = gtk::Label::new(Some(&about(&ask["input"])));
        l.set_xalign(0.0);
        l.set_wrap(true);
        l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        l.set_selectable(true);
        l.add_css_class("monospace");
        card.append(&l);
        for (key, decision) in [("ask-allow", "allow"), ("ask-deny", "deny")] {
            let b = button(wb_core::i18n::t(key));
            let (d, sid, done) = (d.clone(), sid.clone(), done.clone());
            b.connect_clicked(move |_| { let _ = wb_core::ask::answer(&d.root, &sid, at, decision, &json!({})); done(); });
            buttons.append(&b);
        }
    }
    // A held-back question has no terminal ask to fall back on: it is only closed.
    let last = button(wb_core::i18n::t(if by_prompt { "ask-close" } else { "ask-terminal" }));
    last.add_css_class("flat");
    {
        let (d, sid) = (d.clone(), sid.clone());
        last.connect_clicked(move |_| {
            if by_prompt { wb_core::ask::clear(&d.root, &sid); } else { let _ = wb_core::ask::answer(&d.root, &sid, at, "terminal", &json!({})); }
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
    records::lineage(d, session).into_iter().find_map(|id| std::fs::read_to_string(d.root.join("answers").join(format!("{id}.json"))).ok().and_then(|t| serde_json::from_str(&t).ok()))
}

pub fn build(win: &gtk::ApplicationWindow, d: Data) {
    let where_ = gtk::Label::new(None);
    where_.set_xalign(0.0);
    where_.set_hexpand(true);
    where_.set_ellipsize(gtk::pango::EllipsizeMode::End);
    where_.add_css_class("small");
    let (back, forward, drop) = (gtk::Button::with_label("◀"), gtk::Button::with_label("▶"), gtk::Button::with_label("✕"));
    for b in [&back, &forward, &drop] { b.add_css_class("flat"); }
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    bar.set_margin_start(8);
    bar.set_margin_end(4);
    bar.append(&where_);
    bar.append(&back);
    bar.append(&forward);
    bar.append(&drop);
    let web = webkit6::WebView::new();
    crate::common::set_up_web(&web, &d);
    web.set_vexpand(true);
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
    for m in [8, 12] { card.set_margin_start(m); card.set_margin_end(m); }
    card.set_margin_bottom(6);
    card.set_visible(false);
    let area = gtk::Box::new(gtk::Orientation::Vertical, 0);
    area.append(&bar);
    area.append(&card);
    area.append(&web);
    win.set_child(Some(&area));

    let history: Vec<Value> = wb_core::paths::read_kept(&history_file(&d));
    let pos = history.len().saturating_sub(1);
    // A first start takes the last answers already logged; after that, what came while the panel was closed.
    let seen = history.iter().filter_map(|r| r["at"].as_i64()).max().unwrap_or(0);
    let panel = Rc::new(RefCell::new(Panel { history, pos, related: None, followed: None, last_at: 0, seen, scanned: 0 }));
    let w = Rc::new(Widgets { web, where_, back: back.clone(), forward: forward.clone(), drop: drop.clone(), shown: RefCell::default() });
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
    let card_of: RefCell<Option<(String, i64)>> = RefCell::default();
    every(1000, move || {
        wb_core::ask::panel_alive(&d.root);
        // The followed conversation's ask first, else the oldest one waiting.
        let asks = wb_core::ask::pending(&d.root);
        let mine = sessions::target(&d).map(|s| records::lineage(&d, &s)).unwrap_or_default();
        let ask = asks.iter().find(|a| a["session"].as_str().is_some_and(|s| mine.iter().any(|m| m == s))).or(asks.first());
        let key = ask.map(|a| (a["session"].as_str().unwrap_or("").to_string(), a["at"].as_i64().unwrap_or(0)));
        if key != *card_of.borrow() {
            match ask {
                Some(a) => { fill_card(&card, &d, a); status::focus_app(&d, "dev.whaybench.answer"); }
                None => card.set_visible(false),
            }
            *card_of.borrow_mut() = key;
        }
        let mut changed = false;
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
                        changed = true;
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
            if !fresh.is_empty() {
                let mine = session.as_deref().map(|s| records::lineage(&d, s)).unwrap_or_default();
                for (sid, a) in fresh.iter().rev().take(KEEP).rev() {
                    let at = a["at"].as_i64().unwrap_or(0);
                    p.seen = p.seen.max(at);
                    if p.history.iter().any(|r| same(r, at, sid)) { continue; }
                    p.history.push(json!({"at": at, "session": sid, "prompt": a["prompt"], "text": a["text"], "title": cut(&sessions::session_name(&d, sid), 24)}));
                    // The followed session's answer is shown; another's waits in the list.
                    if p.related.is_none() && mine.contains(sid) { p.pos = p.history.len() - 1; p.last_at = at; }
                }
                let skip = p.history.len().saturating_sub(KEEP);
                p.history.drain(..skip);
                p.pos = p.pos.saturating_sub(skip).min(p.history.len().saturating_sub(1));
                save(&d, &p.history);
                changed = true;
            }
            if session != p.followed {
                p.followed = session.clone();
                p.related = None;
                p.last_at = 0;
                changed = true;
            }
            if let Some(a) = session.as_deref().and_then(|s| newest_answer(&d, s)) {
                let at = a["at"].as_i64().unwrap_or(0);
                if at != p.last_at {
                    p.last_at = at;
                    let sid = a["sessionId"].as_str().unwrap_or("");
                    let known = p.history.iter().any(|r| same(r, at, sid));
                    if !known {
                        let name = session.as_deref().map(|s| sessions::session_name(&d, s)).unwrap_or_default();
                        p.history.push(json!({"at": at, "session": a["sessionId"], "prompt": a["prompt"], "text": a["text"], "title": cut(&name, 24)}));
                        let skip = p.history.len().saturating_sub(KEEP);
                        p.history.drain(..skip);
                        save(&d, &p.history);
                        p.related = None;
                    }
                    if p.related.is_none()
                        && let Some(i) = p.history.iter().position(|r| same(r, at, sid)) { p.pos = i; }
                    changed = true;
                }
            }
        }
        // Drawn each time: the page holds the theme's colours, and the same page is not loaded again.
        let _ = changed;
        draw(&panel.borrow(), &w, &d);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
