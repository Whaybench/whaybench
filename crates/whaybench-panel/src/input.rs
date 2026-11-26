//! The input panel: text to the chosen session. Enter sends. Shift+Enter is a new line, also when the
//! input method re-sends the Enter that ends a syllable without Shift: Shift is tracked here.
use std::{cell::{Cell, RefCell}, rc::Rc};

use gtk::{gdk, glib, prelude::*};
use wb_core::{paths::Data, sessions, status};

use crate::common::{ctl, cut};

/// What a text is for: a conversation, or a tab whose agent makes its conversation with its first prompt (Codex).
#[derive(Clone, Debug, PartialEq)]
pub enum To { Session(String), Tab(String) }

impl To {
    fn front(d: &Data) -> Option<To> { sessions::focus(d).map(To::Session).or_else(|| sessions::first_prompt_tab(d).map(To::Tab)) }

    /// Followed to where it lives now: a moved conversation, a tab that has made its own.
    fn now(self, d: &Data) -> To {
        match self {
            To::Session(s) => To::Session(sessions::current_id(d, &s)),
            To::Tab(t) => sessions::open_tabs(d).into_iter().find(|o| o.tab == t).and_then(|o| o.session).map_or(To::Tab(t), To::Session),
        }
    }

    fn project(&self, d: &Data) -> Option<String> {
        match self {
            // A new conversation's record comes with its first hook: until then, the project of the tab that shows it.
            To::Session(s) => wb_core::roots::Roots::new(d.clone()).project_of(s).or_else(|| {
                let tab = sessions::open_tabs(d).into_iter().find(|t| t.session.as_ref() == Some(s))?;
                sessions::tab_project(d, &tab.tab)
            }),
            To::Tab(t) => sessions::tab_project(d, t),
        }
    }
}

/// The project a draft begun now is of ("" is the host's): the conversation's it goes to, or of the tab in front that
/// takes no text yet.
fn owner_of_draft(d: &Data, to: Option<&To>) -> String {
    match to {
        Some(to) => to.project(d),
        None => sessions::empty_tab(d).and_then(|t| sessions::tab_project(d, &t)),
    }.unwrap_or_default()
}

/// Where a sent text goes.
#[derive(Debug, PartialEq)]
pub enum Route { Nothing, Typed, Queued }

pub fn route(text: &str, to: Option<&To>) -> Route {
    if text.is_empty() { return Route::Nothing; }
    match to {
        None => Route::Nothing,
        Some(To::Session(_)) if text.starts_with('/') && !text.contains('\n') => Route::Typed,
        Some(_) => Route::Queued,
    }
}

/// The conversation a draft goes to: the one it was begun for, until the person chooses another. One that only an
/// agent's request brought to the front does not take it.
pub fn draft_goes_to(begun: Option<&To>, front: Option<&To>, by_agent: bool) -> Option<To> {
    if by_agent && begun.is_some() { begun } else { front }.cloned()
}

thread_local! {
    /// Writes the line under the field again (who it goes to): set by `build`.
    static REFRESH: RefCell<Option<Rc<dyn Fn()>>> = RefCell::new(None);
}

/// The line under the field written again `ms` from now (a name that came, "sent" that has had its moment).
fn refresh_soon(ms: u64) {
    if let Some(f) = REFRESH.with(|r| r.borrow().clone()) { glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || f()); }
}

pub fn build(win: &gtk::ApplicationWindow, d: Data) {
    let view = gtk::TextView::new();
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_left_margin(8);
    view.set_right_margin(8);
    view.set_top_margin(6);
    view.set_bottom_margin(6);
    let hint = gtk::Label::new(None);
    hint.set_xalign(0.0);
    hint.add_css_class("small");
    // The hint sits inside the empty field, like a placeholder; it does not take room from the text.
    hint.set_halign(gtk::Align::Start);
    hint.set_valign(gtk::Align::Start);
    hint.set_margin_start(10);
    hint.set_margin_top(6);
    hint.set_can_target(false);
    hint.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&view));
    let area = gtk::Overlay::new();
    area.set_child(Some(&scroll));
    area.add_overlay(&hint);
    win.set_child(Some(&area));
    // The project of the conversation a draft was begun for: it goes only there, and goes when that project locks.
    let owner: Rc<RefCell<Option<String>>> = Rc::default();
    // The conversation a draft goes to (`draft_goes_to`).
    let goes: Rc<RefCell<Option<To>>> = Rc::default();
    {
        let (hint, owner, goes, d) = (hint.clone(), owner.clone(), goes.clone(), d.clone());
        view.buffer().connect_changed(move |b| {
            hint.set_visible(b.char_count() == 0);
            // Emptied: the line under it says again where a prompt goes now.
            if b.char_count() == 0 { *owner.borrow_mut() = None; *goes.borrow_mut() = None; refresh_soon(0); }
            else if owner.borrow().is_none() {
                let to = To::front(&d);
                *owner.borrow_mut() = Some(owner_of_draft(&d, to.as_ref()));
                *goes.borrow_mut() = to;
            }
        });
    }
    // A first letter being composed is not in the text yet.
    { let hint = hint.clone(); view.connect_preedit_changed(move |v, t| hint.set_visible(t.is_empty() && v.buffer().char_count() == 0)); }
    let target = {
        let (d, goes, view) = (d.clone(), goes.clone(), view.clone());
        move || {
            let front = To::front(&d);
            if view.buffer().char_count() == 0 { return front; }
            let to = draft_goes_to(goes.borrow().as_ref(), front.as_ref(), sessions::focus_by_agent(&d)).map(|t| t.now(&d));
            goes.borrow_mut().clone_from(&to);
            to
        }
    };

    let sent_at: Rc<Cell<i64>> = Rc::default();
    {
        let (d, hint, sent_at, owner, view) = (d.clone(), hint.clone(), sent_at.clone(), owner.clone(), view.clone());
        let target = target.clone();
        // The keys that switch the input language, as fcitx5 has them now (they are set in Settings → Input…): asked
        // again when its settings change.
        let switch: Rc<RefCell<String>> = Default::default();
        let keys_changed = Rc::new(Cell::new(true));
        let line: Rc<RefCell<(Option<String>, Option<(String, String)>)>> = Default::default();
        // Who it goes to and what it is called: looked at when the conversations, the tabs, the projects or the input
        // method's settings change, as the system tells; "sent" stays its moment and goes by a timer of its own.
        let d0 = d.clone();
        let k2 = keys_changed.clone();
        let f: Rc<dyn Fn()> = Rc::new(move || {
            let now = glib::monotonic_time();
            // The input method's settings changed (or first): its keys asked again (each ask is a process).
            if keys_changed.replace(false) {
                let keys = wb_core::input::switch_keys();
                *switch.borrow_mut() = if keys.is_empty() { String::new() } else { wb_core::i18n::f("input-switch", &[("keys", keys.join(" / "))]) };
            }
            // A draft for a project that locked: nothing of it stays on screen (spec 3.22).
            let gone = owner.borrow().as_deref().filter(|p| !p.is_empty()).is_some_and(|p| !wb_core::roots::stubs(&d).iter().any(|s| s.id == p && s.open()));
            if gone { view.buffer().set_text(""); }
            // Just sent: "sent" for its moment, and the name asked again after (the first prompt may give one).
            if now - sent_at.get() < 1_500_000 { line.borrow_mut().0 = None; hint.set_text(wb_core::i18n::t("sent")); return; }
            let to = target();
            // A conversation's name can take Claude Code's answer (a process): asked off this thread, once for each
            // conversation the field is empty for, and the line written again when it comes.
            let want = match &to { Some(To::Session(s)) if view.buffer().char_count() == 0 => Some(s.clone()), _ => None };
            if want.is_none() { line.borrow_mut().0 = None; }
            if let Some(s) = want.clone().filter(|w| line.borrow().0.as_ref() != Some(w)) {
                line.borrow_mut().0 = Some(s.clone());
                let (root, line2) = (d.root.clone(), line.clone());
                glib::spawn_future_local(async move {
                    let s2 = s.clone();
                    if let Ok(text) = gtk::gio::spawn_blocking(move || wb_core::agents::session_line(&root, &s2)).await {
                        // Only the one still wanted: an earlier conversation's answer coming last does not take its place.
                        if line2.borrow().0.as_ref() == Some(&s) { line2.borrow_mut().1 = Some((s, text)); refresh_soon(0); }
                    }
                });
            }
            let known = line.borrow().1.clone();
            let who = match to {
                None => wb_core::i18n::t("none").into(),
                Some(To::Tab(t)) => {
                    let agent = sessions::open_tabs(&d).into_iter().find(|o| o.tab == t).and_then(|o| o.agent);
                    format!("{} · {}", wb_core::i18n::t("new-session"), wb_core::agents::name_of(&d.root, agent.as_deref()))
                }
                Some(To::Session(s)) => match known { Some((of, text)) if of == s => text, _ => wb_core::agents::session_label(&d.root, &s) },
            };
            hint.set_text(&wb_core::i18n::f("input-enter-to-send-shift-enter", &[("a1", who), ("switch", switch.borrow().clone())]));
        });
        REFRESH.with(|r| *r.borrow_mut() = Some(f.clone()));
        // fcitx5's own settings folder: the switching keys are set there.
        let fcitx = |_: &Data| -> Vec<(std::path::PathBuf, &'static [&'static str])> { std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
            .map(|c| (c.join("fcitx5"), &[][..])).into_iter().collect() };
        // Its keys asked again only when fcitx5's settings change (each ask is a process); the rest on any change.
        let f2 = f.clone();
        let (k3, f3) = (k2.clone(), f2.clone());
        crate::common::on_change_existing("input-keys", &fcitx(&d0), Rc::new(move || { k2.set(true); f2(); }));
        // fcitx5 coming up (or again) on the session bus: its keys are asked then; it may start after this panel.
        // Watched for the panel's life (the watch is not dropped by its id going).
        let _ = gtk::gio::bus_watch_name(gtk::gio::BusType::Session, "org.fcitx.Fcitx5", gtk::gio::BusNameWatcherFlags::NONE,
            move |_, _, _| { k3.set(true); f3(); }, |_, _| {});
        crate::common::follow_data("input", &d0, 250, |_| Vec::new(), f);
    }
    let shift = Rc::new(Cell::new(false));
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let (shift, view, d, owner) = (shift.clone(), view.clone(), d.clone(), owner.clone());
        keys.connect_key_pressed(move |_, key, _, mods| {
            if matches!(key, gdk::Key::Shift_L | gdk::Key::Shift_R) { shift.set(true); }
            if !matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) { return glib::Propagation::Proceed; }
            if mods.contains(gdk::ModifierType::SHIFT_MASK) || shift.get() {
                view.buffer().insert_at_cursor("\n");
                return glib::Propagation::Stop;
            }
            let buffer = view.buffer();
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).trim().to_string();
            let to = target();
            // Typed for a project's conversation, it goes to no other place's (host text may go anywhere: spec 3.22).
            let here = to.as_ref().and_then(|t| t.project(&d)).unwrap_or_default();
            if owner.borrow().as_ref().is_some_and(|o| !o.is_empty() && *o != here) { status::say(&d, wb_core::i18n::t("briefsafe-draft-stays"), 8); return glib::Propagation::Stop; }
            match (route(&text, to.as_ref()), to) {
                (Route::Typed, Some(To::Session(id))) => {
                    ctl(&d, vec!["--data-dir".into(), d.root.to_string_lossy().to_string(), "session".into(), "type".into(), id, text]);
                    buffer.set_text("");
                }
                (Route::Queued, Some(to)) => {
                    let (queued, id) = match &to {
                        To::Session(id) => (sessions::queue_prompt(&d, id, &text), id),
                        To::Tab(tab) => (sessions::queue_first(&d, tab, &text), tab),
                    };
                    if queued {
                        buffer.set_text("");
                        sent_at.set(glib::monotonic_time());
                        refresh_soon(0);
                        refresh_soon(1600);
                    } else {
                        status::say(&d, &wb_core::i18n::f("request-not-sent-session-is-off", &[("a1", cut(id, 8).to_string())]), 8);
                    }
                }
                _ => {}
            }
            glib::Propagation::Stop
        });
    }
    {
        let shift = shift.clone();
        keys.connect_key_released(move |_, key, _, _| if matches!(key, gdk::Key::Shift_L | gdk::Key::Shift_R) { shift.set(false); });
    }
    view.add_controller(keys);
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave(move |_| shift.set(false));
    view.add_controller(focus);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn where_a_text_goes() {
        let (s, t) = (To::Session("s".into()), To::Tab("t".into()));
        assert_eq!(route("", Some(&s)), Route::Nothing);
        assert_eq!(route("text", None), Route::Nothing);
        assert_eq!(route("/context", Some(&s)), Route::Typed);
        assert_eq!(route("/exit", Some(&s)), Route::Typed);
        assert_eq!(route("/x\nsecond line", Some(&s)), Route::Queued);
        assert_eq!(route("plain text", Some(&s)), Route::Queued);
        assert_eq!(route("plain text", Some(&t)), Route::Queued, "a tab's first prompt");
        assert_eq!(route("/model", Some(&t)), Route::Queued, "no session to name to the typing command");
    }
    #[test]
    fn a_draft_stays_with_its_conversation_unless_the_person_chooses_another() {
        let s = |v: &str| To::Session(v.to_string());
        let (a, b, t) = (s("a"), s("b"), To::Tab("t".into()));
        assert_eq!(draft_goes_to(Some(&a), Some(&b), true), Some(s("a")), "an agent brought b to the front");
        assert_eq!(draft_goes_to(Some(&a), Some(&b), false), Some(s("b")), "the person chose b");
        assert_eq!(draft_goes_to(None, Some(&b), true), Some(s("b")), "begun with no conversation in front");
        assert_eq!(draft_goes_to(Some(&a), Some(&t), true), Some(s("a")), "an agent opened a tab with no session yet");
        assert_eq!(draft_goes_to(Some(&a), None, true), Some(s("a")), "or one that takes no prompt yet");
        assert_eq!(draft_goes_to(Some(&a), None, false), None, "the person did");
    }
    #[test]
    fn a_draft_begun_before_a_projects_tab_takes_text_is_the_projects() {
        let d = Data::new(std::env::temp_dir().join(format!("whaybench-owner-{}", std::process::id())));
        sessions::write_tab(&d, "t", None, false);
        sessions::set_tab_project(&d, "t", Some("p1"));
        sessions::set_focus_empty(&d, "t", "test", false);
        assert!(To::front(&d).is_none(), "it takes no text yet");
        assert_eq!(owner_of_draft(&d, None), "p1");
        sessions::write_tab(&d, "t", Some("new"), false);
        assert_eq!(owner_of_draft(&d, Some(&To::Session("new".into()))), "p1", "a session with no record yet");
        sessions::set_tab_project(&d, "t", None);
        assert_eq!(owner_of_draft(&d, None), "", "a host tab");
        let _ = std::fs::remove_dir_all(&d.root);
    }
    #[test]
    fn a_draft_for_an_empty_tab_follows_the_conversation_it_makes() {
        let d = Data::new(std::env::temp_dir().join(format!("whaybench-input-{}", std::process::id())));
        sessions::write_tab(&d, "t", None, false);
        assert_eq!(To::Tab("t".into()).now(&d), To::Tab("t".into()));
        sessions::write_tab(&d, "t", Some("s1"), false);
        assert_eq!(To::Tab("t".into()).now(&d), To::Session("s1".into()));
        let _ = std::fs::remove_dir_all(&d.root);
    }
}
