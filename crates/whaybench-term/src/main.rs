//! The relay between a terminal panel and Claude Code. Alacritty shows this program; this program
//! holds Claude Code's terminal. It reads the screen as Alacritty would draw it, so it can tell what was typed;
//! it types into the session on request; and when the panel goes, it hands the session to the background.
//! It lives and ends with its tab. It owns no session: Claude Code does.
mod screen;

use std::{
    ffi::CString,
    fs,
    io::Write,
    os::fd::RawFd,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use screen::{Titles, typed_command};
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
const CUT_TITLE: usize = 32;

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
    screen: vt100::Parser,
    titles: Titles,
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
        let mut size = size_of(0);
        let codex = wb_core::codex::active();
        let program = if codex {
            let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
            wb_core::versions::find_program("codex", &std::env::var("PATH").unwrap_or_default(), home.as_deref()).ok_or("codex is not installed")?
        } else { sessions::claude_program() };
        let c_program = CString::new(program.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let mut argv: Vec<CString> = vec![c_program.clone()];
        let mut args = if codex { wb_core::codex::args(args) } else { args.to_vec() };
        // The map is kept every turn: its tool (the plugin's MCP server) runs without asking. Attaching to a session
        // in the background takes no options; that session was started with them.
        if !codex && args.first().is_none_or(|a| a != "attach") {
            args.extend(["--allowedTools".into(), MAP_TOOL.into()]);
            // Answers in the interface language, as Claude Code's own `language` setting does: once in the system
            // prompt, not a note every turn. A language the person set themselves is left alone.
            if let Some(l) = wb_core::i18n::agent_language(&self.data).filter(|_| !sessions::claude_language_set()) {
                args.extend(["--settings".into(), serde_json::json!({"language": l}).to_string()]);
            }
        }
        for a in &args { argv.push(CString::new(a.as_bytes()).map_err(|e| e.to_string())?); }
        let mut ptrs: Vec<*const libc::c_char> = argv.iter().map(|a| a.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        // A new conversation starts in the work folder; one coming back starts where it was: Claude Code
        // keeps conversations by folder, and --resume looks only in the present one.
        let home = std::env::var("HOME").unwrap_or_default();
        let work = paths::work_dir(&self.data, std::path::Path::new(&home));
        let _ = fs::create_dir_all(&work);
        let earlier = self.session.as_deref().and_then(|s| sessions::record_of(&self.data, s)).and_then(|r| r.get("cwd").and_then(|c| c.as_str()).map(String::from));
        let cwd = earlier.filter(|c| std::path::Path::new(c).is_dir()).unwrap_or_else(|| work.to_string_lossy().to_string());
        let c_cwd = CString::new(cwd).map_err(|e| e.to_string())?;
        // All of the child's environment is prepared here, before the fork.
        let mut env: Vec<(String, String)> = vec![
            ("TERM".into(), "xterm-256color".into()), ("COLORTERM".into(), "truecolor".into()),
            ("WHAYBENCH".into(), "1".into()), ("WHAYBENCH_TAB".into(), self.tab.clone()),
            (paths::ENV_DATA_DIR.into(), self.data.root.to_string_lossy().to_string()),
        ];
        if let Some(from) = self.map_from.take() {
            env.push(("WHAYBENCH_MAP_FROM".into(), from));
            // Sent as the session's first prompt, so the map is read and confirmed before the person goes on.
            env.push(("WHAYBENCH_MAP_PROMPT".into(), wb_core::i18n::ai("map-carry-prompt").into()));
        }
        let c_env: Vec<(CString, CString)> = env.iter().filter_map(|(k, v)| Some((CString::new(k.as_str()).ok()?, CString::new(v.as_str()).ok()?))).collect();

        let umask = wb_core::paths::user_umask();
        let mut master = -1;
        let pid = unsafe { libc::forkpty(&mut master, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) };
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
        if bytes == b"\r" {
            if let Some(name) = typed_command(&self.cursor_line()) {
                self.slash(name);
                return;
            }
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
            if let Some(s) = &self.session { wb_core::hook::note_rewind(&self.data.root, s); }
            write_all(self.master, b"\r");
            return;
        }
        if name == "resume" {
            self.resumed_at = Some(Instant::now());
            write_all(self.master, b"\r");
            return;
        }
        let in_background = self.session.as_deref().is_some_and(sessions::background);
        if !in_background && (name == "background" || name == "bg") {
            if let Some(s) = &self.session { wb_core::hook::note_fork(&self.data.root, s); }
        }
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
        if wb_core::codex::active() {
            self.leave_at = Some(Instant::now());
            return;
        }
        // A session nobody spoke in has nothing to keep: it ends with its tab, and does not stay behind in the background.
        let hand_over = self.running && !self.attached && self.session.as_deref().is_some_and(|s| sessions::alive(&self.data, s) && sessions::has_conversation(&self.data, s));
        if hand_over {
            // Started in this tab: hand it to the supervisor instead of ending it.
            if let Some(s) = &self.session { wb_core::hook::note_fork(&self.data.root, s); }
            write_all(self.master, b"/background\r");
            record::decide("session", "moved to the background", "the tab closed; the session goes on", &[("session", self.session.clone().unwrap_or_default())]);
            self.leave_at = Some(Instant::now() + AFTER_BACKGROUND);
        } else {
            // A tab attached to a session nobody spoke in (an empty tab given /background): no tab will ever show it again.
            if self.attached {
                if let Some(s) = self.session.clone().filter(|s| sessions::background(s) && !sessions::has_conversation(&self.data, s)) {
                    record::decide("session", "empty session ended", "the tab of a background session with no conversation closed", &[("session", s.clone())]);
                    sessions::stop(&s);
                }
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
        if self.closing { self.leave_at = Some(Instant::now()); return; }
        // Attaching ended at once: nothing runs under that name any more. The conversation is on disk: open it from there.
        if self.attached && self.started.elapsed() < ATTACH_FAILS_WITHIN {
            if let Some(s) = self.session.clone() {
                record::warn("session", "attach failed", "that session is not in the background; reopened from the record", &[("tab", self.tab.clone()), ("session", s.clone())]);
                self.attached = false;
                self.save_tab();
                write_all(1, b"\x1b[2J\x1b[H");
                if self.start(&["--resume".into(), s]).is_ok() { return; }
            }
        }
        self.set_title(wb_core::i18n::t("ended"));
        if let Some(s) = self.session.clone() {
            if !self.attached && !sessions::background(&s) { sessions::mark_off(&self.data, &s); }
        }
    }

    /// Every 2 seconds: which session this tab holds, its name, and whether it carried on under a new id.
    fn tick(&mut self) {
        if self.closing { return; }
        if let Some(found) = sessions::session_of_tab(&self.data, &self.tab) {
            if self.session.as_deref() != Some(&found) && !self.session.as_deref().is_some_and(|s| sessions::current_id(&self.data, s) != s) {
                record::decide("session", "tab session found", "a session record names this tab", &[("tab", self.tab.clone()), ("old", self.session.clone().unwrap_or_default()), ("new", found.clone())]);
                self.session = Some(found);
                self.save_tab();
            }
        }
        if wb_core::codex::active() {
            if self.running {
                if let Some(s) = &self.session {
                    let _ = fs::create_dir_all(sessions::alive_dir(&self.data));
                    let _ = fs::write(sessions::alive_dir(&self.data).join(s), self.child.to_string());
                }
            }
        } else {
            self.resumed_somewhere();
            self.follow_move();
        }
        if !self.running && self.title == wb_core::i18n::t("ended") { return; }
        if let Some(s) = self.session.clone() {
            let name = sessions::tab_title(&self.data, &s);
            self.set_title(if name.is_empty() { wb_core::i18n::t("new-session") } else { &name });
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
        match wb_core::session_record::moved(&self.data.root, &me, &target, empty.as_deref()) {
            Ok(()) => record::decide("session", "number change", "/resume opened a session running in the background", &[("tab", self.tab.clone()), ("old", me), ("new", target), ("empty", empty.unwrap_or_default())]),
            Err(e) => record::error("session", "move not recorded", &[("error", e.to_string())]),
        }
    }

    /// A session that carried on under a new id gets this tab attached to it, in the same place. One tab per
    /// conversation: if another tab already shows it, this one gives way. A copy left behind is stopped.
    fn follow_move(&mut self) {
        let Some(old) = self.session.clone() else { return };
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
            let mut waiting = false;
            if let Some(s) = &self.session {
                let ready = fs::read_to_string(sessions::state_dir(&self.data).join(format!("{s}.json"))).ok()
                    .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                    .is_some_and(|v| v["state"] == "done");
                waiting = !ready;
            }
            waiting
        };
        let dir = sessions::tab_inbox(&self.data, &self.tab);
        for f in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let v = fs::read_to_string(f.path()).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
            if commands_only && !v.as_ref().and_then(|v| v["text"].as_str()).is_some_and(|t| t.trim_start().starts_with('/')) { continue; }
            let _ = fs::remove_file(f.path());
            let Some(v) = v else { continue };
            if !self.running { continue; }
            let text = v.get("text").and_then(|t| t.as_str()).unwrap_or_default().to_string();
            // Sent from outside, the same commands get the same decision as when typed.
            if v.get("enter").and_then(|e| e.as_bool()).unwrap_or(false) {
                if let Some(name) = typed_command(&format!("> {text}")) {
                    if self.session.as_deref().is_some_and(sessions::background) {
                        self.slash(name);
                        continue;
                    }
                }
            }
            // As a paste, so a request of several lines arrives whole instead of each line being sent.
            write_all(self.master, b"\x1b[200~");
            write_all(self.master, text.as_bytes());
            write_all(self.master, b"\x1b[201~");
            if v.get("enter").and_then(|e| e.as_bool()).unwrap_or(false) {
                // Claude Code reads pasted text and Enter apart only with a moment between them.
                std::thread::sleep(Duration::from_millis(150));
                write_all(self.master, b"\r");
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
        if wb_core::codex::active() {
            if let Some(s) = &self.session { sessions::mark_off(&self.data, s); }
        }
        if self.running {
            unsafe { libc::kill(self.child, libc::SIGHUP); }
        }
        let _ = fs::remove_file(sessions::tab_file(&self.data, &self.tab));
        let _ = fs::remove_dir_all(sessions::tab_inbox(&self.data, &self.tab));
    }
}

fn stop_copy(old: &str) {
    if sessions::background(old) {
        record::decide("session", "left-over copy stopped", "the old number runs empty in the background", &[("session", old.to_string())]);
        sessions::stop(old);
    }
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
    let mut args = std::env::args().skip(1);
    let (mut tab, mut attach, mut resume, mut map_from) = (None, None, None, None);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--tab" => tab = args.next(),
            "--attach" => attach = args.next(),
            "--resume" => resume = args.next(),
            "--map-from" => map_from = args.next(),
            other => { let _ = writeln!(std::io::stderr(), "whaybench-term: unknown argument {other}"); std::process::exit(2); }
        }
    }
    let Some(tab) = tab else { let _ = writeln!(std::io::stderr(), "whaybench-term: --tab is needed"); std::process::exit(2) };
    let Some(root) = paths::resolve(None, |k| std::env::var(k).ok()) else { std::process::exit(2) };
    let data = Data::new(root);
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
    let new_id = (attach.is_none() && resume.is_none() && !wb_core::codex::active())
        .then(|| fs::read_to_string("/proc/sys/kernel/random/uuid").ok().map(|u| u.trim().to_string())).flatten();
    let session = attach.clone().or_else(|| resume.clone()).or_else(|| new_id.clone());
    let mut relay = Relay {
        data, tab, session, attached: attach.is_some(), master: -1, child: 0, running: false, started: Instant::now(),
        screen: vt100::Parser::new(40, 120, 0), titles: Titles::default(), title: String::new(),
        leave_at: None, closing: false, map_from, resumed_at: None,
    };
    relay.set_title(wb_core::i18n::t("new-session"));
    let argv: Vec<String> = match (&attach, &resume) {
        (Some(a), _) => vec!["attach".into(), sessions::attach_name(a)],
        (_, Some(r)) => vec!["--resume".into(), r.clone()],
        _ => new_id.map(|id| vec!["--session-id".into(), id]).unwrap_or_default(),
    };
    if let Err(e) = relay.start(&argv) {
        record::error("session", "program not started", &[("error", e.clone())]);
        let _ = writeln!(std::io::stderr(), "whaybench-term: {e}");
        std::process::exit(1);
    }
    relay.save_tab();
    relay.run();
    relay.finish();
}

