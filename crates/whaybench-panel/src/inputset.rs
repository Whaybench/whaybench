//! Input settings (Settings → Input…): fcitx5's current group of input methods, the keys that switch them, and each
//! method's own settings, drawn from the form fcitx5 describes for it, as fcitx5's own setting program does. Every
//! change goes to fcitx5 at once; nothing is kept here.
use std::{cell::{Cell, RefCell}, rc::Rc};

use gtk::{gdk, glib, prelude::*};
use serde_json::{Map, Value, json};
use wb_core::{input, paths::Data};

fn t(key: &str) -> &'static str { wb_core::i18n::t(key) }

const GLOBAL: &str = "fcitx://config/global";

struct Ui { d: Data, methods: gtk::Box, adds: gtk::Box, detail: gtk::Box, err: gtk::Label }

pub fn build(win: &gtk::ApplicationWindow, d: Data) {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 8);
    page.set_margin_start(12); page.set_margin_end(12); page.set_margin_top(10); page.set_margin_bottom(12);
    let scroll = crate::common::page_scroll(&page);
    win.set_child(Some(&scroll));
    if let Err(e) = input::group() {
        wb_core::log::warn("input", "settings not read", &e, &[]);
        page.append(&gtk::Label::new(Some(t("inputset-none"))));
        return;
    }
    let ui = Rc::new(Ui { d, methods: gtk::Box::new(gtk::Orientation::Vertical, 4), adds: gtk::Box::new(gtk::Orientation::Horizontal, 6), detail: gtk::Box::new(gtk::Orientation::Vertical, 6), err: gtk::Label::new(None) });
    ui.err.add_css_class("dim"); ui.err.set_wrap(true); ui.err.set_halign(gtk::Align::Start);
    page.append(&heading(t("inputset-methods")));
    page.append(&ui.methods);
    page.append(&ui.adds);
    page.append(&heading(t("inputset-trigger")));
    match input::config(GLOBAL) {
        Ok(form) => {
            let err = ui.err.clone();
            let save: Rc<dyn Fn(Value)> = Rc::new(move |keys| report(&err, input::set_config(GLOBAL, &json!({"Hotkey": {"TriggerKeys": keys}}))));
            page.append(&key_list(&form.values["Hotkey"]["TriggerKeys"], save));
        }
        Err(e) => ui.err.set_text(&e),
    }
    page.append(&ui.detail);
    page.append(&ui.err);
    fill(&ui);
}

fn heading(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_halign(gtk::Align::Start);
    l.add_css_class("heading");
    l
}

/// A failed save is shown under the page and kept in the record.
fn report(err: &gtk::Label, r: Result<(), String>) {
    match r {
        Ok(()) => err.set_text(""),
        Err(e) => { wb_core::log::warn("input", "setting not saved", &e, &[]); err.set_text(&e); }
    }
}

/// The group's input methods, each with Up, Settings and Remove, and the ones that can be added.
fn fill(ui: &Rc<Ui>) {
    while let Some(c) = ui.methods.first_child() { ui.methods.remove(&c); }
    while let Some(c) = ui.adds.first_child() { ui.adds.remove(&c); }
    let (Ok((group, layout, items)), Ok(all)) = (input::group(), input::available()) else { return };
    let label = |name: &str| all.iter().find(|m| m.name == name).map(|m| m.label.clone()).unwrap_or_else(|| name.to_string());
    let change = |items: Vec<(String, String)>| {
        let (ui, group, layout) = (ui.clone(), group.clone(), layout.clone());
        move || { report(&ui.err, input::set_group(&group, &layout, &items)); fill(&ui); }
    };
    for (i, (name, _)) in items.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let l = gtk::Label::new(Some(&label(name)));
        l.set_halign(gtk::Align::Start); l.set_hexpand(true);
        row.append(&l);
        if i > 0 {
            let mut moved = items.clone();
            moved.swap(i - 1, i);
            let b = gtk::Button::with_label(t("inputset-up"));
            let go = change(moved);
            b.connect_clicked(move |_| go());
            row.append(&b);
        }
        if all.iter().any(|m| m.name == *name && m.configurable) {
            let b = gtk::Button::with_label(t("inputset-settings"));
            let (ui, name, shown) = (ui.clone(), name.clone(), label(name));
            b.connect_clicked(move |_| show_form(&ui, &name, &shown));
            row.append(&b);
        }
        if items.len() > 1 {
            let mut left = items.clone();
            left.remove(i);
            let b = gtk::Button::with_label(t("inputset-remove"));
            let go = change(left);
            b.connect_clicked(move |_| go());
            row.append(&b);
        }
        ui.methods.append(&row);
    }
    // To add: the engines fcitx5 has, and the keyboard layouts the languages name (fcitx5 has hundreds of layouts).
    let layouts = language_layouts(&ui.d);
    let offer: Vec<&input::Method> = all.iter()
        .filter(|m| !items.iter().any(|(n, _)| *n == m.name))
        .filter(|m| m.name.strip_prefix("keyboard-").is_none_or(|l| layouts.iter().any(|x| x == l)))
        .collect();
    if offer.is_empty() { return; }
    let names: Vec<String> = offer.iter().map(|m| m.name.clone()).collect();
    let shown: Vec<&str> = offer.iter().map(|m| m.label.as_str()).collect();
    let pick = gtk::DropDown::from_strings(&shown);
    pick.set_hexpand(true);
    let b = gtk::Button::with_label(t("inputset-add"));
    {
        let (pick, items, ui, group, layout) = (pick.clone(), items.clone(), ui.clone(), group.clone(), layout.clone());
        b.connect_clicked(move |_| {
            let Some(name) = names.get(pick.selected() as usize) else { return };
            let mut more = items.clone();
            more.push((name.clone(), String::new()));
            report(&ui.err, input::set_group(&group, &layout, &more));
            fill(&ui);
        });
    }
    ui.adds.append(&pick);
    ui.adds.append(&b);
}

/// The keyboard layouts the language add-ons name, and `us`.
fn language_layouts(d: &Data) -> Vec<String> {
    let mut out = vec!["us".to_string()];
    for e in std::fs::read_dir(d.root.join("addons/languages")).into_iter().flatten().flatten() {
        let about: Value = std::fs::read_to_string(e.path().join("language.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        if let Some(l) = about["input"]["layout"].as_str() && !out.iter().any(|x| x == l) { out.push(l.to_string()); }
    }
    out
}

/// The method's own settings, under the list.
fn show_form(ui: &Rc<Ui>, name: &str, shown: &str) {
    while let Some(c) = ui.detail.first_child() { ui.detail.remove(&c); }
    ui.detail.append(&heading(&wb_core::i18n::f("inputset-engine", &[("name", shown.to_string())])));
    let uri = format!("fcitx://config/inputmethod/{name}");
    match input::config(&uri) {
        Ok(form) => {
            let Some((_, top)) = form.types.first() else { return };
            let grid = gtk::Grid::builder().column_spacing(12).row_spacing(6).build();
            let mut row = 0;
            settings(&grid, &mut row, &uri, &form, top, &form.values, &[], &ui.err);
            ui.detail.append(&grid);
        }
        Err(e) => ui.err.set_text(&e),
    }
}

/// One row per setting, by its kind; a nested form's settings follow under its name.
#[allow(clippy::too_many_arguments)]
fn settings(grid: &gtk::Grid, row: &mut i32, uri: &str, form: &input::Form, list: &[input::Setting], values: &Value, path: &[String], err: &gtk::Label) {
    for s in list {
        let value = values.get(&s.key).cloned().unwrap_or_else(|| s.default.clone());
        let text = value.as_str().unwrap_or_default().to_string();
        let save: Rc<dyn Fn(Value)> = {
            let (uri, err, mut at) = (uri.to_string(), err.clone(), path.to_vec());
            at.push(s.key.clone());
            Rc::new(move |v| report(&err, input::set_config(&uri, &nest(&at, v))))
        };
        let w: gtk::Widget = match s.kind.as_str() {
            "Boolean" => {
                let b = gtk::CheckButton::new();
                b.set_active(text == "True");
                b.connect_toggled(move |b| save(json!(if b.is_active() { "True" } else { "False" })));
                b.upcast()
            }
            "Enum" => {
                let list = |key: &str| -> Vec<String> { ordered(&s.notes[key]).into_iter().filter_map(|v| v.as_str().map(String::from)).collect() };
                let values = list("Enum");
                let shown = Some(list("EnumI18n")).filter(|l| l.len() == values.len()).unwrap_or_else(|| values.clone());
                let pick = gtk::DropDown::from_strings(&shown.iter().map(String::as_str).collect::<Vec<_>>());
                pick.set_selected(values.iter().position(|v| *v == text).unwrap_or(0) as u32);
                pick.connect_selected_notify(move |p| if let Some(v) = values.get(p.selected() as usize) { save(json!(v)); });
                pick.upcast()
            }
            "Integer" => {
                let n = |key: &str, or: f64| s.notes[key].as_str().and_then(|v| v.parse().ok()).unwrap_or(or);
                let b = gtk::SpinButton::with_range(n("IntMin", 0.0), n("IntMax", 100_000.0), 1.0);
                b.set_value(text.parse().unwrap_or(0.0));
                b.connect_value_changed(move |b| save(json!((b.value() as i64).to_string())));
                b.upcast()
            }
            "String" => {
                let e = gtk::Entry::new();
                e.set_text(&text);
                e.connect_activate(move |e| save(json!(e.text().to_string())));
                e.upcast()
            }
            "Key" => key_button(&text, Rc::new(move |k| save(json!(k)))).upcast(),
            "List|Key" => key_list(&value, save).upcast(),
            "External" => {
                let command = s.notes["External"].as_str().unwrap_or_default().to_string();
                let program = command.split_whitespace().next().unwrap_or_default().to_string();
                let there = std::path::Path::new(&program).is_file() || std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(&program).is_file()));
                let b = gtk::Button::with_label(if there { t("inputset-open-tool") } else { t("inputset-not-installed") });
                b.set_sensitive(there);
                b.connect_clicked(move |_| {
                    let mut words = command.split_whitespace();
                    if let Some(p) = words.next() && let Err(e) = std::process::Command::new(p).args(words).spawn() { wb_core::log::warn("input", "tool not started", &e.to_string(), &[]); }
                });
                b.upcast()
            }
            nested => {
                // A nested form: its own settings under its name.
                if let Some((_, inner)) = form.types.iter().find(|(n, _)| n == nested) {
                    let h = heading(&s.label);
                    grid.attach(&h, 0, *row, 2, 1);
                    *row += 1;
                    let mut at = path.to_vec();
                    at.push(s.key.clone());
                    settings(grid, row, uri, form, inner, &value, &at, err);
                }
                // shortcut: other kinds (a list of strings, a colour) are not drawn; fcitx5's own tool edits them.
                continue;
            }
        };
        let l = gtk::Label::new(Some(&s.label));
        l.set_halign(gtk::Align::Start); l.set_hexpand(true); l.set_wrap(true);
        grid.attach(&l, 0, *row, 1, 1);
        grid.attach(&w, 1, *row, 1, 1);
        *row += 1;
    }
}

/// The value at `path` inside objects: what SetConfig takes for one setting.
fn nest(path: &[String], v: Value) -> Value {
    path.iter().rev().fold(v, |inner, key| { let mut m = Map::new(); m.insert(key.clone(), inner); Value::Object(m) })
}

/// A list's items in order: fcitx5 keys them "0", "1"...
fn ordered(v: &Value) -> Vec<Value> {
    let mut items: Vec<(usize, Value)> = v.as_object().map(|m| m.iter().filter_map(|(k, x)| Some((k.parse().ok()?, x.clone()))).collect()).unwrap_or_default();
    items.sort_by_key(|(i, _)| *i);
    items.into_iter().map(|(_, x)| x).collect()
}

/// A key as fcitx5 writes it: the modifiers held, then the key's name (`Control+space`, `Hangul`).
fn key_text(key: gdk::Key, state: gdk::ModifierType) -> Option<String> {
    let name = key.name()?.to_string();
    if ["Shift_", "Control_", "Alt_", "Super_", "Meta_", "ISO_Level"].iter().any(|p| name.starts_with(p)) { return None; }
    let mut out = String::new();
    for (mask, word) in [(gdk::ModifierType::CONTROL_MASK, "Control"), (gdk::ModifierType::ALT_MASK, "Alt"), (gdk::ModifierType::SHIFT_MASK, "Shift"), (gdk::ModifierType::SUPER_MASK, "Super")] {
        if state.contains(mask) { out.push_str(word); out.push('+'); }
    }
    out.push_str(&name);
    Some(out)
}

/// A button that shows a key; clicked, it takes the next key pressed (Escape keeps the old one).
fn key_button(now: &str, set: Rc<dyn Fn(String)>) -> gtk::Button {
    let b = gtk::Button::with_label(if now.is_empty() { "—" } else { now });
    let (waiting, before) = (Rc::new(Cell::new(false)), Rc::new(RefCell::new(now.to_string())));
    { let w = waiting.clone(); b.connect_clicked(move |b| { w.set(true); b.set_label(t("inputset-key-press")); }); }
    let keys = gtk::EventControllerKey::new();
    let b2 = b.clone();
    keys.connect_key_pressed(move |_, key, _, state| {
        if !waiting.get() { return glib::Propagation::Proceed; }
        if key == gdk::Key::Escape { waiting.set(false); b2.set_label(&before.borrow()); return glib::Propagation::Stop; }
        let Some(text) = key_text(key, state) else { return glib::Propagation::Stop };
        waiting.set(false);
        b2.set_label(&text);
        *before.borrow_mut() = text.clone();
        set(text);
        glib::Propagation::Stop
    });
    b.add_controller(keys);
    b
}

/// A list of keys: each with a remove button, and a button that adds the next key pressed.
fn key_list(now: &Value, save: Rc<dyn Fn(Value)>) -> gtk::FlowBox {
    let flow = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(6).build();
    let keys: Vec<String> = ordered(now).into_iter().filter_map(|v| v.as_str().map(String::from)).collect();
    fill_keys(&flow, Rc::new(RefCell::new(keys)), save);
    flow
}

fn fill_keys(flow: &gtk::FlowBox, keys: Rc<RefCell<Vec<String>>>, save: Rc<dyn Fn(Value)>) {
    while let Some(c) = flow.first_child() { flow.remove(&c); }
    let changed = {
        let (flow, keys, save) = (flow.clone(), keys.clone(), save.clone());
        Rc::new(move || {
            let list: Map<String, Value> = keys.borrow().iter().enumerate().map(|(i, k)| (i.to_string(), json!(k))).collect();
            save(Value::Object(list));
            let (flow, keys, save) = (flow.clone(), keys.clone(), save.clone());
            // Rebuilt after the click that asked for it has finished with its button.
            glib::idle_add_local_once(move || fill_keys(&flow, keys, save));
        })
    };
    for (i, k) in keys.borrow().iter().enumerate() {
        let chip = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        chip.append(&gtk::Label::new(Some(k)));
        let x = gtk::Button::with_label("×");
        x.set_tooltip_text(Some(t("inputset-remove")));
        let (keys, changed) = (keys.clone(), changed.clone());
        x.connect_clicked(move |_| { keys.borrow_mut().remove(i); changed(); });
        chip.append(&x);
        flow.insert(&chip, -1);
    }
    let add = key_button("", { let (keys, changed) = (keys.clone(), changed.clone()); Rc::new(move |k| { keys.borrow_mut().push(k); changed(); }) });
    add.set_label("+");
    add.set_tooltip_text(Some(t("inputset-add")));
    flow.insert(&add, -1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_setting_is_sent_inside_its_path_and_lists_keep_their_order() {
        assert_eq!(nest(&["Hotkey".into(), "TriggerKeys".into()], json!({"0": "Hangul"})), json!({"Hotkey": {"TriggerKeys": {"0": "Hangul"}}}));
        assert_eq!(nest(&["Keyboard".into()], json!("Sebeolsik390")), json!({"Keyboard": "Sebeolsik390"}));
        assert_eq!(ordered(&json!({"10": "c", "2": "b", "0": "a"})), vec![json!("a"), json!("b"), json!("c")]);
    }
}
