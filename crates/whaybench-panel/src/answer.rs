//! The answer panel: the chosen session's last answer, kept answers with back and forward, and the answers of a
//! map node. The panel's own list holds the last 50 of all sessions.
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
}

fn time_of(ms: i64, format: &str) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|t| t.with_timezone(&chrono::Local).format(format).to_string()).unwrap_or_default()
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
    crate::common::web_menu_in_language(&web);
    crate::common::follow_zoom(&web, &d);
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
    let area = gtk::Box::new(gtk::Orientation::Vertical, 0);
    area.append(&bar);
    area.append(&web);
    win.set_child(Some(&area));

    let history: Vec<Value> = std::fs::read_to_string(history_file(&d)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let pos = history.len().saturating_sub(1);
    let panel = Rc::new(RefCell::new(Panel { history, pos, related: None, followed: None, last_at: 0 }));
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
        // Drawn each time: the page holds the theme's colours, and the same page is not loaded again.
        let _ = changed;
        draw(&panel.borrow(), &w, &d);
    });
}
