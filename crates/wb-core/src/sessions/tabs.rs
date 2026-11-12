//! The tabs sessions run in, and sending a request to a session through its tab.
use super::*;

/// A request's kind for the record: its leading [tag], a slash command, or the person's own text.
pub fn request_kind(text: &str) -> String {
    let t = text.trim_start();
    if t.starts_with('[') {
        if let Some(end) = t.char_indices().take(40).find(|(_, c)| *c == ']').map(|(i, _)| i) { return t[1..end].to_string(); }
    }
    if t.starts_with('/') { return format!("command {}", t.split_whitespace().next().unwrap_or(t)); }
    "person's input".into()
}

/// Queues text for a session's tab to type in, as if typed. Only for a session running now: a request left for a
/// stopped one would run hours later, after the work was done elsewhere. True when queued.
pub fn queue_prompt(d: &Data, id: &str, text: &str) -> bool {
    let now = current_id(d, id);
    let kind = request_kind(text);
    if !alive(d, &now) {
        record::decide("request", "request not sent", "the session is not running (no heartbeat)", &[("session", id.to_string()), ("kind", kind)]);
        return false;
    }
    // The tab's relay types it in once the session has finished its turn, whatever the agent. A session no tab
    // shows (one running in the background) cannot be typed into.
    let tab = open_tabs(d).into_iter().find(|t| t.session.as_deref() == Some(&now)).map(|t| t.tab);
    let Some(tab) = tab else {
        record::decide("request", "request not sent", "no tab shows the session", &[("session", now), ("kind", kind)]);
        return false;
    };
    let seq = SystemTime::now().duration_since(UNIX_EPOCH).map(|t| t.as_nanos()).unwrap_or(0);
    if write_atomic(&tab_inbox(d, &tab).join(format!("{seq}.json")), &serde_json::json!({"text": text, "enter": true}).to_string()).is_err() { return false; }
    record::decide("request", "request queued", "the session is running", &[("request", seq.to_string()), ("session", now), ("tab", tab), ("kind", kind), ("chars", text.chars().count().to_string())]);
    true
}

/// A tab's own record of what it shows, for the program and the other tabs.
pub fn tab_file(d: &Data, tab: &str) -> PathBuf { tabs_dir(d).join(format!("{tab}.json")) }
pub fn tab_inbox(d: &Data, tab: &str) -> PathBuf { tabs_dir(d).join(format!("{tab}.in")) }

pub fn write_tab(d: &Data, tab: &str, session: Option<&str>, attached: bool) {
    let v = serde_json::json!({"tab": tab, "pid": std::process::id(), "session": session, "attached": attached});
    let _ = write_atomic(&tab_file(d, tab), &v.to_string());
}

pub struct OpenTab { pub tab: String, pub session: Option<String>, pub attached: bool }

/// Tabs whose relay still runs.
pub fn open_tabs(d: &Data) -> Vec<OpenTab> {
    let mut out = Vec::new();
    for f in fs::read_dir(tabs_dir(d)).into_iter().flatten().flatten() {
        if f.path().extension().is_none_or(|e| e != "json") { continue; }
        let Some(v) = read_json(&f.path()) else { continue };
        let pid = v.get("pid").and_then(Value::as_u64).unwrap_or(0);
        if pid == 0 || !Path::new(&format!("/proc/{pid}")).exists() { let _ = fs::remove_file(f.path()); continue; }
        out.push(OpenTab {
            tab: v.get("tab").and_then(Value::as_str).unwrap_or_default().to_string(),
            session: v.get("session").and_then(Value::as_str).map(String::from),
            attached: v.get("attached").and_then(Value::as_bool).unwrap_or(false),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::tests::{data, rec};

    #[test]
    fn requests_go_only_to_a_running_session_in_a_tab() {
        let d = data("queue");
        fs::create_dir_all(alive_dir(&d)).unwrap();
        rec(&d, "old", serde_json::json!({"id":"old","movedTo":"new"}));
        rec(&d, "new", serde_json::json!({"id":"new"}));
        assert!(!queue_prompt(&d, "old", "text"), "not running");
        fs::write(alive_dir(&d).join("new"), "1").unwrap();
        assert!(!queue_prompt(&d, "old", "text"), "no tab shows it");
        write_tab(&d, "t1", Some("new"), false);
        assert!(queue_prompt(&d, "old", "first line\nsecond"), "goes to the tab where the session lives now");
        let queued: Vec<_> = fs::read_dir(tab_inbox(&d, "t1")).unwrap().flatten().collect();
        assert_eq!(queued.len(), 1);
    }
    #[test]
    fn codex_requests_require_a_tab_and_go_to_its_relay() {
        let d = data("codex-input");
        rec(&d, "codex1", serde_json::json!({"id":"codex1","agent":"codex"}));
        fs::create_dir_all(alive_dir(&d)).unwrap();
        fs::write(alive_dir(&d).join("codex1"), "1").unwrap();
        assert!(!queue_prompt(&d, "codex1", "hello"));
        write_tab(&d, "codextab", Some("codex1"), false);
        assert!(queue_prompt(&d, "codex1", "first line\nsecond line"));
        let p = fs::read_dir(tab_inbox(&d, "codextab")).unwrap().next().unwrap().unwrap().path();
        let v = read_json(&p).unwrap();
        assert_eq!(v["text"], "first line\nsecond line");
        assert_eq!(v["enter"], true);
        assert!(!d.cache().join("inbox/codex1.json").exists());
    }
    #[test]
    fn request_kinds() {
        assert_eq!(request_kind("[map update] text"), "map update");
        assert_eq!(request_kind("/exit now"), "command /exit");
        assert_eq!(request_kind("plain text"), "person's input");
    }
    #[test]
    fn tabs_with_a_dead_relay_do_not_count() {
        let d = data("tabs");
        write_tab(&d, "t1", Some("s1"), true);
        fs::write(tab_file(&d, "t2"), r#"{"tab":"t2","pid":999999999,"session":"s2"}"#).unwrap();
        let open = open_tabs(&d);
        assert_eq!(open.len(), 1);
        assert_eq!((open[0].tab.as_str(), open[0].session.as_deref(), open[0].attached), ("t1", Some("s1"), true));
        assert!(!tab_file(&d, "t2").exists());
    }
}
