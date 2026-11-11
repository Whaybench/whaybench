//! The input panel: text to the chosen session. Enter sends. Shift+Enter is a new line, also when the
//! input method re-sends the Enter that ends a syllable without Shift: Shift is tracked here.
use std::{cell::Cell, rc::Rc};

use gtk::{gdk, glib, prelude::*};
use wb_core::{paths::Data, sessions, status};

use crate::common::{ctl, cut, every};

/// Where a sent text goes.
#[derive(Debug, PartialEq)]
pub enum Route { Nothing, Typed, Queued }

pub fn route(text: &str, session: Option<&str>) -> Route {
    if text.is_empty() || session.is_none() { return Route::Nothing; }
    if text.starts_with('/') && !text.contains('\n') { Route::Typed } else { Route::Queued }
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
    {
        let hint = hint.clone();
        view.buffer().connect_changed(move |b| hint.set_visible(b.char_count() == 0));
    }

    let sent_at: Rc<Cell<i64>> = Rc::default();
    {
        let (d, hint, sent_at) = (d.clone(), hint.clone(), sent_at.clone());
        every(1000, move || {
            let now = glib::monotonic_time();
            if now - sent_at.get() < 1_500_000 { hint.set_text(wb_core::i18n::t("sent")); return; }
            hint.set_text(&wb_core::i18n::f("input-enter-to-send-shift-enter", &[("a1", (sessions::focus(&d).map(|s| cut(&s, 8)).unwrap_or_else(|| wb_core::i18n::t("none").into())).to_string())]));
        });
    }
    let shift = Rc::new(Cell::new(false));
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let (shift, view, d) = (shift.clone(), view.clone(), d.clone());
        keys.connect_key_pressed(move |_, key, _, mods| {
            if matches!(key, gdk::Key::Shift_L | gdk::Key::Shift_R) { shift.set(true); }
            if !matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) { return glib::Propagation::Proceed; }
            if mods.contains(gdk::ModifierType::SHIFT_MASK) || shift.get() {
                view.buffer().insert_at_cursor("\n");
                return glib::Propagation::Stop;
            }
            let buffer = view.buffer();
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).trim().to_string();
            let session = sessions::focus(&d);
            match route(&text, session.as_deref()) {
                Route::Nothing => {}
                Route::Typed => {
                    let id = session.unwrap_or_default();
                    ctl(&d, vec!["--data-dir".into(), d.root.to_string_lossy().to_string(), "session".into(), "type".into(), id, text]);
                    buffer.set_text("");
                }
                Route::Queued => {
                    let id = session.unwrap_or_default();
                    if sessions::queue_prompt(&d, &id, &text) {
                        buffer.set_text("");
                        sent_at.set(glib::monotonic_time());
                    } else {
                        status::say(&d, &wb_core::i18n::f("request-not-sent-session-is-off", &[("a1", (cut(&id, 8)).to_string())]), 8);
                    }
                }
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
        assert_eq!(route("", Some("s")), Route::Nothing);
        assert_eq!(route("text", None), Route::Nothing);
        assert_eq!(route("/context", Some("s")), Route::Typed);
        assert_eq!(route("/exit", Some("s")), Route::Typed);
        assert_eq!(route("/x\nsecond line", Some("s")), Route::Queued);
        assert_eq!(route("plain text", Some("s")), Route::Queued);
    }
}
