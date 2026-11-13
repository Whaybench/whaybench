//! The status line: the last system message for as long as it was given, else the notice of a newer program installed
//! and waiting for a restart, else of a newer release downloaded for the next start, else of one to install from
//! Whaybench → About.

use gtk::prelude::*;
use wb_core::{paths::Data, status};

use crate::common::every;

pub fn build(win: &gtk::ApplicationWindow, d: Data) {
    let label = gtk::Label::new(None);
    label.set_xalign(0.0);
    label.add_css_class("status");
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    win.set_child(Some(&label));
    every(500, move || {
        let text = status::current(&d).unwrap_or_else(|| if status::new_version_waiting(&d) { wb_core::i18n::t("new-version-available-whaybench-restart-tabs").to_string() }
            else if let Some(v) = wb_core::update::ready(&d) { wb_core::i18n::f("update-ready", &[("v", v)]) }
            else { wb_core::update::newer(&d).map(|v| wb_core::i18n::f("update-available", &[("v", v)])).unwrap_or_default() });
        label.set_text(&text);
    });
}
