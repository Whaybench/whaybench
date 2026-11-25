//! The relay between a terminal panel and Claude Code. Alacritty shows this program; this program
//! holds Claude Code's terminal. It reads the screen as Alacritty would draw it, so it can tell what was typed;
//! it types into the session on request; and when the panel goes, it hands the session to the background.
//! It lives and ends with its tab. It owns no session: Claude Code does.
mod screen;
mod token;

use std::{
    ffi::CString,
    fs,
    io::Write,
    os::fd::RawFd,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use screen::{Mark, Titles, typed_command};
use wb_core::{log as record, paths::{self, Data}, sessions};

const TICK: Duration = Duration::from_secs(2);
/// How long after /resume the relay looks for the session it opened (the old mod looked six times, two seconds apart).
const RESUME_WATCH: Duration = Duration::from_secs(14);
/// An attach that ends this soon did not attach.
const ATTACH_FAILS_WITHIN: Duration = Duration::from_secs(3);
/// The map tool as Claude Code names it: the plugin `context-map`, its MCP server `whaybench`.
const MAP_TOOL: &str = "mcp__plugin_context-map_whaybench__context_map";
/// After /background, how long Claude Code gets before its program is stopped.
const AFTER_BACKGROUND: Duration = Duration::from_secs(4);
/// After ending a session, how long until its tab goes.
const AFTER_END: Duration = Duration::from_secs(3);
const CUT_TITLE: usize = 56;

static RESIZED: AtomicBool = AtomicBool::new(false);
extern "C" fn on_winch(_: libc::c_int) { RESIZED.store(true, Ordering::Relaxed); }

struct Relay {
    data: Data,
    tab: String,
    session: Option<String>,
    attached: bool,
    master: RawFd,
    child: libc::pid_t,
    running: bool,
    started: Instant,
    started_at: std::time::SystemTime,
    /// The agent's first hook of this run has come (looked at for a project's Claude Code).
    hooked: bool,
    /// When text was last typed in from outside (ms), until its turn is seen.
    typed_at: Option<i64>,
    screen: vt100::Parser,
    titles: Titles,
    /// The add-on agent's `ready`.
    ready: Option<Mark>,
    title: String,
    /// When set, the relay ends at that moment.
    leave_at: Option<Instant>,
    closing: bool,
    map_from: Option<String>,
    /// When /resume was typed: for a while, look whether it opened a session running in the background.
    resumed_at: Option<Instant>,
}

fn size_of(fd: RawFd) -> libc::winsize {
    let mut w = libc::winsize { ws_row: 40, ws_col: 120, ws_xpixel: 0, ws_ypixel: 0 };
    unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut w) };
    if w.ws_row == 0 || w.ws_col == 0 { w.ws_row = 40; w.ws_col = 120; }
    w
}

fn cut(text: &str, n: usize) -> String { text.chars().take(n).collect() }

fn write_all(fd: RawFd, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        let n = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
        if n < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted { continue; }
            return;
        }
        bytes = &bytes[n as usize..];
    }
}

impl Relay {
    /// Starts Claude Code on a new terminal of its own, at the size this terminal has now. The size must be
    /// right before the program starts, or Claude Code's input line stays one line high.
    fn start(&mut self, args: &[String]) -> Result<(), String> {
        self.started = Instant::now();
        self.started_at = std::time::SystemTime::now();
        self.hooked = false;
        let size = size_of(0);
        let agent = wb_core::agents::active().map(|id| wb_core::agents::load(&self.data.root, &id)).transpose().map_err(|e| e.to_string())?;
        let addon = agent.is_some();
        self.ready = agent.as_ref().and_then(|a| Mark::new(a.ready.as_deref()?));
        let program = if let Some(a) = &agent {
            let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
            wb_core::versions::find_program(&a.command, &std::env::var("PATH").unwrap_or_default(), home.as_deref()).ok_or_else(|| format!("{} is not installed", a.command))?
        } else { sessions::claude_program() };
        let mut c_program = CString::new(program.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let mut argv: Vec<CString> = vec![c_program.clone()];
        let mut args = agent.as_ref().map_or_else(|| args.to_vec(), |a| a.args(args));
        // An agent that makes its session only at its first prompt (Codex) runs no start hook to take the map in until
        // then: it is started with the request to read the map as that prompt.
        let carry_prompt = self.map_from.as_ref().and(agent.as_ref()).and_then(|a| a.first_prompt(wb_core::i18n::ai("map-carry-prompt")));
        if let Some(p) = &carry_prompt { args.extend(p.iter().cloned()); }
        // A new conversation starts in the work folder; one coming back starts where it was: Claude Code
        // keeps conversations by folder, and --resume looks only in the present one.
        let home = std::env::var("HOME").unwrap_or_default();
        let work = paths::work_dir(&self.data, std::path::Path::new(&home));
        let _ = fs::create_dir_all(&work);
        let earlier = self.session.as_deref().and_then(|s| sessions::record_of(&self.data, s)).and_then(|r| r.get("cwd").and_then(|c| c.as_str()).map(String::from));
        let cwd = earlier.filter(|c| std::path::Path::new(c).is_dir()).unwrap_or_else(|| work.to_string_lossy().to_string());
        // A Briefsafe project's conversation (by its record, else by its folder) runs in the project's folder and only
        // in its container (spec 3.22): a record cannot send it elsewhere, and no project conversation falls to the host.
        // A tab already a project's (asked for as one, or started in one before) stays it: closed, it does not open.
        let noted = sessions::tab_project(&self.data, &self.tab);
        let project = match &noted {
            Some(id) => Some(wb_core::roots::stubs(&self.data).into_iter().find(|st| &st.id == id).filter(|st| st.open() && !wb_core::container::closing(st))
                .ok_or_else(|| wb_core::i18n::t("briefsafe-project-closed").to_string())?),
            None => self.session.as_deref().and_then(|s| wb_core::roots::Roots::new(self.data.clone()).project_of(s))
                .and_then(|id| wb_core::roots::stubs(&self.data).into_iter().find(|st| st.id == id))
                // A folder of a closed project is no place for a host conversation, nor is it that project's.
                .or_else(|| wb_core::roots::project_root_of(&self.data, std::path::Path::new(&cwd)).filter(wb_core::roots::Stub::open)),
        };
        let cwd = if project.is_none() && wb_core::roots::project_root_of(&self.data, std::path::Path::new(&cwd)).is_some() { home.clone() } else { cwd };
        let cwd = match &project {
            Some(p) if wb_core::roots::project_root_of(&self.data, std::path::Path::new(&cwd)).is_none_or(|at| at.id != p.id) => p.folder.to_string_lossy().to_string(),
            _ => cwd,
        };
        let place = match &project {
            None => wb_core::container::Place::Host,
            Some(p) => {
                let own = paths::own_exe().map_err(|e| e.to_string())?;
                // As the project was opened: in its container, then never on the host.
                // (A project opened before this was noted: its running container says so.)
                let isolated = wb_core::container::opened_isolated(p) || (wb_core::container::podman_found() && wb_core::container::running(p));
                wb_core::container::place_for(Some(p), isolated, p.open(), isolated && wb_core::container::running(p),
                    || wb_core::container::life(p, || if p.open() { wb_core::container::start(p, &wb_core::container::programs(&own)) } else { Err(wb_core::i18n::t("briefsafe-project-closed").to_string()) }))
            }
        };
        if let wb_core::container::Place::Refused(why) = &place { return Err(why.clone()); }
        // Which root this tab's sessions are looked for in: a project's records count only for its own tabs.
        if !sessions::set_tab_project(&self.data, &self.tab, project.as_ref().map(|p| p.id.as_str())) { return Err(wb_core::i18n::t("briefsafe-project-closed").to_string()); }
        // The map is kept every turn: its tool (the plugin's MCP server) runs without asking. Attaching to a session
        // in the background takes no options; that session was started with them.
        if !addon && args.first().is_none_or(|a| a != "attach") {
            args.extend(["--allowedTools".into(), MAP_TOOL.into()]);
            // Answers in the interface language, as Claude Code's own `language` setting does: once in the system
            // prompt, not a note every turn. A language the person set themselves is left alone.
            let mut settings = serde_json::Map::new();
            // A project's settings are its own Claude folder's and its folder's, read beneath them (never through a link).
            let set_here = |key: &str| match &project {
                None => sessions::claude_setting_set(key, std::path::Path::new(&cwd)),
                Some(p) => {
                    let has = |root: &std::path::Path, rel: &std::path::Path| wb_core::beneath::read(root, rel, 1 << 20).ok()
                        .and_then(|t| serde_json::from_slice::<serde_json::Value>(&t).ok()).is_some_and(|v| match &v[key] { serde_json::Value::Null => false, serde_json::Value::String(s) => !s.is_empty(), _ => true });
                    let inside = std::path::Path::new(&cwd).strip_prefix(&p.folder).ok().map(std::path::Path::to_path_buf);
                    has(&p.stored().root, std::path::Path::new("claude/settings.json"))
                        || inside.is_some_and(|r| ["settings.json", "settings.local.json"].iter().any(|f| has(&p.folder, &r.join(".claude").join(f))))
                }
            };
            if let Some(l) = wb_core::i18n::agent_language(&self.data).filter(|_| !set_here("language")) {
                settings.insert("language".into(), l.into());
            }
            // Context and plan usage for the notifications panel come from the status line. A status line the person
            // set is theirs to keep (it can pass the same input on to `whaybench statusline`); without one, ours.
            // In a project's container the program is on its PATH (the host's install path is not there).
            if !set_here("statusLine")
                && let Some(exe) = if project.is_some() { Some("whaybench".to_string()) } else { paths::own_exe().ok().and_then(|e| wb_core::agents::shell_quote(&e.with_file_name("whaybench")).ok()) } {
                settings.insert("statusLine".into(), serde_json::json!({"type": "command", "command": format!("{exe} statusline")}));
            }
            if !settings.is_empty() { args.extend(["--settings".into(), serde_json::Value::Object(settings).to_string()]); }
        }
        for a in &args { argv.push(CString::new(a.as_bytes()).map_err(|e| e.to_string())?); }
        let c_cwd = CString::new(cwd.clone()).map_err(|e| e.to_string())?;
        // All of the child's environment is prepared here, before the fork.
        let mut env: Vec<(String, String)> = vec![
            ("TERM".into(), "xterm-256color".into()), ("COLORTERM".into(), "truecolor".into()),
            ("WHAYBENCH".into(), "1".into()), ("WHAYBENCH_TAB".into(), self.tab.clone()),
            (paths::ENV_DATA_DIR.into(), self.data.root.to_string_lossy().to_string()),
        ];
        if let Some(from) = self.map_from.take() {
            env.push(("WHAYBENCH_MAP_FROM".into(), from));
            // Sent as the session's first prompt, so the map is read and confirmed before the person goes on. Given on
            // the command line already, the map itself is told at the start instead.
            if carry_prompt.is_none() { env.push(("WHAYBENCH_MAP_PROMPT".into(), wb_core::i18n::ai("map-carry-prompt").into())); }
        }
        // An add-on agent's own settings folder, made here: the person has confirmed the add-on by now.
        if let (Some(a), None) = (&agent, &project) { env.extend(addon_home(&self.data, a)?); }
        // A Briefsafe project's conversation runs in the project's container (spec 3.22), with its data folder and
        // the Claude Code token from the keyring; every value goes through podman's own environment.
        if let Some(s) = &project {
            env.retain(|(k, _)| k != paths::ENV_DATA_DIR);
            env.push((paths::ENV_DATA_DIR.into(), s.open_at.to_string_lossy().to_string()));
            env.push(("CLAUDE_CONFIG_DIR".into(), s.open_at.join(wb_core::roots::CLAUDE_DIR).to_string_lossy().to_string()));
            for k in ["LANG", "LC_ALL"] { if let Ok(v) = std::env::var(k) { env.push((k.into(), v)); } }
            // An add-on agent (Codex) keeps its settings, sign-in and conversations in the project too.
            if let Some(a) = &agent {
                env.push((wb_core::agents::ENV_AGENT.into(), a.id.clone()));
                let isolated = matches!(place, wb_core::container::Place::Container);
                if let Some((var, folder)) = wb_core::container::agent_home(s, isolated, a)? { env.push((var, folder)); }
            }
            if !addon {
                match wb_core::secret::get(wb_core::secret::CLAUDE_TOKEN) {
                    Ok(Some(t)) => env.push(("CLAUDE_CODE_OAUTH_TOKEN".into(), t)),
                    // Said, not left to a sign-in screen nobody expects: the agent can still sign in by itself.
                    Ok(None) => wb_core::status::say(&self.data, wb_core::i18n::t("briefsafe-no-token"), 15),
                    Err(e) => { record::warn("session", "no token for the project", &e, &[("tab", self.tab.clone())]); wb_core::status::say(&self.data, wb_core::i18n::t("briefsafe-keyring-failed"), 15); }
                }
            }
        }
        if let (Some(s), wb_core::container::Place::Container) = (&project, &place) {
            // The agents' programs as they are now: an update since the container started moved them.
            if let Some(home) = std::env::var_os("HOME") { let _ = wb_core::container::refresh_programs(s, std::path::Path::new(&home)); }
            let podman = wb_core::versions::find_program("podman", &std::env::var("PATH").unwrap_or_default(), None).ok_or("podman is not installed")?;
            let names: Vec<String> = env.iter().map(|(k, _)| k.clone()).collect();
            // By its name: in there it is found among the program links, wherever the host has it.
            let command = agent.as_ref().map_or("claude", |a| a.command.as_str());
            let name = std::path::Path::new(command).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| command.to_string());
            let inner = wb_core::container::exec_args(s, std::path::Path::new(&cwd), &names, &name, &args);
            c_program = CString::new(podman.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
            argv = std::iter::once(Ok(c_program.clone())).chain(inner.iter().map(|a| CString::new(a.as_bytes()).map_err(|e| e.to_string()))).collect::<Result<_, String>>()?;
            record::decide("session", "in the project's container", "the conversation's folder is an open Briefsafe project's", &[("project", s.id.clone())]);
        }
        let mut ptrs: Vec<*const libc::c_char> = argv.iter().map(|a| a.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        let c_env: Vec<(CString, CString)> = env.iter().filter_map(|(k, v)| Some((CString::new(k.as_str()).ok()?, CString::new(v.as_str()).ok()?))).collect();

        let umask = wb_core::paths::user_umask();
        let mut master = -1;
        let pid = unsafe { libc::forkpty(&mut master, std::ptr::null_mut(), std::ptr::null_mut(), &size) };
        if pid < 0 { return Err(std::io::Error::last_os_error().to_string()); }
        if pid == 0 {
            unsafe {
                libc::signal(libc::SIGHUP, libc::SIG_DFL);
                libc::signal(libc::SIGPIPE, libc::SIG_DFL);
                libc::chdir(c_cwd.as_ptr());
                // The agent writes in the person's folders: their umask, not Whaybench's private one.
                libc::umask(umask);
                for (k, v) in &c_env { libc::setenv(k.as_ptr(), v.as_ptr(), 1); }
                libc::execv(c_program.as_ptr(), ptrs.as_ptr());
                libc::_exit(127);
            }
        }
        self.master = master;
        self.child = pid;
        self.running = true;
        self.screen = vt100::Parser::new(size.ws_row, size.ws_col, 0);
        self.titles = Titles::default();
        record::decide("session", "program started", "this tab's agent", &[("tab", self.tab.clone()), ("args", args.join(" "))]);
        Ok(())
    }

    fn set_title(&mut self, title: &str) {
        let title = cut(title, CUT_TITLE);
        if title == self.title { return; }
        self.title = title;
        write_all(1, format!("\x1b]0;{}\x07", self.title).as_bytes());
    }

    fn save_tab(&self) { sessions::write_tab(&self.data, &self.tab, self.session.as_deref(), self.attached); }

    /// What the person typed reached the Enter key.
    fn input(&mut self, bytes: &[u8]) {
        if !self.running { return; }
        if bytes == b"\r"
            && let Some(name) = typed_command(&self.cursor_line()) {
                self.slash(name);
                return;
            }
        write_all(self.master, bytes);
    }

    fn cursor_line(&self) -> String {
        let s = self.screen.screen();
        let (row, _) = s.cursor_position();
        s.rows(0, s.size().1).nth(row as usize).unwrap_or_default()
    }

    /// /exit, /quit, /background or /bg typed alone. A session the tab runs itself gets the command as typed.
    /// A session in the background: /exit ends it for real and closes the tab; /background does nothing,
    /// because on a background session Claude Code would start an empty one.
    fn slash(&mut self, name: &str) {
        // The agent's hooks cannot see these commands: the map does not roll back, and the next session takes it over.
        if ["rewind", "checkpoint", "undo"].contains(&name) {
            if let Some(s) = &self.session { wb_core::hook::note_rewind(&wb_core::roots::for_session(&self.data, s).root, s); }
            write_all(self.master, b"\r");
            return;
        }
        if name == "resume" {
            self.resumed_at = Some(Instant::now());
            write_all(self.master, b"\r");
            return;
        }
        // A project's tab never holds a host background session, whatever number its records gave it.
        let in_background = sessions::tab_project(&self.data, &self.tab).is_none() && self.session.as_deref().is_some_and(sessions::background);
        record::decide("session", "typed command", if in_background { "a background session" } else { "this tab's session" }, &[("tab", self.tab.clone()), ("command", name.to_string())]);
        if !in_background {
            write_all(self.master, b"\r");
            return;
        }
        write_all(self.master, b"\x15");
        let Some(session) = self.session.clone() else { return };
        if name == "exit" || name == "quit" {
            sessions::stop(&session);
            sessions::mark_ended(&self.data, &session);
            record::decide("session", "session ended", "/exit in a background session", &[("session", session)]);
            self.closing = true;
            self.leave_at = Some(Instant::now() + AFTER_END);
        } else {
            record::decide("session", "command not run", "already running in the background", &[("session", session), ("command", name.to_string())]);
        }
    }

    /// The panel closed (the terminal went away).
    fn panel_gone(&mut self) {
        if self.closing { return; }
        self.closing = true;
        // A project's conversation ends with its tab too: the host cannot see or reach what runs on in its container.
        if wb_core::agents::other() || sessions::tab_project(&self.data, &self.tab).is_some() {
            self.leave_at = Some(Instant::now());
            return;
        }
        // A session nobody spoke in has nothing to keep: it ends with its tab, and does not stay behind in the background.
        let hand_over = self.running && !self.attached && self.session.as_deref().is_some_and(|s| sessions::alive(&self.data, s) && sessions::has_conversation(&self.data, s));
        if hand_over {
            // Started in this tab: hand it to the supervisor instead of ending it.
            write_all(self.master, b"/background\r");
            record::decide("session", "moved to the background", "the tab closed; the session goes on", &[("session", self.session.clone().unwrap_or_default())]);
            self.leave_at = Some(Instant::now() + AFTER_BACKGROUND);
        } else {
            // A tab attached to a session nobody spoke in (an empty tab given /background): no tab will ever show it again.
            if self.attached
                && let Some(s) = self.session.clone().filter(|s| sessions::background(s) && !sessions::has_conversation(&self.data, s)) {
                    record::decide("session", "empty session ended", "the tab of a background session with no conversation closed", &[("session", s.clone())]);
                    sessions::stop(&s);
                }
            self.leave_at = Some(Instant::now());
        }
    }

    /// The program in this tab ended. A session the tab ran itself is off now, said at once instead of the
    /// session list waiting for its heartbeat to go stale; an attached tab only showed one that runs on.
    fn program_ended(&mut self) {
        self.running = false;
        unsafe {
            libc::close(self.master);
            libc::waitpid(self.child, std::ptr::null_mut(), 0);
        }
        self.master = -1;
        record::decide("session", "program ended", "this tab's agent ended", &[("tab", self.tab.clone())]);
        // It waits for no first prompt any more.
        self.save_tab();
        if self.closing { self.leave_at = Some(Instant::now()); return; }
        // Attaching ended at once: nothing runs under that name any more. The conversation is on disk: open it from there.
        if self.attached && self.started.elapsed() < ATTACH_FAILS_WITHIN
            && let Some(s) = self.session.clone() {
                record::warn("session", "attach failed", "that session is not in the background; reopened from the record", &[("tab", self.tab.clone()), ("session", s.clone())]);
                self.attached = false;
                self.save_tab();
                write_all(1, b"\x1b[2J\x1b[H");
                if self.start(&["--resume".into(), s]).is_ok() { return; }
            }
        self.set_title(wb_core::i18n::t("ended"));
        if let Some(s) = self.session.clone()
            && !self.attached && !sessions::background(&s) { sessions::mark_off(&self.data, &s); }
    }

    /// Every 2 seconds: which session this tab holds, its name, and whether it carried on under a new id.
    fn tick(&mut self) {
        if self.closing { return; }
        // The tab's project closed: its tab goes too, nothing of it stays on screen or is found again by a later open.
        if let Some(id) = sessions::tab_project(&self.data, &self.tab)
            && !wb_core::roots::stubs(&self.data).iter().any(|s| s.id == id && s.open() && !wb_core::container::closing(s)) {
                record::decide("session", "tab closed", "its project closed", &[("tab", self.tab.clone()), ("project", id)]);
                self.closing = true;
                self.leave_at = Some(Instant::now());
                return;
            }
        let project = sessions::tab_project(&self.data, &self.tab).and_then(|id| wb_core::roots::stubs(&self.data).into_iter().find(|s| s.id == id)).map(|s| s.root());
        if let Some(found) = sessions::session_of_tab(&self.data, &self.tab)
            && self.session.as_deref() != Some(&found)
            // (A project's tab follows no move: its record's word of where it went is not the host's to act on.)
            && (project.is_some() || !self.session.as_deref().is_some_and(|s| sessions::current_id(&self.data, s) != s)) {
                record::decide("session", "tab session found", "a session record names this tab", &[("tab", self.tab.clone()), ("old", self.session.clone().unwrap_or_default()), ("new", found.clone())]);
                if project.is_some() && let Some(old) = &self.session { sessions::mark_off(&self.data, old); }
                self.session = Some(found);
                self.save_tab();
            }
        // That it runs: an add-on agent's, and a project's Claude Code (in its container, so not in the list the host
        // asks Claude Code for). A project's tab writes it in the project, whose record may not be there yet.
        if (wb_core::agents::other() || project.is_some()) && self.running
            && let Some(s) = &self.session {
                let at = project.unwrap_or_else(|| wb_core::roots::for_session(&self.data, s));
                let file = sessions::alive_dir(&at).join(s);
                // A project's Claude Code from its first hook on: before, it may be showing a start screen (sign-in,
                // trust), which a request must not be typed into.
                // Once seen it stays so: the computer may sleep, or its clock be set, meanwhile.
                self.hooked = self.hooked || paths::meta(&file).and_then(|m| m.modified()).is_ok_and(|t| t >= self.started_at);
                if wb_core::agents::other() || self.hooked {
                    let _ = paths::create_dir_all(&sessions::alive_dir(&at));
                    let _ = paths::write_atomic(&file, self.child.to_string());
                }
            }
        if !wb_core::agents::other() && sessions::tab_project(&self.data, &self.tab).is_none() {
            self.resumed_somewhere();
            self.follow_move();
        }
        if !self.running && self.title == wb_core::i18n::t("ended") { return; }
        if let Some(s) = self.session.clone() {
            let title = wb_core::agents::session_line(&self.data.root, &s);
            self.set_title(&title);
        }
    }

    /// After /resume (the picker finishes later): when it opened a session running in the background, this tab's
    /// number moves there, so the tab follows it and the empty session left behind is stopped (`follow_move`).
    fn resumed_somewhere(&mut self) {
        let Some(at) = self.resumed_at else { return };
        if at.elapsed() > RESUME_WATCH { self.resumed_at = None; return; }
        let Some(me) = self.session.clone() else { return };
        let Some((target, empty)) = sessions::resumed_elsewhere(&self.data, &me) else { return };
        self.resumed_at = None;
        // In the conversation's own root: a project's move stays in the project.
        match wb_core::session_record::moved(&wb_core::roots::for_session(&self.data, &me).root, &me, &target, empty.as_deref()) {
            Ok(()) => record::decide("session", "number change", "/resume opened a session running in the background", &[("tab", self.tab.clone()), ("old", me), ("new", target), ("empty", empty.unwrap_or_default())]),
            Err(e) => record::error("session", "move not recorded", &[("error", e.to_string())]),
        }
    }

    /// A session that carried on under a new id gets this tab attached to it, in the same place. One tab per
    /// conversation: if another tab already shows it, this one gives way. A copy left behind is stopped.
    fn follow_move(&mut self) {
        let Some(old) = self.session.clone() else { return };
        // A project's conversation has no background the host sees: what its record says of one (a host session's
        // number) is not followed, stopped or taken over.
        if sessions::tab_project(&self.data, &self.tab).is_some() { return; }
        // Program gone and no move recorded: Claude Code's own note may still say where it went.
        let now = if self.running { sessions::current_id(&self.data, &old) } else { sessions::locate(&self.data, &old) };
        if now == old { return; }
        if !sessions::background(&now) {
            record::detail("session", "waiting for the move", &[("tab", self.tab.clone()), ("old", old), ("new", now)]);
            return;
        }
        record::decide("session", "number change seen", "the old record's movedTo points to the new number, which runs in the background", &[("tab", self.tab.clone()), ("old", old.clone()), ("new", now.clone()), ("source", "movedTo".into())]);
        if let Some(empty) = sessions::record_of(&self.data, &old).and_then(|r| r.get("emptyCarried").and_then(|v| v.as_str().map(String::from))) {
            stop_copy(&empty);
        }
        if sessions::open_tabs(&self.data).iter().any(|t| t.tab != self.tab && t.session.as_deref() == Some(&now)) {
            record::decide("session", "tab closed", "the same conversation is already in another tab (one tab per conversation)", &[("tab", self.tab.clone()), ("session", now)]);
            stop_copy(&old);
            self.closing = true;
            self.leave_at = Some(Instant::now());
            return;
        }
        if self.running {
            unsafe { libc::kill(self.child, libc::SIGHUP); }
            // Its end comes through the master; wait for it here so the new program takes over cleanly.
            unsafe { libc::close(self.master); libc::waitpid(self.child, std::ptr::null_mut(), 0); }
            self.running = false;
        }
        self.session = Some(now.clone());
        self.attached = true;
        self.save_tab();
        write_all(1, b"\x1b[2J\x1b[H");
        match self.start(&["attach".into(), sessions::attach_name(&now)]) {
            Ok(()) => record::decide("session", "tab reattached", "in the same place under the new number", &[("tab", self.tab.clone()), ("old", old.clone()), ("new", now)]),
            Err(e) => record::error("session", "tab not attached", &[("error", e)]),
        }
        stop_copy(&old);
    }

    /// Text the program or a panel wants typed into this session: `{"text": "...", "enter": true}`.
    fn requests(&mut self) {
        // Do not paste a queued prompt into a permission dialog or an active turn, whatever the agent. A command
        // (/exit when the person ends the session) goes in at once: it must not wait for the turn.
        let commands_only = {
            // A project's Claude Code before its first hook may be at a start screen: its last state is of last time.
            let mut waiting = !wb_core::agents::other() && !self.hooked && sessions::tab_project(&self.data, &self.tab).is_some();
            if let Some(s) = &self.session {
                let state = paths::read_text(&sessions::state_dir(&wb_core::roots::for_session(&self.data, s)).join(format!("{s}.json"))).ok()
                    .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).unwrap_or_default();
                // One text a turn: after one, the next waits until its turn is seen (or none comes).
                if self.typed_at.is_some_and(|at| !turn_pending(at, &state, wb_core::clock::now_ms())) { self.typed_at = None; }
                waiting |= state["state"] != "done" || self.typed_at.is_some();
            }
            waiting
        };
        let mut commands_only = commands_only;
        let dir = sessions::tab_inbox(&self.data, &self.tab);
        for (path, v) in inbox(&dir) {
            // While a held question waits for the person, another conversation's words wait too: a prompt takes it away.
            if v.as_ref().is_some_and(|v| v["other"] == true)
                && self.session.as_ref().is_some_and(|s| wb_core::ask::held(&wb_core::roots::for_session(&self.data, s).root, s)) { continue; }
            if commands_only && !v.as_ref().and_then(|v| v["text"].as_str()).is_some_and(|t| t.trim_start().starts_with('/')) { continue; }
            let _ = fs::remove_file(&path);
            let Some(v) = v else { continue };
            sessions::asked::note_request(&self.data, &v, self.running);
            if !self.running { continue; }
            let text = v.get("text").and_then(|t| t.as_str()).unwrap_or_default().to_string();
            // Sent from outside, the same commands get the same decision as when typed.
            if v.get("enter").and_then(|e| e.as_bool()).unwrap_or(false)
                && let Some(name) = typed_command(&format!("> {text}"))
                    && self.session.as_deref().is_some_and(sessions::background) {
                        self.slash(name);
                        continue;
                    }
            // As a paste, so a request of several lines arrives whole instead of each line being sent.
            write_all(self.master, b"\x1b[200~");
            write_all(self.master, text.as_bytes());
            write_all(self.master, b"\x1b[201~");
            if v.get("enter").and_then(|e| e.as_bool()).unwrap_or(false) {
                // Claude Code reads pasted text and Enter apart only with a moment between them.
                std::thread::sleep(Duration::from_millis(150));
                write_all(self.master, b"\r");
                if !text.trim_start().starts_with('/') { self.typed_at = Some(wb_core::clock::now_ms()); commands_only = true; }
                // A first prompt: no other one until its session is known, or the agent waits again.
                if self.session.is_none() { self.save_tab(); }
            }
        }
    }

    fn resized(&mut self) {
        let w = size_of(0);
        if self.running { unsafe { libc::ioctl(self.master, libc::TIOCSWINSZ, &w) }; }
        self.screen.screen_mut().set_size(w.ws_row, w.ws_col);
    }

    fn run(&mut self) {
        let mut buffer = vec![0u8; 65536];
        let mut last_tick = Instant::now();
        loop {
            if self.leave_at.is_some_and(|t| Instant::now() >= t) { break; }
            if RESIZED.swap(false, Ordering::Relaxed) { self.resized(); }
            let mut fds = [
                libc::pollfd { fd: if self.closing { -1 } else { 0 }, events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: if self.running { self.master } else { -1 }, events: libc::POLLIN, revents: 0 },
            ];
            let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, 200) };
            if n > 0 {
                if fds[1].revents != 0 {
                    let got = unsafe { libc::read(self.master, buffer.as_mut_ptr().cast(), buffer.len()) };
                    if got <= 0 {
                        self.program_ended();
                    } else {
                        let bytes = &buffer[..got as usize];
                        self.screen.process(bytes);
                        if self.session.is_none() && self.ready.as_mut().is_some_and(|m| m.seen(bytes)) { sessions::mark_ready(&self.data, &self.tab); }
                        if !self.closing { let shown = self.titles.strip(bytes); write_all(1, &shown); }
                    }
                }
                if fds[0].revents != 0 {
                    let got = unsafe { libc::read(0, buffer.as_mut_ptr().cast(), buffer.len()) };
                    if got <= 0 {
                        if got < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted { continue; }
                        self.panel_gone();
                    } else {
                        let bytes = buffer[..got as usize].to_vec();
                        self.input(&bytes);
                    }
                }
            }
            if last_tick.elapsed() >= TICK {
                last_tick = Instant::now();
                self.tick();
            }
            self.requests();
        }
    }

    fn finish(&mut self) {
        let of_project = sessions::tab_project(&self.data, &self.tab);
        // Found by the tab's project, which is forgotten below.
        let waiting = sessions::tab_inbox(&self.data, &self.tab);
        if self.running {
            unsafe { libc::kill(self.child, libc::SIGHUP); }
            // In a project's container the agent is not the child's own: it is ended there.
            if let Some(p) = sessions::tab_project(&self.data, &self.tab).and_then(|id| wb_core::roots::stubs(&self.data).into_iter().find(|s| s.id == id)) {
                wb_core::container::hang_up(&p, &self.tab);
            }
        }
        // After the agent's own end: its tab no longer says that it runs.
        if (wb_core::agents::other() || of_project.is_some())
            && let Some(s) = &self.session { sessions::mark_off(&self.data, s); }
        let _ = fs::remove_file(sessions::tab_file(&self.data, &self.tab));
        sessions::set_tab_project(&self.data, &self.tab, None);
        // No request finds the tab any more. What was still to be typed in is not: told to what asked, and to the person.
        let left: Vec<serde_json::Value> = inbox(&waiting).into_iter().filter_map(|(_, v)| v).collect();
        for v in &left { sessions::asked::note_request(&self.data, v, false); }
        if left.iter().any(|v| !v["text"].as_str().unwrap_or("").starts_with('/')) {
            wb_core::status::say(&self.data, wb_core::i18n::t("request-lost-with-tab"), 12);
        }
        let _ = fs::remove_dir_all(waiting);
    }
}

/// The requests waiting in a tab's inbox in the order to type them: the person's own (an answer to a held question is
/// one) before other conversations', each oldest first (a name is its time).
fn inbox(dir: &std::path::Path) -> Vec<(std::path::PathBuf, Option<serde_json::Value>)> {
    let mut all: Vec<_> = fs::read_dir(dir).into_iter().flatten().flatten().map(|f| f.path())
        .map(|p| { let v = fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()); (p, v) }).collect();
    all.sort_by_key(|(p, v)| (v.as_ref().is_some_and(|v| v["other"] == true), p.clone()));
    all
}

/// Whether the turn of a text typed in at `at` is still to be seen: the agent says so by its state, or no turn comes of
/// it (10 seconds).
fn turn_pending(at: i64, state: &serde_json::Value, now: i64) -> bool {
    state["state"] == "done" && state["at"].as_i64().unwrap_or(0) <= at && (0..10_000).contains(&(now - at))
}

/// An add-on agent's settings folder on the host, made ready, and the variable that names it. Its hooks name the
/// program's path (this process can outlive an update of its file).
fn addon_home(data: &Data, agent: &wb_core::agents::Agent) -> Result<Option<(String, String)>, String> {
    let exe = paths::own_exe().map_err(|e| e.to_string())?.with_file_name("whaybench");
    Ok(agent.prepare(&data.root, &exe).map_err(|e| e.to_string())?.map(|(var, folder)| (var, folder.to_string_lossy().to_string())))
}

fn stop_copy(old: &str) {
    if sessions::background(old) {
        record::decide("session", "left-over copy stopped", "the old number runs empty in the background", &[("session", old.to_string())]);
        sessions::stop(old);
    }
}

/// Shows what an add-on agent will run and asks; the screen is raw, so lines end in \r\n and one key answers.
fn confirm_agent(data: &Data, agent: &wb_core::agents::Agent) -> bool {
    use std::io::Read;
    let (program, files, told) = agent.summary();
    let t = |k: &str, v: &[(&str, String)]| wb_core::i18n::f(k, v);
    // The long part first, what runs last: the program stays on screen next to the question.
    let mut text = String::new();
    if !told.trim().is_empty() { text += &format!("{}\n{}\n\n", t("agent-trust-context", &[]), told.trim()); }
    text += &format!("{}\n\n    {program}\n\n", t("agent-trust-title", &[("name", agent.name.clone()), ("path", agent.folder().display().to_string())]));
    if !files.is_empty() { text += &format!("{}\n\n", t("agent-trust-files", &[("files", files.join(", "))])); }
    text += &t("agent-trust-ask", &[]);
    // What the terminal sent by itself before the question (answers to its own queries) is not an answer.
    unsafe { libc::tcflush(0, libc::TCIFLUSH); }
    write_all(1, text.replace('\n', "\r\n").as_bytes());
    let mut key = [0u8; 1];
    let yes = loop {
        if !std::io::stdin().read(&mut key).is_ok_and(|n| n == 1) { break false; }
        match key[0] {
            // An escape sequence (a focus change, a terminal's reply): skipped to its final letter.
            0x1b => { while std::io::stdin().read(&mut key).is_ok_and(|n| n == 1) && !key[0].is_ascii_alphabetic() && key[0] != b'~' {} }
            b'y' | b'Y' => break true,
            k if k >= 0x20 || k == b'\r' => break false,
            _ => {}
        }
    };
    if yes {
        if let Err(e) = agent.trust(&data.root) { record::warn("session", "agent confirmation not kept", &e.to_string(), &[]); }
        write_all(1, b"\r\n");
        return true;
    }
    write_all(1, format!("\r\n{}\r\n", t("agent-trust-declined", &[])).as_bytes());
    let _ = std::io::stdin().read(&mut key);
    false
}

fn set_raw() {
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(0, &mut t) == 0 {
            libc::cfmakeraw(&mut t);
            libc::tcsetattr(0, libc::TCSANOW, &t);
        }
    }
}

fn main() {
    wb_core::paths::private_files();
    // The Claude Code token for Briefsafe projects, made and stored without copying it (whaybench-panel token_terminal).
    if std::env::args().nth(1).as_deref() == Some("--token") {
        let Some(program) = std::env::args().nth(2) else { std::process::exit(2) };
        let stored = token::take(&program).and_then(|t| t.ok_or_else(|| "no token on the screen".to_string()))
            .and_then(|t| wb_core::secret::set(wb_core::secret::CLAUDE_TOKEN, wb_core::secret::CLAUDE_TOKEN_LABEL, &t));
        if let Err(e) = stored { let _ = writeln!(std::io::stderr(), "whaybench-term: {e}"); std::process::exit(1); }
        return;
    }
    let mut args = std::env::args().skip(1);
    let (mut tab, mut attach, mut resume, mut map_from, mut project) = (None, None, None, None, None);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--tab" => tab = args.next(),
            "--attach" => attach = args.next(),
            "--resume" => resume = args.next(),
            "--map-from" => map_from = args.next(),
            "--project" => { project = args.next(); if project.is_none() { let _ = writeln!(std::io::stderr(), "whaybench-term: --project needs a project"); std::process::exit(2); } }
            other => { let _ = writeln!(std::io::stderr(), "whaybench-term: unknown argument {other}"); std::process::exit(2); }
        }
    }
    let Some(tab) = tab else { let _ = writeln!(std::io::stderr(), "whaybench-term: --tab is needed"); std::process::exit(2) };
    let Some(root) = paths::resolve(None, |k| std::env::var(k).ok()) else { std::process::exit(2) };
    let data = Data::new(root);
    // Noted before anything starts: the tab is that project's, whatever its records say later.
    if project.is_some() && !sessions::set_tab_project(&data, &tab, project.as_deref()) { std::process::exit(1); }
    let run = record::run_id(&data);
    record::open(&data, &format!("term-{tab}"), &run, "first");
    record::install_panic_hook();

    unsafe {
        // The relay outlives the panel long enough to hand the session over.
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_winch as *const () as usize;
        libc::sigaction(libc::SIGWINCH, &action, std::ptr::null_mut());
    }
    set_raw();

    // A new Claude Code conversation is given its number here, so the tab and the panels that follow it know the
    // session from the start, not only after its first prompt.
    let new_id = (attach.is_none() && resume.is_none() && !wb_core::agents::other())
        .then(|| fs::read_to_string("/proc/sys/kernel/random/uuid").ok().map(|u| u.trim().to_string())).flatten();
    let session = attach.clone().or_else(|| resume.clone()).or_else(|| new_id.clone());
    let mut relay = Relay {
        data, tab, session, attached: attach.is_some(), master: -1, child: 0, running: false, started: Instant::now(), started_at: std::time::SystemTime::now(), hooked: false, typed_at: None,
        screen: vt100::Parser::new(40, 120, 0), titles: Titles::default(), ready: None, title: String::new(),
        leave_at: None, closing: false, map_from, resumed_at: None,
    };
    // Before the session is known (an add-on agent tells it with its first hook): the agent's name.
    let first = match &relay.session { Some(s) => wb_core::agents::new_session_line(&relay.data.root, s),
        None => format!("{} · {}", wb_core::i18n::t("new-session"), wb_core::agents::name_of(&relay.data.root, wb_core::agents::active().as_deref())) };
    relay.set_title(&first);
    let argv: Vec<String> = match (&attach, &resume) {
        (Some(a), _) => vec!["attach".into(), sessions::attach_name(a)],
        (_, Some(r)) => vec!["--resume".into(), r.clone()],
        _ => new_id.map(|id| vec!["--session-id".into(), id]).unwrap_or_default(),
    };
    // An add-on agent runs a program as the person: one not as the program brings it is shown and asked first.
    if let Some(agent) = wb_core::agents::active().and_then(|id| wb_core::agents::load(&relay.data.root, &id).ok()) {
        if !agent.trusted(&relay.data.root) && !confirm_agent(&relay.data, &agent) {
            record::decide("session", "agent not started", "the person did not confirm the add-on", &[("agent", agent.id.clone())]);
            std::process::exit(0);
        }
        if let Some(command) = &agent.trust { wb_core::status::say(&relay.data, &wb_core::i18n::f("agent-hooks", &[("name", agent.name.clone()), ("command", command.clone())]), 8); }
    }
    if let Err(e) = relay.start(&argv) {
        record::error("session", "program not started", &[("error", e.clone())]);
        // The tab closes with the program: the reason stays in the status bar (an agent not installed, for one).
        wb_core::status::say(&relay.data, &wb_core::i18n::f("tab-not-opened", &[("e", e.clone())]), 15);
        let _ = writeln!(std::io::stderr(), "whaybench-term: {e}");
        std::process::exit(1);
    }
    relay.save_tab();
    relay.run();
    relay.finish();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_add_on_agents_folder_has_hooks_that_name_the_program() {
        let dir = tempfile::tempdir().unwrap();
        let data = Data::new(dir.path().to_path_buf());
        wb_core::addons::install_bundled(&data);
        let (var, folder) = addon_home(&data, &wb_core::agents::load(&data.root, "codex").unwrap()).unwrap().unwrap();
        assert_eq!((var.as_str(), folder), ("CODEX_HOME", dir.path().join("codex").to_string_lossy().to_string()));
        let hooks = fs::read_to_string(dir.path().join("codex/hooks.json")).unwrap();
        assert!(hooks.contains("/whaybench' hook codex") && !hooks.contains("(deleted)"), "{hooks}");
    }

    #[test]
    fn one_text_a_turn_the_persons_first_then_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        for (n, t) in [("20", r#"{"text":"b","other":true}"#), ("10", r#"{"text":"a","other":true}"#), ("30", r#"{"text":"answer"}"#)] { fs::write(dir.path().join(format!("{n}.json")), t).unwrap(); }
        let order: Vec<String> = inbox(dir.path()).into_iter().map(|(_, v)| v.unwrap()["text"].as_str().unwrap().to_string()).collect();
        assert_eq!(order, ["answer", "a", "b"]);
        let st = |s: &str, at: i64| serde_json::json!({"state": s, "at": at});
        assert!(turn_pending(100, &st("done", 50), 200), "its hook has not come");
        assert!(!turn_pending(100, &st("working", 150), 200) && !turn_pending(100, &st("done", 180), 200), "its turn began, or is over");
        assert!(!turn_pending(100, &st("done", 50), 20_000), "no turn came of it");
    }
}
