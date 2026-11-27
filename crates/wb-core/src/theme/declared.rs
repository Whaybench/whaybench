//! The settings a theme offers for itself (`settings` in its theme.json).
use super::*;

/// A setting a theme offers for itself, listed in its theme.json under `settings` and shown under the common ones
/// while the theme is in force, as a shader declares its properties for an editor (Unity's Properties, Godot's
/// uniform hints). `key` is the path of a number of the theme, such as `frame.shadow_size`, or, with `choices`,
/// of any value: `palette` or a path under `base` or `frame`, each choice a value to put there whole, after VS Code's
/// `enum` settings. `name` is English, `names` the same in other languages, as the add-on brings them; a language
/// add-on can give them too (`Declared::label`).
#[derive(Clone, Debug, PartialEq)]
pub struct Declared {
    pub key: String, pub name: String, pub names: std::collections::BTreeMap<String, String>, pub min: f64, pub max: f64, pub unit: String, pub value: f64,
    pub choices: Vec<Choice>,
    /// The choice whose value is in force, if one is.
    pub chosen: Option<usize>,
    /// Settings next to each other with the same `group` are shown on one row, after Godot's `@export_group`.
    pub group: String,
}

/// One value a choice setting offers.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice { pub name: String, pub value: Value }

fn names_of(s: &Value) -> std::collections::BTreeMap<String, String> {
    s.get("names").and_then(Value::as_object).map(|m| m.iter().filter_map(|(l, n)| Some((l.clone(), n.as_str()?.to_string()))).collect()).unwrap_or_default()
}

/// Whether `want` is in force in `now`: an object's keys each so, anything else equal; a key not there is null.
fn holds(now: &Value, want: &Value) -> bool {
    match want.as_object() {
        Some(w) => w.iter().all(|(k, v)| holds(now.get(k).unwrap_or(&Value::Null), v)),
        None => now == want,
    }
}

/// A name as a language add-on's key takes it: lowercase letters and digits, the rest one `-`.
fn slug(text: &str) -> String {
    text.to_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-")
}

impl Declared {
    pub(super) fn parse(theme: &Value) -> Vec<Declared> {
        let Some(list) = theme.get("settings").and_then(Value::as_array) else { return Vec::new() };
        list.iter().filter_map(|s| {
            let key = s.get("key")?.as_str()?;
            let path = if key == "palette" { vec!["palette"] } else { setting_path(key)? };
            let name = s.get("name").and_then(Value::as_str).unwrap_or(key).to_string();
            let now = path.iter().try_fold(theme, |v, k| v.get(*k));
            let group = s.get("group").and_then(Value::as_str).unwrap_or("").to_string();
            if let Some(list) = s.get("choices").and_then(Value::as_array) {
                let choices: Vec<Choice> = list.iter().filter_map(|c| Some(Choice { name: c.get("name")?.as_str()?.to_string(), value: c.get("value")?.clone() })).collect();
                if choices.is_empty() { return None; }
                let chosen = choices.iter().position(|c| holds(now.unwrap_or(&Value::Null), &c.value));
                return Some(Declared { key: key.to_string(), name, names: names_of(s), min: 0.0, max: 0.0, unit: String::new(), value: 0.0, choices, chosen, group });
            }
            let (min, max) = (s.get("min")?.as_f64()?, s.get("max")?.as_f64()?);
            if min >= max { return None; }
            // Only a number the theme itself has: a setting cannot add a key the theme does not use.
            let value = now?.as_f64()?.clamp(min, max);
            Some(Declared {
                key: key.to_string(), name, names: names_of(s), min, max,
                unit: s.get("unit").and_then(Value::as_str).unwrap_or("").chars().take(8).collect(), value, choices: Vec::new(), chosen: None, group,
            })
        }).collect()
    }

    /// The name in `lang`, else in English.
    pub fn name_in(&self, lang: &str) -> &str { self.names.get(lang).map_or(self.name.as_str(), String::as_str) }

    /// The name to show for the theme `theme`: the language add-on's `theme-<theme>-<key>` (`.` as `-`), else `name_in`.
    pub fn label(&self, theme: &str) -> String {
        crate::i18n::name_in_language(&format!("theme-{theme}-{}", slug(&self.key))).unwrap_or_else(|| self.name_in(crate::i18n::lang()).to_string())
    }

    /// A choice's name to show: the language add-on's `theme-<theme>-<key>-<choice's English name>`, else that name.
    pub fn choice_label(&self, theme: &str, i: usize) -> String {
        let c = &self.choices[i];
        crate::i18n::name_in_language(&format!("theme-{theme}-{}-{}", slug(&self.key), slug(&c.name))).unwrap_or_else(|| c.name.clone())
    }
}

/// A setting's key as a path: under `base` or `frame`, of words of lowercase letters, digits and `_`.
pub(super) fn setting_path(key: &str) -> Option<Vec<&str>> {
    let path: Vec<&str> = key.split('.').collect();
    let word = |w: &str| !w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    (path.len() >= 2 && matches!(path[0], "base" | "frame") && path.iter().all(|w| word(w))).then_some(path)
}

/// Saves a theme's own setting (a `Declared` key, such as `frame.shadow_size`) over the theme, as `set` does; a
/// choice's value is put there whole.
pub fn set_declared(data: &Data, name: &str, key: &str, value: Value) -> std::io::Result<()> {
    if name != DEFAULT && !valid_id(name) { return Err(std::io::Error::other("no such theme")); }
    let path = if key == "palette" { vec!["palette"] } else { setting_path(key).ok_or_else(|| std::io::Error::other("not a theme key"))? };
    let file = settings_file(data, name);
    let mut over = fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).filter(Value::is_object).unwrap_or_else(|| serde_json::json!({}));
    let mut at = &mut over;
    for k in &path[..path.len() - 1] {
        if !at.get(*k).is_some_and(Value::is_object) { at[*k] = serde_json::json!({}); }
        at = &mut at[*k];
    }
    at[path[path.len() - 1]] = value;
    write_json(&file, &over)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tests::scratch;

    #[test]
    fn a_theme_offers_its_own_settings_and_they_are_kept_over_it() {
        let data = scratch("declared");
        let dir = data.root.join("addons/themes/paper");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("theme.json"), r#"{"format":1,"frame":{"gap":20},"settings":[
            {"key":"frame.gap","name":"Paper","names":{"ko":"Paper (ko)"},"min":0,"max":60,"unit":"%"},
            {"key":"frame.no_such","name":"Nothing","min":0,"max":1},
            {"key":"../x","name":"Bad","min":0,"max":1}]}"#).unwrap();
        let t = Theme::load(&data, "paper").unwrap();
        assert_eq!(t.declared.len(), 1, "only a number the theme has, at a good key");
        assert_eq!((t.declared[0].value, t.declared[0].name_in("ko"), t.declared[0].name_in("fr")), (20.0, "Paper (ko)", "Paper"));
        set_declared(&data, "paper", "frame.gap", 35.0.into()).unwrap();
        let t = Theme::load(&data, "paper").unwrap();
        assert_eq!((t.declared[0].value, t.gap), (35.0, 35.0));
        assert!(set_declared(&data, "paper", "../x", 1.0.into()).is_err());
        assert!(Theme::load(&data, "flat").is_none_or(|t| t.declared.is_empty()), "other themes offer nothing");
    }
    #[test]
    fn a_choice_puts_its_value_whole() {
        let data = scratch("choice");
        let dir = data.root.join("addons/themes/combo");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("theme.json"), r##"{"format":1,"palette":{"ink":"#aa0000","leaf":"#00aa00","also":"#aa0000"},
            "base":{"accent":"ink"},"frame":{"backdrop":{"image":"dots.svg"}},"settings":[
            {"key":"palette","name":"Colours","choices":[{"name":"Red","value":{"ink":"#aa0000","leaf":"#00aa00","also":"#aa0000"}},
                                                     {"name":"Blue","value":{"ink":"#0000aa","leaf":"#00aa00","also":"#123456"}}]},
            {"key":"frame.backdrop.image","name":"Pattern","choices":[{"name":"Dots","value":"dots.svg"},{"name":"None","value":null}]}]}"##).unwrap();
        let t = Theme::load(&data, "combo").unwrap();
        assert_eq!((t.declared[0].chosen, t.declared[1].chosen), (Some(0), Some(0)));
        set_declared(&data, "combo", "palette", t.declared[0].choices[1].value.clone()).unwrap();
        let t = Theme::load(&data, "combo").unwrap();
        assert_eq!((t.declared[0].chosen, t.accent), (Some(1), parse_hex("#0000aa").unwrap()));
        set_declared(&data, "combo", "frame.backdrop.image", Value::Null).unwrap();
        let t = Theme::load(&data, "combo").unwrap();
        assert_eq!(t.declared[1].chosen, Some(1));
        assert!(set_declared(&data, "combo", "palette.ink", Value::Null).is_err(), "a key under palette is no setting");
    }
}
