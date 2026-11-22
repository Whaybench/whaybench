//! A Briefsafe project's container (spec 3.22): rootless podman, one per project. In it the project's agents and the
//! Whaybench commands they call. It gets the project folder and the project's open data place (the same paths as on
//! the host), the host's programs read-only, and nothing of the host's data or settings: no writable way back.
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{paths::Data, roots::{CLAUDE_DIR, PROJECT_DATA, Stub}};

/// The container's name: the project and the data folder it belongs to, so two data folders' projects of the same id
/// never take each other's container.
pub fn name(s: &Stub) -> String {
    let place = s.open_at.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    let hash = place.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    format!("whaybench-{}-{:08x}", s.id, hash as u32)
}

fn os_value<'a>(os_release: &'a str, key: &str) -> Option<&'a str> {
    os_release.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')).map(|v| v.trim().trim_matches('"')).filter(|v| !v.is_empty())
}

/// The official image of the host's distribution and version, so the host's programs run in it unchanged. None: no
/// image fits, and the project runs without isolation, as before (spec 3.22).
pub fn image_for(os_release: &str) -> Option<String> {
    let (id, version) = (os_value(os_release, "ID")?, os_value(os_release, "VERSION_ID"));
    match id {
        "fedora" => Some(format!("registry.fedoraproject.org/fedora-minimal:{}", version?)),
        "ubuntu" => Some(format!("docker.io/library/ubuntu:{}", version?)),
        "debian" => Some(format!("docker.io/library/debian:{}-slim", version?)),
        // Derived from Ubuntu (Mint, Pop!_OS): the Ubuntu release it is built on.
        _ if os_value(os_release, "ID_LIKE").is_some_and(|l| l.split_whitespace().any(|x| x == "ubuntu")) =>
            os_value(os_release, "UBUNTU_CODENAME").map(|c| format!("docker.io/library/ubuntu:{c}")),
        _ => None,
    }
}

pub fn host_image() -> Option<String> { image_for(&std::fs::read_to_string("/etc/os-release").ok()?) }

pub fn podman_found() -> bool { Command::new("podman").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success()) }

/// Whether projects can be isolated here: podman and an image for this distribution. Without, they open as before.
pub fn isolation() -> bool { podman_found() && host_image().is_some() }

pub fn running(s: &Stub) -> bool {
    Command::new("podman").args(["container", "inspect", "-f", "{{.State.Running}}", &name(s)]).stderr(Stdio::null()).output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
}

/// The folder a program is in, followed through links: the host's programs are mounted read-only from there.
fn real_dir(program: &Path) -> Option<PathBuf> { std::fs::canonicalize(program).ok()?.parent().map(Path::to_path_buf) }

/// What the container is started with (spec 3.22). `home` is the person's home folder, `programs` the folders of the
/// programs the agents run (Whaybench's own, Claude Code's, Codex's).
pub fn run_args(s: &Stub, image: &str, home: &Path, programs: &[PathBuf]) -> Vec<String> {
    let path = |p: &Path| p.to_string_lossy().to_string();
    let data = path(&s.open_at);
    // No capabilities and no gaining of privileges: the agents work as the person, in their project.
    let mut a: Vec<String> = ["run", "-d", "--init", "--name", &name(s), "--userns=keep-id", "--security-opt", "label=disable",
        "--cap-drop=ALL", "--security-opt=no-new-privileges"].map(String::from).to_vec();
    let mut bind = |from: &Path, to: &Path, ro: bool| { a.push("-v".into()); a.push(format!("{}:{}{}", path(from), path(to), if ro { ":ro" } else { "" })); };
    bind(&s.folder, &s.folder, false);
    // The data where it rests is not for the agents to touch: they see it at its open place only. An empty place
    // (notmpcopyup: podman would otherwise copy what is under it into the tmpfs).
    let resting = s.folder.join(PROJECT_DATA);
    bind(&s.stored().root, &s.open_at, false);
    for p in ["/usr", "/etc/alternatives", "/etc/pki", "/etc/ssl", "/etc/ca-certificates"].map(Path::new).iter().filter(|p| p.exists()) { bind(p, p, true); }
    // A home of the project's own, writable (tool caches, settings), where the person's is: nothing of theirs in it.
    bind(&s.stored().root.join("home"), home, false);
    // In its ~/.local/bin only links to the agents' programs, made by the host at each start (`programs_bin`).
    bind(&s.stored().root.join("cache/bin"), &home.join(".local/bin"), true);
    // Programs and plugins are code: read-only where they are installed. A rustup toolchain too, where there is one.
    for p in [home.join(".claude/plugins/cache"), home.join(".rustup"), home.join(".cargo/bin")].iter().chain(programs).filter(|p| p.exists()) { bind(p, p, true); }
    a.extend(["--tmpfs".into(), format!("{}:ro,notmpcopyup,size=64k", path(&resting))]);
    for (k, v) in [
        ("HOME", path(home)),
        ("PATH", format!("{}:{}:/usr/bin:/bin", path(&home.join(".local/bin")), path(&home.join(".cargo/bin")))),
        ("WHAYBENCH_DATA_DIR", data.clone()),
        ("CLAUDE_CONFIG_DIR", format!("{data}/{CLAUDE_DIR}")),
        ("CLAUDE_CODE_TMPDIR", format!("{data}/cache/tmp")),
        // The programs are the host's, read-only: they are updated there, not from in here.
        ("DISABLE_AUTOUPDATER", "1".into()),
    ] { a.push("-e".into()); a.push(format!("{k}={v}")); }
    // The person's language for the tools in there (the tab passes its own too).
    for k in ["LANG", "LC_ALL"] { if let Ok(v) = std::env::var(k) { a.push("-e".into()); a.push(format!("{k}={v}")); } }
    a.extend(["-w".into(), path(&s.folder), image.into(), "sleep".into(), "infinity".into()]);
    a
}

/// The project's own Claude Code folder, made once from the person's: their settings and plugin lists (not their
/// sign-in, conversations or history), and the project folder already trusted, since Whaybench made it.
pub fn prepare_claude(s: &Stub, home: &Path) -> Result<(), String> {
    // Written where the project's data rests, which its agents can change: beneath it, never through their links.
    let rest = s.stored().root;
    let exists = |rel: &str| crate::beneath::read(&rest, Path::new(rel), crate::beneath::READ_LIMIT).is_ok();
    let put = |rel: &str, bytes: &[u8]| crate::beneath::write(&rest, Path::new(rel), bytes).map_err(|e| format!("{rel}: {e}"));
    let them = home.join(".claude");
    // Only what the agents need of the person's settings: not their environment, key helper or own hooks.
    const KEEP: [&str; 8] = ["enabledPlugins", "extraKnownMarketplaces", "language", "model", "theme", "permissions", "autoMode", "outputStyle"];
    if !exists("claude/settings.json") {
        let all: serde_json::Value = std::fs::read_to_string(them.join("settings.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let kept: serde_json::Map<String, serde_json::Value> = all.as_object().into_iter().flatten().filter(|(k, _)| KEEP.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
        put("claude/settings.json", serde_json::Value::Object(kept).to_string().as_bytes())?;
    }
    for f in ["plugins/installed_plugins.json", "plugins/known_marketplaces.json"] {
        let rel = format!("{CLAUDE_DIR}/{f}");
        if !exists(&rel) && let Ok(bytes) = std::fs::read(them.join(f)) { put(&rel, &bytes)?; }
    }
    if !exists("claude/.claude.json") {
        let folder = s.folder.to_string_lossy().to_string();
        put("claude/.claude.json", serde_json::json!({"hasCompletedOnboarding": true, "projects": {folder: {"hasTrustDialogAccepted": true}}}).to_string().as_bytes())?;
    }
    Ok(())
}

/// The agents' ~/.local/bin in the container: links to the real places of the person's agent programs (Whaybench's
/// commands, Claude Code, Codex), nothing else of that folder. Made again at each start.
pub fn programs_bin(s: &Stub, home: &Path) -> Result<(), String> {
    let rest = s.stored().root;
    crate::beneath::mkdir(&rest, Path::new("cache/bin")).map_err(|e| e.to_string())?;
    for name in ["whaybench", "whaybench-view", "whaybench-project", "whaybench-files", "claude", "codex"] {
        if let Ok(real) = std::fs::canonicalize(home.join(".local/bin").join(name)) {
            crate::beneath::link(&rest, &Path::new("cache/bin").join(name), &real).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// One at a time per project: two starts (two tabs, a tab and `open`) must not remove each other's container.
fn locked<T>(s: &Stub, work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    use std::os::fd::AsRawFd;
    let file = s.open_at.with_extension("lock");
    if let Some(dir) = file.parent() { std::fs::create_dir_all(dir).map_err(|e| e.to_string())?; }
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&file).map_err(|e| e.to_string())?;
    // Safety: flock on a descriptor this function owns; the lock goes with it.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 { return Err(std::io::Error::last_os_error().to_string()); }
    work()
}

/// Starts the project's container if it is not running, from where the project's data rests: its open place is
/// made only after this worked (spec 3.22).
pub fn start(s: &Stub, programs: &[PathBuf]) -> Result<(), String> {
    locked(s, || {
        if running(s) { return Ok(()); }
        if !s.intact() { return Err(format!("{} does not hold project {}'s data", s.stored().root.display(), s.id)); }
        let image = host_image().ok_or("no container image for this distribution")?;
        let home = std::env::var_os("HOME").map(PathBuf::from).ok_or("no home folder")?;
        for dir in ["cache/tmp", CLAUDE_DIR, "home"] { crate::beneath::mkdir(&s.stored().root, Path::new(dir)).map_err(|e| format!("{dir}: {e}"))?; }
        prepare_claude(s, &home)?;
        programs_bin(s, &home)?;
        // The plugins' marketplaces, as the person's own list names them (the project's copy is its agents' to change).
        let mut read = programs.to_vec();
        read.extend(marketplaces(&home.join(".claude/plugins/known_marketplaces.json")));
        // The first open fetches the image: its progress is shown, not hidden in a run that seems to hang.
        let have = Command::new("podman").args(["image", "exists", &image]).status().is_ok_and(|st| st.success());
        if !have {
            let got = Command::new("podman").args(["pull", &image]).stdout(Stdio::null()).status().map_err(|e| format!("podman: {e}"))?;
            if !got.success() { return Err(format!("podman could not fetch {image}")); }
        }
        // A container left from before (stopped) is made again with today's mounts.
        let _ = Command::new("podman").args(["rm", "-f", "--ignore", &name(s)]).stdout(Stdio::null()).stderr(Stdio::null()).status();
        let out = Command::new("podman").args(run_args(s, &image, &home, &read)).output().map_err(|e| format!("podman: {e}"))?;
        if out.status.success() { Ok(()) } else { Err(format!("podman: {}", String::from_utf8_lossy(&out.stderr).trim())) }
    })
}

fn marketplaces(list: &Path) -> Vec<PathBuf> {
    let v: serde_json::Value = std::fs::read_to_string(list).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    v.as_object().into_iter().flatten().filter_map(|(_, m)| m["installLocation"].as_str().map(PathBuf::from)).collect()
}

/// Stops and removes the project's container. An error is told: the project is not taken for closed while it runs.
pub fn stop(s: &Stub) -> Result<(), String> {
    locked(s, || {
        let out = Command::new("podman").args(["rm", "-f", "--ignore", "-t", "5", &name(s)]).stdout(Stdio::null()).output().map_err(|e| format!("podman: {e}"))?;
        if out.status.success() { Ok(()) } else { Err(format!("podman: {}", String::from_utf8_lossy(&out.stderr).trim())) }
    })
}

/// After a crash or a restart: a project left open without its container, or a container left without its open
/// project, is closed. Where projects are not isolated (no podman), an open link is left as it is.
pub fn reconcile(host: &Data) {
    if !isolation() { return; }
    for s in crate::roots::stubs(host) {
        let (open, runs) = (s.open(), running(&s));
        if open && !runs { let _ = crate::roots::close_plain(&s); crate::log::decide("session", "project closed", "left open without its container", &[("project", s.id.clone())]); }
        if runs && !open { let _ = stop(&s); crate::log::decide("session", "project container stopped", "left running without its open project", &[("project", s.id.clone())]); }
    }
}

/// The folders of the programs the agents run, followed through links: Whaybench's own and each agent on PATH.
pub fn programs(own: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::iter::once(own.to_path_buf()).chain(["claude", "codex"].iter().filter_map(|p| which(p))).filter_map(|p| real_dir(&p)).collect();
    dirs.sort();
    dirs.dedup();
    dirs
}

fn which(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(program)).find(|f| f.is_file()))
}

/// A program run in the project's container, as a tab runs it: with a terminal, in `cwd`. The variables named in
/// `pass` go in with the values podman itself has, so none of them (a token) shows on a command line.
pub fn exec_args(s: &Stub, cwd: &Path, pass: &[String], program: &str, args: &[String]) -> Vec<String> {
    // No detach keys: Ctrl-P belongs to the agent. The program notes its process number in the container, so the tab
    // can end it when it closes (podman exec does not pass the hang-up on).
    let mut a: Vec<String> = vec!["exec".into(), "-it".into(), "--detach-keys=".into(), "-w".into(), cwd.to_string_lossy().to_string()];
    for k in pass { a.push("-e".into()); a.push(k.clone()); }
    a.push(name(s));
    a.extend(["sh".into(), "-c".into(), "echo $$ > \"/tmp/whaybench-tab-$WHAYBENCH_TAB.pid\"; exec \"$@\"".into(), "sh".into(), program.into()]);
    a.extend(args.iter().cloned());
    a
}

/// Old conversations' clean-up for a project's data (what the host does for its own at start), run where the project's
/// agents run: in its container, never by a host program walking folders they can change. In the background.
pub fn tidy(s: &Stub, isolated: bool) {
    let mut c = if isolated {
        let mut c = Command::new("podman");
        c.args(["exec", "-d", &name(s), "whaybench", "tidy"]);
        c
    } else {
        let Ok(own) = crate::paths::own_exe() else { return };
        let mut c = Command::new(own.with_file_name("whaybench"));
        c.arg("tidy").env(crate::paths::ENV_DATA_DIR, &s.open_at);
        c
    };
    let _ = c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

/// Ends a tab's agent in the project's container (the tab closed).
pub fn hang_up(s: &Stub, tab: &str) {
    if crate::map::safe_id(tab).is_err() { return; }
    let script = format!("p=$(cat /tmp/whaybench-tab-{tab}.pid 2>/dev/null) && kill -HUP \"$p\" && rm -f /tmp/whaybench-tab-{tab}.pid");
    let _ = Command::new("podman").args(["exec", &name(s), "sh", "-c", &script]).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

/// Where a tab's agent runs (spec 3.22): a project's conversation only in the project's container. Without
/// isolation on this computer the project runs as before; otherwise a project not open, or whose container cannot
/// start, gets no tab rather than an agent on the host.
#[derive(Debug, PartialEq)]
pub enum Place { Host, Container, Refused(String) }

pub fn place_for(project: Option<&Stub>, isolation: bool, open: bool, runs: bool, start: impl FnOnce() -> Result<(), String>) -> Place {
    let Some(s) = project else { return Place::Host };
    if !isolation { return if open { Place::Host } else { Place::Refused(format!("project {} is not open", s.id)) }; }
    if !open { return Place::Refused(format!("project {} is not open: open it first", s.id)); }
    if runs { return Place::Container; }
    match start() { Ok(()) => Place::Container, Err(e) => Place::Refused(format!("project {}'s container did not start: {e}", s.id)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_image_follows_the_host_and_none_fits_others() {
        assert_eq!(image_for("NAME=\"Fedora Linux\"\nID=fedora\nVERSION_ID=44\n").as_deref(), Some("registry.fedoraproject.org/fedora-minimal:44"));
        assert_eq!(image_for("ID=ubuntu\nVERSION_ID=\"24.04\"\n").as_deref(), Some("docker.io/library/ubuntu:24.04"));
        assert_eq!(image_for("ID=debian\nVERSION_ID=\"12\"\n").as_deref(), Some("docker.io/library/debian:12-slim"));
        assert_eq!(image_for("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\nVERSION_ID=22\nUBUNTU_CODENAME=noble\n").as_deref(), Some("docker.io/library/ubuntu:noble"));
        assert_eq!(image_for("ID=arch\n"), None);
    }

    #[test]
    fn the_container_gets_the_project_and_read_only_programs_but_no_host_data() {
        let host = Data::new(PathBuf::from("/h/.local/share/whaybench"));
        let s = Stub::new(&host, "up", "Up", Path::new("/w/up"));
        let a = run_args(&s, "img:1", Path::new("/h"), &[]).join(" ");
        assert!(a.contains("-v /w/up:/w/up "), "{a}");
        assert!(a.contains(&format!(":{}", s.open_at.display())), "the open place at its own path");
        assert!(a.contains("--tmpfs /w/up/.whaybench:ro"), "where the data rests is hidden");
        assert!(a.contains("WHAYBENCH_DATA_DIR=/h/.local/share/whaybench/cache/open/up"));
        assert!(a.contains("CLAUDE_CONFIG_DIR=/h/.local/share/whaybench/cache/open/up/claude"));
        assert!(!a.contains("-v /h/.local/share/whaybench:"), "never the host's data");
        assert!(!a.contains(".claude:") && !a.contains(".codex:"), "never the host's agent settings");
        assert!(a.ends_with("img:1 sleep infinity"));
        let e = exec_args(&s, Path::new("/w/up"), &["TERM".into(), "CLAUDE_CODE_OAUTH_TOKEN".into()], "claude", &["--resume".into(), "x".into()]);
        assert!(e.join(" ").starts_with(&format!("exec -it --detach-keys= -w /w/up -e TERM -e CLAUDE_CODE_OAUTH_TOKEN {} sh -c", name(&s))));
        assert_eq!(&e[e.len() - 4..], ["sh", "claude", "--resume", "x"]);
        assert!(a.contains("--cap-drop=ALL") && a.contains("--security-opt=no-new-privileges"));
        assert!(!a.contains("/h/.local/lib") && a.contains(":/h/.local/bin:ro"), "only the agents' program links");
        let other = Stub::new(&Data::new(PathBuf::from("/h2/whaybench")), "up", "Up", Path::new("/w/up"));
        assert_ne!(name(&s), name(&other), "another data folder's project of the same id");
    }

    #[test]
    fn a_projects_conversation_runs_only_in_its_container() {
        let s = Stub::new(&Data::new(PathBuf::from("/h")), "up", "Up", Path::new("/w/up"));
        let never = || -> Result<(), String> { panic!("not started") };
        assert_eq!(place_for(None, true, false, false, never), Place::Host);
        assert_eq!(place_for(Some(&s), true, true, true, never), Place::Container);
        assert_eq!(place_for(Some(&s), true, true, false, || Ok(())), Place::Container);
        assert!(matches!(place_for(Some(&s), true, true, false, || Err("x".into())), Place::Refused(_)), "never on the host");
        assert!(matches!(place_for(Some(&s), true, false, false, never), Place::Refused(_)), "closed");
        assert_eq!(place_for(Some(&s), false, true, false, never), Place::Host, "no isolation here: as before");
        assert!(matches!(place_for(Some(&s), false, false, false, never), Place::Refused(_)));
    }
}
