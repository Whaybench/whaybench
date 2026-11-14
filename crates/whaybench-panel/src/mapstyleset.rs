//! Display style settings (Conversation map → Display style settings…): the map styles there are and, for the one
//! chosen, the settings its frame has. A change is saved at once in the style's settings.json and the map follows.
use std::{cell::RefCell, rc::Rc};

use gtk::{glib, prelude::*};
use serde_json::{Value, json};
use wb_core::paths::Data;

use crate::mapstyles::{self, Setting, Style};

fn t(key: &str) -> &'static str { wb_core::i18n::t(key) }

/// A value's words in the program's language (`""` is "none": everything).
fn value_word(key: &str, value: &str) -> &'static str { t(&format!("mapstyleset-{key}-{}", if value.is_empty() { "none" } else { value })) }

/// One setting's editor, showing the value in force and saving a change to the style.
fn editor(d: &Data, style: &Style, key: &'static str, setting: Setting) -> gtk::Widget {
    let save = { let (d, id) = (d.clone(), style.id.clone()); move |v: Value| { let _ = mapstyles::set(&d, &id, key, v); } };
    let now = style.raw.get(key).cloned().unwrap_or(Value::Null);
    match setting {
        Setting::Choice(options) => {
            let words: Vec<&str> = options.iter().map(|o| value_word(key, o)).collect();
            let b = gtk::DropDown::from_strings(&words);
            b.set_selected(options.iter().position(|o| Some(*o) == now.as_str()).unwrap_or(0) as u32);
            b.connect_selected_notify(move |b| save(json!(options[(b.selected() as usize).min(options.len() - 1)])));
            b.upcast()
        }
        Setting::Flag => {
            let b = gtk::CheckButton::new();
            b.set_active(now == Value::Bool(true));
            b.connect_toggled(move |b| save(json!(b.is_active())));
            b.upcast()
        }
        Setting::Count(lo, hi) => {
            let b = gtk::SpinButton::with_range(lo as f64, hi as f64, 1.0);
            b.set_value(now.as_u64().unwrap_or(lo) as f64);
            b.connect_value_changed(move |b| save(json!(b.value() as u64)));
            b.upcast()
        }
        Setting::List => {
            let words: Vec<&str> = now.as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            let b = gtk::Entry::builder().text(words.join(", ")).placeholder_text(t("mapstyleset-list-hint")).hexpand(true).build();
            // Saved when the person is done with it (Enter, or leaving the field), not at every letter.
            let keep = move |e: &gtk::Entry| {
                let list: Vec<String> = e.text().split(',').map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect();
                save(if list.is_empty() { Value::Null } else { json!(list) });
            };
            let k2 = keep.clone();
            b.connect_activate(move |e| k2(e));
            let focus = gtk::EventControllerFocus::new();
            { let e = b.clone(); focus.connect_leave(move |_| keep(&e)); }
            b.add_controller(focus);
            b.upcast()
        }
    }
}

/// The chosen style's settings, or why it has none.
fn fields(d: &Data, style: &Style) -> gtk::Box {
    let out = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let head = gtk::Label::new(Some(&style.name));
    head.add_css_class("h3");
    head.set_halign(gtk::Align::Start);
    out.append(&head);
    let settings = style.frame().map(mapstyles::settings_of).unwrap_or(&[]);
    if settings.is_empty() {
        let none = gtk::Label::new(Some(t("mapstyleset-no-settings")));
        none.set_halign(gtk::Align::Start);
        none.set_wrap(true);
        none.add_css_class("dim");
        out.append(&none);
        return out;
    }
    let grid = gtk::Grid::builder().column_spacing(12).row_spacing(6).build();
    for (row, (key, setting)) in settings.iter().enumerate() {
        let label = gtk::Label::new(Some(t(&format!("mapstyleset-{key}"))));
        label.set_halign(gtk::Align::Start);
        label.set_hexpand(true);
        grid.attach(&label, 0, row as i32, 1, 1);
        grid.attach(&editor(d, style, key, *setting), 1, row as i32, 1, 1);
    }
    out.append(&grid);
    out
}

/// Default, the tree: the program's own, with one setting.
fn default_fields(d: &Data) -> gtk::Box {
    let out = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let head = gtk::Label::new(Some(t("default")));
    head.add_css_class("h3");
    head.set_halign(gtk::Align::Start);
    out.append(&head);
    let grid = gtk::Grid::builder().column_spacing(12).row_spacing(6).build();
    let label = gtk::Label::new(Some(t("mapstyleset-indent")));
    label.set_halign(gtk::Align::Start);
    label.set_hexpand(true);
    let (lo, hi, _) = mapstyles::INDENT;
    let b = gtk::SpinButton::with_range(lo as f64, hi as f64, 1.0);
    b.set_value(mapstyles::default_indent(d) as f64);
    { let d = d.clone(); b.connect_value_changed(move |b| { let _ = mapstyles::set_default_indent(&d, b.value() as u64); }); }
    grid.attach(&label, 0, 0, 1, 1);
    grid.attach(&b, 1, 0, 1, 1);
    out.append(&grid);
    out
}

pub fn build(win: &gtk::ApplicationWindow, d: Data) {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 8);
    page.set_margin_start(12);
    page.set_margin_end(12);
    page.set_margin_top(8);
    page.set_margin_bottom(12);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    page.append(&list);
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.append(&holder);
    let reset = gtk::Button::with_label(t("mapstyleset-reset"));
    reset.set_halign(gtk::Align::Start);
    page.append(&reset);
    let scroll = crate::common::page_scroll(&page);
    win.set_child(Some(&scroll));

    // The style on show in the right part, and the styles the list was made from.
    let chosen: Rc<RefCell<String>> = Rc::default();
    let shown: Rc<RefCell<Vec<String>>> = Rc::default();
    let show = { let (d, holder, chosen, reset) = (d.clone(), holder.clone(), chosen.clone(), reset.clone()); move || {
        while let Some(c) = holder.first_child() { holder.remove(&c); }
        let id = chosen.borrow().clone();
        // Default has no add-on file to go back to.
        reset.set_visible(id != mapstyles::DEFAULT);
        if id == mapstyles::DEFAULT { holder.append(&default_fields(&d)); }
        else if let Some(style) = mapstyles::find(&d, &id) { holder.append(&fields(&d, &style)); }
    } };
    let refresh = { let (d, list, chosen, shown, show) = (d.clone(), list.clone(), chosen.clone(), shown.clone(), show.clone()); move || {
        let styles = mapstyles::list(&d);
        let mut ids: Vec<String> = vec![mapstyles::DEFAULT.to_string()];
        ids.extend(styles.iter().map(|s| s.id.clone()));
        if *shown.borrow() == ids { return; }
        *shown.borrow_mut() = ids.clone();
        while let Some(r) = list.row_at_index(0) { list.remove(&r); }
        // The style the main map shows comes chosen.
        let current = mapstyles::current(&d, "map");
        if !ids.contains(&chosen.borrow()) { *chosen.borrow_mut() = if ids.contains(&current) { current } else { mapstyles::DEFAULT.to_string() }; }
        let names = std::iter::once((mapstyles::DEFAULT.to_string(), t("default").to_string())).chain(styles.iter().map(|s| (s.id.clone(), s.name.clone())));
        for (id, name) in names {
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&gtk::Label::builder().label(&name).halign(gtk::Align::Start).build()));
            row.set_widget_name(&id);
            list.append(&row);
            if id == *chosen.borrow() { list.select_row(Some(&row)); }
        }
        show();
    } };
    refresh();
    {
        let (chosen, show) = (chosen.clone(), show.clone());
        list.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            if *chosen.borrow() == row.widget_name() { return; }
            *chosen.borrow_mut() = row.widget_name().to_string();
            show();
        });
    }
    { let (d, chosen) = (d.clone(), chosen.clone()); reset.connect_clicked(move |_| { mapstyles::reset(&d, &chosen.borrow()); show(); }); }
    // A style added or deleted meanwhile: the list follows.
    glib::timeout_add_local(std::time::Duration::from_secs(2), move || { refresh(); glib::ControlFlow::Continue });
}
