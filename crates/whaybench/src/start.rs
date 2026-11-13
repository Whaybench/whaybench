//! What happens before the window: the header of the record, the check of what the program needs,
//! and the way to the person's desktop. A missing part is told, never fatal, except
//! having no desktop to open a window on.
use std::{ffi::OsStr, path::{Path, PathBuf}, process::Command, sync::atomic::{AtomicBool, Ordering}, time::Duration};

use serde::{Deserialize, Serialize};
use wb_core::{log as record, paths, versions};

/// Components without which part of the product does not work, and what to do.
const NEEDED: [(&str, &str); 3] = [("Alacritty", "install the alacritty package"), ("wl-clipboard", "install wl-clipboard for the clipboard with the desktop"), ("GTK", "install the gtk4 development package")];

/// The commands a session uses, where every session finds them: one in the background keeps the environment of the
/// day it started, whose PATH may not have this build's folder. A file of another origin stays as it is.
fn link_commands() {
    let (Some(home), Some(bins)) = (std::env::var_os("HOME"), paths::own_exe().ok()) else { return };
    let Some(bins) = bins.parent().map(PathBuf::from) else { return };
    let dir = PathBuf::from(home).join(".local/bin");
    let _ = std::fs::create_dir_all(&dir);
    for name in ["whaybench", "whaybench-view", "whaybench-project", "whaybench-files"] {
        let (link, target) = (dir.join(name), bins.join(name));
        if !target.exists() { continue; }
        let ours = std::fs::symlink_metadata(&link).is_ok_and(|m| m.file_type().is_symlink());
        if ours { let _ = std::fs::remove_file(&link); }
        if ours || !link.exists() {
            if let Err(e) = std::os::unix::fs::symlink(&target, &link) { record::warn("run", "command not linked", &e.to_string(), &[("command", name.to_string())]); }
        }
    }
}

pub fn prepare() -> Result<(), String> {
    link_commands();
    let runtime = std::env::var("XDG_RUNTIME_DIR").map(PathBuf::from).map_err(|_| "XDG_RUNTIME_DIR is not set: start from a desktop session".to_string())?;
    let env = std::env::var("WAYLAND_DISPLAY").ok();
    let host = versions::host_socket(env.as_deref(), &runtime, versions::desktop_shell_socket);
    record::header(&format!("environment · user={} desktop={} session_type={} input_method={}", var("USER"), var("XDG_CURRENT_DESKTOP"), var("XDG_SESSION_TYPE"), input_method()));

    let host = host.ok_or("no desktop Wayland socket found: start from a Wayland desktop session")?;
    // winit reads this when it opens the window. Set before any thread starts reading the environment.
    unsafe { std::env::set_var("WAYLAND_DISPLAY", &host) };
    record::decide("run", "display connected", "the user's desktop socket", &[("socket", host)]);

    let (early, rest) = versions::read_components(Duration::from_millis(500), Duration::from_secs(20));
    let mut version = vec![("Whaybench".to_string(), format!("{} ({})", env!("CARGO_PKG_VERSION"), env!("WB_COMMIT"))), ("Rust".to_string(), env!("WB_RUSTC").to_string()), ("Smithay".to_string(), env!("WB_SMITHAY").to_string())];
    version.extend(early);
    record::header(&format!("versions · {}", fields(&version)));
    record::header(&format!("settings · detail={}", record::value(&std::env::var(record::ENV_TRACE).unwrap_or_default())));
    record::header("open before · none");

    // The comparison and the check need every version; slow programs must not hold up the window.
    std::thread::spawn(move || {
        let late = rest();
        if !late.is_empty() { record::header(&format!("versions+ · {}", fields(&late))); }
        version.extend(late);
        let release = versions::parse_release(include_str!(concat!(env!("OUT_DIR"), "/release.toml")));
        for (name, tested, now) in versions::differences(&release, &version) {
            record::warn("run", "component version differs", "differs from the release record; running on", &[("name", name), ("tested", tested), ("now", now)]);
        }
        check_needed(&version);
    });
    Ok(())
}

fn fields(v: &[(String, String)]) -> String {
    v.iter().map(|(k, x)| format!("{k}={}", record::value(x))).collect::<Vec<_>>().join(" ")
}
fn var(k: &str) -> String { std::env::var(k).unwrap_or_default() }

/// Whether fcitx5 inside Whaybench is the input method: when it is installed and settings.json does not say
/// `"input_method": "host"`.
pub fn builtin_input_method(data: &paths::Data) -> bool {
    let host = std::fs::read_to_string(data.root.join("settings.json")).ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .is_some_and(|v| v["input_method"] == "host");
    let fcitx = Command::new("fcitx5").arg("--version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
    record::decide("input", "input method", if host { "settings.json says host" } else if fcitx { "fcitx5 is installed" } else { "no fcitx5" }, &[("built_in", (fcitx && !host).to_string())]);
    fcitx && !host
}

/// Asks the running fcitx5 to attach to this display too; with none running (a WSL session has none), starts one on
/// it that keeps running after it. Off the main thread: fcitx5 connects back, which needs the event loop running.
pub fn attach_input_method(socket: &OsStr) {
    let path = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_default().join(socket);
    std::thread::spawn(move || {
        let quiet = || std::process::Stdio::null();
        let asked = Command::new("dbus-send").args(["--session", "--print-reply", "--dest=org.fcitx.Fcitx5", "/controller", "org.fcitx.Fcitx.Controller1.OpenWaylandConnection", &format!("string:{}", path.display())])
            .stdout(quiet()).stderr(quiet()).status().is_ok_and(|s| s.success());
        if asked { return record::decide("input", "fcitx5 attached", "the running fcitx5 opened this display", &[]); }
        // -k: it outlives this display, so the next Whaybench (a restart starts it before this one ends) finds it.
        // Without it fcitx5 exits with its first display and the new one is left attached to nothing.
        let started = Command::new("fcitx5").args(["-d", "-k"]).env("WAYLAND_DISPLAY", &path).stdout(quiet()).stderr(quiet()).status().is_ok_and(|s| s.success());
        if started { record::decide("input", "fcitx5 started", "none was running", &[]); } else { record::warn("input", "no input method", "fcitx5 did not start; Korean and other composed input will not work", &[]); }
    });
}

fn input_method() -> String {
    ["XMODIFIERS", "GTK_IM_MODULE", "QT_IM_MODULE"].iter().map(|k| var(k)).find(|v| !v.is_empty()).unwrap_or_default()
}

fn check_needed(parts: &[(String, String)]) {
    for (name, advice) in NEEDED {
        let now = parts.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());
        if now == Some(versions::MISSING) {
            record::warn("run", "component missing", advice, &[("name", name.to_string())]);
        }
    }
}

/// What a new conversation tab starts with.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct TermOpen { pub attach: Option<String>, pub resume: Option<String>, pub map_from: Option<String>, #[serde(default)] pub agent: Option<String> }

fn new_tab_id() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{:012x}", (nanos ^ ((std::process::id() as u128) << 40)) & 0xffff_ffff_ffff)
}

/// The panel app id of a conversation tab carries its tab id, so the panel knows which tab it is.
pub const TERM_APP: &str = "whaybench-term";

/// The command for a conversation tab: Alacritty showing the relay, the relay holding Claude Code.
pub fn terminal_command(open: &TermOpen, data: &paths::Data) -> Result<String, String> {
    let config = wb_core::theme::write_terminal_config(data).map_err(|e| e.to_string())?;
    let relay = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name("whaybench-term");
    let tab = new_tab_id();
    // The terminal knows its tab too: a link opened from it goes to the views of this tab's session.
    let mut command = format!("exec env WHAYBENCH_TAB={tab} {} alacritty --class {TERM_APP}-{tab} --config-file '{}' -e '{}' --tab {tab}", agent_env(open, data)?, config.display(), relay.display());
    if let Some(a) = &open.attach { command += &format!(" --attach {a}"); }
    if let Some(r) = &open.resume { command += &format!(" --resume {r}"); }
    if let Some(m) = &open.map_from { command += &format!(" --map-from '{m}'"); }
    Ok(command)
}

/// The tab's agent, as variables for the relay and all it starts: the one asked for, else the agent of the session it
/// opens or takes its map from, else Claude Code. An add-on's settings folder is made ready here.
fn agent_env(open: &TermOpen, data: &paths::Data) -> Result<String, String> {
    let of_session = || [&open.attach, &open.resume, &open.map_from].into_iter().flatten()
        .find_map(|s| wb_core::sessions::record_of(data, s)?.get("agent")?.as_str().map(String::from));
    let id = open.agent.clone().or_else(of_session).unwrap_or_else(|| wb_core::agents::CLAUDE.into());
    if id == wb_core::agents::CLAUDE { return Ok(format!("{}={id}", wb_core::agents::ENV_AGENT)); }
    let agent = wb_core::agents::load(&data.root, &id).map_err(|e| e.to_string())?;
    // This process runs for days and updates replace its file: the hooks must name the path, not the old file.
    let exe = paths::own_exe().map_err(|e| e.to_string())?;
    let mut env = format!("{}={id}", wb_core::agents::ENV_AGENT);
    if let Some((var, folder)) = agent.prepare(&data.root, &exe).map_err(|e| e.to_string())? {
        env += &format!(" {var}={}", wb_core::agents::shell_quote(&folder).map_err(|e| e.to_string())?);
    }
    if let Some(command) = &agent.trust { wb_core::status::say(data, &wb_core::i18n::f("agent-hooks", &[("name", agent.name.clone()), ("command", command.clone())]), 8); }
    Ok(env)
}

/// An app that belongs in a panel: it gets our socket, the data directory and the run.
/// The apps this run started, waited for when they end so none is left as a zombie.
static APPS: std::sync::Mutex<Vec<std::process::Child>> = std::sync::Mutex::new(Vec::new());

/// Called often: the apps that have ended are waited for.
pub fn reap_apps() {
    APPS.lock().unwrap().retain_mut(|c| !matches!(c.try_wait(), Ok(Some(_))));
    // SAFETY: waitpid on one known child with WNOHANG only reaps it if it has ended.
    LEFT.lock().unwrap().retain(|pid| unsafe { libc::waitpid(*pid, std::ptr::null_mut(), libc::WNOHANG) } == 0);
}

/// At the very start: a restart keeps the process, so the apps the run before started are its children still. Those
/// that ended are reaped now; the others are remembered and reaped when they end (reap_apps).
pub fn reap_left_over() {
    let me = std::process::id().to_string();
    let left: Vec<i32> = std::fs::read_dir("/proc").into_iter().flatten().flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<i32>().ok())
        .filter(|pid| std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()
            .and_then(|t| t.rsplit_once(") ").map(|(_, rest)| rest.split(' ').nth(1) == Some(me.as_str()))).unwrap_or(false))
        .collect();
    *LEFT.lock().unwrap() = left;
    reap_apps();
}

/// Children of the run before a restart, not yet ended.
static LEFT: std::sync::Mutex<Vec<i32>> = std::sync::Mutex::new(Vec::new());

/// The apps inside open links and files with xdg-open (a terminal's link, Claude Code's login page). Theirs goes to
/// the view panel: a browser started from inside would otherwise hand the address to one already
/// running on the desktop and show it there. Returns the folder to put first in PATH, with its colon.
fn open_shim(data: &paths::Data, bins: &str) -> String {
    let dir = data.cache().join("bin");
    let script = format!("#!/bin/sh\n# Whaybench: what the apps inside open goes to the view panel.\nexec '{bins}/whaybench-view' open \"$@\"\n");
    let file = dir.join("xdg-open");
    if std::fs::read_to_string(&file).ok().as_deref() != Some(script.as_str()) {
        use std::os::unix::fs::PermissionsExt;
        let written = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&file, &script))
            .and_then(|_| std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)));
        if let Err(e) = written { record::warn("screen", "open shim not made", &e.to_string(), &[]); return String::new(); }
    }
    format!("{}:", dir.display())
}

/// Set once the window's GPU is offered to the apps through linux-dmabuf.
pub static GPU_APPS: AtomicBool = AtomicBool::new(false);

pub fn spawn_app(command: &str, socket: &OsStr, data: &paths::Data, run: &str) {
    // The program's own commands (whaybench-project and the like) are found by the apps and by Claude Code.
    let bins = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())).map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
    let shims = open_shim(data, &bins);
    let path = format!("{shims}{bins}:{}", std::env::var("PATH").unwrap_or_default());
    let umask = wb_core::paths::user_umask();
    let mut sh = Command::new("sh");
    // The apps write in the person's folders: their umask, not Whaybench's private one.
    // SAFETY: umask is async-signal-safe.
    unsafe { std::os::unix::process::CommandExt::pre_exec(&mut sh, move || { libc::umask(umask); Ok(()) }); }
    let started = sh.arg("-c").arg(command)
        .env("PATH", path)
        .env("WAYLAND_DISPLAY", socket)
        // Inside, Korean and other input methods come through this compositor's text input; a GTK or Qt module of
        // the desktop's input method would talk past it and the apps would get no composed text.
        .env_remove("GTK_IM_MODULE").env_remove("QT_IM_MODULE")
        // Without linux-dmabuf (no render node, as under WSLg) Mesa would look for a GPU device, warn and fall back
        // to drawing on the CPU anyway, so it is told to from the start.
        .envs((!GPU_APPS.load(Ordering::Relaxed)).then_some(("LIBGL_ALWAYS_SOFTWARE", "1")))
        .env(paths::ENV_DATA_DIR, Path::new(&data.root))
        .env(record::ENV_RUN, run)
        .spawn();
    match started {
        Ok(c) => {
            record::decide("screen", "app started", "a requested app", &[("command", command.to_string()), ("process", c.id().to_string())]);
            APPS.lock().unwrap().push(c);
        }
        Err(e) => record::error("screen", "app not started", &[("command", command.to_string()), ("error", e.to_string())]),
    }
}

#[cfg(test)]
mod agent_tests {
    use super::*;

    #[test]
    fn a_tab_runs_the_agent_asked_for_else_its_sessions_else_claude_code() {
        let dir = tempfile::tempdir().unwrap();
        let data = paths::Data::new(dir.path().to_path_buf());
        wb_core::addons::install_bundled(&data);
        std::fs::create_dir_all(dir.path().join("sessions")).unwrap();
        std::fs::write(dir.path().join("sessions/cx.json"), r#"{"id":"cx","agent":"codex"}"#).unwrap();
        std::fs::write(dir.path().join("sessions/cl.json"), r#"{"id":"cl"}"#).unwrap();
        let env = |open: TermOpen| agent_env(&open, &data).unwrap();
        assert_eq!(env(TermOpen::default()), "WHAYBENCH_AGENT=claude");
        assert_eq!(env(TermOpen { resume: Some("cl".into()), ..Default::default() }), "WHAYBENCH_AGENT=claude");
        let codex = env(TermOpen { resume: Some("cx".into()), ..Default::default() });
        assert!(codex.starts_with("WHAYBENCH_AGENT=codex CODEX_HOME="), "{codex}");
        assert!(env(TermOpen { agent: Some("codex".into()), ..Default::default() }).starts_with("WHAYBENCH_AGENT=codex "));
        let hooks = std::fs::read_to_string(dir.path().join("codex/hooks.json")).unwrap();
        assert!(!hooks.contains("(deleted)"), "the hooks name the program's path");
        // Claude Code asked for, on a Codex session's map: Claude Code.
        assert_eq!(env(TermOpen { map_from: Some("cx".into()), agent: Some("claude".into()), ..Default::default() }), "WHAYBENCH_AGENT=claude");
        std::fs::remove_dir_all(dir.path().join("addons/agents/codex")).unwrap();
        assert!(agent_env(&TermOpen { resume: Some("cx".into()), ..Default::default() }, &data).is_err(), "a deleted add-on is said, not run as Claude Code");
    }
}
