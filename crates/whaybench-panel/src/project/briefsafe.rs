//! A Briefsafe project's steps from the panel (log in, log out, snapshot, new), run in the background; the keyring's
//! password they may wait for; its sign-ins; the lines over its pages.
use super::*;

/// A step of `whaybench-project` running in the background for one Briefsafe project.
pub(super) struct Job {
    id: String,
    /// open, recover, close or snapshot.
    kind: String,
    /// What the page says while it runs.
    pub(super) what: String,
    child: std::process::Child,
    started: std::time::Instant,
    /// Its output, in files: a pipe filled up (podman's download progress) would stop it.
    out: std::path::PathBuf,
    err: std::path::PathBuf,
    /// Its arguments and input, to run it again once the keyring is open; what was shown when it began.
    args: Vec<String>,
    input: Option<String>,
    from: String,
}

/// A step that waits for the keyring's password, asked in a dialog.
pub(super) struct Asking { /** What was shown when the step began. */ from: String, id: String, kind: String, what: String, args: Vec<String>, input: Option<String>, new: bool, /** Why the last try did not work. */ problem: Option<String>, /** It was the password. */ wrong: bool }

pub(super) fn stub_of(d: &Data, id: &str) -> Option<wb_core::roots::Stub> { wb_core::roots::stubs(d).into_iter().find(|s| s.id == id) }

/// Runs a step of `whaybench-project` for a Briefsafe project in the background (keyring and podman can take long);
/// `input` goes to its standard input (a recovery key), never on its command line. One at a time: another is said.
pub(super) fn start_job(p: &mut P, id: &str, kind: &str, what: &str, args: &[&str], input: Option<String>) {
    if let Some(j) = &p.job { status::say(&p.d, &wb_core::i18n::f("briefsafe-busy", &[("what", j.what.clone())]), 5); return; }
    // Said when it cannot even start (no program, a full disk), not left as a press that did nothing.
    let fail = |p: &mut P, why: String| {
        // A project not made yet has no page to say it on.
        if kind == "new" { let _ = std::fs::remove_file(new_map_file(&p.d, id)); status::say(&p.d, &wb_core::i18n::f("creation-failed", &[("e", why)]), 12); } else { p.bs_msg.insert(id.into(), (kind.into(), why)); }
    };
    let Ok(exe) = std::env::current_exe() else { return fail(p, "whaybench-project not found".into()) };
    let dir = p.d.cache().join("project-jobs");
    let _ = std::fs::create_dir_all(&dir);
    let (out, err) = (dir.join(format!("{id}.out")), dir.join(format!("{id}.err")));
    let files = std::fs::File::create(&out).and_then(|o| std::fs::File::create(&err).map(|e| (o, e)));
    let (o, e) = match files { Ok(f) => f, Err(e) => return fail(p, e.to_string()) };
    let mut c = std::process::Command::new(exe.with_file_name("whaybench-project"));
    // A session of its own: stopped past its time, what it started (podman, gocryptfs) goes with it, and the terminal
    // Whaybench was started from is not its to write a recovery key on.
    // Safety: setsid is async-signal-safe.
    unsafe { std::os::unix::process::CommandExt::pre_exec(&mut c, || { libc::setsid(); Ok(()) }); }
    c.args(args).env(wb_core::paths::ENV_DATA_DIR, &p.d.root).stdin(std::process::Stdio::piped()).stdout(o).stderr(e);
    match c.spawn() {
        Ok(mut child) => {
            if let Some(mut i) = child.stdin.take() { let _ = std::io::Write::write_all(&mut i, input.as_deref().unwrap_or_default().as_bytes()); }
            let from = shown_at(p);
            p.job = Some(Job { id: id.into(), kind: kind.into(), what: what.into(), child, started: std::time::Instant::now(), out, err, args: args.iter().map(|a| a.to_string()).collect(), input, from });
            // Another project's message stays for it.
            p.bs_msg.remove(id);
            p.stamp.clear();
        }
        Err(e) => fail(p, e.to_string()),
    }
}

/// The command's usual reasons in the person's language (it speaks English, for agents too); any other as it came, with
/// what podman or gocryptfs said in it.
fn for_person(said: &str) -> String {
    let plain = |k: &str| wb_core::i18n::t(k).to_string();
    let with = |k: &str, what: &str| wb_core::i18n::f(k, &[("what", what.trim().to_string())]);
    match said {
        t if t.contains("does not open this project's vault") => plain("failed-recovery-key"),
        t if t.contains("64 hexadecimal digits") => plain("failed-recovery-form"),
        t if t.contains("the keyring did not answer") => plain("failed-keyring-silent"),
        t if t.contains("opens only in its container") => plain("failed-needs-container"),
        t if t.contains("was written by a newer Whaybench") => plain("failed-newer"),
        t if t.contains("podman could not fetch ") => with("failed-image", t.rsplit("podman could not fetch ").next().unwrap_or_default()),
        t if t.contains("container could not be paused") => plain("failed-pause"),
        t if t.contains("vault stays open") => plain("failed-vault-close"),
        t if t.contains(" does not hold project ") => with("failed-no-data", t.split(" does not hold project ").next().unwrap_or_default()),
        t => t.to_string(),
    }
}

/// A finished step: what it said, kept for its project. A step past its time (a keyring that never answers, an image
/// that never comes) is stopped and said so. True when one finished.
pub(super) fn poll_job(p: &mut P) -> bool {
    if p.job.is_none() { next_close(p); }
    let Some(job) = p.job.as_mut() else { return false };
    let limit = match job.kind.as_str() { "open" | "recover" => 600, "new" => 420, _ => 120 };
    let done = matches!(job.child.try_wait(), Ok(Some(_)));
    let late = !done && job.started.elapsed().as_secs() > limit;
    if !done && !late { return false; }
    let Some(mut job) = p.job.take() else { return false };
    if late {
        // Safety: kill on the process group the job leads (made with setsid); its own id is that group's.
        unsafe { libc::kill(-(job.child.id() as i32), libc::SIGKILL) };
        // A snapshot stopped half way would leave the project's container paused.
        // Off the panel's thread: podman may be what is slow.
        if job.kind == "snapshot" && let Some(st) = stub_of(&p.d, &job.id) { std::thread::spawn(move || wb_core::container::unpause(&st)); }
        // A log in stopped half way may leave the vault open behind a locked page: closed again.
        if (job.kind == "open" || job.kind == "recover") && let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe.with_file_name("whaybench-project")).args(["close", &job.id]).env(wb_core::paths::ENV_DATA_DIR, &p.d.root)
                .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
        }
    }
    let status = job.child.wait();
    let ok = !late && status.is_ok_and(|st| st.success());
    let said = for_person(std::fs::read_to_string(&job.err).unwrap_or_default().trim().trim_start_matches("error: "));
    let out: Value = std::fs::read_to_string(&job.out).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let _ = (std::fs::remove_file(&job.out), std::fs::remove_file(&job.err));
    // The keyring's password is asked here, then the step runs again.
    let keyring = !late && !ok && [wb_core::secret::LOCKED, wb_core::secret::NOT_MADE].iter().any(|k| said.contains(k));
    if job.kind == "unlock" {
        if let Some(a) = p.after_unlock.take() {
            if ok { p.ask_keyring = None; p.stamp.clear(); let args: Vec<&str> = a.args.iter().map(String::as_str).collect(); start_job(p, &a.id, &a.kind, &a.what, &args, a.input.clone()); if let Some(j) = &mut p.job { j.from = a.from; } }
            else {
                let wrong = !late && said.contains(wb_core::secret::LOCKED);
                let problem = if wrong { wb_core::i18n::t("keyring-wrong").to_string() } else if late { wb_core::i18n::t("briefsafe-too-long").to_string() } else if said.is_empty() { wb_core::i18n::t("failed-keyring-silent").to_string() } else { said };
                p.ask_keyring = Some(Asking { wrong, problem: Some(problem), ..a });
            }
        }
        return true;
    }
    if keyring {
        p.ask_keyring = Some(Asking { from: job.from, new: said.contains(wb_core::secret::NOT_MADE), id: job.id, kind: job.kind, what: job.what, args: job.args, input: job.input, problem: None, wrong: false });
        p.stamp.clear();
        return true;
    }
    if job.kind == "new" { let _ = std::fs::remove_file(new_map_file(&p.d, &job.id)); }
    // A project that could not be made has no page to say it on: the status line does.
    if job.kind == "new" && (late || !ok) {
        let why = if late { wb_core::i18n::t("briefsafe-too-long").to_string() } else { said.clone() };
        status::say(&p.d, &wb_core::i18n::f("creation-failed", &[("e", why)]), 12);
    }
    if late { p.bs_msg.insert(job.id.clone(), (job.kind.clone(), wb_core::i18n::t("briefsafe-too-long").to_string())); }
    else if !ok { p.bs_msg.insert(job.id.clone(), (job.kind.clone(), said)); }
    else if job.kind == "new" {
        // Made: shown, locked, ready to log in to, unless the person has gone on to something else meanwhile.
        if shown_at(p) == job.from && p.dialog.as_ref().and_then(|w| w.upgrade()).is_none_or(|w| !w.is_visible()) { choose(p, Some(format!("{BS}{}", job.id))); }
        p.rows.clear();
        // What it answered besides: no vault here, a first state not saved, a recovery key that could not be shown.
        let told: Vec<String> = [s(&out, "not_encrypted"), s(&out, "warning"), if out["recovery_key_shown"] == false { s(&out, "recovery_key") } else { "" }].into_iter().filter(|t| !t.is_empty()).map(String::from).collect();
        if told.is_empty() { status::say(&p.d, wb_core::i18n::t("briefsafe-created"), 8); }
        else { let told = told.join(" · "); status::say(&p.d, &told, 30); p.bs_msg.insert(job.id.clone(), ("told".into(), told)); }
    }
    else if job.kind == "snapshot" {
        let warn = s(&out, "warning");
        status::say(&p.d, &if warn.is_empty() { wb_core::i18n::t("briefsafe-snapshot-taken").to_string() } else { wb_core::i18n::f("briefsafe-snapshot-warning", &[("why", warn.to_string())]) }, 8);
    }
    p.stamp.clear();
    next_close(p);
    true
}

/// The next log out waiting, of a project still open (the others go after it, one step at a time).
fn next_close(p: &mut P) {
    while !p.close_after.is_empty() && !stub_of(&p.d, &p.close_after[0]).is_some_and(|st| st.open()) { p.close_after.remove(0); }
    if p.close_after.is_empty() { return; }
    let id = p.close_after.remove(0);
    start_job(p, &id, "close", wb_core::i18n::t("briefsafe-closing"), &["close", &id], None);
}

/// What the last step of this project said, if it did not work.
fn msg_for<'a>(p: &'a P, id: &str) -> Option<(&'a str, &'a str)> { p.bs_msg.get(id).map(|m| (m.0.as_str(), m.1.as_str())) }

/// A step for this project is running, and it hides its content (logging in).
pub(super) fn opening(p: &P, id: &str) -> bool { p.job.as_ref().is_some_and(|j| j.id == id && (j.kind == "open" || j.kind == "recover")) }

/// The page of a Briefsafe project that is not open: locked, a step running, or why the last one did not work.
pub(super) fn closed_page(p: &P, st: &wb_core::roots::Stub) -> String {
    PAGE_OF.with(|k| *k.borrow_mut() = bs_key(&st.id));
    let t = |k: &str| wb_core::i18n::t(k).to_string();
    let body = if let Some(j) = p.job.as_ref().filter(|j| j.id == st.id) {
        format!("<p>{}</p>", e(&wb_core::i18n::f("briefsafe-working", &[("what", j.what.clone())])))
    } else {
        let mut b = String::new();
        match msg_for(p, &st.id) {
            // No key: said in the person's words, with the way on (and still there after a recovery key that did not fit).
            Some((kind, why)) if st.vault && (why.contains(wb_core::vault::NO_KEY) || kind == "recover" || (kind == "open" && why.contains("gocryptfs"))) => {
                // What went wrong, unless it was only the missing key (a gocryptfs that would not start is not that).
                if kind == "recover" || !why.contains(wb_core::vault::NO_KEY) { b += &format!("<p class='warn'>{}</p>", e(&wb_core::i18n::f("briefsafe-failed", &[("why", why.to_string())]))); }
                b += &format!("<p>{}</p><p>{}</p>", e(&t("briefsafe-no-key-page")), link("bs", &format!("{}:recover", st.id), &e(&t("briefsafe-enter-recovery"))));
            }
            Some(("told", why)) => b += &format!("<p class='m'>{}</p>", e(why)),
            Some((_, why)) => b += &format!("<p class='warn'>{}</p>", e(&wb_core::i18n::f("briefsafe-failed", &[("why", why.to_string())]))),
            None => {}
        }
        b + &format!("<p>{}</p>", link("bs", &format!("{}:open", st.id), &e(&t("briefsafe-login"))))
    };
    format!("<!doctype html><meta charset='utf-8'><style>{}</style><body>{body}</body>", style(&p.d))
}

/// Over an open Briefsafe project's pages, in lines of the panel's own (none of the project's text is in them but its
/// name, escaped): its name and when its state was last saved; what can be done with it; a step running, or why the
/// last one did not work.
pub(super) fn set_lines(p: &P, st: Option<&wb_core::roots::Stub>) {
    let Some(st) = st else {
        for l in [&p.actions, &p.agents, &p.note] { l.set_visible(false); }
        p.path.add_css_class("small");
        return;
    };
    let t = |k: &str| e(wb_core::i18n::t(k));
    let set = |l: &gtk::Label, markup: String| {
        l.set_visible(!markup.is_empty());
        if l.label() != markup { l.set_markup(&markup); }
    };
    let a = |what: &str, label: &str| format!("<a href=\"{}\"><span underline='none'>{label}</span></a>", e(&format!("{}:{what}", st.id)));
    let saved = wb_store::folder::last_snapshot(&st.folder).map(|at| {
        let hour: u32 = wb_core::clock::local(at, "%H").parse().unwrap_or(0);
        wb_core::i18n::f("briefsafe-last-saved", &[("half", wb_core::i18n::t(if hour < 12 { "am" } else { "pm" }).to_string()),
            ("time", wb_core::clock::local(at, "%I:%M").trim_start_matches('0').to_string()), ("date", wb_core::clock::local(at, "%m-%d-%Y"))])
    }).unwrap_or_default();
    p.path.remove_css_class("small");
    set(&p.path, format!("<span size='larger' weight='bold'>{}</span>  <span size='smaller' alpha='60%'>{}</span>", e(&st.name), e(&saved)));
    // Claude Code's sign-in is one token for every project; an add-on agent's is the project's own, made in its
    // container. Without a container its agents use the person's own.
    let mut logins = vec![a("token", "Claude")];
    if wb_core::container::opened_isolated(st) {
        for (id, name) in wb_core::agents::list(&p.d.root) {
            if wb_core::agents::load(&p.d.root, &id).is_ok_and(|g| !g.login.is_empty() && g.home_var().is_some()) { logins.push(a(&format!("login:{id}"), &e(&name))); }
        }
    }
    set(&p.actions, format!("{} · {} · {}  {}", a("snapshot", &t("briefsafe-snapshot")), a("close", &t("briefsafe-logout")), t("briefsafe-inner-login"), logins.join(" · ")));
    let mut note = match (p.job.as_ref().filter(|j| j.id == st.id), msg_for(p, &st.id)) {
        (Some(j), _) => e(&wb_core::i18n::f("briefsafe-working", &[("what", j.what.clone())])),
        (None, Some(("told", why))) => e(why),
        (None, Some((_, why))) => e(&wb_core::i18n::f("briefsafe-failed", &[("why", why.to_string())])),
        _ => String::new(),
    };
    if p.unreadable { note = [note, t("briefsafe-unreadable")].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join("\n"); }
    // Opened without a container (no podman or image for this system): said, its agents run as the person.
    if !wb_core::container::opened_isolated(st) { note = [note, t("briefsafe-not-isolated")].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join("\n"); }
    set(&p.note, note);
    // The Sessions tab: a new conversation of each agent, over the list.
    let new = wb_core::agents::list(&p.d.root).into_iter().map(|(id, name)| a(&format!("new:{id}"), &e(if id == wb_core::agents::CLAUDE { "Claude" } else { &name }))).collect::<Vec<_>>().join(" · ");
    set(&p.agents, if p.tab == SESSIONS && p.over.is_none() { new } else { String::new() });
}

pub(super) fn new_map_file(d: &Data, pid: &str) -> std::path::PathBuf { d.cache().join("project-jobs").join(format!("{pid}.map.json")) }

/// Asks the keyring's password for a step that could not go on without it (or a new password, where there is no
/// keyring yet), opens the keyring with it and runs the step again. Nothing of it is kept.
pub(super) fn ask_keyring(s_: &Shared) {
    let (win, a) = { let mut p = s_.borrow_mut(); let Some(a) = p.ask_keyring.take() else { return }; (p.win.clone(), a) };
    let t = |k: &str| wb_core::i18n::t(k);
    let dlg = gtk::Window::builder().title(t(if a.new { "keyring-new-title" } else { "keyring-unlock" })).transient_for(&win).modal(true).decorated(false).build();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    body.set_margin_top(10); body.set_margin_bottom(10); body.set_margin_start(10); body.set_margin_end(10);
    let line = |text: &str| { let l = gtk::Label::new(Some(text)); l.set_xalign(0.0); l.set_wrap(true); l.set_max_width_chars(28); l };
    body.append(&line(t(if a.new { "keyring-new-label" } else { "keyring-label" })));
    let problem = line(a.problem.as_deref().unwrap_or_default());
    problem.add_css_class("warn");
    problem.set_visible(a.problem.is_some());
    let (one, two) = (gtk::PasswordEntry::new(), gtk::PasswordEntry::new());
    one.set_placeholder_text(Some(t("keyring-field")));
    two.set_placeholder_text(Some(t("keyring-field-again")));
    body.append(&one);
    if a.new { body.append(&two); }
    body.append(&problem);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_halign(gtk::Align::End);
    let (cancel, ok) = (gtk::Button::with_label(t("cancel")), gtk::Button::with_label(t(if a.new { "keyring-set" } else { "keyring-open" })));
    // The password is lost: a new keyring, the old one kept beside it.
    let again = (a.from.clone(), a.id.clone(), a.kind.clone(), a.what.clone(), a.args.clone(), a.input.clone());
    let (wrong, new) = (a.wrong, a.new);
    let a = Rc::new(RefCell::new(Some(a)));
    if wrong {
        let anew = gtk::Button::with_label(t("keyring-anew"));
        let (w, s2, win2, held) = (dlg.clone(), s_.clone(), win.clone(), a.clone());
        anew.connect_clicked(move |_| {
            let (w, s2, a2, held) = (w.clone(), s2.clone(), again.clone(), held.clone());
            crate::common::confirm(&win2, wb_core::i18n::t("keyring-anew-confirm"), &[], wb_core::i18n::t("keyring-anew"), move || {
                held.borrow_mut().take();
                w.close();
                let (from, id, kind, what, args, input) = a2.clone();
                { let mut p = s2.borrow_mut(); p.keyring_anew = true; p.ask_keyring = Some(Asking { from, id, kind, what, args, input, new: true, problem: None, wrong: false }); }
                let s3 = s2.clone();
                glib::idle_add_local_once(move || ask_keyring(&s3));
            });
        });
        row.append(&anew);
    }
    row.append(&cancel); row.append(&ok);
    body.append(&row);
    dlg.set_child(Some(&body));
    {
        let w = dlg.clone();
        cancel.connect_clicked(move |_| w.close());
        let (s2, a) = (s_.clone(), a.clone());
        // Closed without a password (Cancel, the title bar's ✕); given one, `a` is taken already.
        dlg.connect_close_request(move |_| {
            // Not now: said on its page (or the status line), and asked again when pressed again.
            if let Some(a) = a.borrow_mut().take() {
                let mut p = s2.borrow_mut();
                p.keyring_anew = false;
                let why = wb_core::i18n::t("keyring-waits").to_string();
                if a.kind == "new" { let _ = std::fs::remove_file(new_map_file(&p.d, &a.id)); status::say(&p.d, &wb_core::i18n::f("creation-failed", &[("e", why)]), 10); }
                else { p.bs_msg.insert(a.id, (a.kind, why)); }
                p.stamp.clear();
            }
            glib::Propagation::Proceed
        });
    }
    let go = {
        let (w, s2, one, two, problem) = (dlg.clone(), s_.clone(), one.clone(), two.clone(), problem.clone());
        Rc::new(move || {
            let password = one.text().to_string();
            if password.is_empty() { return; }
            if new && two.text() != password { problem.set_text(wb_core::i18n::t("keyring-mismatch")); problem.set_visible(true); return; }
            if let Some(what) = s2.borrow().job.as_ref().map(|j| j.what.clone()) { problem.set_text(&wb_core::i18n::f("briefsafe-busy", &[("what", what)])); problem.set_visible(true); return; }
            let Some(a) = a.borrow_mut().take() else { return };
            w.close();
            let mut p = s2.borrow_mut();
            let anew = std::mem::take(&mut p.keyring_anew);
            let (id, what) = (a.id.clone(), wb_core::i18n::t("keyring-unlocking"));
            p.after_unlock = Some(a);
            start_job(&mut p, &id, "unlock", what, if anew { &["unlock-keyring", "--anew"] } else { &["unlock-keyring"] }, Some(password));
        })
    };
    { let g = go.clone(); ok.connect_clicked(move |_| g()); }
    { let g = go.clone(); let two2 = two.clone(); one.connect_activate(move |_| if new { two2.grab_focus(); } else { g() }); }
    two.connect_activate(move |_| go());
    dlg.present();
    one.grab_focus();
}

/// A terminal in the view where the person makes the Claude Code token the projects' conversations sign in with
/// (`claude setup-token`): whaybench-term reads it off the screen and stores it in the keyring, and the terminal
/// closes by itself; where it finds none, the person pastes it (`whaybench token set`). The token is seen there only,
/// never in a conversation.
fn token_terminal(d: &Data) {
    let Ok(exe) = std::env::current_exe() else { return };
    let (claude, whaybench, term) = (sessions::claude_program(), exe.with_file_name("whaybench"), exe.with_file_name("whaybench-term"));
    let q = |p: &std::path::Path| format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"));
    let say = |k: &str| format!("'{}'", wb_core::i18n::t(k).replace('\'', "'\\''"));
    let script = format!("{} --token {} && {{ echo; printf '%s\\n' {}; sleep 3; exit 0; }}; echo; printf '%s\\n' {}; {} token set && printf '%s\\n' {} || printf '%s\\n' {}; printf '%s' {}; read _",
        q(&term), q(&claude), say("briefsafe-token-stored"), say("briefsafe-token-paste"), q(&whaybench), say("briefsafe-token-stored"), say("briefsafe-token-not-stored"), say("briefsafe-token-close"));
    if let Err(e) = wb_core::view::terminal(d, &["sh".into(), "-c".into(), script]) { status::say(d, &wb_core::i18n::f("briefsafe-failed", &[("why", e.to_string())]), 8); }
}

/// Signs an add-on agent in for this project, in its container: a terminal of the view with the agent's own sign-in,
/// the page it asks for in the browser beside it. The login stays in the project.
fn agent_login(d: &Data, id: &str, agent: &str) {
    let (Some(st), Ok(agent)) = (stub_of(d, id).filter(|st| st.open() && wb_core::container::opened_isolated(st)), wb_core::agents::load(&d.root, agent)) else { return };
    // Not the program's own add-on as it came: confirmed where its conversations are, first.
    if !agent.trusted(&d.root) { status::say(d, &wb_core::i18n::f("briefsafe-login-confirm-first", &[("name", agent.name.clone())]), 10); return; }
    let d = d.clone();
    // Off the panel's thread: podman may be slow.
    std::thread::spawn(move || {
        let done = wb_core::container::login_args(&st, &agent).and_then(|args| {
            if let Some(page) = agent.login_page.as_deref().filter(|u| u.starts_with("https://")) { let _ = wb_core::view::open_in_firefox(&d, page, None); }
            let script = format!("podman \"$@\"; echo; printf '%s' '{}'; read _", wb_core::i18n::t("briefsafe-token-close").replace('\'', "'\\''"));
            let argv: Vec<String> = ["sh".to_string(), "-c".into(), script, "sh".into()].into_iter().chain(args).collect();
            wb_core::view::terminal(&d, &argv).map_err(|e| e.to_string())
        });
        if let Err(why) = done { status::say(&d, &wb_core::i18n::f("briefsafe-failed", &[("why", why)]), 10); }
    });
}

/// A Briefsafe project's step from its page: log in or out, a snapshot, a conversation (spec 3.22).
pub(super) fn briefsafe_clicked(s_: &Shared, what: &str) {
    let (d, win, id) = { let p = s_.borrow(); (p.d.clone(), p.win.clone(), p.bs.clone()) };
    let Some(id) = id else { return };
    // A link names the project its page was drawn for: pressed on a page left from another project, it does nothing.
    let Some(what) = what.strip_prefix(&format!("{id}:")) else { return };
    let t = |k: &str| wb_core::i18n::t(k).to_string();
    match what {
        "open" => start_job(&mut s_.borrow_mut(), &id, "open", &t("briefsafe-opening"), &["open", &id], None),
        "recover" => {
            let s2 = s_.clone();
            let id2 = id.clone();
            ask_text(&win, &t("briefsafe-enter-recovery"), &t("briefsafe-recovery-label"), move |key| {
                start_job(&mut s2.borrow_mut(), &id2, "recover", wb_core::i18n::t("briefsafe-opening"), &["open", &id2, "--recovery"], Some(format!("{key}\n")));
            });
        }
        "snapshot" => {
            if s_.borrow().job.is_none() { status::say(&d, wb_core::i18n::t("briefsafe-snapshotting"), 6); }
            start_job(&mut s_.borrow_mut(), &id, "snapshot", &t("briefsafe-snapshotting"), &["snapshot", &id], None);
        }
        "close" => {
            // Its conversations close with it: said first when some are open.
            let tabs = sessions::open_tabs(&d).iter().filter(|tab| sessions::tab_project(&d, &tab.tab).as_deref() == Some(id.as_str())).count();
            let s2 = s_.clone();
            let id2 = id.clone();
            let go = move || {
                let mut p = s2.borrow_mut();
                // Another step running (a snapshot): done once it ends, said now.
                if p.job.is_some() { if !p.close_after.contains(&id2) { p.close_after.push(id2.clone()); } let what = p.job.as_ref().map(|j| j.what.clone()).unwrap_or_default(); status::say(&p.d, &wb_core::i18n::f("briefsafe-logout-queued", &[("what", what)]), 6); return; }
                start_job(&mut p, &id2, "close", wb_core::i18n::t("briefsafe-closing"), &["close", &id2], None);
            };
            if tabs == 0 { go(); } else { crate::common::confirm(&win, &wb_core::i18n::f("briefsafe-logout-confirm", &[("n", tabs.to_string())]), &[], &t("briefsafe-logout"), go); }
        }
        "token" => token_terminal(&d),
        other => {
            // A step of this project (its state being saved holds its container still): after it.
            let busy = s_.borrow().job.as_ref().filter(|j| j.id == id).map(|j| j.what.clone());
            if let Some(what) = busy { status::say(&d, &wb_core::i18n::f("briefsafe-busy", &[("what", what)]), 5); return; }
            if let Some(agent) = other.strip_prefix("new:") {
                crate::common::ctl(&d, vec!["--data-dir".into(), d.root.to_string_lossy().to_string(), "session".into(), "new".into(), "--project".into(), id, "--agent".into(), agent.to_string()]);
            } else if let Some(agent) = other.strip_prefix("login:") {
                agent_login(&d, &id, agent);
            }
        }
    }
}
