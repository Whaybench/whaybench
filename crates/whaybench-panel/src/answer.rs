//! The answer panel: the chosen session's last answer, kept answers with back and forward, and the answers of a
//! map node. The panel's own list holds the last 50 of all sessions.
use std::{cell::RefCell, rc::Rc};

use gtk::prelude::*;
use serde_json::{Value, json};
use wb_core::{paths::Data, records, sessions, status};

use crate::{common::{cut, every}, markdown};

const KEEP: usize = 50;

struct Panel {
    history: Vec<Value>,
    pos: usize,
    /// Kept answers while showing the answers of a node, and where in them.
    related: Option<(Vec<Value>, usize)>,
    followed: Option<String>,
    last_at: i64,
}

fn time_of(ms: i64, format: &str) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|t| t.with_timezone(&chrono::Local).format(format).to_string()).unwrap_or_default()
}

fn history_file(d: &Data) -> std::path::PathBuf { d.root.join("answer-history.json") }

fn save(d: &Data, h: &[Value]) {
    let tmp = history_file(d).with_extension("tmp");
    if std::fs::write(&tmp, Value::Array(h.to_vec()).to_string()).is_ok() { let _ = std::fs::rename(tmp, history_file(d)); }
}

struct Widgets { body: gtk::Box, where_: gtk::Label, back: gtk::Button, forward: gtk::Button, drop: gtk::Button, scroll: gtk::ScrolledWindow }

fn draw(p: &Panel, w: &Widgets, d: &Data) {
    while let Some(c) = w.body.first_child() { w.body.remove(&c); }
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
    let Some(row) = row else { return };
    let prompt = row.get("prompt").and_then(Value::as_str).unwrap_or("");
    let head = gtk::Label::new(Some(&format!("{} · {}", time_of(row["at"].as_i64().unwrap_or(0), "%H:%M"), cut(prompt, 120))));
    head.set_xalign(0.0);
    head.set_wrap(true);
    head.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    head.add_css_class("small");
    w.body.append(&head);
    markdown::show(row.get("text").and_then(Value::as_str).unwrap_or(""), &w.body);
    w.scroll.vadjustment().set_value(0.0);
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
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    for m in [8] { body.set_margin_start(m + 2); body.set_margin_end(m + 2); body.set_margin_top(m); body.set_margin_bottom(m); }
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&body));
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_vexpand(true);
    let area = gtk::Box::new(gtk::Orientation::Vertical, 0);
    area.append(&bar);
    area.append(&scroll);
    win.set_child(Some(&area));

    let history: Vec<Value> = std::fs::read_to_string(history_file(&d)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let pos = history.len().saturating_sub(1);
    let panel = Rc::new(RefCell::new(Panel { history, pos, related: None, followed: None, last_at: 0 }));
    let w = Rc::new(Widgets { body, where_, back: back.clone(), forward: forward.clone(), drop: drop.clone(), scroll });
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
    every(1000, move || {
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
                        let tidied = rows[i]["prompt"].as_str().is_some_and(|t| t.starts_with(wb_core::i18n::t("map-update")));
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
                    let known = p.history.iter().any(|r| r["at"].as_i64() == Some(at));
                    if !known {
                        let name = session.as_deref().map(|s| sessions::session_name(&d, s)).unwrap_or_default();
                        p.history.push(json!({"at": at, "session": a["sessionId"], "prompt": a["prompt"], "text": a["text"], "title": cut(&name, 24)}));
                        let skip = p.history.len().saturating_sub(KEEP);
                        p.history.drain(..skip);
                        save(&d, &p.history);
                        p.related = None;
                    }
                    if p.related.is_none() {
                        if let Some(i) = p.history.iter().position(|r| r["at"].as_i64() == Some(at)) { p.pos = i; }
                    }
                    changed = true;
                }
            }
        }
        if changed { draw(&panel.borrow(), &w, &d); }
    });
}
