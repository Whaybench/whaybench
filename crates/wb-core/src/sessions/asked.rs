//! What became of text one conversation had typed into another (`ctl session type`): left, typed in (its turn over),
//! or never typed (its tab closed). `ctl session wait` takes only an answer given after the text was typed. One file per
//! asked conversation and asking tab, in the asked one's own root, so two that ask the same one do not mix.
use std::path::PathBuf;

use serde_json::{Value, json};

use super::{at, plain_id, read_json};
use crate::{clock::now_ms, paths::{Data, write_atomic}};

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Became { Left, Typed(i64), Refused }

/// The name of one asked conversation and asking tab (a tab with no plain name, or none, is "-").
fn name(id: &str, tab: &str) -> Option<String> { plain_id(id).then(|| format!("{id}.{}", if plain_id(tab) { tab } else { "-" })) }
fn file(d: &Data, id: &str, tab: &str) -> Option<PathBuf> { Some(at(d, id).cache().join("asked").join(name(id, tab)?)) }

/// When the request was left in ask/, beside it: until the host has taken it, what an earlier one became says nothing.
pub fn left_file(ask_dir: &std::path::Path, id: &str, tab: &str) -> Option<PathBuf> { Some(ask_dir.join(format!("{}.asked", name(id, tab)?))) }

/// When it was left and what became of it.
pub fn read(d: &Data, id: &str, tab: &str) -> Option<(i64, Became)> {
    let v = read_json(&file(d, id, tab)?)?;
    let became = if v["refused"] == true { Became::Refused } else { v["typed"].as_i64().map_or(Became::Left, Became::Typed) };
    Some((v["left"].as_i64()?, became))
}

/// Notes it, unless a later request to the same conversation from the same tab is noted already.
pub fn note(d: &Data, id: &str, tab: &str, left: i64, became: Became) {
    // Known here only: nothing of a closed project's conversation is noted in the host's root.
    if !plain_id(id) || crate::roots::known_session(d, id).is_none() { return; }
    let Some(f) = file(d, id, tab) else { return };
    if read(d, id, tab).is_some_and(|(l, _)| l > left) { return; }
    let v = match became { Became::Left => json!({"left": left}), Became::Typed(t) => json!({"left": left, "typed": t}), Became::Refused => json!({"left": left, "refused": true}) };
    let _ = write_atomic(&f, v.to_string());
}

/// The relay's: what became of a request in its tab's inbox (its `asked` part), if it came from a conversation.
pub fn note_request(d: &Data, request: &Value, typed: bool) {
    let a = &request["asked"];
    if let (Some(id), Some(left)) = (a["id"].as_str(), a["left"].as_i64()) {
        note(d, id, a["tab"].as_str().unwrap_or(""), left, if typed { Became::Typed(now_ms()) } else { Became::Refused });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn each_asker_has_its_own_and_a_later_request_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let d = Data::new(dir.path().to_path_buf());
        write_atomic(&super::super::sessions_dir(&d).join("s.json"), r#"{"id":"s"}"#).unwrap();
        assert_eq!(read(&d, "s", "t1"), None);
        note(&d, "s", "t1", 10, Became::Left);
        note_request(&d, &json!({"asked": {"id": "s", "tab": "t2", "left": 5}}), false);
        assert_eq!((read(&d, "s", "t1"), read(&d, "s", "t2")), (Some((10, Became::Left)), Some((5, Became::Refused))));
        note(&d, "s", "t1", 20, Became::Left);
        note(&d, "s", "t1", 10, Became::Typed(11));
        assert_eq!(read(&d, "s", "t1"), Some((20, Became::Left)));
        note(&d, "../s", "t1", 1, Became::Left);
        assert!(!dir.path().join("cache/s.t1").exists());
    }
}
