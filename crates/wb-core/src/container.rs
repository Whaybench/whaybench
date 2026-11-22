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

/// Noted by the host beside the open place when a project was opened in its container: from then until it closes its
/// conversations run there or not at all, whatever podman answers later (spec 3.22).
fn isolated_mark(s: &Stub) -> PathBuf { s.open_at.with_extension("isolated") }
pub fn note_isolated(s: &Stub, on: bool) -> std::io::Result<()> {
    if on { std::fs::write(isolated_mark(s), "") } else { std::fs::remove_file(isolated_mark(s)).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }) }
}
pub fn opened_isolated(s: &Stub) -> bool { std::fs::symlink_metadata(isolated_mark(s)).is_ok() }

pub fn running(s: &Stub) -> bool {
    // Paused (for a snapshot) is running too: podman says Running false then.
    Command::new("podman").args(["container", "inspect", "-f", "{{.State.Status}}", &name(s)]).stderr(Stdio::null()).output()
        .is_ok_and(|o| matches!(String::from_utf8_lossy(&o.stdout).trim(), "running" | "paused"))
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
    // A vault (a FUSE mount) opens inside a folder of the person's own, bound in its place over `cache/open`: the
    // container's runtime cannot look at the FUSE mount itself, and `rslave` shows the vault opened there.
    if s.vault && let Some(open) = s.open_at.parent() { a.push("-v".into()); a.push(format!("{}:{}:rslave", path(&crate::vault::run_dir(s)), path(open))); }
    let mut bind = |from: &Path, to: &Path, ro: bool| { a.push("-v".into()); a.push(format!("{}:{}{}", path(from), path(to), if ro { ":ro" } else { "" })); };
    bind(&s.folder, &s.folder, false);
    // The data where it rests is not for the agents to touch: they see it at its open place only. An empty place
    // (notmpcopyup: podman would otherwise copy what is under it into the tmpfs).
    let resting = s.folder.join(PROJECT_DATA);
    if !s.vault { bind(&s.stored().root, &s.open_at, false); }
    // The host's agent add-ons, read-only where the project's data names them: a project's agents are set up from them.
    // Apart from the project's data (a vault's FUSE folder cannot take a mount): found there by WHAYBENCH_AGENT_ADDONS.
    if let Some(host) = s.open_at.ancestors().nth(3) && host.join("addons/agents").is_dir() { bind(&host.join("addons/agents"), Path::new(crate::agents::ADDONS_IN_CONTAINER), true); }
    for p in ["/usr", "/etc/alternatives", "/etc/pki", "/etc/ssl", "/etc/ca-certificates"].map(Path::new).iter().filter(|p| p.exists()) { bind(p, p, true); }
    // A home of the project's own, writable (tool caches, settings), where the person's is: nothing of theirs in it.
    // A vault's is in the vault, named by HOME only (a FUSE folder cannot be mounted).
    if !s.vault { bind(&s.stored().root.join("home"), home, false); }
    // In its ~/.local/bin only links to the agents' programs, made by the host at each start (`programs_bin`).
    bind(&bin_dir(s), &home.join(".local/bin"), true);
    // Programs and plugins are code: read-only where they are installed. A rustup toolchain too, where there is one.
    for p in [home.join(".claude/plugins/cache"), home.join(".rustup"), home.join(".cargo/bin")].iter().chain(programs).filter(|p| p.exists()) { bind(p, p, true); }
    a.extend(["--tmpfs".into(), format!("{}:ro,notmpcopyup,size=64k", path(&resting))]);
    for (k, v) in [
        ("HOME", if s.vault { format!("{data}/home") } else { path(home) }),
        (crate::agents::ENV_ADDONS, crate::agents::ADDONS_IN_CONTAINER.into()),
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
/// The program links' folder: in the project's data, or for a vault in its run folder (a FUSE folder cannot be mounted).
fn bin_dir(s: &Stub) -> PathBuf { if s.vault { crate::vault::run_dir(s).join("bin") } else { s.stored().root.join("cache/bin") } }

pub fn programs_bin(s: &Stub, home: &Path) -> Result<(), String> {
    if s.vault {
        // The person's own folder, not the agents': made again whole.
        let dir = bin_dir(s);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        for (name, real) in agent_programs(home) { std::os::unix::fs::symlink(&real, dir.join(name)).map_err(|e| e.to_string())?; }
        // Its home is in the vault: `$HOME/.local/bin` (the plugins' hooks name it) leads to the same links.
        crate::beneath::link(&s.stored().root, Path::new("home/.local/bin"), &home.join(".local/bin")).map_err(|e| e.to_string())?;
        return Ok(());
    }
    let rest = s.stored().root;
    crate::beneath::mkdir(&rest, Path::new("cache/bin")).map_err(|e| e.to_string())?;
    for (name, real) in agent_programs(home) { crate::beneath::link(&rest, &Path::new("cache/bin").join(name), &real).map_err(|e| e.to_string())?; }
    Ok(())
}

/// The agents' programs and where they really are.
fn agent_programs(home: &Path) -> Vec<(&'static str, PathBuf)> {
    let mut out = Vec::new();
    for name in ["whaybench", "whaybench-view", "whaybench-project", "whaybench-files", "claude", "codex"] {
        // Where the host finds it: ~/.local/bin first, else on PATH (an npm prefix); a program that is a file in
        // ~/.local/bin itself is not mounted (that folder is this one in there).
        let found = std::fs::canonicalize(home.join(".local/bin").join(name)).ok().or_else(|| which(name).and_then(|p| std::fs::canonicalize(p).ok()));
        if let Some(real) = found.filter(|r| r.parent() != Some(&home.join(".local/bin"))) { out.push((name, real)); }
    }
    out
}

/// One at a time per project: two starts (two tabs, a tab and `open`) must not remove each other's container.
fn locked<T>(s: &Stub, work: impl FnOnce() -> Result<T, String>) -> Result<T, String> { flocked(&s.open_at.with_extension("lock"), work) }

/// One of opening, closing and restoring a project at a time: a restore must not run while an open starts its agents.
/// (A lock of its own: `start` takes the container's lock inside an open.)
pub fn life<T>(s: &Stub, work: impl FnOnce() -> Result<T, String>) -> Result<T, String> { flocked(&s.open_at.with_extension("life"), work) }

pub fn flocked<T>(file: &Path, work: impl FnOnce() -> Result<T, String>) -> Result<T, String> { flock_with(file, libc::LOCK_EX, work) }

fn flock_with<T>(file: &Path, how: i32, work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    use std::os::fd::AsRawFd;
    if let Some(dir) = file.parent() { std::fs::create_dir_all(dir).map_err(|e| e.to_string())?; }
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(file).map_err(|e| e.to_string())?;
    // Safety: flock on a descriptor this function owns; the lock goes with it.
    if unsafe { libc::flock(f.as_raw_fd(), how) } != 0 { return Err(std::io::Error::last_os_error().to_string()); }
    work()
}

/// Starts the project's container if it is not running, from where the project's data rests: its open place is
/// made only after this worked (spec 3.22).
pub fn start(s: &Stub, programs: &[PathBuf]) -> Result<(), String> {
    locked(s, || {
        if running(s) { return Ok(()); }
        if s.vault && !crate::vault::mounted(s) { return Err(format!("project {}'s vault is not open", s.id)); }
        if !s.intact() { return Err(format!("{} does not hold project {}'s data", s.stored().root.display(), s.id)); }
        let image = host_image().ok_or("no container image for this distribution")?;
        let home = std::env::var_os("HOME").map(PathBuf::from).ok_or("no home folder")?;
        for dir in ["cache/tmp", CLAUDE_DIR, "home"] { crate::beneath::mkdir(&s.stored().root, Path::new(dir)).map_err(|e| format!("{dir}: {e}"))?; }
        prepare_claude(s, &home)?;
        programs_bin(s, &home)?;
        // The plugins' marketplaces, as the person's own list names them (the project's copy is its agents' to change).
        let mut read = programs.to_vec();
        read.extend(marketplaces(&home.join(".claude/plugins/known_marketplaces.json"), &home));
        // The first open fetches the image: its progress is shown, not hidden in a run that seems to hang.
        let have = Command::new("podman").args(["image", "exists", &image]).status().is_ok_and(|st| st.success());
        if !have {
            let got = Command::new("podman").args(["pull", &image]).stdout(Stdio::null()).status().map_err(|e| format!("podman: {e}"))?;
            if !got.success() { return Err(format!("podman could not fetch {image}")); }
        }
        // A container left from before (stopped) is made again with today's mounts.
        let _ = Command::new("podman").args(["rm", "-f", "--ignore", &name(s)]).stdout(Stdio::null()).stderr(Stdio::null()).status();
        // What is mounted inside the project's home is made there first, by the person: podman would make the folders
        // on the way as another user, and the agents could not write their own settings there (~/.codex).
        // (A vault's home is not mounted: nothing to make there.)
        for p in read.iter().chain([home.join(".claude/plugins/cache"), home.join(".rustup"), home.join(".cargo/bin"), home.join(".local/bin"), s.folder.clone(), s.open_at.clone()].iter()).filter(|_| !s.vault) {
            if let Ok(rel) = p.strip_prefix(&home) && (p.exists() || p == &s.open_at) { crate::beneath::mkdir(&s.stored().root, &Path::new("home").join(rel)).map_err(|e| format!("{}: {e}", rel.display()))?; }
        }
        let out = Command::new("podman").args(run_args(s, &image, &home, &read)).output().map_err(|e| format!("podman: {e}"))?;
        if out.status.success() { Ok(()) } else { Err(format!("podman: {}", String::from_utf8_lossy(&out.stderr).trim())) }
    })
}

fn marketplaces(list: &Path, home: &Path) -> Vec<PathBuf> {
    let v: serde_json::Value = std::fs::read_to_string(list).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    // One Claude Code keeps is mounted whole; a folder of the person's own (a "directory" marketplace, often their
    // source) only by its manifest: the plugins themselves are read from Claude Code's installed copies.
    let own = home.join(".claude/plugins");
    v.as_object().into_iter().flatten().filter_map(|(_, m)| m["installLocation"].as_str().map(PathBuf::from))
        .flat_map(|loc| if loc.starts_with(&own) { vec![loc] } else { manifest_and_plugins(&loc) }).collect()
}

/// A directory marketplace's manifest and the plugin folders it names (read in place by Claude Code), nothing else of
/// that folder; a plugin named outside it is left out.
fn manifest_and_plugins(loc: &Path) -> Vec<PathBuf> {
    let manifest = loc.join(".claude-plugin");
    let v: serde_json::Value = std::fs::read_to_string(manifest.join("marketplace.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let real = |p: PathBuf| std::fs::canonicalize(p).ok().filter(|r| std::fs::canonicalize(loc).is_ok_and(|l| r.starts_with(&l) && *r != l));
    let plugins = v["plugins"].as_array().into_iter().flatten().filter_map(|p| p["source"].as_str()).filter_map(|src| real(loc.join(src)));
    std::iter::once(manifest).chain(plugins).collect()
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
    let podman = podman_found();
    for s in crate::roots::stubs(host) {
      // In turn with open, close, snapshot and restore: never between an open's container and its open place. One
      // being done right now is left to finish (the window does not wait for an image to download).
      let _ = flock_with(&s.open_at.with_extension("life"), libc::LOCK_EX | libc::LOCK_NB, || {
        let (open, runs) = (s.open(), podman && running(&s));
        // Opened in its container and the container is gone (or podman is): closed, not left for the host to run.
        if open && !runs && opened_isolated(&s) {
            let _ = crate::roots::close_plain(&s);
            let _ = note_isolated(&s, false);
            crate::log::decide("session", "project closed", "left open without its container", &[("project", s.id.clone())]);
        }
        if !open { let _ = note_isolated(&s, false); }
        // A vault left open by a crash, its project not open (or just closed above): closed.
        if s.vault && !s.open() { let _ = crate::vault::unmount(&s); }
        if runs && !open { let _ = stop(&s); crate::log::decide("session", "project container stopped", "left running without its open project", &[("project", s.id.clone())]); }
        Ok(())
      });
    }
}

/// The folders of the programs the agents run, followed through links: Whaybench's own and each agent on PATH.
pub fn programs(own: &Path) -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    // Not ~/.local/bin itself: it is the program links' own folder in there.
    let mut dirs: Vec<PathBuf> = std::iter::once(own.to_path_buf()).chain(["claude", "codex"].iter().filter_map(|p| which(p))).filter_map(|p| real_dir(&p))
        .filter(|d| *d != home.join(".local/bin")).collect();
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

/// An add-on agent's settings folder for a project's tab (`CODEX_HOME` and where), made in the project's data: in its
/// container (from the add-on's files and the project's own home, so its login is the project's), or without one by
/// the host's `whaybench` on the open place.
pub fn agent_home(s: &Stub, isolated: bool, agent: &crate::agents::Agent) -> Result<Option<(String, String)>, String> {
    // Named by the host from the add-on, never taken from what the program in there prints.
    let Some(var) = agent.home_var() else { return Ok(None) };
    let id = agent.id.as_str();
    if !isolated {
        // No container: the agents run as the person anyway; the host's add-on makes the folder in the project's data.
        let own = crate::paths::own_exe().map_err(|e| e.to_string())?;
        agent.prepare(&s.open_at, &own.with_file_name("whaybench")).map_err(|e| e.to_string())?;
        return Ok(Some((var.to_string(), s.open_at.join(id).to_string_lossy().to_string())));
    }
    let out = Command::new("podman").args(["exec", &name(s), "whaybench", "agent-home", id]).stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
    if !out.status.success() { return Err(String::from_utf8_lossy(&out.stderr).trim().to_string()); }
    // Where `prepare` makes it in the project's data (its data folder is the open place there).
    Ok(Some((var.to_string(), s.open_at.join(id).to_string_lossy().to_string())))
}

/// Runs `work` with the project's container paused, if it runs (a snapshot of one moment).
pub fn paused<T>(s: &Stub, work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let podman = |what: &str| Command::new("podman").args([what, &name(s)]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|x| x.success());
    let runs = running(s);
    // Not paused, not taken: a snapshot of files being written is not one moment.
    if runs && !podman("pause") { return Err(format!("the project's container could not be paused (podman pause {}): no snapshot taken", name(s))); }
    let held = runs;
    let out = work();
    if held && !podman("unpause") { return Err(format!("the project's container stays paused: podman unpause {}", name(s))); }
    out
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
