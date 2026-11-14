//! What a language types with: the fcitx5 engines its add-on names in `language.json` (`input.engines`), whether
//! they are installed, and how a missing one is installed.
use std::{fs, path::{Path, PathBuf}, process::Command};

use serde_json::Value;

use crate::paths::Data;

/// The package to install so the language can be typed: none when it needs no engine or one of its engines is
/// installed, else the first (default) engine's. The others are choices, not needs.
pub fn missing(d: &Data, lang: &str) -> Option<String> {
    let about: Value = fs::read_to_string(d.root.join("addons/languages").join(lang).join("language.json"))
        .ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    missing_in(&about, &data_dirs())
}

fn missing_in(about: &Value, dirs: &[PathBuf]) -> Option<String> {
    let engines = about["input"]["engines"].as_array().cloned().unwrap_or_default();
    // An engine is installed when fcitx5 finds its description: <data dir>/fcitx5/inputmethod/<name>.conf.
    let installed = |e: &Value| e["name"].as_str().is_some_and(|n| dirs.iter().any(|d| d.join("fcitx5/inputmethod").join(format!("{n}.conf")).is_file()));
    if engines.is_empty() || engines.iter().any(installed) { return None; }
    engines[0]["package"].as_str().map(String::from)
}

/// Puts the language's engine into fcitx5's current input method group, so the language types at once: the first of
/// its engines that is installed, unless one of them is in the group already. Only adds: what the person set up
/// stays. A group with no keyboard gets the language's layout first. Ok(true) when the group changed.
pub fn enable(d: &Data, lang: &str) -> Result<bool, String> {
    let about: Value = fs::read_to_string(d.root.join("addons/languages").join(lang).join("language.json"))
        .ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let dirs = data_dirs();
    let engines: Vec<&str> = about["input"]["engines"].as_array().map(|a| a.iter().filter_map(|e| e["name"].as_str()).collect()).unwrap_or_default();
    let Some(engine) = engines.iter().find(|n| dirs.iter().any(|d| d.join("fcitx5/inputmethod").join(format!("{n}.conf")).is_file())) else { return Ok(false) };
    // fcitx5 reads its engines once: one installed after it started is unknown, and a group naming it drops it.
    // Refresh loads it (as its own setting program does after an install); then it is listed in a moment.
    if !known(engine)? {
        busctl(&["Refresh"])?;
        let started = std::time::Instant::now();
        while !known(engine)? {
            if started.elapsed() > std::time::Duration::from_secs(10) { return Err(format!("fcitx5 does not list {engine} after Refresh")); }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    }
    let (group, layout, items) = group()?;
    let keyboard = about["input"]["layout"].as_str().unwrap_or("us");
    let Some((layout, items)) = with_engine(&layout, items, &engines, engine, keyboard) else { return Ok(false) };
    set_group(&group, &layout, &items)?;
    Ok(true)
}

/// An input method fcitx5 has: the name a group uses, the name shown, and whether it has settings.
pub struct Method { pub name: String, pub label: String, pub configurable: bool }

/// Every input method fcitx5 has loaded (AvailableInputMethods: name, display name, native name, icon, label,
/// language, configurable).
pub fn available() -> Result<Vec<Method>, String> {
    let all = busctl(&["AvailableInputMethods"])?;
    Ok(all["data"][0].as_array().map(|a| a.iter().map(|m| Method {
        name: m[0].as_str().unwrap_or_default().to_string(),
        label: m[1].as_str().filter(|l| !l.is_empty()).or(m[0].as_str()).unwrap_or_default().to_string(),
        configurable: m[6].as_bool().unwrap_or(false),
    }).collect()).unwrap_or_default())
}

/// The current group: its name, default layout and input methods (each with its own layout, mostly empty).
pub fn group() -> Result<(String, String, Vec<(String, String)>), String> {
    let name = busctl(&["CurrentInputMethodGroup"])?["data"][0].as_str().ok_or("no current input method group")?.to_string();
    let info = busctl(&["InputMethodGroupInfo", "s", &name])?;
    let layout = info["data"][0].as_str().unwrap_or_default().to_string();
    let items = info["data"][1].as_array().map(|a| a.iter().map(|p| (p[0].as_str().unwrap_or_default().to_string(), p[1].as_str().unwrap_or_default().to_string())).collect()).unwrap_or_default();
    Ok((name, layout, items))
}

/// Sets the group's input methods and saves fcitx5's profile.
pub fn set_group(name: &str, layout: &str, items: &[(String, String)]) -> Result<(), String> {
    let mut args = vec!["SetInputMethodGroupInfo".to_string(), "ssa(ss)".into(), name.into(), layout.into(), items.len().to_string()];
    for (n, l) in items { args.push(n.clone()); args.push(l.clone()); }
    busctl(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
    busctl(&["Save"]).map(|_| ())
}

/// One setting of a form: its key, kind (Boolean, Enum, Integer, String, Key, List|Key, External, or the name of
/// a nested form), the text shown, its default and its notes (Enum's values, IntMin, External's command...).
pub struct Setting { pub key: String, pub kind: String, pub label: String, pub default: Value, pub notes: Value }

/// A settings form as fcitx5 describes it (GetConfig): the values, and each form type's settings, the first the
/// top one. Values are plain: a string, or an object of them (a list is keyed "0", "1"...).
pub struct Form { pub values: Value, pub types: Vec<(String, Vec<Setting>)> }

pub fn config(uri: &str) -> Result<Form, String> {
    let got = busctl(&["GetConfig", "s", uri])?;
    let values = plain(&got["data"][0]);
    let types = got["data"][1].as_array().map(|a| a.iter().map(|t| (
        t[0].as_str().unwrap_or_default().to_string(),
        t[1].as_array().map(|o| o.iter().map(|s| Setting {
            key: s[0].as_str().unwrap_or_default().to_string(),
            kind: s[1].as_str().unwrap_or_default().to_string(),
            label: s[2].as_str().unwrap_or_default().to_string(),
            default: plain(&s[3]),
            notes: Value::Object(s[4].as_object().map(|n| n.iter().map(|(k, v)| (k.clone(), plain(v))).collect()).unwrap_or_default()),
        }).collect()).unwrap_or_default(),
    )).collect()).unwrap_or_default();
    Ok(Form { values, types })
}

/// Saves the settings given, a part of the form: fcitx5 keeps the ones left out.
pub fn set_config(uri: &str, part: &Value) -> Result<(), String> {
    let mut args = vec!["SetConfig".to_string(), "sv".into(), uri.into()];
    encode(part, &mut args);
    busctl(&args.iter().map(String::as_str).collect::<Vec<_>>()).map(|_| ())
}

/// busctl's JSON of a variant ({"type": "s"|"a{sv}", "data": ...}) as plain values.
fn plain(v: &Value) -> Value {
    match v["type"].as_str() {
        Some("s") => v["data"].clone(),
        Some("a{sv}") => Value::Object(v["data"].as_object().map(|m| m.iter().map(|(k, x)| (k.clone(), plain(x))).collect()).unwrap_or_default()),
        _ => v.clone(),
    }
}

/// A plain value as busctl's arguments for a variant: a string is `s <text>`, an object `a{sv} <n> <key> <value>...`.
fn encode(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            out.push("a{sv}".into());
            out.push(m.len().to_string());
            for (k, x) in m { out.push(k.clone()); encode(x, out); }
        }
        Value::String(t) => { out.push("s".into()); out.push(t.clone()); }
        other => { out.push("s".into()); out.push(other.to_string()); }
    }
}

/// Whether fcitx5 lists the input method (its first field is the name a group uses).
fn known(name: &str) -> Result<bool, String> {
    let all = busctl(&["AvailableInputMethods"])?;
    Ok(all["data"][0].as_array().is_some_and(|a| a.iter().any(|m| m[0].as_str() == Some(name))))
}

/// The group with the engine added, or None when one of the language's engines is in it already.
fn with_engine(layout: &str, mut items: Vec<(String, String)>, engines: &[&str], engine: &str, keyboard: &str) -> Option<(String, Vec<(String, String)>)> {
    if items.iter().any(|(n, _)| engines.contains(&n.as_str())) { return None; }
    let layout = if layout.is_empty() { keyboard.to_string() } else { layout.to_string() };
    if !items.iter().any(|(n, _)| n.starts_with("keyboard-")) { items.insert(0, (format!("keyboard-{keyboard}"), String::new())); }
    items.push((engine.to_string(), String::new()));
    Some((layout, items))
}

/// A call to fcitx5's controller on the session bus, its answer as busctl's JSON.
fn busctl(args: &[&str]) -> Result<Value, String> {
    let out = Command::new("busctl").args(["--user", "--json=short", "call", "org.fcitx.Fcitx5", "/controller", "org.fcitx.Fcitx.Controller1"]).args(args)
        .output().map_err(|e| e.to_string())?;
    if !out.status.success() { return Err(String::from_utf8_lossy(&out.stderr).trim().to_string()); }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(if text.trim().is_empty() { Value::Null } else { serde_json::from_str(&text).map_err(|e| e.to_string())? })
}

/// Where fcitx5 looks for its data, as XDG_DATA_DIRS says.
fn data_dirs() -> Vec<PathBuf> {
    let dirs = std::env::var("XDG_DATA_DIRS").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect()
}

/// The command that installs the package, or None when there is no way without a terminal.
/// Under WSL the account has no sudo: wsl.exe runs apt-get as root in this distribution (no Recommends: those are
/// setting programs). On a desktop PackageKit installs, and the system asks for the password (polkit).
pub fn install_command(package: &str) -> Option<Command> {
    if Path::new("/proc/sys/fs/binfmt_misc/WSLInterop").exists() {
        let exe = which("wsl.exe").unwrap_or_else(|| PathBuf::from("/mnt/c/Windows/System32/wsl.exe"));
        let distro = std::env::var("WSL_DISTRO_NAME").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "Whaybench".into());
        let mut c = Command::new(exe);
        c.args(["-d", &distro, "-u", "root", "--", "apt-get", "install", "-y", "-q", "--no-install-recommends"]).arg(package);
        return Some(c);
    }
    let pkcon = which("pkcon")?;
    let mut c = Command::new(pkcon);
    c.args(["install", "-y"]).arg(package);
    Some(c)
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(name)).find(|f| f.is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_package_is_missing_only_when_no_engine_of_the_language_is_installed() {
        let dir = std::env::temp_dir().join(format!("wb-input-{}", std::process::id()));
        fs::create_dir_all(dir.join("fcitx5/inputmethod")).unwrap();
        let dirs = std::slice::from_ref(&dir);
        let zh = serde_json::json!({"input": {"layout": "us", "engines": [
            {"name": "pinyin", "package": "fcitx5-chinese-addons"}, {"name": "rime", "package": "fcitx5-rime"}]}});
        assert_eq!(missing_in(&zh, dirs).as_deref(), Some("fcitx5-chinese-addons"));
        fs::write(dir.join("fcitx5/inputmethod/rime.conf"), "").unwrap();
        assert_eq!(missing_in(&zh, dirs), None, "another engine of the language is there");
        assert_eq!(missing_in(&serde_json::json!({"input": {"layout": "de"}}), dirs), None);
        assert_eq!(missing_in(&Value::Null, dirs), None);
        let _ = fs::remove_dir_all(&dir);
    }
    #[test]
    fn an_engine_is_added_once_and_what_was_there_stays() {
        let s = |v: &[(&str, &str)]| v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
        let (layout, items) = with_engine("us", s(&[("keyboard-us", "")]), &["hangul"], "hangul", "us").unwrap();
        assert_eq!((layout.as_str(), items), ("us", s(&[("keyboard-us", ""), ("hangul", "")])));
        assert!(with_engine("us", s(&[("keyboard-us", ""), ("rime", "")]), &["pinyin", "rime"], "pinyin", "us").is_none(), "one of the language's is there");
        let (layout, items) = with_engine("", vec![], &["mozc"], "mozc", "jp").unwrap();
        assert_eq!((layout.as_str(), items), ("jp", s(&[("keyboard-jp", ""), ("mozc", "")])), "an empty group gets the language's layout");
        let (layout, items) = with_engine("de", s(&[("keyboard-de", ""), ("hangul", "")]), &["mozc"], "mozc", "jp").unwrap();
        assert_eq!((layout.as_str(), items), ("de", s(&[("keyboard-de", ""), ("hangul", ""), ("mozc", "")])), "the person's layout stays");
    }
    #[test]
    fn a_form_part_goes_to_busctl_as_nested_variants_and_comes_back_plain() {
        let part = serde_json::json!({"Hotkey": {"TriggerKeys": {"0": "Hangul", "1": "Shift+space"}}});
        let mut args = Vec::new();
        encode(&part, &mut args);
        assert_eq!(args, ["a{sv}", "1", "Hotkey", "a{sv}", "1", "TriggerKeys", "a{sv}", "2", "0", "s", "Hangul", "1", "s", "Shift+space"]);
        let got = serde_json::json!({"type": "a{sv}", "data": {"Keyboard": {"type": "s", "data": "Dubeolsik"},
            "Keys": {"type": "a{sv}", "data": {"0": {"type": "s", "data": "F9"}}}}});
        assert_eq!(plain(&got), serde_json::json!({"Keyboard": "Dubeolsik", "Keys": {"0": "F9"}}));
    }
}
