//! Every text for people goes through here. English is the program's own (config/lang/en.ftl); every other language is
//! an add-on, a folder in `addons/languages/` of the data directory holding a Fluent file. A key the add-on lacks shows
//! in English, so a new version of the program needs no translation to run. Debug records are not translated.
use std::{collections::HashMap, fs, sync::{Mutex, OnceLock}};

use fluent_bundle::{FluentArgs, FluentResource, concurrent::FluentBundle};
use serde_json::{Value, json};

use crate::paths::{self, Data};

pub const DEFAULT: &str = "en";
pub const DEFAULT_NAME: &str = "English";
const ENGLISH: &str = include_str!("../../../config/lang/en.ftl");
/// What only the agent reads, in English whatever the interface language.
const FOR_AGENT: &str = include_str!("../../../config/lang/ai.ftl");
/// For tests and for trying a language without choosing it.
pub const ENV_LANG: &str = "WHAYBENCH_LANG";
const BUNDLED_KO: &str = include_str!("../../../addons/languages/ko/messages.ftl");

type Bundle = FluentBundle<FluentResource>;

fn bundle(text: &str, tag: &str) -> Bundle {
    let lang = tag.parse().unwrap_or_else(|_| "en".parse().unwrap());
    let mut b = Bundle::new_concurrent(vec![lang]);
    b.set_use_isolating(false);
    let res = FluentResource::try_new(text.to_string()).unwrap_or_else(|(partial, _)| partial);
    let _ = b.add_resource(res);
    b
}

fn valid_id(id: &str) -> bool { id != DEFAULT && crate::addons::valid_id(id) }
fn root(d: &Data) -> std::path::PathBuf { d.root.join("addons/languages") }
fn choice(d: &Data) -> std::path::PathBuf { d.root.join("language.json") }

/// The languages there are now, the default first: id and the name the menu shows.
pub fn list(d: &Data) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = fs::read_dir(root(d)).into_iter().flatten().flatten().filter(|e| e.path().join("messages.ftl").is_file()).filter_map(|e| {
        let id = e.file_name().to_string_lossy().to_string();
        if !valid_id(&id) { return None; }
        let about: Value = fs::read_to_string(e.path().join("language.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let name = about.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).map_or_else(|| id.clone(), String::from);
        Some((id, name))
    }).collect();
    found.sort();
    found.insert(0, (DEFAULT.to_string(), DEFAULT_NAME.to_string()));
    found
}

/// The language the agent answers in, in English for its `language` setting ("Korean"): the interface language's
/// `english` in its language.json. None for the default (English) or an add-on without one.
pub fn agent_language(d: &Data) -> Option<String> {
    let about: Value = fs::read_to_string(root(d).join(lang()).join("language.json")).ok().and_then(|t| serde_json::from_str(&t).ok())?;
    about.get("english").and_then(Value::as_str).filter(|n| !n.is_empty()).map(String::from)
}

/// The chosen language; with no choice, the system's if there is an add-on for it; if its folder is gone, the default.
pub fn active(d: &Data, system: Option<&str>) -> String {
    let there = |id: &str| id == DEFAULT || (valid_id(id) && root(d).join(id).join("messages.ftl").is_file());
    let chosen: Option<String> = fs::read_to_string(choice(d)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v.get("active")?.as_str().map(String::from));
    if let Some(c) = chosen { return if there(&c) { c } else { DEFAULT.to_string() }; }
    let sys = system.unwrap_or("").split(['_', '.', '-']).next().unwrap_or("").to_lowercase();
    if !sys.is_empty() && there(&sys) { sys } else { DEFAULT.to_string() }
}

pub fn choose(d: &Data, id: &str) { let _ = crate::paths::write_atomic(&choice(d), json!({"active": id}).to_string()); }

struct Texts { lang: String, main: Bundle, english: Bundle, interned: Mutex<HashMap<String, &'static str>> }

fn texts() -> &'static Texts {
    static TEXTS: OnceLock<Texts> = OnceLock::new();
    TEXTS.get_or_init(|| {
        let forced = std::env::var(ENV_LANG).ok().filter(|v| !v.is_empty());
        let data = paths::resolve(None, |k| std::env::var(k).ok()).map(Data::new);
        let system = std::env::var("LC_ALL").ok().filter(|v| !v.is_empty()).or_else(|| std::env::var("LANG").ok());
        let lang = forced.clone().or_else(|| data.as_ref().map(|d| active(d, system.as_deref()))).unwrap_or_else(|| DEFAULT.to_string());
        let text = if lang == DEFAULT { ENGLISH.to_string() } else {
            data.as_ref().and_then(|d| fs::read_to_string(root(d).join(&lang).join("messages.ftl")).ok()).or_else(|| (forced.is_some() && lang == "ko").then(|| BUNDLED_KO.to_string())).unwrap_or_else(|| ENGLISH.to_string())
        };
        Texts { main: bundle(&text, &lang), english: bundle(ENGLISH, DEFAULT), lang, interned: Mutex::new(HashMap::new()) }
    })
}

pub fn lang() -> &'static str { &texts().lang }

fn format_with(b: &Bundle, key: &str, args: Option<&FluentArgs>) -> Option<String> {
    let pattern = b.get_message(key)?.value()?;
    let mut errors = Vec::new();
    Some(b.format_pattern(pattern, args, &mut errors).to_string())
}

fn lookup(key: &str, args: Option<&FluentArgs>) -> String {
    let t = texts();
    format_with(&t.main, key, args).or_else(|| format_with(&t.english, key, args)).unwrap_or_else(|| key.to_string())
}

/// A text with nothing to fill in. Kept for the life of the program: there are only so many keys.
pub fn t(key: &str) -> &'static str {
    let mut m = texts().interned.lock().unwrap();
    if let Some(v) = m.get(key) { return v; }
    let v: &'static str = Box::leak(lookup(key, None).into_boxed_str());
    m.insert(key.to_string(), v);
    v
}

fn agent_bundle() -> &'static Bundle {
    static B: OnceLock<Bundle> = OnceLock::new();
    B.get_or_init(|| bundle(FOR_AGENT, DEFAULT))
}

/// A text for the agent (a request, a worker's prompt), with nothing to fill in. Always English.
pub fn ai(key: &str) -> &'static str {
    static M: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let mut m = M.get_or_init(Default::default).lock().unwrap();
    if let Some(v) = m.get(key) { return v; }
    let v: &'static str = Box::leak(format_with(agent_bundle(), key, None).unwrap_or_else(|| key.to_string()).into_boxed_str());
    m.insert(key.to_string(), v);
    v
}

/// A text for the agent with values to fill in. Always English.
pub fn ai_f(key: &str, values: &[(&str, String)]) -> String {
    let mut args = FluentArgs::new();
    for (k, v) in values { args.set(*k, v.clone()); }
    format_with(agent_bundle(), key, Some(&args)).unwrap_or_else(|| key.to_string())
}

/// A name a language add-on gives to an add-on, if it gives one. English names live in the add-ons themselves.
pub fn name_in_language(key: &str) -> Option<String> {
    let t = texts();
    if t.lang == DEFAULT { return None; }
    format_with(&t.main, key, None)
}

/// A text with values to fill in.
pub fn f(key: &str, values: &[(&str, String)]) -> String {
    let mut args = FluentArgs::new();
    for (k, v) in values { args.set(*k, v.clone()); }
    lookup(key, Some(&args))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn keys(text: &str) -> Vec<String> { text.lines().filter_map(|l| l.split_once(" = ").map(|x| x.0.to_string())).filter(|k| !k.starts_with('#')).collect() }
    fn scratch(tag: &str) -> Data {
        let dir = std::env::temp_dir().join(format!("wb-i18n-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Data::new(dir)
    }
    #[test]
    fn both_files_read_and_korean_has_every_key() {
        for (name, text) in [("en", ENGLISH), ("ko", BUNDLED_KO), ("ai", FOR_AGENT)] {
            if let Err((_, errors)) = FluentResource::try_new(text.to_string()) { panic!("{name}: {errors:?}"); }
        }
        // The names of add-ons are only in a language add-on.
        let own = |k: &String| !k.starts_with("map-style-") && !k.starts_with("theme-") && !k.starts_with("doc-");
        let (en, ko) = (keys(ENGLISH).into_iter().filter(own).collect::<Vec<_>>(), keys(BUNDLED_KO).into_iter().filter(own).collect::<Vec<_>>());
        assert!(en.len() > 300);
        assert_eq!(en, ko);
        // What the agent reads is in no language add-on (a name people see too, such as `done`, is both).
        let leaked: Vec<String> = keys(FOR_AGENT).into_iter().filter(|k| keys(BUNDLED_KO).contains(k) && !keys(ENGLISH).contains(k)).collect();
        assert!(leaked.is_empty(), "agent texts in the Korean file: {leaked:?}");
        let vars = |t: &str| { let mut v: Vec<String> = t.split("{ $").skip(1).filter_map(|x| x.split(' ').next().map(String::from)).collect(); v.sort(); v };
        for (a, b) in ENGLISH.lines().zip(BUNDLED_KO.lines()) { assert_eq!(vars(a), vars(b), "{a}"); }
    }
    #[test]
    fn values_fill_in_and_a_missing_key_shows_english_then_the_key() {
        let en = bundle("hello = Hello { $name }\nonly = Only here\n", "en");
        let ko = bundle("hello = { $name } hallo\n", "ko");
        let mut args = FluentArgs::new();
        args.set("name", "A".to_string());
        assert_eq!(format_with(&ko, "hello", Some(&args)).unwrap(), "A hallo", "no isolation marks");
        assert!(format_with(&ko, "only", None).is_none());
        assert_eq!(format_with(&en, "only", None).unwrap(), "Only here");
    }
    #[test]
    fn the_system_language_applies_only_with_its_add_on_and_a_choice_wins() {
        let d = scratch("pick");
        assert_eq!(active(&d, Some("ko_KR.UTF-8")), "en", "no add-on, the default");
        let dir = root(&d).join("ko");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("messages.ftl"), "a = b\n").unwrap();
        fs::write(dir.join("language.json"), r#"{"name":"Korean"}"#).unwrap();
        assert_eq!(active(&d, Some("ko_KR.UTF-8")), "ko");
        assert_eq!(active(&d, Some("en_US.UTF-8")), "en");
        assert_eq!(list(&d), vec![("en".to_string(), "English".to_string()), ("ko".to_string(), "Korean".to_string())]);
        choose(&d, "en");
        assert_eq!(active(&d, Some("ko_KR.UTF-8")), "en", "the person's choice wins");
        choose(&d, "ko");
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(active(&d, None), "en", "its folder is gone");
        assert_eq!(list(&d).len(), 1);
    }
}
