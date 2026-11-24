//! A Briefsafe project's own browser (spec 3.22): a Firefox that runs in the project's container, its profile in the
//! project's data, its window on the display Whaybench gives that project. What it shows is read in there, over a
//! session bus of its own: the host gets the window's picture, never the page's text.
use std::{
    os::unix::{fs::DirBuilderExt, net::UnixStream, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use crate::{container, i18n, paths::Data, view};

/// Whaybench's display for this project, if this program runs in a container that has one.
fn display() -> Option<PathBuf> {
    let dir = Path::new(container::DISPLAY_IN_CONTAINER);
    dir.is_dir().then(|| dir.join(container::DISPLAY_SOCKET))
}

fn runtime() -> PathBuf { PathBuf::from("/tmp/whaybench-browser") }
fn bus() -> PathBuf { runtime().join("bus") }

/// Whether this program is where a project's own browser can be; why not, in words for who asked. Its bus is the one
/// this process, and what it starts, use from here on.
pub fn enter() -> Result<PathBuf, String> {
    let socket = display().ok_or_else(|| i18n::t(if Path::new("/run/.containerenv").exists() { "briefsafe-browser-no-display" } else { "briefsafe-browser-no-container" }).to_string())?;
    // Safety: called first thing in the command, before any other thread runs.
    unsafe {
        std::env::set_var("XDG_RUNTIME_DIR", runtime());
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", bus().display()));
    }
    Ok(socket)
}

/// The browser's session bus, started when there is none: one program at a time.
fn start_bus() -> Result<(), String> {
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(runtime()).map_err(|e| format!("{}: {e}", runtime().display()))?;
    container::flocked(&runtime().join("lock"), || {
        if UnixStream::connect(bus()).is_ok() { return Ok(()); }
        let _ = std::fs::remove_file(bus());
        let out = Command::new("dbus-daemon").args(["--session", "--fork", "--nopidfile", &format!("--address=unix:path={}", bus().display())])
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).output().map_err(|e| format!("dbus-daemon: {e}"))?;
        if out.status.success() { Ok(()) } else { Err(format!("dbus-daemon: {}", String::from_utf8_lossy(&out.stderr).trim())) }
    })
}

/// The page in a new tab of the project's browser, which starts when it does not run; without a page, it only starts.
pub fn open(d: &Data, page: Option<&str>) -> Result<(), String> {
    let socket = enter()?;
    UnixStream::connect(&socket).map_err(|e| match e.kind() {
        // A sandbox of the agent's: said as it is, not as a window that is gone.
        std::io::ErrorKind::PermissionDenied => format!("{}: {e}", socket.display()),
        _ => i18n::t("briefsafe-browser-no-window").to_string(),
    })?;
    start_bus()?;
    // Its window may be closed and the program still ending: waited for, or this would show nothing. (Not under the
    // lock: an agent's page does not wait for it.)
    let ending = if page.is_none() { view::firefox_pid(d).filter(|_| !view::firefox_gone(d, Duration::from_secs(15))) } else { None };
    // One at a time, until Firefox is seen: two opens at once would start two on one profile.
    container::flocked(&runtime().join("open.lock"), || {
        let pid = view::firefox_pid(d);
        let runs = pid.is_some();
        if runs && page.is_none() { return if pid == ending { Err(i18n::t("briefsafe-browser-ending").to_string()) } else { Ok(()) }; }
        view::prepare_profile(d);
        let mut c = Command::new("firefox");
        c.arg("--profile").arg(view::profile(d));
        if runs { c.arg("--new-tab"); }
        c.args(page).env("WAYLAND_DISPLAY", &socket)
            .env("MOZ_ENABLE_WAYLAND", "1").env("GNOME_ACCESSIBILITY", "1")
            // Firefox's sandbox would change root in a namespace of its own, which the container does not allow (it
            // drops every capability) and which ends its pages: it goes on without that layer, its system call filter
            // kept.
            .env("MOZ_ASSUME_USER_NS", "0")
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        // It outlives the command that started it (an agent's shell ends with its group).
        // Safety: setsid is async-signal-safe.
        unsafe { c.pre_exec(|| { libc::setsid(); Ok(()) }); }
        let mut child = c.spawn().map_err(|e| format!("firefox: {e}"))?;
        // One that cannot run (a snap's, which is not in here) ends at once: said, not left as a page that never comes.
        let (started, least, most) = (Instant::now(), Duration::from_millis(1200), Duration::from_secs(15));
        while started.elapsed() < most && (started.elapsed() < least || view::firefox_pid(d).is_none()) {
            match child.try_wait() {
                Ok(Some(st)) if st.success() => break,
                Ok(Some(st)) => return Err(format!("firefox: {st}")),
                _ => std::thread::sleep(Duration::from_millis(100)),
            }
        }
        if view::firefox_pid(d).is_none() { return Err("firefox: not running after 15 s".into()); }
        Ok(())
    })
}

/// Asks the project's browser to end, so it writes its tabs, and waits for it (six seconds at most).
pub fn end(d: &Data) {
    let Some(pid) = view::firefox_pid(d) else { return };
    // Safety: a signal to a process of this user's own.
    unsafe { libc::kill(pid as i32, libc::SIGTERM); }
    let until = Instant::now() + Duration::from_secs(6);
    while view::firefox_pid(d).is_some() && Instant::now() < until { std::thread::sleep(Duration::from_millis(100)); }
}

/// A picture of the project's browser, taken by Whaybench of that window only and left in the project's data.
pub fn shot(d: &Data) -> Result<PathBuf, String> {
    enter()?;
    let file = d.cache().join(format!("view-shot-{}.png", std::process::id()));
    let error = { let mut s = file.clone().into_os_string(); s.push(".error"); PathBuf::from(s) };
    // One that came too late for an earlier command goes too (a minute later: another command may be waiting for its).
    let old = |f: &Path| crate::paths::meta(f).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|a| a.as_secs() > 60);
    for f in crate::paths::list_dir(&d.cache()) {
        if f == file || f == error || f.file_name().is_some_and(|n| n.to_string_lossy().starts_with("view-shot-")) && old(&f) { let _ = std::fs::remove_file(f); }
    }
    crate::status::post(d, &serde_json::json!({"cmd": "shot", "out": file, "panel": null, "app": view::FIREFOX_APP})).map_err(|e| e.to_string())?;
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if file.exists() { return Ok(file); }
        if let Ok(e) = std::fs::read_to_string(&error) { let _ = std::fs::remove_file(&error); return Err(e); }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(i18n::t("briefsafe-browser-no-window").to_string())
}
