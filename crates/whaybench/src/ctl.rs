//! Requests to the running compositor, as files like everything else between programs:
//! `whaybench ctl` writes one, a timer here picks it up. Shots wait for the next frame.
use std::{fs, path::{Path, PathBuf}, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};

use serde::{Deserialize, Serialize};
use smithay::reexports::calloop::{EventLoop, timer::{TimeoutAction, Timer}};
use wb_core::{log as record, paths::Data, sessions};

use crate::{App, start::TermOpen};

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum Request {
    Quit,
    Shot { out: PathBuf, panel: Option<usize>, #[serde(default)] app: Option<String> },
    Theme { name: String },
    OpenTerm { open: crate::start::TermOpen },
    FocusTerm { tab: String },
    FocusApp { app: String },
    FrontApp { app: String },
    Restart,
    ResetLayout,
    /// A panel closed if open, opened with `command` if not.
    TogglePanel { key: String, #[serde(default)] command: Option<String> },
    /// An app to run with this compositor's socket, so its window lands in a panel.
    Spawn { command: String },
    CloseTerm { tab: String },
    /// For tests: the same path as a person's pointer, without a person.
    Pointer { x: f64, y: f64 },
    Press { right: bool },
    Release { right: bool },
    /// For tests: a line per panel with its place, tabs and stacking.
    Panels { out: PathBuf },
}

/// A shot waiting for the next frame.
pub struct Shot { pub out: PathBuf, pub panel: Option<usize>, pub app: Option<String> }

#[derive(Serialize, Deserialize)]
pub struct Instance { pub pid: u32, pub socket: String, pub run: String }

pub fn write_instance(data: &Data, run: &str, socket: &str) -> std::io::Result<()> {
    let text = serde_json::to_string(&Instance { pid: std::process::id(), socket: socket.into(), run: run.into() })?;
    write_atomic(&data.instance(), text.as_bytes())
}

pub use wb_core::paths::write_atomic;

fn running(data: &Data) -> bool {
    fs::read_to_string(data.instance()).ok()
        .and_then(|s| serde_json::from_str::<Instance>(&s).ok())
        .is_some_and(|i| Path::new(&format!("/proc/{}", i.pid)).exists())
}

pub fn parse_request(words: &[String]) -> Result<Request, String> {
    match words.first().map(String::as_str) {
        Some("quit") => Ok(Request::Quit),
        Some("restart") => Ok(Request::Restart),
        Some("shot") => {
            let (mut out, mut panel, mut app) = (None, None, None);
            let mut it = words[1..].iter();
            while let Some(w) = it.next() {
                match w.as_str() {
                    "--out" => out = it.next().map(PathBuf::from),
                    "--panel" => panel = Some(it.next().and_then(|n| n.parse().ok()).ok_or("--panel needs a number")?),
                    "--app" => app = it.next().cloned(),
                    other => return Err(format!("unknown argument: {other}")),
                }
            }
            Ok(Request::Shot { out: out.ok_or("shot needs --out FILE.png")?, panel, app })
        }
        Some("theme") => Ok(Request::Theme { name: words.get(1).cloned().ok_or("theme needs a NAME")? }),
        Some("pointer") => {
            let n = |i: usize| words.get(i).and_then(|v| v.parse().ok()).ok_or("pointer needs X Y");
            Ok(Request::Pointer { x: n(1)?, y: n(2)? })
        }
        Some("press") => Ok(Request::Press { right: words.get(1).is_some_and(|w| w == "right") }),
        Some("release") => Ok(Request::Release { right: words.get(1).is_some_and(|w| w == "right") }),
        Some("panels") => Ok(Request::Panels { out: words.get(1).map(PathBuf::from).ok_or("panels needs FILE")? }),
        _ => Err("ctl needs quit, restart, shot, theme, pointer, press, release or panels".into()),
    }
}

/// Leaves one request for the running program.
pub fn post(data: &Data, request: &Request) -> Result<(), String> {
    fs::create_dir_all(data.ctl()).map_err(|e| e.to_string())?;
    let name = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let text = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    write_atomic(&data.ctl().join(format!("{name}-{}.json", std::process::id())), &text).map_err(|e| e.to_string())
}

/// Where to go back to once an asked session answers (`session type` from another tab).
pub fn return_file(data: &Data) -> std::path::PathBuf { data.cache().join("focus-return.json") }

/// `whaybench ctl session ...`: decided here, not in the compositor, because asking Claude Code what runs
/// can take seconds and the window must not wait.
fn session_command(data: &Data, words: &[String]) -> Result<(), String> {
    if !running(data) { return Err(format!("not running with data directory {}", data.root.display())); }
    match words.first().map(String::as_str) {
        Some("new") => {
            let (agent, map_from) = match &words[1..] {
                [flag, id, rest @ ..] if flag == "--agent" => (Some(id.clone()), rest.first().cloned()),
                rest => (None, rest.first().cloned()),
            };
            if let Some(m) = &map_from {
                if !m.starts_with('/') && !data.root.join(format!("{m}.json")).exists() {
                    return Err(wb_core::i18n::t("new-tab-keep-map-not-opened-2").into());
                }
            }
            post(data, &Request::OpenTerm { open: TermOpen { map_from, agent, ..Default::default() } })
        }
        Some("show") => show_session(data, words.get(1).ok_or("session show needs an id")?),
        Some("type") => {
            let id = words.get(1).ok_or("session type needs an id")?;
            let text = words.get(2).ok_or("session type needs text")?;
            let tab = sessions::open_tabs(data).into_iter().find(|t| t.session.as_deref() == Some(id.as_str()));
            let Some(tab) = tab else {
                let head = text.split_whitespace().next().unwrap_or(text);
                return Err(wb_core::i18n::f("not-run-no-conversation-tab-open", &[("head", (head).to_string())]));
            };
            let dir = sessions::tab_inbox(data, &tab.tab);
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let name = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
            write_atomic(&dir.join(format!("{name}.json")), serde_json::json!({"text": text, "enter": true}).to_string().as_bytes()).map_err(|e| e.to_string())?;
            // Asked from another conversation tab: when the answer comes, the person is taken back to the one that asked.
            if let Some(from) = std::env::var("WHAYBENCH_TAB").ok().filter(|f| *f != tab.tab) {
                let back = serde_json::json!({"session": id, "tab": from, "at": now_ms()});
                write_atomic(&return_file(data), back.to_string().as_bytes()).map_err(|e| e.to_string())?;
            }
            post(data, &Request::FocusTerm { tab: tab.tab })
        }
        Some("end") => {
            // A background session is stopped, one the tab runs gets /exit; then the tab goes.
            let id = words.get(1).ok_or("session end needs an id")?;
            let tab = sessions::open_tabs(data).into_iter().find(|t| t.session.as_deref() == Some(id.as_str()));
            if sessions::background(id) {
                sessions::stop(id);
            } else if let Some(t) = &tab {
                let dir = sessions::tab_inbox(data, &t.tab);
                fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                write_atomic(&dir.join("end.json"), serde_json::json!({"text": "/exit", "enter": true}).to_string().as_bytes()).map_err(|e| e.to_string())?;
            }
            sessions::mark_ended(data, id);
            record::decide("session", "session ended", "the person asked", &[("session", id.clone())]);
            if let Some(t) = tab {
                std::thread::sleep(Duration::from_secs(3));
                post(data, &Request::CloseTerm { tab: t.tab })?;
            }
            println!("{}", wb_core::i18n::f("session-ended-hidden-in-the-session", &[]));
            Ok(())
        }
        _ => Err("session needs new, show, type or end".into()),
    }
}

/// A session chosen: its tab, or attached in a new one, or started again in one.
pub fn show_session(data: &Data, id: &str) -> Result<(), String> {
    let located = sessions::locate(data, id);
    if let Some(t) = sessions::open_tabs(data).into_iter().find(|t| t.session.as_deref() == Some(located.as_str())) {
        return post(data, &Request::FocusTerm { tab: t.tab });
    }
    open_session(data, &located)
}

/// The session in a new tab: attached if it runs in the background, else started again from where it was saved.
fn open_session(data: &Data, id: &str) -> Result<(), String> {
    match sessions::agent(id) {
        Some(a) if a.get("kind").and_then(|k| k.as_str()) == Some("interactive") => {
            Err(wb_core::i18n::t("running-in-another-terminal-type-bg").into())
        }
        Some(_) => post(data, &Request::OpenTerm { open: TermOpen { attach: Some(id.to_string()), ..Default::default() } }),
        None => {
            let resume = sessions::resumable(data, id);
            if resume != id {
                record::decide("session", "reopened under the earlier number", "no conversation is saved under this number", &[("session", id.to_string()), ("resumed", resume.clone())]);
                sessions::moved_back(data, &resume);
            }
            post(data, &Request::OpenTerm { open: TermOpen { resume: Some(resume), ..Default::default() } })
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Reattach {
    at: u64,
    /// Every open tab in order: its session, or nothing for a tab without a conversation.
    tabs: Vec<Option<String>>,
}

fn now_secs() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }
pub fn now_ms() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0) }

/// Closing the window: the tabs hand their sessions to the background, and the next start attaches them.
pub fn remember_tabs(data: &Data) {
    // Tab ids grow with time, so this is the order they were opened in.
    let mut open = sessions::open_tabs(data);
    open.sort_by(|a, b| a.tab.cmp(&b.tab));
    let tabs = open.into_iter().map(|t| t.session.filter(|s| sessions::has_conversation(data, s))).collect();
    let text = serde_json::to_string(&Reattach { at: now_secs(), tabs }).unwrap_or_default();
    let _ = write_atomic(&data.cache().join("reattach.json"), text.as_bytes());
}

/// At start: the tabs of last time, attached to where their sessions live now, or one new tab.
pub fn open_first_tabs(data: &Data) {
    let file = data.cache().join("reattach.json");
    let saved = fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Reattach>(&t).ok());
    let _ = fs::remove_file(&file);
    let at = saved.as_ref().map_or(0, |s| s.at);
    let tabs = saved.map(|r| r.tabs).unwrap_or_default();
    if tabs.is_empty() {
        let _ = post(data, &Request::OpenTerm { open: TermOpen::default() });
        return;
    }
    let data = data.clone();
    std::thread::spawn(move || {
        // Sessions handed to the background take a few seconds to carry on under their new number, and a tab of the
        // last run that showed one lets go of it a moment after the window closed: go as soon as every one runs in the
        // background, and at most 10 seconds after the window closed.
        // An add-on agent's session has no background: it is opened again from where it was saved.
        let ready = |s: &String| sessions::record_of(&data, s).is_some_and(|r| r.get("agent").is_some()) || sessions::agent(&sessions::locate(&data, s)).is_some_and(|a| a.get("kind").and_then(|k| k.as_str()) != Some("interactive"));
        while now_secs() < at + 10 {
            sessions::refresh_agents();
            if tabs.iter().flatten().all(ready) { break; }
            std::thread::sleep(Duration::from_millis(300));
        }
        for (i, s) in tabs.into_iter().enumerate() {
            // One at a time, or the terminals race and the tabs come back in another order.
            if i > 0 { std::thread::sleep(Duration::from_millis(800)); }
            // A tab without a conversation comes back as what it was: an empty tab.
            let Some(s) = s else {
                let _ = post(&data, &Request::OpenTerm { open: TermOpen::default() });
                continue;
            };
            let now = sessions::locate(&data, &s);
            record::decide("session", "tab reopened", "a tab open last time", &[("old", s), ("new", now.clone())]);
            // Always a new tab: the last run's tabs may still be handing their sessions over and look open.
            if let Err(e) = open_session(&data, &now) { record::warn("session", "tab not reopened", &e, &[("session", now)]); }
        }
    });
}

/// `whaybench ctl`: leaves the request and, for a shot, waits for the file.
pub fn send(data: &Data, words: &[String]) -> Result<(), String> {
    if words.first().is_some_and(|w| w == "session") { return session_command(data, &words[1..]); }
    let mut request = parse_request(words)?;
    if !running(data) { return Err(format!("not running with data directory {}", data.root.display())); }
    let wait = match &mut request {
        Request::Shot { out, .. } => {
            if out.is_relative() { *out = std::env::current_dir().map_err(|e| e.to_string())?.join(&*out); }
            let _ = fs::remove_file(error_file(out));
            let _ = fs::remove_file(&*out);
            Some(out.clone())
        }
        Request::Panels { out } => {
            if out.is_relative() { *out = std::env::current_dir().map_err(|e| e.to_string())?.join(&*out); }
            let _ = fs::remove_file(&*out);
            Some(out.clone())
        }
        _ => None,
    };
    post(data, &request)?;
    let Some(out) = wait else { return Ok(()) };
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        if out.exists() { println!("{}", out.display()); return Ok(()); }
        if let Ok(e) = fs::read_to_string(error_file(&out)) { let _ = fs::remove_file(error_file(&out)); return Err(e); }
        std::thread::sleep(Duration::from_millis(30));
    }
    Err("no shot in 5 seconds: the window may be minimised or on another desktop".into())
}

pub fn error_file(out: &Path) -> PathBuf {
    let mut s = out.as_os_str().to_os_string();
    s.push(".error");
    s.into()
}

/// Checks for requests ten times a second.
pub fn listen(event_loop: &EventLoop<App>, data: &Data) -> Result<(), Box<dyn std::error::Error>> {
    let dir = data.ctl();
    fs::create_dir_all(&dir)?;
    // A request left by an earlier run is not for this one.
    for f in fs::read_dir(&dir)?.flatten() { let _ = fs::remove_file(f.path()); }
    let mut ticks = 0u32;
    event_loop.handle().insert_source(Timer::from_duration(Duration::from_millis(100)), move |_, _, state| {
        crate::start::reap_apps();
        if let Some(mime) = state.clip.pending.take() {
            if let Ok((read, write)) = std::io::pipe() {
                if smithay::wayland::selection::data_device::request_data_device_client_selection(&state.seat, mime, write.into()).is_ok() { state.clip.to_desktop(read); }
            }
        }
        if state.clip.from_desktop().is_some() {
            let types = crate::clipboard::TEXT_TYPES.map(String::from).into();
            smithay::wayland::selection::data_device::set_data_device_selection(&state.display_handle, &state.seat, types, ());
        }
        if let Ok(files) = fs::read_dir(&dir) {
            for f in files.flatten().filter(|f| f.path().extension().is_some_and(|e| e == "json")) {
                let parsed = fs::read(f.path()).ok().and_then(|b| serde_json::from_slice::<Request>(&b).ok());
                let _ = fs::remove_file(f.path());
                match parsed {
                    Some(Request::Quit) => { state.end_reason = "ctl quit".into(); state.loop_signal.stop(); }
                    Some(Request::Shot { out, panel, app }) => state.shots.push(Shot { out, panel, app }),
                    Some(Request::Theme { name }) => state.set_theme(&name),
                    Some(Request::OpenTerm { open }) => state.open_terminal(&open),
                    Some(Request::FocusTerm { tab }) => state.focus_term(&tab),
                    Some(Request::FocusApp { app }) => state.focus_app(&app),
                    Some(Request::FrontApp { app }) => state.front_app(&app),
                    Some(Request::Restart) => state.restart(),
                    Some(Request::ResetLayout) => state.reset_layout(),
                    Some(Request::TogglePanel { key, command }) => state.toggle_panel(&key, command.as_deref()),
                    Some(Request::Spawn { command }) => state.spawn(&command),
                    Some(Request::CloseTerm { tab }) => state.close_term(&tab),
                    Some(Request::Pointer { x, y }) => state.test_pointer(x, y),
                    Some(Request::Press { right }) => state.test_button(true, if right { 0x111 } else { 0x110 }),
                    Some(Request::Release { right }) => state.test_button(false, if right { 0x111 } else { 0x110 }),
                    Some(Request::Panels { out }) => { let _ = write_atomic(&out, state.describe().as_bytes()); }
                    None => record::warn("run", "request not read", "the request file format differs", &[("file", f.file_name().to_string_lossy().to_string())]),
                }
            }
        }
        ticks += 1;
        if ticks % 10 == 0 { state.return_focus(); state.sync_focus(); }
        TimeoutAction::ToDuration(Duration::from_millis(100))
    }).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn w(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn requests_are_understood() {
        assert_eq!(parse_request(&w(&["quit"])).unwrap(), Request::Quit);
        assert_eq!(parse_request(&w(&["shot", "--out", "/a.png", "--panel", "2"])).unwrap(), Request::Shot { out: "/a.png".into(), panel: Some(2), app: None });
        assert_eq!(parse_request(&w(&["shot", "--out", "/a.png", "--app", "x"])).unwrap(), Request::Shot { out: "/a.png".into(), panel: None, app: Some("x".into()) });
        assert!(parse_request(&w(&["shot"])).is_err());
        assert!(parse_request(&w(&["shot", "--out", "/a", "--panel", "x"])).is_err());
        assert!(parse_request(&w(&["other"])).is_err());
    }
    #[test]
    fn a_request_survives_the_trip_through_a_file() {
        let t = serde_json::to_vec(&Request::Shot { out: "/a.png".into(), panel: None, app: None }).unwrap();
        assert_eq!(serde_json::from_slice::<Request>(&t).unwrap(), Request::Shot { out: "/a.png".into(), panel: None, app: None });
    }
}
