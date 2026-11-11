//! What every panel shares: the data directory, the theme as a style sheet, the program's own commands.
use std::{cell::RefCell, path::PathBuf, process::{Command, Stdio}, rc::Rc, sync::OnceLock, time::Duration};

use gtk::{gdk, glib, prelude::*};
use webkit6::prelude::WebViewExt;
use serde_json::{Value, json};
use wb_core::{paths::{self, Data}, theme::{self, Colour, Theme}};

/// This panel's app id, under which its zoom is kept. Unset for the menu bar and the status line: their height is fixed.
static PANEL: OnceLock<String> = OnceLock::new();

pub fn set_panel(app_id: &str) { let _ = PANEL.set(app_id.to_string()); }

fn zoom_file(d: &Data) -> PathBuf { d.root.join("panel-zoom.json") }

/// This panel's zoom, 1.0 when none was set. Ctrl+wheel changes it; it is kept per panel over restarts.
pub fn zoom(d: &Data) -> f64 {
    let Some(key) = PANEL.get() else { return 1.0 };
    std::fs::read_to_string(zoom_file(d)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get(key).and_then(Value::as_f64)).unwrap_or(1.0).clamp(0.5, 3.0)
}

fn set_zoom(d: &Data, z: f64) {
    let Some(key) = PANEL.get() else { return };
    let mut v = std::fs::read_to_string(zoom_file(d)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}));
    v[key.as_str()] = json!(z);
    let tmp = zoom_file(d).with_extension("json.tmp");
    if std::fs::write(&tmp, v.to_string()).is_ok() { let _ = std::fs::rename(&tmp, zoom_file(d)); }
}

/// Ctrl+wheel anywhere in the window makes this panel larger or smaller, in steps of 10%.
pub fn zoom_by_wheel(win: &gtk::ApplicationWindow, d: &Data) {
    if PANEL.get().is_none() { return; }
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
    let d = d.clone();
    scroll.connect_scroll(move |c, _, dy| {
        if !c.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) || dy == 0.0 { return glib::Propagation::Proceed; }
        let step = if dy < 0.0 { 0.1 } else { -0.1 };
        set_zoom(&d, ((zoom(&d) + step) * 10.0).round() / 10.0);
        glib::Propagation::Stop
    });
    win.add_controller(scroll);
}

/// A web view in the panel follows the panel's zoom, until it is gone.
pub fn follow_zoom(web: &webkit6::WebView, d: &Data) {
    let (weak, d) = (web.downgrade(), d.clone());
    let apply = move || {
        let Some(web) = weak.upgrade() else { return glib::ControlFlow::Break };
        let z = zoom(&d);
        if (web.zoom_level() - z).abs() > 0.001 { web.set_zoom_level(z); }
        glib::ControlFlow::Continue
    };
    apply();
    glib::timeout_add_local(Duration::from_millis(250), apply);
}

pub fn data() -> Data {
    let root = paths::resolve(None, |k| std::env::var(k).ok()).unwrap_or_else(|| PathBuf::from("."));
    Data::new(root)
}

fn css_colour(c: Colour) -> String { format!("rgba({},{},{},{})", (c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, c[3]) }
fn ink(c: [u8; 3], a: f32) -> String { format!("rgba({},{},{},{a})", c[0], c[1], c[2]) }

pub fn style_sheet(t: &Theme) -> String {
    let (bg, text, soft, accent, line) = (css_colour(t.panel), ink(t.ink, 1.0), ink(t.ink, 0.47), css_colour(t.accent), css_colour(t.border));
    let bar = css_colour(t.bar_bottom);
    let (font, size, mono) = (t.font.replace('\'', ""), t.font_size, t.terminal_font.replace('\'', ""));
    let (small, h1, h2, h3) = (size - 1.0, size + 5.0, size + 3.0, size + 1.0);
    format!("
window, box, list, row, scrolledwindow, viewport, label, overlay {{ background: transparent; background-color: transparent; }}
list {{ color: {text}; }}
window {{ background: {bg}; color: {text}; font-family: '{font}'; font-size: {size}pt; }}
label {{ color: {text}; }}
.dim {{ color: {soft}; }}
.now {{ font-weight: bold; }}
.small {{ font-size: {small}pt; color: {soft}; }}
row {{ padding: 2px 6px; border-radius: 6px; }}
row:hover {{ background: alpha({accent}, 0.10); }}
row:selected, row.chosen {{ background: alpha({accent}, 0.22); color: {text}; }}
entry, textview, textview text {{ background: {bar}; color: {text}; caret-color: {text}; }}
entry {{ border: 1px solid {line}; border-radius: 6px; padding: 2px 6px; min-height: 22px; }}
spinbutton {{ background: {bar}; color: {text}; border: 1px solid {line}; border-radius: 6px; }}
spinbutton text {{ background: transparent; color: {text}; caret-color: {text}; }}
spinbutton button {{ background: transparent; border: none; color: {text}; min-height: 0; padding: 0 6px; }}
button {{ background: {bar}; color: {text}; border: 1px solid {line}; border-radius: 6px; padding: 2px 10px; min-height: 22px; }}
button:hover {{ background: alpha({accent}, 0.18); }}
button:disabled {{ color: {soft}; }}
button.flat {{ background: transparent; border: none; padding: 0 4px; }}
button.flat.chosen {{ background: alpha({accent}, 0.22); font-weight: bold; }}
flowboxchild {{ padding: 0; min-height: 0; }}
.code {{ font-family: '{mono}', monospace; background: {bar}; border-radius: 6px; padding: 6px 8px; }}
.cell {{ padding: 3px 8px; border: 1px solid {line}; }}
.head {{ font-weight: bold; background: {bar}; }}
.h1 {{ font-size: {h1}pt; font-weight: bold; }} .h2 {{ font-size: {h2}pt; font-weight: bold; }} .h3 {{ font-size: {h3}pt; font-weight: bold; }}
.status {{ font-size: {small}pt; padding: 0 8px; }}
listview, gridview, columnview, treeview, .view, window.dialog, dialog {{ background: {bg}; color: {text}; }}
listview > row, gridview > child {{ color: {text}; }}
headerbar {{ background: {bar}; color: {text}; border-bottom: 1px solid {line}; box-shadow: none; }}
headerbar label, headerbar .title {{ color: {text}; }}
popover contents {{ background: {bg}; color: {text}; }}
menubar {{ background: {bar}; color: {text}; min-height: 28px; font-size: {size}pt; }}
menubar > item {{ padding: 2px 10px; color: {text}; }}
menubar > item:hover, menubar > item:selected {{ background: alpha({accent}, 0.22); color: {text}; }}
check {{ background: {bar}; color: {text}; border: 1px solid {line}; }}
check:checked {{ background: alpha({accent}, 0.30); color: {text}; }}
popover scrollbar {{ opacity: 0; }}
/* A menu's scroll area is never shorter than its scroll bar: a one-item menu got a strip of nothing under the item. */
popover scrollbar, popover scrollbar range, popover scrollbar trough, popover scrollbar slider {{ min-height: 0; min-width: 0; }}
modelbutton {{ color: {text}; }} modelbutton:hover {{ background: alpha({accent}, 0.18); }}
")
}

/// Keeps the panel in the chosen theme; a change takes effect within two seconds.
pub fn follow_theme(d: &Data) {
    let provider = gtk::CssProvider::new();
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    let shown: Rc<RefCell<String>> = Rc::default();
    let d = d.clone();
    let apply = move || {
        let mut t = theme::load_active(&d);
        t.font_size *= zoom(&d);
        // GTK's own widgets, a font chooser for one, start from its light or dark look: the one that fits the theme.
        let dark = 0.2126 * t.panel[0] + 0.7152 * t.panel[1] + 0.0722 * t.panel[2] < 0.5;
        if let Some(s) = gtk::Settings::default() {
            // GTK 4.20 and later: set by name, so the program still builds and runs with an older GTK.
            const SCHEME: &str = "gtk-interface-color-scheme";
            let want = if dark { "dark" } else { "light" };
            if let Some(class) = s.find_property(SCHEME).and_then(|p| glib::EnumClass::with_type(p.value_type())) {
                let now = s.property_value(SCHEME);
                if glib::EnumValue::from_value(&now).map(|(_, v)| v.nick()) != Some(want) {
                    if let Some(v) = class.to_value_by_nick(want) { s.set_property_from_value(SCHEME, &v); }
                }
            }
            // The desktop's settings.ini may still ask for the dark look the old way; both have to agree, or GTK's own
            // parts (a menu now and then) come out dark.
            #[allow(deprecated)]
            if s.is_gtk_application_prefer_dark_theme() != dark { s.set_gtk_application_prefer_dark_theme(dark); }
            // GTK's own theme, never the desktop's: a desktop GTK theme (Breeze on KDE) styles menus its own way, and a
            // menu now and then came out in its colours with a frame around it.
            if s.gtk_theme_name().as_deref() != Some("Default") { s.set_gtk_theme_name(Some("Default")); }
        }
        let css = style_sheet(&t);
        if *shown.borrow() != css {
            provider.load_from_string(&css);
            *shown.borrow_mut() = css;
        }
        glib::ControlFlow::Continue
    };
    let first = apply.clone();
    first();
    // Often enough that the panels change with the frame the compositor draws, which looks just as often.
    glib::timeout_add_local(Duration::from_millis(250), apply);
}

fn sibling(name: &str) -> PathBuf {
    std::env::current_exe().ok().map(|e| e.with_file_name(name)).filter(|p| p.exists()).unwrap_or_else(|| name.into())
}

/// Runs `whaybench ctl ...` away from the window, because it can take seconds; what it reports goes to the status line.
pub fn ctl(d: &Data, args: Vec<String>) {
    let d = d.clone();
    std::thread::spawn(move || {
        let out = Command::new(sibling("whaybench")).arg("ctl").args(&args).stdin(Stdio::null()).output();
        if let Ok(o) = out {
            let said = String::from_utf8_lossy(if o.status.success() { &o.stdout } else { &o.stderr }).trim().trim_start_matches("whaybench: ").to_string();
            if !said.is_empty() { wb_core::status::say(&d, &said, 8); }
        }
    });
}

/// In a Markdown table a `|` inside backticks would end the cell: the documents write commands with them.
pub fn table_pipes(text: &str) -> String {
    text.split('\n').map(|line| {
        if !line.trim_start().starts_with('|') { return line.to_string(); }
        let (mut out, mut code, mut last) = (String::with_capacity(line.len()), false, '\0');
        for c in line.chars() {
            if c == '`' { code = !code; }
            if c == '|' && code && last != '\\' { out.push('\\'); }
            out.push(c);
            last = c;
        }
        out
    }).collect::<Vec<_>>().join("\n")
}

pub fn ago(ms: i64) -> String {
    let minutes = (chrono::Utc::now().timestamp_millis() - ms) / 60_000;
    if minutes < 1 { wb_core::i18n::t("just-now").into() } else if minutes < 60 { wb_core::i18n::f("min-ago", &[("minutes", (minutes).to_string())]) } else if minutes < 1440 { wb_core::i18n::f("h-ago", &[("a1", (minutes / 60).to_string())]) } else { wb_core::i18n::f("d-ago", &[("a1", (minutes / 1440).to_string())]) }
}

pub fn cut(t: &str, n: usize) -> String { t.chars().take(n).collect() }

/// Runs `f` on a timer with no way to stop it: panels live as long as their window.
pub fn every(ms: u64, mut f: impl FnMut() + 'static) {
    f();
    glib::timeout_add_local(Duration::from_millis(ms), move || { f(); glib::ControlFlow::Continue });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pipes_in_code_stay_in_their_cell() {
        assert_eq!(table_pipes("| `a [b | c]` | d |"), "| `a [b \\| c]` | d |");
        assert_eq!(table_pipes("| a | b |"), "| a | b |");
        assert_eq!(table_pipes("text `a | b`"), "text `a | b`", "only in a table");
        assert_eq!(table_pipes("| `a \\| b` |"), "| `a \\| b` |", "already escaped");
    }
}

/// WebKit's own right-click menu in the program's language: its Korean leaves Back, Forward and Stop in English.
pub fn web_menu_in_language(web: &webkit6::WebView) {
    use webkit6::{ContextMenuAction as A, ContextMenuItem, prelude::*};
    web.connect_context_menu(|_, menu, _| {
        for (i, item) in menu.items().into_iter().enumerate() {
            let key = match item.stock_action() {
                A::GoBack => "web-back", A::GoForward => "web-forward", A::Stop => "web-stop", A::Reload => "web-reload",
                A::Copy => "web-copy", A::Cut => "web-cut", A::Paste => "web-paste", A::SelectAll => "web-select-all",
                A::OpenLink => "web-open-link", A::CopyLinkToClipboard => "web-copy-link",
                A::CopyImageToClipboard => "web-copy-image", A::CopyImageUrlToClipboard => "web-copy-image-address",
                _ => continue,
            };
            menu.remove(&item);
            menu.insert(&ContextMenuItem::from_stock_action_with_label(item.stock_action(), wb_core::i18n::t(key)), i as i32);
        }
        false
    });
}
