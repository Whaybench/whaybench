//! Messages for the status line, requests to the compositor and between panels: small files, like the rest.
use std::{fs, path::PathBuf, time::{SystemTime, UNIX_EPOCH}};

use serde_json::json;

use crate::paths::{Data, write_atomic as write};


fn now_ms() -> u128 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0) }

pub fn status_file(d: &Data) -> PathBuf { d.cache().join("status.json") }

/// A system message in the status line for `seconds`.
pub fn say(d: &Data, text: &str, seconds: u64) {
    let _ = write(&status_file(d), json!({"text": text, "until": now_ms() + seconds as u128 * 1000}).to_string());
}

/// The message to show now, if any.
pub fn current(d: &Data) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(status_file(d)).ok()?).ok()?;
    (v.get("until")?.as_u64()? as u128 > now_ms()).then(|| v.get("text")?.as_str().map(String::from)).flatten()
}

/// Leaves a request for the running compositor (see `whaybench ctl`).
pub fn post(d: &Data, request: &serde_json::Value) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let name = format!("{}-{}-{}.json", now_ms(), std::process::id(), N.fetch_add(1, Ordering::Relaxed));
    write(&d.ctl().join(name), request.to_string())
}

/// Asks the compositor to bring the panel of this app to the front.
pub fn focus_app(d: &Data, app: &str) { let _ = post(d, &json!({"cmd": "focusapp", "app": app})); }

/// The tab of this app comes up in its panel; the keyboard stays where it is.
pub fn front_app(d: &Data, app: &str) { let _ = post(d, &json!({"cmd": "frontapp", "app": app})); }

/// Whether the running program is older than the one now installed: its file has been replaced.
pub fn new_version_waiting(d: &Data) -> bool {
    let Some(pid) = fs::read_to_string(d.instance()).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).and_then(|v| v.get("pid")?.as_u64()) else { return false };
    is_replaced(&fs::read_link(format!("/proc/{pid}/exe")).unwrap_or_default().to_string_lossy())
}

fn is_replaced(exe: &str) -> bool { exe.ends_with(" (deleted)") }

/// A message from one panel to another: the file holds the latest; the receiver takes it and removes it.
pub fn send(d: &Data, to: &str, v: serde_json::Value) { let _ = write(&d.cache().join("panels").join(format!("{to}.json")), v.to_string()); }

pub fn take(d: &Data, me: &str) -> Option<serde_json::Value> {
    let p = d.cache().join("panels").join(format!("{me}.json"));
    // Taken by moving it first: read-then-remove would remove a message sent in between, unread.
    let mine = p.with_extension(format!("{}.taken", std::process::id()));
    fs::rename(&p, &mine).ok()?;
    let text = fs::read_to_string(&mine);
    let _ = fs::remove_file(&mine);
    serde_json::from_str(&text.ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn data(name: &str) -> Data {
        let d = std::env::temp_dir().join(format!("wb-status-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        Data::new(d)
    }
    #[test]
    fn a_message_ends_at_its_time() {
        let d = data("say");
        assert_eq!(current(&d), None);
        say(&d, "text", 5);
        assert_eq!(current(&d).as_deref(), Some("text"));
        say(&d, "ended", 0);
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert_eq!(current(&d), None);
    }
    #[test]
    fn a_replaced_program_is_noticed() {
        assert!(is_replaced("/h/whaybench/target/release/whaybench (deleted)"));
        assert!(!is_replaced("/h/whaybench/target/release/whaybench"));
    }
    #[test]
    fn a_panel_message_is_taken_once() {
        let d = data("take");
        send(&d, "answer", json!({"x": 1}));
        assert_eq!(take(&d, "answer").unwrap()["x"], 1);
        assert!(take(&d, "answer").is_none());
    }
}
