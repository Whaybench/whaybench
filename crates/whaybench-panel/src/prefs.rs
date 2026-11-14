//! Preferences (Settings → Preferences…): the settings kept in the data folder's settings.json that have no menu of
//! their own. A change is saved at once; the other keys of the file stay as they are.
use gtk::prelude::*;
use serde_json::{Value, json};
use wb_core::paths::Data;

fn t(key: &str) -> &'static str { wb_core::i18n::t(key) }

fn file(d: &Data) -> std::path::PathBuf { d.root.join("settings.json") }

/// One key set, or taken out when `None` (its default then applies). An unreadable file is set aside first.
fn save(d: &Data, key: &str, value: Option<Value>) {
    let mut all: serde_json::Map<String, Value> = wb_core::paths::read_kept(&file(d));
    match value { Some(v) => { all.insert(key.into(), v); } None => { all.remove(key); } }
    let _ = wb_core::paths::write_atomic(&file(d), serde_json::to_string_pretty(&Value::Object(all)).unwrap_or_default());
}

/// A number of days or MB, `default` when the file has none.
fn count(d: &Data, now: &Value, key: &'static str, default: u64, hi: f64) -> gtk::Widget {
    let b = gtk::SpinButton::with_range(0.0, hi, 1.0);
    b.set_value(now[key].as_u64().unwrap_or(default) as f64);
    let d = d.clone();
    b.connect_value_changed(move |b| save(&d, key, Some(json!(b.value() as u64))));
    b.upcast()
}

pub fn build(win: &gtk::ApplicationWindow, d: Data) {
    let now: Value = Value::Object(wb_core::paths::read_kept(&file(&d)));
    let grid = gtk::Grid::builder().column_spacing(12).row_spacing(8).margin_start(12).margin_end(12).margin_top(10).margin_bottom(12).build();
    let mut row = 0;
    let mut add = |label: &str, hint: &str, w: gtk::Widget| {
        let l = gtk::Label::new(Some(label));
        l.set_halign(gtk::Align::Start);
        l.set_hexpand(true);
        grid.attach(&l, 0, row, 1, 1);
        grid.attach(&w, 1, row, 1, 1);
        row += 1;
        if !hint.is_empty() {
            let h = gtk::Label::new(Some(hint));
            h.set_halign(gtk::Align::Start);
            h.set_wrap(true);
            h.add_css_class("dim");
            grid.attach(&h, 0, row, 2, 1);
            row += 1;
        }
    };

    add(t("prefs-keep-days"), t("prefs-zero-no-limit"), count(&d, &now, "keep_days", 0, 3650.0));
    add(t("prefs-keep-mb"), t("prefs-zero-no-limit"), count(&d, &now, "keep_mb", 0, 100_000.0));
    add(t("prefs-trash-days"), t("prefs-trash-days-hint"), count(&d, &now, "trash_days", 180, 3650.0));

    let update = gtk::CheckButton::new();
    update.set_active(now["auto_update"] != Value::Bool(false));
    { let d = d.clone(); update.connect_toggled(move |b| save(&d, "auto_update", (!b.is_active()).then_some(json!(false)))); }
    add(t("prefs-auto-update"), "", update.upcast());

    let input = gtk::DropDown::from_strings(&[t("prefs-input-built-in"), t("prefs-input-desktop")]);
    input.set_selected(u32::from(now["input_method"] == "host"));
    { let d = d.clone(); input.connect_selected_notify(move |b| save(&d, "input_method", (b.selected() == 1).then(|| json!("host")))); }
    add(t("prefs-input"), t("prefs-input-hint"), input.upcast());

    let scroll = crate::common::page_scroll(&grid);
    win.set_child(Some(&scroll));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_change_keeps_the_other_keys_and_none_takes_the_key_out() {
        let dir = std::env::temp_dir().join(format!("wb-prefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = Data::new(dir.clone());
        std::fs::write(file(&d), r#"{"map_indent": 2}"#).unwrap();
        save(&d, "keep_days", Some(json!(30)));
        save(&d, "auto_update", Some(json!(false)));
        save(&d, "auto_update", None);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(file(&d)).unwrap()).unwrap();
        assert_eq!(v, json!({"map_indent": 2, "keep_days": 30}));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
