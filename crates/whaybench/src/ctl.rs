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
    /// `agent`: an agent's doing, not the person's choice.
    FocusTerm { tab: String, #[serde(default)] agent: bool },
    FocusApp { app: String },
    FrontApp { app: String },
    Restart,
    ResetLayout,
    /// A panel closed if open, opened with `command` if not.
    TogglePanel { key: String, #[serde(default)] command: Option<String> },
    /// An app to run with this compositor's socket, so its window lands in a panel.
    Spawn { command: String },
    /// The panel's Firefox out to a window of its own on the desktop (`out`), or back into a panel.
    Browser { out: bool },
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
        Some("browser") => match words.get(1).map(String::as_str) {
            Some("detach") => Ok(Request::Browser { out: true }),
            Some("attach") => Ok(Request::Browser { out: false }),
            _ => Err("browser needs detach or attach".into()),
        },
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

/// Text one conversation's agent puts in another: marked as coming from that conversation, so the receiving agent
/// does not take it for the person's own words; a command (`/`) or a shell line (`!`) is refused.
fn from_other(text: &str, who: &str) -> Result<String, String> {
    let head = text.trim_start();
    if head.starts_with('/') || head.starts_with('!') { return Err(wb_core::i18n::t("type-command-from-another-session").into()); }
    Ok(format!("{} {text}", wb_core::i18n::ai_f("typed-from-session", &[("session", who.to_string())])))
}

/// Requests to type in another conversation, from an agent whose sandbox keeps the tabs' input out of its reach.
pub fn ask_dir(data: &Data) -> PathBuf { data.root.join("ask") }

enum Refused { Rule(String), Write(String) }

/// Types text in the tab that shows session `id`. From another conversation tab, that is from its agent: said so, and
/// never a command or a shell line; when the answer comes, the person is taken back to the tab that asked.
/// `left`: when it was left to be typed.
fn deliver(data: &Data, id: &str, text: &str, from: Option<&str>, asked: bool, left: i64) -> Result<(), Refused> {
    // A conversation that carried on (a restart, /background) has a new number: the one it was given is followed.
    let now = sessions::current_id(data, id);
    let tab = sessions::open_tabs(data).into_iter().find(|t| t.session.as_deref() == Some(now.as_str()));
    let Some(tab) = tab else {
        let head = text.split_whitespace().next().unwrap_or(text);
        return Err(Refused::Rule(wb_core::i18n::f("not-run-no-conversation-tab-open", &[("head", (head).to_string())])));
    };
    // Typing in one's own tab is the agent's own; a request from ask/ names its sender itself, so it is always marked.
    let from = from.filter(|f| asked || *f != tab.tab);
    let text = match from {
        Some(f) => from_other(text, &sessions::session_of_tab(data, f).map(|s| wb_core::agents::session_label(&data.root, &s)).unwrap_or_else(|| f.to_string())).map_err(Refused::Rule)?,
        None => text.to_string(),
    };
    let dir = sessions::tab_inbox(data, &tab.tab);
    let name = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    // What becomes of it is noted for the asker's `session wait` (before the tab can take it).
    let asker = from.unwrap_or("");
    let v = serde_json::json!({"text": text, "enter": true, "other": from.is_some(), "asked": {"id": id, "tab": asker, "left": left}});
    sessions::asked::note(data, id, asker, left, sessions::asked::Became::Left);
    if let Err(e) = fs::create_dir_all(&dir).and_then(|_| write_atomic(&dir.join(format!("{name}.json")), v.to_string().as_bytes())) { return Err(Refused::Write(e.to_string())); }
    // Typed in: what fails after this is not sent again, or the other conversation would get it twice.
    if let Some(from) = from {
        let back = serde_json::json!({"session": id, "tab": from, "at": now_ms()});
        write_atomic(&return_file(data), back.to_string().as_bytes()).map_err(|e| Refused::Rule(e.to_string()))?;
    }
    post(data, &Request::FocusTerm { tab: tab.tab, agent: from.is_some() }).map_err(Refused::Rule)
}

/// The requests agents left in ask/: checked and delivered as a `session type` from their tab would be. The tab a
/// request names is the agent's own word; the text is marked as from another conversation whatever it names.
fn deliver_asked(data: &Data) {
    deliver_asked_in(data, data, None);
    // A project's agents ask in their own root, and only for the project's own conversations (spec 3.22).
    for (pid, root) in wb_core::roots::Roots::new(data.clone()).projects() { deliver_asked_in(data, &root, Some(&pid)); }
}

/// The longest request file taken, in bytes.
const ASK_MAX: u64 = 64 * 1024;

/// The request files waiting in a folder, oldest first, each taken (removed) as it is read. Only plain files, small,
/// a few each tick: the folder is an agent's to write, and a pipe named like a request would block this loop (the
/// whole window). In a project's open place nothing is reached through a link (`beneath`).
fn take_files(dir: &std::path::Path) -> Vec<Vec<u8>> {
    const MAX: u64 = ASK_MAX;
    let mut names: Vec<std::path::PathBuf> = wb_core::paths::list_dir(dir).into_iter().filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
    names.sort();
    names.into_iter().take(20).filter_map(|p| {
        match wb_core::beneath::split(&p) {
            Some((root, rel)) => { let b = wb_core::beneath::read(&root, &rel, MAX).ok(); let _ = wb_core::beneath::remove(&root, &rel); b }
            None => {
                let plain = fs::symlink_metadata(&p).is_ok_and(|m| m.is_file() && m.len() <= MAX);
                let b = plain.then(|| fs::read(&p).ok()).flatten();
                let _ = fs::remove_file(&p);
                b
            }
        }
    }).collect()
}

fn deliver_asked_in(data: &Data, root: &Data, project: Option<&str>) {
    for bytes in take_files(&ask_dir(root)) {
        let Some(v) = serde_json::from_slice::<serde_json::Value>(&bytes).ok() else { continue };
        let (to, text) = (v["to"].as_str().unwrap_or(""), v["text"].as_str().unwrap_or(""));
        let mut from = v["from"].as_str().unwrap_or("another conversation").to_string();
        let left = v["left"].as_i64().unwrap_or_else(now_ms);
        // Told to the asker's `session wait`, for a conversation of the root it was asked in.
        let refused = |from: &str| if wb_core::roots::known_session(data, to).is_some_and(|k| k.root == root.root) { sessions::asked::note(data, to, from, left, sessions::asked::Became::Refused) };
        if let Some(pid) = project {
            // Where it would land in the end (after moves), and where it is asked from, must both be this project's own;
            // the sender is the project, not what the request says; no control bytes reach a terminal.
            let mine = |s: &str| wb_core::roots::known_session(data, s).is_some_and(|k| k.root == root.root);
            let landing = sessions::current_id(data, to);
            let clean = !text.chars().any(|c| c.is_control() && c != '\n' && c != '\t');
            if !mine(to) || !mine(&landing) || !clean {
                record::warn("request", "agent's request not delivered", "a project's agent may type only plain text in its own project's conversations", &[("session", if wb_core::map::safe_id(to).is_ok() { to.to_string() } else { "?".into() }), ("project", pid.to_string())]);
                refused(&from);
                continue;
            }
            // A tab of this project may be named (its conversation is said, and the person taken back to it); else the project.
            if sessions::tab_project(data, &from).as_deref() != Some(pid) {
                from = wb_core::roots::stubs(data).into_iter().find(|s| s.id == pid).map_or_else(|| pid.to_string(), |s| format!("project {}", s.name));
            }
        }
        let from = from.as_str();
        match deliver(data, to, text, Some(from), true, left) {
            Ok(()) => {
                record::decide("request", "asked by an agent", "a sandboxed agent's request, checked here", &[("session", to.to_string()), ("from", from.to_string())]);
            }
            // A project's request: a fixed reason (the error may quote its text).
            Err(Refused::Rule(e) | Refused::Write(e)) => {
                refused(from);
                record::warn("request", "agent's request not delivered", if project.is_some() { "refused for a project's request" } else { &e }, &[("session", to.to_string()), ("from", from.to_string())]);
            }
        }
    }
}

/// Whether an agent in a conversation tab runs this command (the panels, the person's, have no tab).
fn by_agent() -> bool { std::env::var_os("WHAYBENCH_TAB").is_some() }

/// Where to go back to once an asked session answers (`session type` from another tab).
pub fn return_file(data: &Data) -> std::path::PathBuf { data.cache().join("focus-return.json") }

/// `whaybench ctl session ...`: decided here, not in the compositor, because asking Claude Code what runs
/// can take seconds and the window must not wait.
fn session_command(data: &Data, words: &[String]) -> Result<(), String> {
    // In a Briefsafe project's container its data folder is the project's: `type` and `wait` work there through the
    // project's own folder (the host delivers what it leaves in ask/); the rest needs the host's Whaybench.
    let in_project = wb_core::roots::is_project_root(data);
    let here = matches!(words.first().map(String::as_str), Some("type" | "wait"));
    if in_project && words.first().map(String::as_str) == Some("list") {
        for r in sessions::all(data).iter().filter(|r| r["movedTo"].as_str().unwrap_or("").is_empty()) {
            let id = r["id"].as_str().unwrap_or("");
            println!("{id} · {} · {} · {}", wb_core::agents::name_of(&data.root, r["agent"].as_str()), if sessions::alive(data, id) { "running" } else { "not running" }, sessions::session_name(data, id).lines().next().unwrap_or_default());
        }
        return Ok(());
    }
    if in_project && !here {
        return Err("in a Briefsafe project only these work: `whaybench ctl session list` (the project's conversations), `session type ID \"TEXT\"`, `session wait ID [SECONDS]`".into());
    }
    if !in_project && !running(data) { return Err(format!("not running with data directory {}", data.root.display())); }
    match words.first().map(String::as_str) {
        Some("new") => {
            // `--project ID`: a new conversation in that Briefsafe project (it must be open; the tab refuses otherwise).
            let mut words = words.to_vec();
            let project = match words.iter().position(|w| w == "--project") {
                Some(i) => { let v = words.get(i + 1).cloned().ok_or("session new --project needs a project")?; words.drain(i..i + 2); Some(v) }
                None => None,
            };
            if let Some(p) = &project && !wb_core::roots::stubs(data).iter().any(|s| &s.id == p && s.open()) { return Err(wb_core::i18n::t("briefsafe-project-closed").into()); }
            let (agent, map_from) = match &words[1..] {
                [flag, id, rest @ ..] if flag == "--agent" => (Some(id.clone()), rest.first().cloned()),
                rest => (None, rest.first().cloned()),
            };
            // A project's map (its session's, or a file in its data) starts a conversation in that project only, a host
            // map a host one or a project's (host -> project is allowed; spec 3.22).
            let map_project = map_from.as_deref().and_then(|m| if m.starts_with('/') {
                let real = std::fs::canonicalize(m).unwrap_or_else(|_| m.into());
                wb_core::beneath::split(std::path::Path::new(m)).and_then(|(r, _)| r.file_name().map(|n| n.to_string_lossy().to_string()))
                    .or_else(|| wb_core::roots::stubs(data).into_iter().find(|s| std::fs::canonicalize(s.stored().root).is_ok_and(|r| real.starts_with(r))).map(|s| s.id))
            } else { wb_core::roots::Roots::new(data.clone()).project_of(m) });
            if map_project.is_some() && map_project != project { return Err(wb_core::i18n::t("briefsafe-map-stays").into()); }
            if let Some(m) = &map_from
                && !m.starts_with('/') && !wb_core::roots::for_session(data, m).root.join(format!("{m}.json")).exists() {
                    return Err(wb_core::i18n::t("new-tab-keep-map-not-opened-2").into());
                }
            post(data, &Request::OpenTerm { open: TermOpen { map_from, agent, project, by_agent: by_agent(), ..Default::default() } })
        }
        Some("show") => match (words.get(1).map(String::as_str), words.get(2), words.get(3)) {
            // `--project ID SESSION`: a Briefsafe project's conversation, opened only as that project's while it is open.
            (Some("--project"), Some(p), Some(id)) => {
                if !wb_core::roots::stubs(data).iter().any(|s| &s.id == p && s.open()) { return Err(wb_core::i18n::t("briefsafe-project-closed").into()); }
                let located = sessions::locate(data, id);
                // Both the one asked for and where its moves lead are the project's.
                let roots = wb_core::roots::Roots::new(data.clone());
                if [id.as_str(), located.as_str()].iter().any(|s| roots.project_of(s).as_deref() != Some(p.as_str())) { return Err(wb_core::i18n::t("briefsafe-outside-project").into()); }
                if let Some(t) = sessions::open_tabs(data).into_iter().find(|t| t.session.as_deref() == Some(located.as_str())) { return post(data, &Request::FocusTerm { tab: t.tab, agent: by_agent() }); }
                open_session_of(data, &located, Some(p.clone()))
            }
            (Some("--project"), _, _) => Err("session show --project needs a project and a session".into()),
            _ => show_session(data, words.get(1).ok_or("session show needs an id")?),
        },
        Some("type") => {
            let id = words.get(1).ok_or("session type needs an id")?;
            let text = words.get(2).ok_or("session type needs text")?;
            let from = std::env::var("WHAYBENCH_TAB").ok();
            let left = now_ms();
            // From a project's conversation: always through ask/ (the host checks it is this project's own). One that
            // cannot take it is said now, not left to a wait for an answer.
            if in_project {
                let now = sessions::current_id(data, id);
                if wb_core::roots::known_session(data, &now).is_none() { return Err(format!("{id} is not a conversation of this project (`whaybench ctl session list`)")); }
                if !sessions::alive(data, &now) { return Err(format!("{id} is not running: its tab is closed, the person opens it in the Projects panel")); }
                from_other(text, "this project").map_err(|e| e.to_string())?;
                // What the host would not type: said here, where the agent sees it.
                if text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') { return Err("the text has control characters (colour codes, carriage returns): send plain text".into()); }
            }
            let tried = if in_project { Err(Refused::Write(String::new())) } else { deliver(data, id, text, from.as_deref(), false, left) };
            match tried {
                // An agent in a sandbox (Codex) may not write other tabs' input: it leaves a request in ask/, which
                // Whaybench, outside the sandbox, checks the same way and delivers. A refusal stays a refusal.
                Err(Refused::Write(_)) if from.is_some() || in_project => {
                    let name = format!("{}-{}.json", SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0), std::process::id());
                    let request = serde_json::json!({"to": id, "text": text, "from": from, "left": left}).to_string();
                    if request.len() as u64 > ASK_MAX { return Err(format!("the text is too long ({} bytes; at most {ASK_MAX}): write it to a file and send its path", request.len())); }
                    // First, so a `session wait` started at once does not take an earlier answer for this one's.
                    if let Some(f) = sessions::asked::left_file(&ask_dir(data), id, from.as_deref().unwrap_or("")) { let _ = write_atomic(&f, left.to_string().as_bytes()); }
                    write_atomic(&ask_dir(data).join(name), request.as_bytes()).map_err(|e| format!("not sent: {e}"))
                }
                Err(Refused::Write(e) | Refused::Rule(e)) => Err(e),
                Ok(()) => Ok(()),
            }
        }
        // After `type`: waits for the session's next answer and prints it, so the asking agent can run this in the
        // background and be told when the answer is there instead of looking again and again.
        Some("wait") => {
            let id = words.get(1).ok_or("session wait needs an id")?;
            let limit = words.get(2).map(|s| s.parse::<u64>().map_err(|_| "session wait: SECONDS is a number")).transpose()?.unwrap_or(900);
            // A project's answer goes only to a tab of that project (spec 3.22: project -> host is blocked), checked at
            // every read: the project may be logged in to while this waits.
            let caller = std::env::var("WHAYBENCH_TAB").ok().and_then(|t| sessions::tab_project(data, &t));
            let outside = |sid: &str| wb_core::roots::Roots::new(data.clone()).project_of(sid).is_some_and(|p| caller.as_deref() != Some(p.as_str()));
            if outside(id) { return Err(wb_core::i18n::t("briefsafe-outside-project").into()); }
            // From the last time something was typed into it, if that was in the last hour: the answer may have come
            // already between `type` and this wait.
            let now = now_ms();
            let tab = std::env::var("WHAYBENCH_TAB").unwrap_or_default();
            let fresh = |at: i64| now - at < 3_600_000;
            // Left in ask/ and not yet taken by the host: what became of an earlier request says nothing of it.
            let left = sessions::asked::left_file(&ask_dir(data), id, &tab).and_then(|f| wb_core::paths::read_text(&f).ok()).and_then(|t| t.trim().parse::<i64>().ok()).filter(|at| fresh(*at));
            // Only what came after the text was typed in: the answer of the turn before it is not its answer.
            let since = || -> Result<Option<i64>, String> {
                let asked = sessions::asked::read(data, id, &tab).filter(|(at, _)| fresh(*at) && left.is_none_or(|l| *at >= l));
                match asked {
                    Some((_, sessions::asked::Became::Typed(at))) => Ok(Some(at)),
                    Some((_, sessions::asked::Became::Refused)) => Err(format!("not delivered to {id}: its tab does not take it (closed, or the text is not plain)")),
                    Some((_, sessions::asked::Became::Left)) => Ok(None),
                    None => Ok(left.is_none().then_some(now)),
                }
            };
            for _ in 0..limit {
                let now = sessions::current_id(data, id);
                // One root for the check and both reads: a log in between them changes nothing.
                let root = wb_core::roots::for_session(data, &now);
                // A root other than the caller's own is a project's: only that project's tab may read it.
                let mine = caller.as_deref().map(|c| wb_core::roots::open_dir(data, c));
                if root.root != data.root && mine.as_ref() != Some(&root.root) { return Err(wb_core::i18n::t("briefsafe-outside-project").into()); }
                // In the conversation's own root (a project's answers are in the project).
                let answer = || -> Option<serde_json::Value> { serde_json::from_str(&wb_core::paths::read_text(&root.root.join("answers").join(format!("{now}.json"))).ok()?).ok() };
                let since = since()?;
                if let Some(v) = since.and_then(|s| answer().filter(|v| v["at"].as_i64().is_some_and(|at| at >= s))) { println!("{}", v["text"].as_str().unwrap_or("")); return Ok(()); }
                // Stopped on a question to the person (a permission, an input): the asker is told, not left waiting.
                // Already waiting when the wait began counts too: it went on to ask between `type` and `wait`.
                let state: Option<serde_json::Value> = wb_core::paths::read_text(&sessions::state_dir(&root).join(format!("{now}.json"))).ok().and_then(|b| serde_json::from_str(&b).ok());
                if let Some(v) = state.filter(|v| v["state"] == "waiting") {
                    return Err(format!("{id} waits for the person: {}", v["note"].as_str().unwrap_or("a permission or an input")));
                }
                if since.is_none() && wb_core::ask::held(&root.root, &now) { return Err(format!("{id} waits for the person: a question in the Answer panel; it takes your text after")); }
                std::thread::sleep(Duration::from_secs(1));
            }
            Err(format!("no answer from {id} within {limit} seconds"))
        }
        Some("end") => {
            // A background session is stopped, one the tab runs gets /exit; then the tab goes.
            let id = &sessions::current_id(data, words.get(1).ok_or("session end needs an id")?);
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
        return post(data, &Request::FocusTerm { tab: t.tab, agent: by_agent() });
    }
    open_session(data, &located)
}

/// The session in a new tab: attached if it runs in the background, else started again from where it was saved.
fn open_session(data: &Data, id: &str) -> Result<(), String> { open_session_of(data, id, None) }

/// `project`: the one the session is known to be of (a tab of last time); else found by its record.
fn open_session_of(data: &Data, id: &str, project: Option<String>) -> Result<(), String> {
    let project = project.or_else(|| wb_core::roots::Roots::new(data.clone()).project_of(id));
    // Known nowhere (a project's whose project has closed since its row was drawn): not opened on the host instead.
    if project.is_none() && wb_core::roots::known_session(data, id).is_none() && sessions::agent(id).is_none() {
        return Err(wb_core::i18n::t("briefsafe-project-closed").into());
    }
    match sessions::agent(id) {
        Some(a) if a.get("kind").and_then(|k| k.as_str()) == Some("interactive") => {
            Err(wb_core::i18n::t("running-in-another-terminal-type-bg").into())
        }
        Some(_) => post(data, &Request::OpenTerm { open: TermOpen { attach: Some(id.to_string()), project, by_agent: by_agent(), ..Default::default() } }),
        None => {
            let resume = sessions::resumable(data, id);
            if resume != id {
                record::decide("session", "reopened under the earlier number", "no conversation is saved under this number", &[("session", id.to_string()), ("resumed", resume.clone())]);
                sessions::moved_back(data, &resume);
            }
            post(data, &Request::OpenTerm { open: TermOpen { resume: Some(resume), project, by_agent: by_agent(), ..Default::default() } })
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Reattach {
    at: u64,
    /// Every open tab in order: its session, or nothing for a tab without a conversation.
    tabs: Vec<Option<String>>,
    /// The Briefsafe project of each tab, in the same order (none for the host's).
    #[serde(default)]
    projects: Vec<Option<String>>,
}

fn now_secs() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }
pub fn now_ms() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0) }

/// Closing the window: the tabs hand their sessions to the background, and the next start attaches them.
pub fn remember_tabs(data: &Data) {
    // Tab ids grow with time, so this is the order they were opened in.
    let mut open = sessions::open_tabs(data);
    open.sort_by(|a, b| a.tab.cmp(&b.tab));
    let projects = open.iter().map(|t| sessions::tab_project(data, &t.tab)).collect();
    let tabs = open.into_iter().map(|t| t.session.filter(|s| sessions::has_conversation(data, s))).collect();
    let text = serde_json::to_string(&Reattach { at: now_secs(), tabs, projects }).unwrap_or_default();
    let _ = write_atomic(&data.cache().join("reattach.json"), text.as_bytes());
}

/// At start: the tabs of last time, attached to where their sessions live now, or one new tab.
pub fn open_first_tabs(data: &Data) {
    let file = data.cache().join("reattach.json");
    let saved = fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Reattach>(&t).ok());
    let _ = fs::remove_file(&file);
    let at = saved.as_ref().map_or(0, |s| s.at);
    let (tabs, mut projects) = saved.map(|r| (r.tabs, r.projects)).unwrap_or_default();
    projects.resize(tabs.len(), None);
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
        let in_project = |s: &String| tabs.iter().zip(&projects).any(|(t, p)| t.as_ref() == Some(s) && p.is_some());
        let ready = |s: &String| in_project(s) || sessions::record_of(&data, s).is_some_and(|r| r.get("agent").is_some()) || sessions::agent(&sessions::locate(&data, s)).is_some_and(|a| a.get("kind").and_then(|k| k.as_str()) != Some("interactive"));
        while now_secs() < at + 10 {
            sessions::refresh_agents();
            if tabs.iter().flatten().all(ready) { break; }
            std::thread::sleep(Duration::from_millis(300));
        }
        for (i, (s, project)) in tabs.into_iter().zip(projects).enumerate() {
            // One at a time, or the terminals race and the tabs come back in another order.
            if i > 0 { std::thread::sleep(Duration::from_millis(800)); }
            // A tab without a conversation comes back as what it was: an empty tab.
            // A project's tab comes back as that project's: it opens only while the project is open, never on the host.
            if let Some(p) = &project && !wb_core::roots::stubs(&data).iter().any(|st| &st.id == p && st.open()) {
                record::decide("session", "tab not reopened", "its project is closed", &[("project", p.clone())]);
                continue;
            }
            let Some(s) = s else {
                let _ = post(&data, &Request::OpenTerm { open: TermOpen { project, ..Default::default() } });
                continue;
            };
            let now = sessions::locate(&data, &s);
            record::decide("session", "tab reopened", "a tab open last time", &[("old", s), ("new", now.clone())]);
            // Always a new tab: the last run's tabs may still be handing their sessions over and look open.
            if let Err(e) = open_session_of(&data, &now, project) { record::warn("session", "tab not reopened", &e, &[("session", now)]); }
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
        if out.exists() {
            println!("{}", out.display());
            // Said to whoever asked, not only in the log: a picture taken while a project is open goes after a while.
            if matches!(request, Request::Shot { .. }) && (!wb_core::roots::Roots::new(data.clone()).projects().is_empty()
                || wb_core::paths::list_dir(&data.cache()).iter().any(|f| f.file_name().is_some_and(|n| n.to_string_lossy().starts_with("view-protected-")))) {
                eprintln!("{}", wb_core::i18n::t("shot-removed-later"));
            }
            return Ok(());
        }
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
    // A sandbox binds its writable folders when the agent starts: this one must be there by then.
    fs::create_dir_all(ask_dir(data))?;
    // A request left by an earlier run is not for this one.
    for f in fs::read_dir(&dir)?.flatten() { let _ = fs::remove_file(f.path()); }
    crate::awake::start(data.clone());
    // The Windows start script could not attach the work disk (3.20): ~/work is on the distribution's own disk now.
    if std::env::var("WHAYBENCH_WORK_DISK").as_deref() == Ok("missing") { wb_core::status::say(data, wb_core::i18n::t("work-disk-missing"), 30); }
    let mut ticks = 0u32;
    let asked_data = data.clone();
    event_loop.handle().insert_source(Timer::from_duration(Duration::from_millis(100)), move |_, _, state| {
        crate::start::reap_apps();
        if let Some(mime) = state.clip.pending.take()
            && let Ok((read, write)) = std::io::pipe()
                && smithay::wayland::selection::data_device::request_data_device_client_selection(&state.seat, mime, write.into()).is_ok() { state.clip.to_desktop(read); }
        deliver_asked(&asked_data);
        if state.clip.from_desktop().is_some() {
            let types = crate::clipboard::TEXT_TYPES.map(String::from).into();
            smithay::wayland::selection::data_device::set_data_device_selection(&state.display_handle, &state.seat, types, ());
        }
        if let Ok(files) = fs::read_dir(&dir) {
            // In the order they were left (names are the time): a press and its release read the other way round
            // left the app holding the button.
            let mut files: Vec<_> = files.flatten().filter(|f| f.path().extension().is_some_and(|e| e == "json")).collect();
            files.sort_by_key(|f| f.file_name());
            for f in files {
                state.redraw = true;
                let parsed = fs::read(f.path()).ok().and_then(|b| serde_json::from_slice::<Request>(&b).ok());
                let _ = fs::remove_file(f.path());
                match parsed {
                    Some(Request::Quit) => { state.end_reason = "ctl quit".into(); state.loop_signal.stop(); }
                    Some(Request::Shot { out, panel, app }) => state.shots.push(Shot { out, panel, app }),
                    Some(Request::Theme { name }) => state.set_theme(&name),
                    Some(Request::OpenTerm { open }) => state.open_terminal(&open),
                    Some(Request::FocusTerm { tab, agent }) => state.focus_term(&tab, agent),
                    Some(Request::FocusApp { app }) => state.focus_app(&app),
                    Some(Request::FrontApp { app }) => state.front_app(&app),
                    Some(Request::Restart) => state.restart(),
                    Some(Request::ResetLayout) => state.reset_layout(),
                    Some(Request::TogglePanel { key, command }) => state.toggle_panel(&key, command.as_deref()),
                    Some(Request::Spawn { command }) => state.spawn(&command),
                    Some(Request::Browser { out }) => state.move_browser(out),
                    Some(Request::CloseTerm { tab }) => state.close_term(&tab),
                    Some(Request::Pointer { x, y }) => state.test_pointer(x, y),
                    Some(Request::Press { right }) => state.test_button(true, if right { 0x111 } else { 0x110 }),
                    Some(Request::Release { right }) => state.test_button(false, if right { 0x111 } else { 0x110 }),
                    Some(Request::Panels { out }) => { let _ = write_atomic(&out, state.describe().as_bytes()); }
                    None => record::warn("run", "request not read", "the request file format differs", &[("file", f.file_name().to_string_lossy().to_string())]),
                }
            }
        }
        if ticks.is_multiple_of(10) { state.tell_waiting(); }
        // Pictures taken while a project was open, once their time is up (and at the first tick, after a restart).
        if ticks.is_multiple_of(100) { crate::render::shots::sweep_expiring(&asked_data); }
        // A project's programs leave requests in its own root; only those that touch nothing outside are taken.
        for (pid, root) in wb_core::roots::Roots::new(asked_data.clone()).projects() {
            for bytes in take_files(&root.ctl()) {
                match serde_json::from_slice::<Request>(&bytes).ok() {
                    Some(Request::FocusApp { app }) => state.focus_app(&app),
                    Some(Request::FrontApp { app }) => state.front_app(&app),
                    Some(_) => record::warn("run", "request refused", "from a project's root only bringing a panel forward is taken", &[("project", pid.clone())]),
                    None => {}
                }
            }
        }
        ticks += 1;
        if ticks.is_multiple_of(10) { state.return_focus(); state.sync_focus(); }
        TimeoutAction::ToDuration(Duration::from_millis(100))
    }).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn another_conversations_text_says_where_it_is_from_and_runs_no_command() {
        let t = super::from_other("does read work?", "Claude Code a3cf3125").unwrap();
        assert!(t.starts_with("[From the Claude Code a3cf3125 conversation") && t.ends_with("does read work?"), "{t}");
        for bad in ["/exit", "  !rm -rf x", "!ls"] { assert!(super::from_other(bad, "x").is_err(), "{bad}"); }
    }

    use super::*;

    #[test]
    fn the_browser_moves_out_and_back_by_ctl_and_as_whaybench_view_writes_it() {
        let w = |t: &str| t.split_whitespace().map(String::from).collect::<Vec<_>>();
        assert_eq!(parse_request(&w("browser detach")), Ok(Request::Browser { out: true }));
        assert_eq!(parse_request(&w("browser attach")), Ok(Request::Browser { out: false }));
        assert!(parse_request(&w("browser")).is_err());
        assert_eq!(serde_json::from_str::<Request>(r#"{"cmd": "browser", "out": true}"#).unwrap(), Request::Browser { out: true });
    }

    #[test]
    fn an_agents_request_is_delivered_marked_and_a_command_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let data = Data::new(dir.path().to_path_buf());
        sessions::write_tab(&data, "t-to", Some("s-to"), false);
        fs::create_dir_all(ask_dir(&data)).unwrap();
        let ask = |name: &str, text: &str| write_atomic(&ask_dir(&data).join(name), serde_json::json!({"to": "s-to", "text": text, "from": "t-codex"}).to_string().as_bytes()).unwrap();
        ask("1.json", "what do you think?");
        ask("2.json", "! rm -rf ~");
        // Naming the receiving tab as its sender does not make it the person's own.
        write_atomic(&ask_dir(&data).join("3.json"), serde_json::json!({"to": "s-to", "text": "/exit", "from": "t-to"}).to_string().as_bytes()).unwrap();
        // A pipe named like a request is not read: it would block the window.
        std::process::Command::new("mkfifo").arg(ask_dir(&data).join("4.json")).status().unwrap();
        deliver_asked(&data);
        assert_eq!(fs::read_dir(ask_dir(&data)).unwrap().count(), 0, "every request is taken, the pipe too");
        let typed: Vec<String> = fs::read_dir(sessions::tab_inbox(&data, "t-to")).unwrap().flatten()
            .map(|f| serde_json::from_slice::<serde_json::Value>(&fs::read(f.path()).unwrap()).unwrap()["text"].as_str().unwrap().to_string()).collect();
        assert_eq!(typed.len(), 1, "the shell line and the spoofed /exit are refused: {typed:?}");
        assert!(typed[0].ends_with("what do you think?") && typed[0] != "what do you think?", "marked as from another conversation: {}", typed[0]);
    }
    #[test]
    fn a_projects_agent_types_only_plain_text_into_its_own_projects_conversations() {
        use wb_core::roots::{Stub, open_plain, write_marker, write_stub};
        let dir = tempfile::tempdir().unwrap();
        let data = Data::new(dir.path().join("host"));
        let stub = Stub::new(&data, "up", "Up", &dir.path().join("work/up"));
        write_stub(&data, &stub).unwrap();
        let st = stub.stored();
        write_marker(&st, "up").unwrap();
        open_plain(&stub).unwrap();
        let rec = |d: &Data, id: &str, extra: serde_json::Value| { let mut v = serde_json::json!({"id": id}); v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone()); write_atomic(&sessions::sessions_dir(d).join(format!("{id}.json")), v.to_string().as_bytes()).unwrap(); };
        rec(&data, "s-host", serde_json::json!({}));
        rec(&stub.root(), "p1", serde_json::json!({}));
        // A record the agent wrote to steer text to the host's conversation.
        rec(&stub.root(), "p2", serde_json::json!({"movedTo": "s-host"}));
        sessions::write_tab(&data, "t-host", Some("s-host"), false);
        sessions::write_tab(&data, "t-p", Some("p1"), false);
        sessions::set_tab_project(&data, "t-p", Some("up"));
        let ask = |n: &str, to: &str, text: &str| write_atomic(&ask_dir(&stub.root()).join(n), serde_json::json!({"to": to, "text": text, "from": "t-host"}).to_string().as_bytes()).unwrap();
        ask("1.json", "s-host", "to the host");
        ask("2.json", "p2", "through a forged move");
        ask("3.json", "p1", "\u{1b}[2J clear");
        ask("4.json", "p1", "hi");
        deliver_asked(&data);
        assert!(fs::read_dir(sessions::tab_inbox(&data, "t-host")).map(|d| d.count()).unwrap_or(0) == 0, "nothing reaches the host's tab");
        let typed: Vec<String> = fs::read_dir(sessions::tab_inbox(&data, "t-p")).unwrap().flatten()
            .map(|f| serde_json::from_slice::<serde_json::Value>(&fs::read(f.path()).unwrap()).unwrap()["text"].as_str().unwrap().to_string()).collect();
        assert_eq!(typed.len(), 1, "{typed:?}");
        assert!(typed[0].contains("project Up") && typed[0].ends_with("hi"), "the sender is the project: {}", typed[0]);
        // What became of each, for the asker's wait: kept in the project, never in the host's root.
        use sessions::asked::{Became, read};
        assert_eq!(read(&data, "p1", "t-host").map(|r| r.1), Some(Became::Refused), "not plain text");
        assert_eq!(read(&data, "p1", "-").map(|r| r.1), Some(Became::Left));
        assert!(!data.cache().join("asked").exists());
    }

    #[test]
    fn text_for_a_conversation_that_moved_goes_to_the_tab_it_is_in_now() {
        let dir = tempfile::tempdir().unwrap();
        let data = Data::new(dir.path().to_path_buf());
        fs::create_dir_all(sessions::sessions_dir(&data)).unwrap();
        write_atomic(&sessions::sessions_dir(&data).join("s-old.json"), br#"{"id":"s-old","movedTo":"s-new"}"#).unwrap();
        sessions::write_tab(&data, "t-new", Some("s-new"), false);
        assert!(deliver(&data, "s-old", "hello", None, false, 1).is_ok());
        assert_eq!(fs::read_dir(sessions::tab_inbox(&data, "t-new")).unwrap().count(), 1);
    }
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
