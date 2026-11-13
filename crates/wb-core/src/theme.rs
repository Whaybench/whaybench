//! What a theme can change in the frame the compositor draws: not only colours but also the space around panels,
//! borders, corners, shadows and gradients. The format for people who make themes is in
//! addons/themes/README.md, and a key added here is added there.
//!
//! The program itself holds only `default`. Every other theme is an add-on, a folder in `addons/themes/` of the data
//! directory: delete the folder and the theme and its settings are gone (see `addons`).
use std::{fs, path::{Path, PathBuf}};

use serde_json::Value;
use crate::{log as record, paths::Data};

pub const DEFAULT: &str = "default";
const DEFAULT_TEXT: &str = include_str!("../../../config/themes/default.json");

pub type Colour = [f32; 4];

/// WCAG 2 contrast ratio of two colours, 1 to 21.
fn contrast(a: Colour, b: Colour) -> f32 {
    let lum = |c: Colour| {
        let f = |v: f32| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2])
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// The title of a panel behind: the text faded toward the background, but no further than it still reads on the title
/// bar, 4.5:1 (WCAG 2 AA for text). A theme whose own text is below that gets its text.
fn soft_ink(text: Colour, background: Colour, bar: Colour) -> Colour {
    (10..=20).map(|i| i as f32 / 20.0)
        .map(|t| [text[0] * t + background[0] * (1.0 - t), text[1] * t + background[1] * (1.0 - t), text[2] * t + background[2] * (1.0 - t), 1.0])
        .find(|c| contrast(*c, bar) >= 4.5)
        .unwrap_or(text)
}

/// When the lines that show which panels hold together are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointShow {
    Always,
    /// While a panel or a border is being dragged, or the pointer is on a border.
    Active,
    Never,
}

impl JointShow {
    fn parse(text: Option<&str>) -> Self {
        match text { Some("active") => JointShow::Active, Some("never") => JointShow::Never, _ => JointShow::Always }
    }
    pub fn visible(self, busy: bool) -> bool {
        match self { JointShow::Always => true, JointShow::Active => busy, JointShow::Never => false }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub name: String,
    pub surface_top: Colour,
    pub surface_bottom: Colour,
    pub panel: Colour,
    pub bar_top: Colour,
    pub bar_bottom: Colour,
    pub front_top: Colour,
    pub front_bottom: Colour,
    pub border: Colour,
    pub front_border: Colour,
    pub shadow: Colour,
    pub tab_front: Colour,
    pub ink: [u8; 3],
    pub ink_soft: [u8; 3],
    pub accent: Colour,
    pub shared: Colour,
    pub wall: Colour,
    pub edge: Colour,
    /// Empty space between the window's edge and a panel on a wall.
    pub outer: f64,
    /// Empty space between two panels that sit together.
    pub gap: f64,
    /// Panel frame around the app, left, right and below.
    pub pad: f64,
    pub radius: f64,
    pub border_width: f64,
    pub shadow_size: f64,
    pub joint_width: f64,
    pub joint_show: JointShow,
    pub background: Colour,
    pub surface: Colour,
    /// Font of the panels and its size in points.
    pub font: String,
    pub font_size: f64,
    /// Size of the title bar text, in pixels.
    pub title_size: f64,
    /// Font of the conversation terminals and its size in points.
    pub terminal_font: String,
    pub terminal_size: f64,
}

/// What the theme settings show for a key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind { Colour, Number(f64, f64), Font, Joint }

/// Every key the theme settings show, in groups: (group, part, key, kind). The format is in addons/themes/README.md.
pub const FIELDS: &[(&str, &str, &str, Kind)] = &[
    ("base", "base", "background", Kind::Colour), ("base", "base", "surface", Kind::Colour),
    ("base", "base", "text", Kind::Colour), ("base", "base", "accent", Kind::Colour),
    ("font", "base", "font", Kind::Font), ("font", "base", "font_size", Kind::Number(6.0, 24.0)),
    ("font", "base", "title_size", Kind::Number(8.0, 32.0)),
    ("font", "base", "terminal_font", Kind::Font), ("font", "base", "terminal_size", Kind::Number(6.0, 32.0)),
    ("panel", "frame", "outer", Kind::Number(0.0, 64.0)), ("panel", "frame", "gap", Kind::Number(0.0, 64.0)),
    ("panel", "frame", "pad", Kind::Number(0.0, 64.0)), ("panel", "frame", "radius", Kind::Number(0.0, 64.0)),
    ("panel", "frame", "border_width", Kind::Number(0.0, 64.0)), ("panel", "frame", "shadow_size", Kind::Number(0.0, 64.0)),
    ("panel", "frame", "shadow", Kind::Colour), ("panel", "frame", "panel", Kind::Colour),
    ("bar", "frame", "bar_top", Kind::Colour), ("bar", "frame", "bar_bottom", Kind::Colour),
    ("bar", "frame", "front_top", Kind::Colour), ("bar", "frame", "front_bottom", Kind::Colour),
    ("bar", "frame", "tab_front", Kind::Colour),
    ("border", "frame", "border", Kind::Colour), ("border", "frame", "front_border", Kind::Colour),
    ("surface", "frame", "surface_top", Kind::Colour), ("surface", "frame", "surface_bottom", Kind::Colour),
    ("joint", "frame", "joint_show", Kind::Joint), ("joint", "frame", "joint_width", Kind::Number(0.0, 64.0)),
    ("joint", "frame", "shared", Kind::Colour), ("joint", "frame", "wall", Kind::Colour), ("joint", "frame", "edge", Kind::Colour),
];

fn parse_hex(text: &str) -> Option<Colour> {
    let h = text.strip_prefix('#')?;
    if !h.is_ascii() || (h.len() != 6 && h.len() != 8) { return None; }
    let at = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|v| v as f32 / 255.0);
    Some([at(0)?, at(2)?, at(4)?, if h.len() == 8 { at(6)? } else { 1.0 }])
}

/// Colours in a theme are as written; drawing wants them premultiplied.
pub fn premultiplied(c: Colour) -> Colour { [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]] }

fn bytes(c: Colour) -> [u8; 3] { [(c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8] }

impl Theme {
    pub fn parse(name: &str, text: &str) -> Option<Theme> {
        let v: Value = serde_json::from_str(text).ok()?;
        let palette = v.get("palette").cloned().unwrap_or(Value::Null);
        let colour = |part: &str, key: &str, default: &str| -> Colour {
            let raw = v.get(part).and_then(|p| p.get(key)).and_then(Value::as_str).unwrap_or(default);
            let named = palette.get(raw).and_then(Value::as_str).unwrap_or(raw);
            parse_hex(named).or_else(|| parse_hex(default)).unwrap_or([0.0, 0.0, 0.0, 1.0])
        };
        let text_of = |key: &str, default: &str| v.get("base").and_then(|p| p.get(key)).and_then(Value::as_str).filter(|t| !t.trim().is_empty()).unwrap_or(default).to_string();
        let size = |key: &str, range: std::ops::RangeInclusive<f64>, default: f64| v.get("base").and_then(|p| p.get(key)).and_then(Value::as_f64).filter(|n| range.contains(n)).unwrap_or(default);
        let number = |key: &str, default: f64| v.get("frame").and_then(|p| p.get(key)).and_then(Value::as_f64).filter(|n| (0.0..=64.0).contains(n)).unwrap_or(default);

        let background = colour("base", "background", "#eff1f5");
        let surface = colour("base", "surface", "#e6e9ef");
        let text_colour = colour("base", "text", "#4c4f69");
        let accent = colour("base", "accent", "#1e66f5");
        let hex = |c: Colour| format!("#{:02x}{:02x}{:02x}", bytes(c)[0], bytes(c)[1], bytes(c)[2]);
        let (bg, sf) = (hex(background), hex(surface));
        let surface_top = colour("frame", "surface_top", &bg);
        let bar_top = colour("frame", "bar_top", &sf);
        let front_top = colour("frame", "front_top", &hex(bar_top));
        let soft_mix = soft_ink(text_colour, background, bar_top);
        Some(Theme {
            name: name.to_string(),
            surface_top,
            surface_bottom: colour("frame", "surface_bottom", &hex(surface_top)),
            panel: colour("frame", "panel", &bg),
            bar_top,
            bar_bottom: colour("frame", "bar_bottom", &hex(bar_top)),
            front_top,
            front_bottom: colour("frame", "front_bottom", &hex(front_top)),
            border: colour("frame", "border", &sf),
            front_border: colour("frame", "front_border", &hex(accent)),
            shadow: colour("frame", "shadow", "#00000040"),
            tab_front: colour("frame", "tab_front", &bg),
            ink: bytes(text_colour),
            ink_soft: bytes(soft_mix),
            accent,
            shared: colour("frame", "shared", "#fe640b"),
            wall: colour("frame", "wall", "#04a5e5"),
            edge: colour("frame", "edge", "#d20f39"),
            outer: number("outer", 0.0),
            gap: number("gap", 0.0),
            pad: number("pad", 0.0),
            radius: number("radius", 0.0),
            border_width: number("border_width", 0.0),
            shadow_size: number("shadow_size", 0.0),
            joint_width: number("joint_width", 2.0),
            joint_show: JointShow::parse(v.get("frame").and_then(|p| p.get("joint_show")).and_then(Value::as_str)),
            background,
            surface,
            font: text_of("font", "sans-serif"),
            font_size: size("font_size", 6.0..=24.0, 10.0),
            title_size: size("title_size", 8.0..=32.0, 13.0),
            terminal_font: text_of("terminal_font", "monospace"),
            terminal_size: size("terminal_size", 6.0..=32.0, 12.0),
        })
    }

    /// The value in force for a key of FIELDS: a colour as `#rrggbb` or `#rrggbbaa`, a number, a font or a joint choice.
    pub fn value(&self, key: &str) -> Value {
        let c = |c: Colour| {
            let b = |x: f32| (x * 255.0).round() as u8;
            Value::from(if c[3] >= 1.0 { format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2])) } else { format!("#{:02x}{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]), b(c[3])) })
        };
        let ink = |i: [u8; 3]| Value::from(format!("#{:02x}{:02x}{:02x}", i[0], i[1], i[2]));
        match key {
            "background" => c(self.background), "surface" => c(self.surface), "text" => ink(self.ink), "accent" => c(self.accent),
            "font" => self.font.clone().into(), "font_size" => self.font_size.into(), "title_size" => self.title_size.into(),
            "terminal_font" => self.terminal_font.clone().into(), "terminal_size" => self.terminal_size.into(),
            "outer" => self.outer.into(), "gap" => self.gap.into(), "pad" => self.pad.into(), "radius" => self.radius.into(),
            "border_width" => self.border_width.into(), "shadow_size" => self.shadow_size.into(), "joint_width" => self.joint_width.into(),
            "joint_show" => match self.joint_show { JointShow::Always => "always", JointShow::Active => "active", JointShow::Never => "never" }.into(),
            "shadow" => c(self.shadow), "panel" => c(self.panel), "bar_top" => c(self.bar_top), "bar_bottom" => c(self.bar_bottom),
            "front_top" => c(self.front_top), "front_bottom" => c(self.front_bottom), "tab_front" => c(self.tab_front),
            "border" => c(self.border), "front_border" => c(self.front_border), "surface_top" => c(self.surface_top),
            "surface_bottom" => c(self.surface_bottom), "shared" => c(self.shared), "wall" => c(self.wall), "edge" => c(self.edge),
            _ => Value::Null,
        }
    }

    /// `default`, or the add-on of that id; what the person changed lies over it.
    pub fn load(data: &Data, name: &str) -> Option<Theme> {
        let text = if name == DEFAULT { DEFAULT_TEXT.to_string() } else {
            if !valid_id(name) { return None; }
            fs::read_to_string(folder(data, name).join("theme.json")).ok()?
        };
        let settings = settings_file(data, name);
        let joined = match fs::read_to_string(settings).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
            Some(over) => { let Ok(mut base) = serde_json::from_str::<Value>(&text) else { return None }; merge(&mut base, &over); base.to_string() }
            None => text,
        };
        let t = Theme::parse(name, &joined);
        if t.is_none() { record::warn("screen", "theme not read", "the file format differs", &[("theme", name.to_string())]); }
        t
    }
}

fn settings_file(data: &Data, name: &str) -> PathBuf {
    if name == DEFAULT { data.root.join("addons/themes-default-settings.json") } else { folder(data, name).join("settings.json") }
}

fn write_json(path: &Path, v: &Value) -> std::io::Result<()> {
    crate::paths::write_atomic(path, serde_json::to_string_pretty(v)?)
}

/// A value the person changed in the theme settings: it goes in the theme's settings.json, over its theme.json.
pub fn set(data: &Data, name: &str, part: &str, key: &str, value: Value) -> std::io::Result<()> {
    if name != DEFAULT && !valid_id(name) { return Err(std::io::Error::other("no such theme")); }
    let path = settings_file(data, name);
    let mut over = fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).filter(Value::is_object).unwrap_or_else(|| serde_json::json!({}));
    if !over.get(part).is_some_and(Value::is_object) { over[part] = serde_json::json!({}); }
    over[part][key] = value;
    write_json(&path, &over)
}

/// Back to the theme as it came: what the person changed is gone.
pub fn reset(data: &Data, name: &str) -> std::io::Result<()> {
    match fs::remove_file(settings_file(data, name)) { Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e), _ => Ok(()) }
}

/// A new theme from the one in force and what was changed in it: a new add-on folder, its id returned.
pub fn save_as(data: &Data, from: &str, name: &str) -> std::io::Result<String> {
    let name = name.trim();
    if name.is_empty() { return Err(std::io::Error::other("a theme needs a name")); }
    let base = if from == DEFAULT { DEFAULT_TEXT.to_string() } else { fs::read_to_string(folder(data, from).join("theme.json"))? };
    let mut theme: Value = serde_json::from_str(&base).map_err(std::io::Error::other)?;
    if let Some(over) = fs::read_to_string(settings_file(data, from)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) { merge(&mut theme, &over); }
    theme["name"] = name.into();
    let slug: String = name.to_ascii_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let slug = slug.trim_matches('-').to_string();
    let stem = if valid_id(&slug) { slug } else { "theme".to_string() };
    let id = (0..).map(|n| if n == 0 { stem.clone() } else { format!("{stem}-{n}") }).find(|id| valid_id(id) && !folder(data, id).exists()).unwrap();
    write_json(&folder(data, &id).join("theme.json"), &theme)?;
    Ok(id)
}

/// The conversation terminal's settings (Alacritty, read again by it when the file changes): the theme's terminal
/// font, size and colours, and Shift+Enter as Claude Code's new line.
pub fn write_terminal_config(data: &Data) -> std::io::Result<PathBuf> {
    let t = load_active(data);
    let hex = |c: [u8; 3]| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
    let text = format!("{}\n[font]\nnormal = {{ family = {:?} }}\nsize = {:.1}\n\n[colors.primary]\nbackground = {:?}\nforeground = {:?}\n",
        include_str!("../../../config/alacritty.toml"), t.terminal_font, t.terminal_size, hex(bytes(t.panel)), hex(t.ink));
    let path = data.cache().join("alacritty.toml");
    if fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
        fs::create_dir_all(data.cache())?;
        fs::write(&path, text)?;
    }
    Ok(path)
}

fn valid_id(id: &str) -> bool { id != DEFAULT && crate::addons::valid_id(id) }

fn folder(data: &Data, id: &str) -> PathBuf { data.root.join("addons/themes").join(id) }

fn merge(into: &mut Value, over: &Value) {
    match (into.as_object_mut(), over.as_object()) {
        (Some(a), Some(b)) => for (k, v) in b { match a.get_mut(k) { Some(x) => merge(x, v), None => { a.insert(k.clone(), v.clone()); } } },
        _ => *into = over.clone(),
    }
}

/// The themes there are now, default first: their id and the name the menu shows.
pub fn list(data: &Data) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = fs::read_dir(data.root.join("addons/themes")).into_iter().flatten().flatten().filter(|e| e.path().is_dir()).filter_map(|e| {
        let id = e.file_name().to_string_lossy().to_string();
        if !valid_id(&id) { return None; }
        let v: Value = serde_json::from_str(&fs::read_to_string(e.path().join("theme.json")).ok()?).ok()?;
        let name = crate::i18n::name_in_language(&format!("theme-{id}")).or_else(|| v.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).map(String::from)).unwrap_or_else(|| id.clone());
        Some((id, name))
    }).collect();
    found.sort();
    found.insert(0, (DEFAULT.to_string(), crate::i18n::t("default").to_string()));
    found
}

fn active_file(data: &Data) -> std::path::PathBuf { data.root.join("theme.json") }

/// The theme chosen last time, or the default.
pub fn load_active(data: &Data) -> Theme {
    let chosen = fs::read_to_string(active_file(data)).ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get("active").and_then(Value::as_str).map(String::from))
        .unwrap_or_else(|| DEFAULT.to_string());
    // Its folder may be gone: then the default, which is always there.
    Theme::load(data, &chosen).or_else(|| Theme::load(data, DEFAULT)).expect("the default theme is part of the program")
}

pub fn save_active(data: &Data, name: &str) -> std::io::Result<()> {
    let text = serde_json::json!({ "active": name }).to_string();
    crate::paths::write_atomic(Path::new(&active_file(data)), text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_title_behind_reads_on_its_bar() {
        let hex = |h: &str| { let v = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap() as f32 / 255.0; [v(1), v(3), v(5), 1.0] };
        // The default theme: half way to the background was 2.4:1.
        let soft = soft_ink(hex("#4c4f69"), hex("#eff1f5"), hex("#f4f6fa"));
        assert!(contrast(soft, hex("#f4f6fa")) >= 4.5);
        assert!(contrast(soft, hex("#f4f6fa")) < contrast(hex("#4c4f69"), hex("#f4f6fa")), "still fainter than the front title");
        assert_eq!(soft_ink(hex("#777777"), hex("#ffffff"), hex("#888888")), hex("#777777"), "text below 4.5:1 is kept");
    }
    fn scratch(tag: &str) -> Data {
        let dir = std::env::temp_dir().join(format!("wb-theme-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Data::new(dir)
    }
    #[test]
    fn the_default_is_in_the_program_and_black_is_an_add_on() {
        let d = scratch("seed");
        assert!(Theme::load(&d, DEFAULT).is_some_and(|t| t.gap > 0.0 && t.radius > 0.0 && t.border_width > 0.0));
        assert!(Theme::load(&d, "black").is_none(), "nothing but the default before it is put there");
        crate::addons::install_bundled(&d);
        let black = Theme::load(&d, "black").unwrap();
        assert_ne!(black.surface_top, Theme::load(&d, DEFAULT).unwrap().surface_top);
        assert_eq!(black.border_width, 2.0);
        let ids: Vec<String> = list(&d).into_iter().map(|x| x.0).collect();
        assert_eq!(ids, vec![DEFAULT, "black", "catppuccin-mocha", "flat", "fluent", "gruvbox-dark", "solarized-light", "wada-carmine", "wada-dream", "wada-jasper"]);
        assert_eq!(list(&d)[1].1, "Black", "the name in the theme file");
    }
    #[test]
    fn a_change_lies_over_the_theme_and_a_new_theme_keeps_it() {
        let d = scratch("set");
        crate::addons::install_bundled(&d);
        set(&d, "flat", "frame", "radius", 6.0.into()).unwrap();
        set(&d, "flat", "base", "font", "Noto Sans".into()).unwrap();
        let t = Theme::load(&d, "flat").unwrap();
        assert_eq!((t.radius, t.font.as_str()), (6.0, "Noto Sans"));
        assert_eq!(t.value("radius"), Value::from(6.0));
        let id = save_as(&d, "flat", "ÄÖ").unwrap();
        assert_eq!(id, "theme");
        assert_eq!(save_as(&d, "flat", "Flat Two").unwrap(), "flat-two");
        let mine = Theme::load(&d, &id).unwrap();
        assert_eq!((mine.radius, mine.font.as_str()), (6.0, "Noto Sans"));
        reset(&d, "flat").unwrap();
        assert_eq!(Theme::load(&d, "flat").unwrap().radius, 0.0);
        assert_eq!(Theme::load(&d, &id).unwrap().radius, 6.0, "the new theme keeps what it was made with");
        for (_, _, key, _) in FIELDS { assert!(!mine.value(key).is_null(), "{key}"); }
    }
    #[test]
    fn deleting_the_folder_removes_the_theme_and_it_stays_deleted() {
        let d = scratch("gone");
        crate::addons::install_bundled(&d);
        save_active(&d, "black").unwrap();
        fs::write(folder(&d, "black").join("settings.json"), r#"{"frame":{"gap":40}}"#).unwrap();
        assert_eq!(load_active(&d).name, "black");
        assert_eq!(load_active(&d).gap, 40.0, "its settings lie over it");
        fs::remove_dir_all(folder(&d, "black")).unwrap();
        assert_eq!(load_active(&d).name, DEFAULT, "back to the default");
        assert!(!list(&d).iter().any(|x| x.0 == "black"));
        crate::addons::install_bundled(&d);
        assert!(!folder(&d, "black").exists(), "a deleted one does not come back at the next start");
    }
    #[test]
    fn someone_elses_theme_is_found_and_bad_ones_are_not() {
        let d = scratch("other");
        let put = |id: &str, text: &str| { fs::create_dir_all(folder(&d, id)).unwrap(); fs::write(folder(&d, id).join("theme.json"), text).unwrap(); };
        put("sea", r##"{"format":1,"base":{"background":"#001122"},"later":{"unknown":1}}"##);
        put("broken", "{ not json");
        fs::create_dir_all(folder(&d, "empty")).unwrap();
        let ids: Vec<String> = list(&d).into_iter().map(|x| x.0).collect();
        assert_eq!(ids, vec![DEFAULT, "sea"], "a name that is its folder's when it has none; unreadable ones are left out");
        assert_eq!(Theme::load(&d, "sea").unwrap().panel, parse_hex("#001122").unwrap());
        assert!(Theme::load(&d, "broken").is_none());
        assert!(Theme::load(&d, "../sea").is_none(), "an id stays inside the folder");
    }
    #[test]
    fn the_guide_names_every_key_the_program_reads() {
        for key in ["background", "surface", "text", "accent", "outer", "gap", "pad", "radius", "border_width", "shadow_size", "joint_width", "joint_show", "surface_top", "surface_bottom", "panel", "bar_top", "bar_bottom", "front_top", "front_bottom", "border", "front_border", "tab_front", "shadow", "shared", "wall", "edge"] {
            assert!(include_str!("../../../addons/themes/README.md").contains(&format!("`{key}`")), "{key} is not in addons/themes/README.md");
            assert!(DEFAULT_TEXT.contains(&format!("\"{key}\"")), "{key} is not in the default theme");
        }
    }
    #[test]
    fn palette_names_and_bad_values() {
        let t = Theme::parse("x", r##"{"palette":{"sky":"#102030"},"base":{"background":"sky","text":"no-such"},"frame":{"gap":12,"radius":-3,"shadow":"#00000080"}}"##).unwrap();
        assert_eq!(t.panel, parse_hex("#102030").unwrap());
        assert_eq!(t.ink, bytes(parse_hex("#4c4f69").unwrap()), "a bad colour takes the default");
        assert_eq!(t.gap, 12.0);
        assert_eq!(t.radius, 0.0, "a bad number takes the default");
        assert!((t.shadow[3] - 0.502).abs() < 0.01);
        assert!(Theme::parse("x", "{ not json").is_none());
    }
    #[test]
    fn joint_lines_default_to_2px_always_and_can_be_set() {
        let t = Theme::parse("x", "{}").unwrap();
        assert_eq!((t.joint_width, t.joint_show), (2.0, JointShow::Always));
        let b = Theme::parse(DEFAULT, DEFAULT_TEXT).unwrap();
        assert_eq!((b.joint_width, b.joint_show), (2.0, JointShow::Always));
        let a = Theme::parse("x", r#"{"frame":{"joint_width":1,"joint_show":"active"}}"#).unwrap();
        assert_eq!((a.joint_width, a.joint_show), (1.0, JointShow::Active));
        assert_eq!(Theme::parse("x", r#"{"frame":{"joint_show":"never"}}"#).unwrap().joint_show, JointShow::Never);
        assert_eq!(Theme::parse("x", r#"{"frame":{"joint_show":"what"}}"#).unwrap().joint_show, JointShow::Always);
    }
    #[test]
    fn when_joint_lines_show() {
        assert!(JointShow::Always.visible(false));
        assert!(JointShow::Active.visible(true) && !JointShow::Active.visible(false));
        assert!(!JointShow::Never.visible(true));
    }
    #[test]
    fn premultiplying() {
        assert_eq!(premultiplied([1.0, 0.5, 0.0, 0.5]), [0.5, 0.25, 0.0, 0.5]);
    }
}
