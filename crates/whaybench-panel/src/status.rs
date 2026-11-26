//! The status line: the last system message for as long as it was given, else the notice of a newer program installed
//! and waiting for a restart, else of a newer release downloaded for the next start, else of one to install from
//! Whaybench → About.

use gtk::prelude::*;
use wb_core::{paths::Data, status};

use std::rc::Rc;


pub fn build(win: &gtk::ApplicationWindow, d: Data) {
    let label = gtk::Label::new(None);
    label.set_xalign(0.0);
    label.add_css_class("status");
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    win.set_child(Some(&label));
    // Read when a message comes or goes, an update is found or a new program put in place, as the system tells; and once
    // more when the message shown runs out (it has a time), not on a clock.
    let show: crate::common::Wake = Rc::default();
    let (d0, again) = (d.clone(), show.clone());
    let f: Rc<dyn Fn()> = Rc::new(move || {
        let shown = status::current_until(&d);
        let text = shown.as_ref().map(|m| m.0.clone()).unwrap_or_else(|| if status::new_version_waiting(&d) { wb_core::i18n::t("new-version-available-whaybench-restart-tabs").to_string() }
            else if let Some(v) = wb_core::update::ready(&d) { wb_core::i18n::f("update-ready", &[("v", v)]) }
            else { wb_core::update::newer(&d).map(|v| wb_core::i18n::f("update-available", &[("v", v)])).unwrap_or_default() });
        if label.text() != text { label.set_text(&text); }
        if let Some((_, until)) = shown {
            crate::common::wake_at(&again, std::time::Duration::from_millis(until.saturating_sub(wb_core::clock::now_ms() as u128) as u64 + 30));
        }
    });
    show.borrow_mut().0 = Some(f.clone());
    crate::common::follow_data("status", &d0, 250, |d| status::program_folder(d).map(|p| vec![(p, &[][..])]).unwrap_or_default(), f);
}
