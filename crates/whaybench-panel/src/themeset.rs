//! Theme settings (Settings → Theme settings…): the themes there are, a new theme from the one in force, its
//! changes cleared, and every key of the theme format by group. A change is saved at once in the theme's
//! settings.json and the whole window follows within two seconds.
use std::{cell::RefCell, rc::Rc};

use gtk::{gdk, glib, pango, prelude::*};
use serde_json::{Value, json};
use wb_core::{paths::Data, status, theme::{self, Kind}};

const JOINTS: [&str; 3] = ["always", "active", "never"];

fn t(key: &str) -> &'static str { wb_core::i18n::t(key) }

fn rgba(text: &str) -> gdk::RGBA { gdk::RGBA::parse(text).unwrap_or(gdk::RGBA::BLACK) }

fn hex(c: &gdk::RGBA) -> String {
    let b = |x: f32| (x * 255.0).round() as u8;
    if c.alpha() >= 1.0 { format!("#{:02x}{:02x}{:02x}", b(c.red()), b(c.green()), b(c.blue())) }
    else { format!("#{:02x}{:02x}{:02x}{:02x}", b(c.red()), b(c.green()), b(c.blue()), b(c.alpha())) }
}

/// One key's editor, showing the value in force and saving a change to the theme `name`.
fn editor(d: &Data, name: &str, part: &'static str, key: &'static str, kind: Kind, now: &Value) -> gtk::Widget {
    let save = { let (d, name) = (d.clone(), name.to_string()); move |v: Value| { let _ = theme::set(&d, &name, part, key, v); } };
    match kind {
        Kind::Colour => {
            let b = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::builder().with_alpha(true).build()));
            b.set_rgba(&rgba(now.as_str().unwrap_or("#000000")));
            b.connect_rgba_notify(move |b| save(hex(&b.rgba()).into()));
            b.upcast()
        }
        Kind::Number(lo, hi) => {
            let b = gtk::SpinButton::with_range(lo, hi, if key.ends_with("size") && part == "base" { 1.0 } else { 0.5 });
            b.set_digits(if b.adjustment().step_increment() < 1.0 { 1 } else { 0 });
            b.set_value(now.as_f64().unwrap_or(lo));
            b.connect_value_changed(move |b| save(b.value().into()));
            b.upcast()
        }
        Kind::Font => {
            let b = gtk::FontDialogButton::new(Some(gtk::FontDialog::new()));
            b.set_level(gtk::FontLevel::Family);
            b.set_font_desc(&pango::FontDescription::from_string(now.as_str().unwrap_or("")));
            b.connect_font_desc_notify(move |b| if let Some(f) = b.font_desc().and_then(|f| f.family()) { save(f.to_string().into()) });
            b.upcast()
        }
        Kind::Joint => {
            let labels: Vec<&str> = JOINTS.iter().map(|j| t(&format!("themeset-joint-{j}"))).collect();
            let b = gtk::DropDown::from_strings(&labels);
            b.set_selected(JOINTS.iter().position(|j| Some(*j) == now.as_str()).unwrap_or(0) as u32);
            b.connect_selected_notify(move |b| save(JOINTS[(b.selected() as usize).min(2)].into()));
            b.upcast()
        }
    }
}

/// The keys of the theme in force, by group.
fn fields(d: &Data) -> gtk::Box {
    let now = theme::load_active(d);
    let out = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let mut group = "";
    let mut grid = gtk::Grid::new();
    let mut row = 0;
    for (g, part, key, kind) in theme::FIELDS {
        if *g != group {
            group = g;
            let head = gtk::Label::new(Some(t(&format!("themeset-group-{g}"))));
            head.add_css_class("h3");
            head.set_halign(gtk::Align::Start);
            head.set_margin_top(10);
            out.append(&head);
            grid = gtk::Grid::builder().column_spacing(12).row_spacing(4).build();
            out.append(&grid);
            row = 0;
        }
        let label = gtk::Label::new(Some(t(&format!("themeset-field-{key}"))));
        label.set_halign(gtk::Align::Start);
        label.set_hexpand(true);
        grid.attach(&label, 0, row, 1, 1);
        grid.attach(&editor(d, &now.name, part, key, *kind, &now.value(key)), 1, row, 1, 1);
        row += 1;
    }
    out
}

pub fn build(win: &gtk::ApplicationWindow, d: Data) {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 6);
    page.set_margin_start(12);
    page.set_margin_end(12);
    page.set_margin_top(8);
    page.set_margin_bottom(12);

    // The themes, always open: a choice takes effect at once.
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    page.append(&list);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let save_as = gtk::Button::with_label(t("themeset-save-as"));
    let reset = gtk::Button::with_label(t("themeset-reset"));
    buttons.append(&save_as);
    buttons.append(&reset);
    page.append(&buttons);
    // The new theme's name, asked here: a dialog would sit in the panel anyway.
    let ask = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let entry = gtk::Entry::builder().placeholder_text(t("themeset-new-name")).hexpand(true).build();
    let ok = gtk::Button::with_label(t("themeset-save"));
    ask.append(&entry);
    ask.append(&ok);
    ask.set_visible(false);
    page.append(&ask);

    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.append(&holder);
    let scroll = crate::common::page_scroll(&page);
    win.set_child(Some(&scroll));

    let shown: Rc<RefCell<(String, Vec<String>)>> = Rc::default();
    let refresh = {
        let (d, list, holder, shown) = (d.clone(), list.clone(), holder.clone(), shown.clone());
        move |force: bool| {
            let active = theme::load_active(&d).name;
            let themes = theme::list(&d);
            let ids: Vec<String> = themes.iter().map(|x| x.0.clone()).collect();
            if !force && *shown.borrow() == (active.clone(), ids.clone()) { return; }
            *shown.borrow_mut() = (active.clone(), ids);
            while let Some(r) = list.row_at_index(0) { list.remove(&r); }
            for (id, name) in &themes {
                let row = gtk::ListBoxRow::new();
                row.set_child(Some(&gtk::Label::builder().label(name).halign(gtk::Align::Start).build()));
                row.set_widget_name(id);
                list.append(&row);
                if *id == active { list.select_row(Some(&row)); }
            }
            while let Some(c) = holder.first_child() { holder.remove(&c); }
            holder.append(&fields(&d));
        }
    };
    refresh(true);
    {
        let d = d.clone();
        list.connect_row_activated(move |_, row| { let _ = status::post(&d, &json!({"cmd": "theme", "name": row.widget_name().to_string()})); });
    }
    { let ask = ask.clone(); save_as.connect_clicked(move |_| ask.set_visible(!ask.is_visible())); }
    {
        let (d, r) = (d.clone(), refresh.clone());
        reset.connect_clicked(move |_| { let _ = theme::reset(&d, &theme::load_active(&d).name); r(true); });
    }
    {
        let (d, field, ask) = (d.clone(), entry.clone(), ask.clone());
        let save = move || {
            let entry = &field;
            let from = theme::load_active(&d).name;
            match theme::save_as(&d, &from, &entry.text()) {
                Ok(id) => {
                    status::say(&d, &wb_core::i18n::f("themeset-saved", &[("name", entry.text().to_string())]), 6);
                    let _ = status::post(&d, &json!({"cmd": "theme", "name": id}));
                    entry.set_text("");
                    ask.set_visible(false);
                }
                Err(e) => status::say(&d, &wb_core::i18n::f("themeset-save-failed", &[("e", e.to_string())]), 8),
            }
        };
        let s2 = save.clone();
        ok.connect_clicked(move |_| s2());
        entry.connect_activate(move |_| save());
    }
    // A theme chosen elsewhere, or a new one: the list and the keys follow.
    glib::timeout_add_local(std::time::Duration::from_secs(1), move || { refresh(false); glib::ControlFlow::Continue });
}
