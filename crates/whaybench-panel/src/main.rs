//! The product panels: each runs as its own app inside a panel of the compositor, and they talk through files
//! `whaybench-panel map | sessions | input | answer | status`.
mod answer;
mod common;
mod files;
mod input;
mod map;
mod mapdraw;
mod mapfill;
mod mapimport;
mod menu;
mod project;
mod markdown;
mod sessions;
mod status;
mod themeset;
mod view;

use gtk::{gio, glib, prelude::*};

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let kind = args.first().cloned().unwrap_or_default();
    let view_name = args.get(1).cloned().unwrap_or_else(|| wb_core::view::FIRST_VIEW.to_string());
    let view_app = wb_core::view::app_of(&view_name);
    let view_title = wb_core::view::panel_title(&view_name);
    let imported_app = args.get(1).and_then(|a| a.strip_prefix('@')).map(mapimport::app_id).unwrap_or_default();
    let (id, title) = match kind.as_str() {
        "map" if !imported_app.is_empty() => (imported_app.as_str(), wb_core::i18n::t("conversation-map")),
        "view" => (view_app.as_str(), view_title.as_str()),
        "menu" => ("dev.whaybench.menu", wb_core::i18n::t("menu")),
        "project" => ("dev.whaybench.project", wb_core::i18n::t("project")),
        "map" => ("dev.whaybench.map", wb_core::i18n::t("conversation-map")),
        "sessions" => ("dev.whaybench.sessions", wb_core::i18n::t("sessions")),
        "input" => ("dev.whaybench.input", wb_core::i18n::t("input")),
        "answer" => ("dev.whaybench.answer", wb_core::i18n::t("answer")),
        "files" => ("dev.whaybench.files", wb_core::i18n::t("files")),
        "status" => ("dev.whaybench.status", wb_core::i18n::t("status-line")),
        "theme" => ("dev.whaybench.theme", wb_core::i18n::t("themeset-title")),
        other => { eprintln!("whaybench-panel: unknown panel '{other}': map, sessions, input, answer, files, view, status"); return glib::ExitCode::FAILURE; }
    };
    let title = title.to_string();
    let data = common::data();
    // GTK's own texts (a text field's Cut, Copy, Paste) follow the program's language, not the system's.
    // No other thread exists yet.
    unsafe { std::env::set_var("LANGUAGE", wb_core::i18n::lang()); }
    wb_core::log::open(&data, &format!("panel-{kind}"), &wb_core::log::run_id(&data), "first");
    wb_core::log::install_panic_hook();
    let app_id = id.to_string();
    let app = gtk::Application::builder().application_id(id).flags(gio::ApplicationFlags::NON_UNIQUE).build();
    app.connect_activate(move |app| {
        let data = common::data();
        if !matches!(kind.as_str(), "menu" | "status") { common::set_panel(&app_id); }
        common::follow_theme(&data);
        let win = gtk::ApplicationWindow::new(app);
        win.set_title(Some(&title));
        win.set_decorated(false); // the compositor's panel has the title bar
        common::zoom_by_wheel(&win, &data);
        match kind.as_str() {
            "map" => map::build(&win, data, args.get(1).zip(args.get(2)).map(|(n, l)| (n.clone(), l.clone()))),
            "sessions" => sessions::build(&win, data),
            "input" => input::build(&win, data),
            "answer" => answer::build(&win, data),
            "files" => files::build(&win, data),
            "menu" => menu::build(&win, data),
            "project" => project::build(&win, data),
            "theme" => themeset::build(&win, data),
            "view" => view::build(&win, data, args.get(1).cloned().unwrap_or_else(|| wb_core::view::FIRST_VIEW.to_string())),
            _ => status::build(&win, data),
        }
        win.present();
    });
    app.run_with_args(&["whaybench-panel"])
}
