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
            let b = gtk::SpinButton::with_range(lo, hi, if key.ends_with("size") && part == "base" { 1.0 } else if key == "title_height" { 5.0 } else { 0.5 });
            b.set_digits(if b.adjustment().step_increment() == 1.0 { 0 } else { 1 });
            b.set_width_chars(4);
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

/// A key's name, wrapped when long so the columns stay narrow.
fn name_label(name: &str) -> gtk::Label {
    gtk::Label::builder().label(name).halign(gtk::Align::Start).hexpand(true).wrap(true).width_chars(12).max_width_chars(18).xalign(0.0).build()
}

/// The grid a card's keys are laid out in, a name and an editor a row.
fn keys_grid() -> gtk::Grid {
    gtk::Grid::builder().column_spacing(20).row_spacing(10).margin_start(16).margin_end(16).margin_top(10).margin_bottom(16).build()
}

/// A group's card: its name over its keys, a name and an editor a row. The card is taken by its name and dropped in
/// another place (`drop_cards`); only the name, so dragging in a number field still selects its text.
fn card(group: &str, title: &str, rows: impl IntoIterator<Item = (String, gtk::Widget)>) -> gtk::Frame {
    let grid = keys_grid();
    for (row, (name, editor)) in rows.into_iter().enumerate() {
        grid.attach(&name_label(&name), 0, row as i32, 1, 1);
        editor.set_valign(gtk::Align::Center);
        grid.attach(&editor, 1, row as i32, 1, 1);
    }
    let frame = gtk::Frame::new(None);
    frame.set_widget_name(group);
    frame.set_child(Some(&grid));
    let head = gtk::Label::new(Some(title));
    head.set_cursor_from_name(Some("grab"));
    let drag = gtk::DragSource::new();
    drag.set_actions(gdk::DragAction::MOVE);
    let weak = frame.downgrade();
    // The card itself is what is carried: a text field takes a dropped string, not a frame.
    drag.connect_prepare(move |_, _, _| weak.upgrade().map(|f| gdk::ContentProvider::for_value(&f.to_value())));
    drag.connect_drag_begin(|drag, _| if let Some(f) = drag.content().and_then(|c| c.value(gtk::Frame::static_type()).ok()).and_then(|v| v.get::<gtk::Frame>().ok()) {
        drag.set_icon(Some(&gtk::WidgetPaintable::new(Some(&f))), 0, 0);
    });
    head.add_controller(drag);
    frame.set_label_widget(Some(&head));
    frame
}

/// Where the person put the cards: a list of group names a column, kept for each number of columns.
fn cards_file(d: &Data) -> std::path::PathBuf { d.root.join("themeset-cards.json") }

/// The cards in four columns, the number on a 1920-pixel screen, before the person moves any; with fewer columns they
/// flow into the shortest.
const CARDS: [&[&str]; 4] = [&["font", "light"], &["bar"], &["panel", "border", "opacity"], &["base", "surface", "dock", "joint"]];

fn saved_cards(d: &Data, n: usize) -> Option<Vec<Vec<String>>> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(cards_file(d)).ok()?).ok()?;
    serde_json::from_value::<Vec<Vec<String>>>(v.get(n.to_string())?.clone()).ok().filter(|c| c.len() == n)
}

/// A card dropped on a column goes where it is let go: before the first card whose middle is below that point.
fn drop_cards(d: &Data, cols: &[gtk::Box]) {
    for col in cols {
        let target = gtk::DropTarget::new(gtk::Frame::static_type(), gdk::DragAction::MOVE);
        let (d, here, all) = (d.clone(), col.clone(), cols.to_vec());
        target.connect_drop(move |_, value, _, y| {
            let Ok(card) = value.get::<gtk::Frame>() else { return false };
            let card: gtk::Widget = card.upcast();
            let Some(from) = card.parent().and_downcast::<gtk::Box>().filter(|p| all.contains(p)) else { return false };
            let mut before = here.first_child();
            while let Some(c) = before.clone() {
                if c != card && c.compute_bounds(&here).is_some_and(|b| b.y() + b.height() / 2.0 > y as f32) { break; }
                before = c.next_sibling();
            }
            from.remove(&card);
            match before.filter(|b| *b != card) {
                Some(b) => here.insert_child_after(&card, b.prev_sibling().as_ref()),
                None => here.append(&card),
            }
            let names: Vec<Vec<String>> = all.iter().map(|c| {
                let mut out = Vec::new();
                let mut w = c.first_child();
                while let Some(x) = w { out.push(x.widget_name().to_string()); w = x.next_sibling(); }
                out
            }).collect();
            let mut v: Value = std::fs::read_to_string(cards_file(&d)).ok().and_then(|t| serde_json::from_str(&t).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}));
            v[all.len().to_string()] = json!(names);
            let _ = wb_core::paths::write_atomic(&cards_file(&d), v.to_string());
            true
        });
        col.add_controller(target);
    }
}

/// Columns the cards flow into: as many as the screen is wide for, past the theme list, a card about 400 pixels wide;
/// two to four.
fn columns() -> usize {
    let width = gdk::Display::default().and_then(|d| d.monitors().item(0)).and_downcast::<gdk::Monitor>().map_or(1280, |m| m.geometry().width());
    ((width - 300) / 400).clamp(2, 4) as usize
}

/// The keys of the theme in force: what the theme offers for itself first, across the width, then every group of the
/// format as a card: where the person dragged them, else each into the shortest column so far, in the format's order,
/// so no column runs on below the others.
fn fields(d: &Data, holder: &gtk::Box) -> gtk::Box {
    let now = theme::load_active(d);
    let out = gtk::Box::new(gtk::Orientation::Vertical, 18);
    let n = columns();
    // What the theme offers for itself: only while it is in force; its keys side by side, as many to a row as there are columns.
    if !now.declared.is_empty() {
        let grid = keys_grid();
        let places = places(&now.declared, n);
        for (i, s) in now.declared.iter().enumerate() {
            let label = s.label(&now.name);
            let name = if s.unit.is_empty() { label } else { format!("{label} ({})", s.unit) };
            let (row, col) = places[i];
            grid.attach(&name_label(&name), col, row, 1, 1);
            let (d, theme, key) = (d.clone(), now.name.clone(), s.key.clone());
            let editor: gtk::Widget = if s.choices.is_empty() {
                let b = gtk::SpinButton::with_range(s.min, s.max, if s.max - s.min <= 4.0 { 0.05 } else { 1.0 });
                b.set_digits(if s.max - s.min <= 4.0 { 2 } else { 0 });
                b.set_width_chars(4);
                b.set_value(s.value);
                b.connect_value_changed(move |b| { let _ = theme::set_declared(&d, &theme, &key, b.value().into()); });
                b.upcast()
            } else {
                let labels: Vec<String> = (0..s.choices.len()).map(|c| s.choice_label(&now.name, c)).collect();
                let b = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
                b.set_selected(s.chosen.map_or(gtk::INVALID_LIST_POSITION, |c| c as u32));
                let (values, holder) = (s.choices.iter().map(|c| c.value.clone()).collect::<Vec<_>>(), holder.clone());
                // A choice changes many keys at once (a colour combination: every colour): the cards show them anew.
                b.connect_selected_notify(move |b| {
                    let Some(v) = values.get(b.selected() as usize) else { return };
                    let _ = theme::set_declared(&d, &theme, &key, v.clone());
                    let (d, holder) = (d.clone(), holder.clone());
                    glib::idle_add_local_once(move || crate::common::keep_scroll(&holder, || {
                        while let Some(c) = holder.first_child() { holder.remove(&c); }
                        holder.append(&fields(&d, &holder));
                    }));
                });
                b.upcast()
            };
            editor.set_valign(gtk::Align::Center);
            grid.attach(&editor, col + 1, row, 1, 1);
        }
        let frame = gtk::Frame::new(Some(t("themeset-group-theme")));
        frame.set_child(Some(&grid));
        out.append(&frame);
    }
    let columns = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    columns.set_homogeneous(true);
    let cols: Vec<gtk::Box> = (0..n).map(|_| { let c = gtk::Box::new(gtk::Orientation::Vertical, 18); columns.append(&c); c }).collect();
    let mut cards: Vec<(&str, usize, gtk::Frame)> = groups().into_iter().map(|(g, keys)| {
        let rows: Vec<(String, gtk::Widget)> = keys.into_iter()
            .map(|(part, key, kind)| (t(&format!("themeset-field-{key}")).to_string(), editor(d, &now.name, part, key, kind, &now.value(key)))).collect();
        (g, rows.len(), card(g, t(&format!("themeset-group-{g}")), rows))
    }).collect();
    // A card's height counted in rows, its name as two. Where the person put them first, the rest (a group a newer
    // version brings) each into the shortest column so far.
    let mut heights = vec![0usize; n];
    for (i, names) in saved_cards(d, n).or_else(|| (n == CARDS.len()).then(|| CARDS.iter().map(|c| c.iter().map(|g| g.to_string()).collect()).collect())).unwrap_or_default().iter().enumerate() {
        for name in names {
            if let Some(at) = cards.iter().position(|c| c.0 == name) {
                let (_, rows, card) = cards.remove(at);
                heights[i] += rows + 2;
                cols[i].append(&card);
            }
        }
    }
    for (_, rows, card) in cards {
        let shortest = (0..n).min_by_key(|i| heights[*i]).unwrap_or(0);
        heights[shortest] += rows + 2;
        cols[shortest].append(&card);
    }
    drop_cards(d, &cols);
    out.append(&columns);
    out
}

/// Where each of the theme's own settings goes, (row, grid column), `n` to a row: a run of settings with the same
/// `group` stays on one row, starting a new row when the rest of the row is too short for it.
fn places(settings: &[theme::Declared], n: usize) -> Vec<(i32, i32)> {
    let (mut out, mut row, mut used, mut i) = (Vec::new(), 0, 0, 0);
    while i < settings.len() {
        let g = &settings[i].group;
        let size = if g.is_empty() { 1 } else { settings[i..].iter().take_while(|s| s.group == *g).count() };
        if used > 0 && used + size.min(n) > n { row += 1; used = 0; }
        for _ in 0..size {
            if used == n { row += 1; used = 0; }
            out.push((row, used as i32 * 2));
            used += 1;
        }
        i += size;
    }
    out
}

/// The format's keys by group, in its order.
fn groups() -> Vec<(&'static str, Vec<(&'static str, &'static str, Kind)>)> {
    let mut out: Vec<(&'static str, Vec<_>)> = Vec::new();
    for (g, part, key, kind) in theme::FIELDS {
        match out.last_mut() { Some((last, keys)) if last == g => keys.push((*part, *key, *kind)), _ => out.push((*g, vec![(*part, *key, *kind)])) }
    }
    out
}

pub fn build(win: &gtk::ApplicationWindow, d: Data) {
    // Left the themes and what to do with the one in force, right its keys: one page, wide rather than long.
    let page = gtk::Box::new(gtk::Orientation::Vertical, 10);
    page.set_size_request(220, -1);

    // The themes, always open: a choice takes effect at once.
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    page.append(&gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&list).build());

    let buttons = gtk::Box::new(gtk::Orientation::Vertical, 8);
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
    let keys = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).propagate_natural_width(true).propagate_natural_height(true)
        .max_content_height(880).hexpand(true).vexpand(true).child(&holder).build();
    let whole = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    whole.set_margin_start(20);
    whole.set_margin_end(20);
    whole.set_margin_top(16);
    whole.set_margin_bottom(20);
    whole.append(&page);
    whole.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    whole.append(&keys);
    win.set_child(Some(&whole));

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
            holder.append(&fields(&d, &holder));
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
    // A theme chosen elsewhere, or a new one: the list and the keys follow, when the system tells of the change.
    arm(&d, Rc::new(refresh));
}

/// Watches the file naming the theme in force, the folder of themes and each theme's theme.json (a theme folder made
/// first and filled after shows when its theme.json comes); watched again when the folder of themes changes.
fn arm(d: &Data, refresh: Rc<dyn Fn(bool)>) {
    let mut folders: Vec<(std::path::PathBuf, &[&str])> = vec![(d.root.clone(), &["theme.json"]), (d.root.join("addons/themes"), &[])];
    folders.extend(theme::list(d).into_iter().filter(|(id, _)| id != theme::DEFAULT).map(|(id, _)| (d.root.join("addons/themes").join(id), &["theme.json"][..])));
    if let Ok(dirs) = std::fs::read_dir(d.root.join("addons/themes")) {
        for e in dirs.flatten().filter(|e| e.path().is_dir()) {
            let p = e.path();
            if !folders.iter().any(|(f, _)| *f == p) { folders.push((p, &["theme.json"])); }
        }
    }
    let (d2, r2) = (d.clone(), refresh.clone());
    crate::common::on_change("themeset", &folders, Rc::new(move || { r2(false); arm(&d2, r2.clone()); }));
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setting(group: &str) -> theme::Declared {
        theme::Declared { key: String::new(), name: String::new(), names: Default::default(), min: 0.0, max: 1.0, unit: String::new(), value: 0.0, choices: Vec::new(), chosen: None, group: group.into() }
    }
    #[test]
    fn a_group_of_settings_stays_on_one_row() {
        let s: Vec<_> = ["", "bg", "bg", "edge", "edge", ""].iter().map(|g| setting(g)).collect();
        assert_eq!(places(&s, 4), vec![(0, 0), (0, 2), (0, 4), (1, 0), (1, 2), (1, 4)]);
        assert_eq!(places(&s, 2), vec![(0, 0), (1, 0), (1, 2), (2, 0), (2, 2), (3, 0)]);
        assert_eq!(places(&s[..1], 2), vec![(0, 0)]);
    }
}
