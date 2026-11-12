//! Add-ons are folders in `addons/` of the data directory: delete one and it is gone with its settings. Those the
//! program brings are the same kind of add-on. People do not edit add-ons, so each start writes the version this program
//! brings; a folder the person deleted stays deleted.
use std::fs;

use serde_json::Value;
use crate::{log as record, paths::Data};

/// The add-ons the program brings: where, file, content.
const BUNDLED: [(&str, &str, &str); 11] = [
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
    ("project-docs/wbs", "WBS.md", include_str!("../../../addons/project-docs/wbs/WBS.md")),
];
/// What the people who make add-ons read; written every time, it is not an add-on.
const GUIDES: [(&str, &str); 4] = [
    ("languages", include_str!("../../../addons/languages/README.md")),
    ("map-styles", include_str!("../../../addons/map-styles/README.md")),
    ("themes", include_str!("../../../addons/themes/README.md")),
    ("project-docs", include_str!("../../../addons/project-docs/README.md")),
];

/// An add-on's id is its folder's name: letters, digits, `-` and `_`, so it is safe in a path and in a file.
pub fn valid_id(id: &str) -> bool { !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') }

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
        let _ = fs::remove_dir_all(&dir);
    }
}
