//! The tabs sessions run in, and sending a request to a session through its tab.
use super::*;

/// A request's kind for the record: its leading [tag], a slash command, or the person's own text.
pub fn request_kind(text: &str) -> String {
    let t = text.trim_start();
    if t.starts_with('[')
        && let Some(end) = t.char_indices().take(40).find(|(_, c)| *c == ']').map(|(i, _)| i) { return t[1..end].to_string(); }
    if t.starts_with('/') { return format!("command {}", t.split_whitespace().next().unwrap_or(t)); }
    "person's input".into()
}

/// Queues text for a session's tab to type in, as if typed. Only for a session running now: a request left for a
/// stopped one would run hours later, after the work was done elsewhere. True when queued.
pub fn queue_prompt(d: &Data, id: &str, text: &str) -> bool {
    let now = current_id(d, id);
    // A project's request is its text: the host's log names only that it was one.
    let kind = if crate::roots::Roots::new(d.clone()).project_of(&now).is_some() { "project".to_string() } else { request_kind(text) };
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
    put(d, &tab, &now, kind, text, "the session is running")
}

/// Queues an agent's first prompt for a tab that has no session yet and waits for one (`OpenTab::ready`). True when queued.
pub fn queue_first(d: &Data, tab: &str, text: &str) -> bool {
    let kind = if tab_project(d, tab).is_some() { "project".to_string() } else { request_kind(text) };
    if !open_tabs(d).iter().any(|t| t.tab == tab && t.session.is_none() && t.ready) {
        record::decide("request", "request not sent", "the tab does not wait for a first prompt", &[("tab", tab.to_string()), ("kind", kind)]);
        return false;
    }
    put(d, tab, "", kind, text, "the tab waits for its first prompt")
}

fn put(d: &Data, tab: &str, session: &str, kind: String, text: &str, why: &str) -> bool {
    let seq = SystemTime::now().duration_since(UNIX_EPOCH).map(|t| t.as_nanos()).unwrap_or(0);
    if write_atomic(&tab_inbox(d, tab).join(format!("{seq}.json")), serde_json::json!({"text": text, "enter": true}).to_string()).is_err() { return false; }
    record::decide("request", "request queued", why, &[("request", seq.to_string()), ("session", session.to_string()), ("tab", tab.to_string()), ("kind", kind), ("chars", text.chars().count().to_string())]);
    true
}

/// A tab's own record of what it shows, for the program and the other tabs.
pub fn tab_file(d: &Data, tab: &str) -> PathBuf { tabs_dir(d).join(format!("{tab}.json")) }

/// Where the requests to a tab wait. A project's are its content: in memory, not in the host's data folder (spec 3.22).
/// Without a runtime folder (no vault opens there either) they wait where the host's do.
pub fn tab_inbox(d: &Data, tab: &str) -> PathBuf {
    let runtime = crate::vault::runtime_dir();
    let of_project = tab_project(d, tab).is_some() && runtime.parent().is_some_and(Path::is_dir);
    if of_project { runtime.join("tabs") } else { tabs_dir(d) }.join(format!("{tab}.in"))
}

/// At start: what still waits for a tab that ended without its own clean-up is not for anyone.
pub fn drop_dead_inboxes(d: &Data) {
    let open: Vec<String> = open_tabs(d).into_iter().map(|t| t.tab).collect();
    for f in crate::paths::list_dir(&tabs_dir(d)) {
        let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let Some(tab) = name.strip_suffix(".in").or_else(|| name.strip_suffix(".project")) else { continue };
        if open.iter().any(|o| o == tab) { continue; }
        // Both places: a project's waited in the host's data folder before.
        for inbox in [tab_inbox(d, tab), tabs_dir(d).join(format!("{tab}.in"))] { let _ = fs::remove_dir_all(inbox); }
        set_tab_project(d, tab, None);
    }
}

pub fn write_tab(d: &Data, tab: &str, session: Option<&str>, attached: bool) {
    // The agent the tab runs, for a tab that has no conversation to tell it by.
    let v = serde_json::json!({"tab": tab, "pid": std::process::id(), "session": session, "attached": attached, "agent": crate::agents::active()});
    let _ = write_atomic(&tab_file(d, tab), v.to_string());
}

/// Notes whether the tab's agent waits for its first prompt. `write_tab` takes the note off.
pub fn mark_ready(d: &Data, tab: &str, ready: bool) {
    let Some(mut v) = read_json(&tab_file(d, tab)).filter(|v| (v["ready"] == true) != ready) else { return };
    v["ready"] = Value::Bool(ready);
    let _ = write_atomic(&tab_file(d, tab), v.to_string());
}

/// The project a tab runs its agent for (its container's), noted by the host's terminal. A tab without one is the
/// host's. What a project's records say about tabs counts only for that project's own tabs.
fn tab_project_file(d: &Data, tab: &str) -> PathBuf { tabs_dir(d).join(format!("{tab}.project")) }

/// Notes (or forgets) the project a tab is of. Whether the note is there now: a tab that cannot note its project does
/// not start (it would be taken for a host tab later).
pub fn set_tab_project(d: &Data, tab: &str, project: Option<&str>) -> bool {
    match project {
        Some(p) => write_atomic(&tab_project_file(d, tab), p).is_ok() && tab_project(d, tab).as_deref() == Some(p),
        None => { let _ = fs::remove_file(tab_project_file(d, tab)); true }
    }
}

pub fn tab_project(d: &Data, tab: &str) -> Option<String> { fs::read_to_string(tab_project_file(d, tab)).ok().filter(|p| !p.is_empty()) }

pub struct OpenTab { pub tab: String, pub session: Option<String>, pub attached: bool,
    /// Its add-on agent; none is Claude Code.
    pub agent: Option<String>,
    /// Without a session, it waits for its first prompt.
    pub ready: bool }

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
            agent: v.get("agent").and_then(Value::as_str).map(String::from),
            ready: v["ready"] == true,
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
    fn a_first_prompt_goes_only_to_a_tab_that_waits_for_one() {
        let d = data("first-prompt");
        write_tab(&d, "t1", None, false);
        set_focus_empty(&d, "t1", "test", false);
        assert!(first_prompt_tab(&d).is_none() && !queue_first(&d, "t1", "hello"), "a sign-in or trust screen");
        mark_ready(&d, "t1", true);
        assert_eq!(first_prompt_tab(&d).as_deref(), Some("t1"));
        assert!(queue_first(&d, "t1", "hello"));
        assert_eq!(fs::read_dir(tab_inbox(&d, "t1")).unwrap().count(), 1);
        mark_ready(&d, "t1", false);
        assert!(first_prompt_tab(&d).is_none() && !queue_first(&d, "t1", "hello"), "it works");
        write_tab(&d, "t2", None, false);
        mark_ready(&d, "t2", true);
        set_focus_empty(&d, "t2", "test", false);
        assert_eq!(first_prompt_tab(&d).as_deref(), Some("t2"), "another empty tab came to the front");
        write_tab(&d, "t2", Some("s2"), false);
        assert!(first_prompt_tab(&d).is_none() && !queue_first(&d, "t2", "hello"), "it has its session");
        set_focus(&d, "s2", "test");
        assert!(first_prompt_tab(&d).is_none());
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
    fn a_projects_requests_wait_outside_the_hosts_data_and_go_with_a_dead_tab() {
        let d = data("project-inbox");
        // Its own name: the runtime folder is the same for every data folder.
        let tab = format!("test{}", std::process::id());
        rec(&d, "p1", serde_json::json!({"id":"p1"}));
        fs::create_dir_all(alive_dir(&d)).unwrap();
        fs::write(alive_dir(&d).join("p1"), "1").unwrap();
        set_tab_project(&d, &tab, Some("up"));
        write_tab(&d, &tab, Some("p1"), false);
        assert!(queue_prompt(&d, "p1", "the project's words"));
        let inbox = tab_inbox(&d, &tab);
        assert_eq!(fs::read_dir(&inbox).unwrap().count(), 1);
        if crate::vault::runtime_dir().parent().unwrap().is_dir() {
            assert!(!inbox.starts_with(&d.root), "not in the host's data");
            assert!(!tabs_dir(&d).join(format!("{tab}.in")).exists());
        }
        drop_dead_inboxes(&d);
        assert!(inbox.exists(), "its tab runs");
        // Left in the host's data by a version from before, and the tab dead.
        let old = tabs_dir(&d).join(format!("{tab}.in"));
        fs::create_dir_all(&old).unwrap();
        fs::write(tab_file(&d, &tab), format!(r#"{{"tab":"{tab}","pid":999999999,"session":"p1"}}"#)).unwrap();
        drop_dead_inboxes(&d);
        assert!(!inbox.exists() && !old.exists() && tab_project(&d, &tab).is_none());
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
        assert!(open[0].agent.is_none(), "Claude Code");
        fs::write(tab_file(&d, "t1"), serde_json::json!({"tab": "t1", "pid": std::process::id(), "agent": "codex"}).to_string()).unwrap();
        assert_eq!(open_tabs(&d)[0].agent.as_deref(), Some("codex"), "an empty tab's agent");
    }
}
