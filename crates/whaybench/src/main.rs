//! Whaybench: one window, one surface, panels on it. Each panel is an app of its own, shown and
//! operated only inside its area, which this nested Wayland compositor gives it.
mod ctl;
mod handlers;
mod input;
mod layout;
mod mcp;
mod render;
mod agent;
mod chrome;
mod clipboard;
mod start;
mod state;

use std::path::PathBuf;

use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
pub use state::App;
use wb_core::{log as record, paths};

const USAGE: &str = "whaybench [--data-dir DIR] [--agent codex] [--app COMMAND]... [--no-session]    the agent is Claude Code unless given
whaybench map [--data-dir DIR] SESSION [--write < CHANGE.json | --take-over < MAP.json | --start-from < MAP.json | --end-turn] | --file MAP.json    read, change or take over a conversation map, count a finished turn, or read a saved map
whaybench memory [--data-dir DIR] list SESSION | changes SESSION | show MEMID|SESSION/NODE | search WORD...    what the memory holds: a session's memory list, what changed since it last looked, one node, a search
whaybench record [--data-dir DIR] SESSION touch|state|alive|off|answer|moved|carry-from ...    a session's record, state, heartbeat, answers
whaybench hook [--data-dir DIR] claude|codex < EVENT.json    run by the agent's hooks
whaybench mcp [--data-dir DIR]    the conversation map tool over MCP (stdio), for an agent in a Whaybench tab
whaybench statusline [--data-dir DIR] < STATUS.json    a Claude Code status line: context and plan usage, kept for the notifications panel
whaybench ctl [--data-dir DIR] session new [MAP-FROM] | show ID | type ID TEXT | end ID
whaybench ctl [--data-dir DIR] quit
whaybench ctl [--data-dir DIR] shot --out FILE.png [--panel N]    N: panel number, default the whole surface
whaybench ctl [--data-dir DIR] theme NAME    default or a theme in addons/themes of the data directory
whaybench ctl [--data-dir DIR] pointer X Y | press | release | panels FILE    for tests
whaybench --version";

struct Args {
    data_dir: Option<PathBuf>,
    apps: Vec<String>,
    no_session: bool,
    codex: bool,
    ctl: Option<Vec<String>>,
}

fn parse_args(mut args: Vec<String>) -> Result<Args, String> {
    let mut out = Args { data_dir: None, apps: Vec::new(), no_session: false, codex: false, ctl: None };
    // ctl, map, record and hook take the words after them as their own; map, record and hook stay as the first of them.
    if args.first().is_some_and(|a| matches!(a.as_str(), "ctl" | "map" | "record" | "hook" | "memory" | "statusline" | "mcp")) {
        if args[0] == "ctl" { args.remove(0); }
        out.ctl = Some(Vec::new());
    }
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--data-dir" => out.data_dir = Some(it.next().ok_or("--data-dir needs a folder")?.into()),
            "--app" if out.ctl.is_none() => out.apps.push(it.next().ok_or("--app needs a command")?),
            "--no-session" if out.ctl.is_none() => out.no_session = true,
            "--agent" if out.ctl.is_none() => match it.next().as_deref() {
                Some("codex") => out.codex = true,
                Some("claude") => out.codex = false,
                _ => return Err("--agent takes claude or codex".into()),
            },
            _ => match &mut out.ctl {
                Some(rest) => rest.push(a),
                None => return Err(format!("unknown argument: {a}")),
            },
        }
    }
    Ok(out)
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.iter().any(|a| a == "--version") {
        println!("whaybench {} ({})", env!("CARGO_PKG_VERSION"), env!("WB_COMMIT"));
        return;
    }
    if raw.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return;
    }
    let args = parse_args(raw).unwrap_or_else(|e| fail(&format!("{e}\n{USAGE}")));
    let root = paths::resolve(args.data_dir.as_deref(), |k| std::env::var(k).ok())
        .unwrap_or_else(|| fail("no data directory: give --data-dir or set HOME"));
    let data = paths::Data::new(root);

    if let Some(rest) = args.ctl {
        let done = match rest.first().map(String::as_str) {
            Some("map") => agent::map(&data, &rest[1..]),
            Some("record") => agent::record(&data, &rest[1..]),
            Some("hook") => agent::hook(&data, &rest[1..]),
            Some("memory") => agent::memory(&data, &rest[1..]),
            Some("statusline") => agent::statusline(&data),
            Some("mcp") => mcp::serve(&data),
            _ => ctl::send(&data, &rest),
        };
        if let Err(e) = done { fail(&e); }
        return;
    }
    if args.codex { agent::start_codex(&data).unwrap_or_else(|e| fail(&e)); }
    if let Err(e) = run(data, args.apps, args.no_session) { fail(&e); }
}

fn fail(message: &str) -> ! {
    eprintln!("whaybench: {message}");
    std::process::exit(1)
}

fn run(data: paths::Data, apps: Vec<String>, no_session: bool) -> Result<(), String> {
    std::fs::create_dir_all(data.cache()).map_err(|e| format!("cannot use {}: {e}", data.root.display()))?;
    let _ = std::fs::create_dir_all(data.maps());
    // Conversations, answers and the view panel's sign-ins are in here: only this account may read it.
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&data.root, std::fs::Permissions::from_mode(0o700));
    }
    start::reap_left_over();
    let began = std::time::Instant::now();
    // The record comes first, so a failure in anything below is in it.
    let run_id = record::run_id(&data);
    // Everything started from here belongs to this run. No other thread exists yet.
    unsafe { std::env::set_var(record::ENV_RUN, &run_id) };
    // The program's own texts are found through this too.
    unsafe { std::env::set_var(paths::ENV_DATA_DIR, &data.root) };
    record::open(&data, "whaybench", &run_id, if std::env::var_os(ENV_AGAIN).is_some() { "again" } else { "first" });
    record::install_panic_hook();
    wb_core::sessions::save_lost_maps(&data);
    wb_core::sessions::prune(&data);
    wb_core::map::empty_old_trash(&data.root);

    let again = std::env::args().skip(1).collect::<Vec<_>>();
    let result = start::prepare().and_then(|_| serve(&data, &run_id, apps, no_session));
    match &result {
        Ok(how) => record::decide("run", "ended", "the program ended", &[("took", format!("{}s", began.elapsed().as_secs())), ("how", how.clone())]),
        Err(e) => record::error("run", "start failed", &[("error", e.clone())]),
    }
    if result.as_deref() == Ok(state::RESTART) { start_again(&data, again); }
    result.map(|_| ())
}

/// Becomes the program again, the newest build if it was replaced; on failure the program just ends.
fn start_again(data: &paths::Data, args: Vec<String>) {
    use std::os::unix::process::CommandExt;
    // The panel's Firefox ends with the window; the new one must find its profile free.
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while wb_core::view::firefox_pid(data).is_some() && std::time::Instant::now() < until { std::thread::sleep(std::time::Duration::from_millis(100)); }
    if let Some(pid) = wb_core::view::firefox_pid(data) { let _ = std::process::Command::new("kill").arg(pid.to_string()).status(); }
    let exe = std::env::current_exe().map(|p| std::path::PathBuf::from(p.to_string_lossy().trim_end_matches(" (deleted)"))).unwrap_or_else(|_| "whaybench".into());
    let e = std::process::Command::new(exe).args(args).env_remove(record::ENV_RUN).env(ENV_AGAIN, "1").exec();
    record::error("run", "restart failed", &[("error", e.to_string())]);
}

const ENV_AGAIN: &str = "WHAYBENCH_AGAIN";

fn serve(data: &paths::Data, run_id: &str, apps: Vec<String>, no_session: bool) -> Result<String, String> {
    let mut event_loop: EventLoop<App> = EventLoop::try_new().map_err(|e| e.to_string())?;
    let display: Display<App> = Display::new().map_err(|e| e.to_string())?;
    wb_core::addons::install_bundled(data);
    let saved = state::read_layout(data);
    let mut state = App::new(&mut event_loop, display, data.clone(), saved);

    render::init_window(&mut event_loop, &mut state).map_err(|e| e.to_string())?;
    ctl::listen(&event_loop, data).map_err(|e| e.to_string())?;
    ctl::write_instance(data, run_id, &state.socket_name.to_string_lossy()).map_err(|e| e.to_string())?;
    record::decide("run", "start", "the program started", &[("run", run_id.to_string()), ("reason", if std::env::var_os(ENV_AGAIN).is_some() { "again" } else { "first" }.into()), ("socket", state.socket_name.to_string_lossy().to_string())]);

    // Only the apps get our socket; this process stays connected to the desktop.
    for app in &apps {
        start::spawn_app(app, &state.socket_name, data, run_id);
    }
    if !no_session {
        for panel in ["menu", "status", "map", "sessions", "files", "answer", "input", "project", "notify"] {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name("whaybench-panel");
            start::spawn_app(&format!("exec '{}' {panel}", exe.display()), &state.socket_name, data, run_id);
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name("whaybench-panel");
        // Branch and imported map panels of last time.
        let kept: Vec<Vec<String>> = std::fs::read_to_string(data.root.join("map-panels.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        for args in kept {
            let quoted: Vec<String> = args.iter().map(|a| format!("'{}'", a.replace('\'', "'\\''"))).collect();
            start::spawn_app(&format!("exec '{}' map {}", exe.display(), quoted.join(" ")), &state.socket_name, data, run_id);
        }
        for (name, _) in wb_core::view::views(data) {
            start::spawn_app(&format!("exec '{}' view '{name}'", exe.display()), &state.socket_name, data, run_id);
        }
    }

    if state.builtin_ime { start::attach_input_method(&state.socket_name); }
    if !no_session { ctl::open_first_tabs(data); }

    event_loop.run(None, &mut state, |_| {}).map_err(|e| e.to_string())?;
    state.save_layout();
    ctl::remember_tabs(data);
    let _ = std::fs::remove_file(data.instance());
    Ok(state.end_reason.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn a(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }
    #[test]
    fn arguments_for_the_program_and_for_ctl() {
        let r = parse_args(a(&["--data-dir", "/d", "--app", "x y", "--app", "z"])).unwrap();
        assert_eq!(r.data_dir.unwrap(), PathBuf::from("/d"));
        assert_eq!(r.apps, ["x y", "z"]);
        let c = parse_args(a(&["ctl", "--data-dir", "/d", "shot", "--out", "p"])).unwrap();
        assert_eq!(c.ctl.unwrap(), ["shot", "--out", "p"]);
        assert!(parse_args(a(&["--what"])).is_err());
        assert_eq!(parse_args(a(&["map", "--data-dir", "/d", "s1", "--write"])).unwrap().ctl.unwrap(), ["map", "s1", "--write"]);
        assert!(parse_args(a(&["--agent", "codex"])).unwrap().codex);
        assert!(parse_args(a(&["--agent", "gpt"])).is_err());
        assert!(parse_args(a(&["--app", "map"])).unwrap().ctl.is_none());
        assert!(parse_args(a(&["--data-dir"])).is_err());
    }
}
