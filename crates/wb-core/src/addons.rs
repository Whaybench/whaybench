//! Add-ons are folders in `addons/` of the data directory: delete one and it is gone with its settings. Those the
//! program brings are the same kind of add-on. People do not edit add-ons, so each start writes the version this program
//! brings; a folder the person deleted stays deleted.
use std::fs;

use serde_json::Value;
use crate::{log as record, paths::Data};

/// The add-ons the program brings: where, file, content.
const BUNDLED: [(&str, &str, &str); 21] = [
    ("languages/ko", "messages.ftl", include_str!("../../../addons/languages/ko/messages.ftl")),
    ("languages/ko", "language.json", include_str!("../../../addons/languages/ko/language.json")),
    ("map-styles/ibis", "style.json", include_str!("../../../addons/map-styles/ibis/style.json")),
    ("map-styles/route", "style.json", include_str!("../../../addons/map-styles/route/style.json")),
    ("map-styles/timeline", "style.json", include_str!("../../../addons/map-styles/timeline/style.json")),
    ("map-styles/focus", "style.json", include_str!("../../../addons/map-styles/focus/style.json")),
    ("map-styles/kanban", "style.json", include_str!("../../../addons/map-styles/kanban/style.json")),
    ("themes/black", "theme.json", include_str!("../../../addons/themes/black/theme.json")),
    ("themes/flat", "theme.json", include_str!("../../../addons/themes/flat/theme.json")),
    ("themes/fluent", "theme.json", include_str!("../../../addons/themes/fluent/theme.json")),
    ("themes/catppuccin-mocha", "theme.json", include_str!("../../../addons/themes/catppuccin-mocha/theme.json")),
    ("themes/gruvbox-dark", "theme.json", include_str!("../../../addons/themes/gruvbox-dark/theme.json")),
    ("themes/solarized-light", "theme.json", include_str!("../../../addons/themes/solarized-light/theme.json")),
    ("themes/wada-carmine", "theme.json", include_str!("../../../addons/themes/wada-carmine/theme.json")),
    ("themes/wada-dream", "theme.json", include_str!("../../../addons/themes/wada-dream/theme.json")),
    ("themes/wada-jasper", "theme.json", include_str!("../../../addons/themes/wada-jasper/theme.json")),
    ("project-docs/wbs", "WBS.md", include_str!("../../../addons/project-docs/wbs/WBS.md")),
    ("agents/codex", "agent.json", include_str!("../../../addons/agents/codex/agent.json")),
    ("agents/codex", "config.toml", include_str!("../../../addons/agents/codex/config.toml")),
    ("agents/codex", "hooks.json", include_str!("../../../addons/agents/codex/hooks.json")),
    ("agents/codex", "context.md", include_str!("../../../addons/agents/codex/context.md")),
];
/// What the people who make add-ons read; written every time, it is not an add-on.
const GUIDES: [(&str, &str); 5] = [
    ("languages", include_str!("../../../addons/languages/README.md")),
    ("map-styles", include_str!("../../../addons/map-styles/README.md")),
    ("themes", include_str!("../../../addons/themes/README.md")),
    ("project-docs", include_str!("../../../addons/project-docs/README.md")),
    ("agents", include_str!("../../../addons/agents/README.md")),
];

/// An add-on's id is its folder's name: letters, digits, `-` and `_`, so it is safe in a path and in a file.
pub fn valid_id(id: &str) -> bool { !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') }

fn valid_file(f: &str) -> bool { !f.is_empty() && !f.starts_with('.') && !f.contains('/') }

/// Whether an add-on is exactly as the program brings it: then it is the program's own and needs no confirmation.
pub fn bundled_as_is(data: &Data, place: &str) -> bool {
    let mine: Vec<_> = BUNDLED.iter().filter(|(p, _, _)| *p == place).collect();
    !mine.is_empty() && mine.iter().all(|(p, f, text)| fs::read_to_string(data.root.join("addons").join(p).join(f)).ok().as_deref() == Some(*text))
}

pub fn install_bundled(data: &Data) {
    let root = data.root.join("addons");
    for (kind, text) in GUIDES {
        if let Err(e) = fs::create_dir_all(root.join(kind)) { record::warn("run", "add-on folder not made", &e.to_string(), &[]); return; }
        let _ = fs::write(root.join(kind).join("README.md"), text);
    }
    let marker = root.join("installed.json");
    let mut done: Vec<String> = fs::read_to_string(&marker).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let before = done.clone();
    for (place, file, text) in BUNDLED {
        let dir = root.join(place);
        if before.iter().any(|d| d == place) && !dir.exists() { continue; }
        if fs::read_to_string(dir.join(file)).ok().as_deref() == Some(text) { if !done.iter().any(|d| d == place) { done.push(place.to_string()); } continue; }
        if fs::create_dir_all(&dir).is_ok() && fs::write(dir.join(file), text).is_ok() && !done.iter().any(|d| d == place) { done.push(place.to_string()); }
    }
    // A file an older version brought and this one does not is removed; what the program wrote beside them stays.
    for place in done.iter().filter(|p| BUNDLED.iter().any(|(b, _, _)| b == p)) {
        let dir = root.join(place);
        if !dir.is_dir() { continue; }
        let now: Vec<&str> = BUNDLED.iter().filter(|(b, _, _)| b == place).map(|(_, f, _)| *f).collect();
        let list = dir.join(".bundled.json");
        let before: Vec<String> = fs::read_to_string(&list).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        for old in before.iter().filter(|f| !now.contains(&f.as_str()) && valid_file(f)) { let _ = fs::remove_file(dir.join(old)); }
        if before.iter().map(String::as_str).ne(now.iter().copied()) { let _ = fs::write(&list, Value::from(now).to_string()); }
    }
    // What a person making a language starts from; it is not a language.
    let _ = fs::write(root.join("languages/en.ftl"), include_str!("../../../config/lang/en.ftl"));
    let _ = fs::write(marker, Value::from(done).to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn each_goes_in_once_and_a_deleted_one_stays_deleted() {
        let dir = std::env::temp_dir().join(format!("wb-addons-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let d = Data::new(dir.clone());
        install_bundled(&d);
        assert!(dir.join("addons/project-docs/wbs/WBS.md").exists() && dir.join("addons/project-docs/README.md").exists());
        fs::remove_dir_all(dir.join("addons/project-docs/wbs")).unwrap();
        install_bundled(&d);
        assert!(!dir.join("addons/project-docs/wbs").exists());
        // An older version is replaced; what the program wrote beside it stays.
        fs::write(dir.join("addons/themes/black/theme.json"), "old").unwrap();
        fs::write(dir.join("addons/themes/black/settings.json"), "{}").unwrap();
        install_bundled(&d);
        assert_ne!(fs::read_to_string(dir.join("addons/themes/black/theme.json")).unwrap(), "old");
        assert!(dir.join("addons/themes/black/settings.json").exists());
        // A file a newer version no longer brings goes; the person's own beside it stays.
        fs::write(dir.join("addons/agents/codex/.bundled.json"), r#"["agent.json","mcp.toml"]"#).unwrap();
        fs::write(dir.join("addons/agents/codex/mcp.toml"), "old").unwrap();
        install_bundled(&d);
        assert!(!dir.join("addons/agents/codex/mcp.toml").exists());
        assert!(dir.join("addons/agents/codex/agent.json").exists() && dir.join("addons/themes/black/settings.json").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
