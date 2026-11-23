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
    // The whole 64 bits: two data folders must not share a container.
    format!("whaybench-{}-{:016x}", s.id, hash)
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

fn status(s: &Stub) -> Option<String> {
    Command::new("podman").args(["container", "inspect", "-f", "{{.State.Status}}", &name(s)]).stderr(Stdio::null()).output().ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

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
    // The time of day is the computer's (a container's is UTC).
    let mut a: Vec<String> = ["run", "-d", "--init", "--name", &name(s), "--userns=keep-id", "--security-opt", "label=disable", "--tz=local",
        "--cap-drop=ALL", "--security-opt=no-new-privileges"].map(String::from).to_vec();
    // A vault (a FUSE mount) opens inside a folder of the person's own, bound in its place over `cache/open`: the
    // container's runtime cannot look at the FUSE mount itself, and `rslave` shows the vault opened there. Read-only:
    // the agents write in the vault only (its own mount stays writable), not beside it or in the program links.
    if s.vault && let Some(open) = s.open_at.parent() { a.push("-v".into()); a.push(format!("{}:{}:ro,rslave", path(&crate::vault::run_dir(s)), path(open))); }
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
        // rustup finds its toolchains where they are mounted, not in the vault's home.
        ("RUSTUP_HOME", path(&home.join(".rustup"))),
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
    let host_json = |f: &str| -> serde_json::Value { std::fs::read_to_string(them.join(f)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default() };
    let own_json = |rel: &str| -> serde_json::Value { crate::beneath::read(&rest, Path::new(rel), crate::beneath::READ_LIMIT).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default() };
    // Plugins installed from the person's own (their cache): theirs to remove too.
    let cache = them.join("plugins/cache");
    let mut removed: Vec<String> = Vec::new();
    for (f, key) in [("plugins/installed_plugins.json", Some("plugins")), ("plugins/known_marketplaces.json", None)] {
        let rel = format!("{CLAUDE_DIR}/{f}");
        let host = host_json(f);
        if host.is_null() { continue; }
        if !exists(&rel) { put(&rel, host.to_string().as_bytes())?; continue; }
        // The person's entries over the project's copy (their install paths change with each update), the rest kept.
        let mut own = own_json(&rel);
        let changed = match key {
            Some(k) => own.is_object() && { let (c, r) = merge_installs(&mut own, &host, k, &cache); removed = r; c },
            None => own.is_object() && { let keys: Vec<String> = host.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default(); let keys: Vec<&str> = keys.iter().map(String::as_str).collect(); merge_top(&mut own, &host, &keys) },
        };
        if changed { put(&rel, own.to_string().as_bytes())?; }
    }
    if !exists("claude/settings.json") {
        let all = host_json("settings.json");
        let kept: serde_json::Map<String, serde_json::Value> = all.as_object().into_iter().flatten().filter(|(k, _)| KEEP.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
        put("claude/settings.json", serde_json::Value::Object(kept).to_string().as_bytes())?;
        put("claude/whaybench-seen.json", serde_json::json!({"enabledPlugins": all.get("enabledPlugins").cloned().unwrap_or_default()}).to_string().as_bytes())?;
    } else {
        // At each log in (spec 3.22): a plugin the person added is on here too, unless the project's agents set it
        // themselves (their on or off stays); the person's marketplaces as theirs are now; one the person removed goes.
        let (host, mut own) = (host_json("settings.json"), own_json("claude/settings.json"));
        if own.is_object() {
            // The person's on or off as last brought in: where the project's still equals it, the person's change follows.
            let seen = own_json("claude/whaybench-seen.json");
            let mut changed = follow_person(&mut own, &host, &seen, "enabledPlugins") | merge_keys(&mut own, &host, &["extraKnownMarketplaces"], false);
            if let Some(on) = own.get_mut("enabledPlugins").and_then(serde_json::Value::as_object_mut) {
                for id in &removed { changed |= on.remove(id).is_some(); }
            }
            if changed { put("claude/settings.json", own.to_string().as_bytes())?; }
            let now = serde_json::json!({"enabledPlugins": host.get("enabledPlugins").cloned().unwrap_or_default()});
            if seen != now { put("claude/whaybench-seen.json", now.to_string().as_bytes())?; }
        }
    }
    if !exists("claude/.claude.json") {
        let folder = s.folder.to_string_lossy().to_string();
        put("claude/.claude.json", serde_json::json!({"hasCompletedOnboarding": true, "projects": {folder: {"hasTrustDialogAccepted": true}}}).to_string().as_bytes())?;
    }
    Ok(())
}

/// Puts the person's entries of each named object into the project's (theirs win, the project's own others stay).
/// Whether anything changed.
/// One object of the person's settings into the project's: a name the project lacks is added, and one whose value
/// is still what the person had last time (`seen`) takes the person's value now; one the project's agents changed
/// stays theirs.
fn follow_person(own: &mut serde_json::Value, host: &serde_json::Value, seen: &serde_json::Value, key: &str) -> bool {
    let Some(theirs) = host.get(key).and_then(serde_json::Value::as_object) else { return false };
    let before = seen.get(key).and_then(serde_json::Value::as_object);
    let Some(mine) = own.as_object_mut().map(|o| o.entry(key.to_string()).or_insert_with(|| serde_json::json!({}))).and_then(serde_json::Value::as_object_mut) else { return false };
    let mut changed = false;
    for (name, v) in theirs {
        let take = match mine.get(name) { None => true, Some(m) => m != v && before.and_then(|b| b.get(name)) == Some(m) };
        if take { mine.insert(name.clone(), v.clone()); changed = true; }
    }
    changed
}

/// `fill_only`: only names the project has not set.
fn merge_keys(own: &mut serde_json::Value, host: &serde_json::Value, keys: &[&str], fill_only: bool) -> bool {
    let mut changed = false;
    for k in keys {
        let Some(theirs) = host.get(*k).and_then(serde_json::Value::as_object) else { continue };
        let mine = own.as_object_mut().map(|o| o.entry(k.to_string()).or_insert_with(|| serde_json::json!({})));
        let Some(mine) = mine.and_then(serde_json::Value::as_object_mut) else { continue };
        for (name, v) in theirs { let put = if fill_only { !mine.contains_key(name) } else { mine.get(name) != Some(v) }; if put { mine.insert(name.clone(), v.clone()); changed = true; } }
    }
    changed
}

/// Installs of each plugin: the person's (user scope) over the project's, the project's own (project or local scope)
/// kept beside them.
fn merge_installs(own: &mut serde_json::Value, host: &serde_json::Value, key: &str, cache: &Path) -> (bool, Vec<String>) {
    let Some(theirs) = host.get(key).and_then(serde_json::Value::as_object) else { return (false, Vec::new()) };
    let Some(mine) = own.as_object_mut().map(|o| o.entry(key.to_string()).or_insert_with(|| serde_json::json!({}))).and_then(serde_json::Value::as_object_mut) else { return (false, Vec::new()) };
    let from_person = |e: &serde_json::Value| e.get("installPath").and_then(serde_json::Value::as_str).is_some_and(|p| Path::new(p).starts_with(cache));
    let mut changed = false;
    for (id, list) in theirs {
        let mut next: Vec<serde_json::Value> = list.as_array().cloned().unwrap_or_default();
        // The project's own installs stay beside them; older copies of the person's (from their cache) do not.
        for e in mine.get(id).and_then(serde_json::Value::as_array).into_iter().flatten() {
            if !from_person(e) && !next.contains(e) { next.push(e.clone()); }
        }
        let next = serde_json::Value::Array(next);
        if mine.get(id) != Some(&next) { mine.insert(id.clone(), next); changed = true; }
    }
    // One the person has removed (it came from their cache, gone now): it goes here too.
    let mut removed = Vec::new();
    let ids: Vec<String> = mine.keys().filter(|id| !theirs.contains_key(*id)).cloned().collect();
    for id in ids {
        let Some(list) = mine.get(&id).and_then(serde_json::Value::as_array).cloned() else { continue };
        let kept: Vec<serde_json::Value> = list.iter().filter(|e| !from_person(e)).cloned().collect();
        if kept.len() == list.len() { continue; }
        changed = true;
        if kept.is_empty() { mine.remove(&id); removed.push(id); } else { mine.insert(id, serde_json::Value::Array(kept)); }
    }
    (changed, removed)
}

/// The same for top-level entries.
fn merge_top(own: &mut serde_json::Value, host: &serde_json::Value, keys: &[&str]) -> bool {
    let Some(mine) = own.as_object_mut() else { return false };
    let mut changed = false;
    for k in keys { if let Some(v) = host.get(*k) && mine.get(*k) != Some(v) { mine.insert(k.to_string(), v.clone()); changed = true; } }
    changed
}

/// The agents' ~/.local/bin in the container: links to the real places of the person's agent programs (Whaybench's
/// commands, Claude Code, Codex), nothing else of that folder. Made again at each start.
/// The program links' folder: in the project's data, or for a vault in its run folder (a FUSE folder cannot be mounted).
fn bin_dir(s: &Stub) -> PathBuf { if s.vault { crate::vault::run_dir(s).join("bin") } else { s.stored().root.join("cache/bin") } }

pub fn programs_bin(s: &Stub, home: &Path) -> Result<(), String> {
    if s.vault {
        // The person's own folder (read-only in there). The folder itself stays: a running container has it mounted,
        // and a folder made anew would leave it the old one, emptied. Its links are put in place one by one.
        let dir = bin_dir(s);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let programs = agent_programs(home);
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            if !programs.iter().any(|(n, _)| e.file_name() == *n) { let _ = std::fs::remove_file(e.path()); }
        }
        for (name, real) in programs {
            let at = dir.join(name);
            if std::fs::read_link(&at).is_ok_and(|t| t == real) { continue; }
            // Made beside and renamed over: a tab starting meanwhile finds the old link or the new one, never none.
            let tmp = dir.join(format!(".{name}.{}", std::process::id()));
            let _ = std::fs::remove_file(&tmp);
            std::os::unix::fs::symlink(&real, &tmp).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &at).map_err(|e| e.to_string())?;
        }
        // Its home is in the vault: `$HOME/.local/bin` (the plugins' hooks name it) leads to the same links.
        // A convenience only (PATH names the links' own folder): the agents can spoil it in their home, never the open.
        if let Err(e) = crate::beneath::link(&s.stored().root, Path::new("home/.local/bin"), &home.join(".local/bin")) {
            crate::log::warn("session", "program links not named in the vault's home", &e.to_string(), &[("project", s.id.clone())]);
        }
        return Ok(());
    }
    let rest = s.stored().root;
    crate::beneath::mkdir(&rest, Path::new("cache/bin")).map_err(|e| e.to_string())?;
    for (name, real) in agent_programs(home) { crate::beneath::link(&rest, &Path::new("cache/bin").join(name), &real).map_err(|e| e.to_string())?; }
    Ok(())
}

/// `programs_bin` for a tab opening into a running container: in turn with a start or another tab doing the same.
pub fn refresh_programs(s: &Stub, home: &Path) -> Result<(), String> { locked(s, || programs_bin(s, home)) }

/// Noted while a project is being closed: its tabs close first, ending their agents, before its container stops.
fn closing_mark(s: &Stub) -> PathBuf { s.open_at.with_extension("closing") }
/// Only while the close that made the mark runs: one left by a close that was killed does not keep tabs away.
pub fn closing(s: &Stub) -> bool {
    std::fs::read_to_string(closing_mark(s)).ok().and_then(|t| t.trim().parse::<u32>().ok()).is_some_and(|pid| Path::new(&format!("/proc/{pid}")).exists())
}

/// Asks the project's tabs to close and waits for them, `wait` at most (an answer being written is let finish its
/// file; a tab that will not go is left to the container's stop).
pub fn close_tabs(s: &Stub, host: &Data, wait: std::time::Duration) {
    let _ = std::fs::write(closing_mark(s), std::process::id().to_string());
    let until = std::time::Instant::now() + wait;
    while std::time::Instant::now() < until {
        let mine = crate::sessions::open_tabs(host).iter().any(|t| crate::sessions::tab_project(host, &t.tab).as_deref() == Some(s.id.as_str()));
        if !mine { break; }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}
pub fn closing_done(s: &Stub) { let _ = std::fs::remove_file(closing_mark(s)); }

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
        if s.vault && !crate::vault::ready(s) { return Err(format!("project {}'s vault is not open", s.id)); }
        if running(s) {
            // Paused and left so (a snapshot that did not finish): going again.
            if status(s).as_deref() == Some("paused") { let _ = Command::new("podman").args(["unpause", &name(s)]).stdout(Stdio::null()).stderr(Stdio::null()).status(); }
            return Ok(());
        }
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
/// The host's open link is there (whether or not what it leads to is ready).
fn fs_link(s: &Stub) -> bool { std::fs::symlink_metadata(&s.open_at).is_ok_and(|m| m.file_type().is_symlink()) }

pub fn reconcile(host: &Data) {
    let podman = podman_found();
    for s in crate::roots::stubs(host) {
      // In turn with open, close, snapshot and restore: never between an open's container and its open place. One
      // being done right now is left to finish (the window does not wait for an image to download).
      let _ = flock_with(&s.open_at.with_extension("life"), libc::LOCK_EX | libc::LOCK_NB, || {
        let (open, runs) = (s.open(), podman && running(&s));
        // Left paused by a snapshot that did not finish: going again, before any tab comes back to it.
        if runs && status(&s).as_deref() == Some("paused") { let _ = Command::new("podman").args(["unpause", &name(&s)]).stdout(Stdio::null()).stderr(Stdio::null()).status(); }
        if !closing(&s) { let _ = std::fs::remove_file(closing_mark(&s)); }
        // Opened in its container and the container is gone (or podman is): closed, not left for the host to run.
        // (A vault: its link goes only once the vault is off, so its data is never left in the clear unseen.)
        let gone = !runs && opened_isolated(&s) && fs_link(&s);
        if gone && (!s.vault || crate::vault::unmount(&s).is_ok()) {
            let _ = crate::roots::close_plain(&s);
            let _ = note_isolated(&s, false);
            crate::log::decide("session", "project closed", "left open without its container", &[("project", s.id.clone())]);
        }
        if !fs_link(&s) { let _ = note_isolated(&s, false); }
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

/// podman's arguments for signing an add-on agent in inside the project's container (`Agent::login`), its settings
/// folder the project's: the login is the project's own. A copy of the person's would not do: these logins renew
/// themselves by a key used once, so two copies sign each other out.
pub fn login_args(s: &Stub, agent: &crate::agents::Agent) -> Result<Vec<String>, String> {
    let (var, folder) = agent_home(s, true, agent)?.ok_or("the add-on names no settings folder")?;
    let program = Path::new(&agent.command).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut a: Vec<String> = ["exec", "-it", "--detach-keys=", "-e", "TERM", "-e"].map(String::from).into();
    a.extend([format!("{var}={folder}"), name(s), program]);
    a.extend(agent.login.iter().cloned());
    Ok(a)
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

/// Lets a project's container go on if it was left paused (a snapshot stopped half way).
pub fn unpause(s: &Stub) {
    if status(s).as_deref() == Some("paused") { let _ = Command::new("podman").args(["unpause", &name(s)]).stdout(Stdio::null()).stderr(Stdio::null()).status(); }
}

/// Ends a tab's agent in the project's container (the tab closed).
pub fn hang_up(s: &Stub, tab: &str) {
    if crate::map::safe_id(tab).is_err() { return; }
    // Waited for, five seconds at most: the agent writes what it has as it ends, before its project closes under it.
    let script = format!("p=$(cat /tmp/whaybench-tab-{tab}.pid 2>/dev/null) && kill -HUP \"$p\" && i=0; while [ $i -lt 50 ] && kill -0 \"$p\" 2>/dev/null; do sleep 0.1; i=$((i+1)); done; rm -f /tmp/whaybench-tab-{tab}.pid");
    // Bounded here too: what is in there (the pid file) is the agents' to change, a pipe there would hold `cat`.
    let _ = Command::new("timeout").args(["8", "podman", "exec", &name(s), "sh", "-c", &script]).stdout(Stdio::null()).stderr(Stdio::null()).status();
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
    #[test]
    fn the_persons_plugins_come_in_and_the_projects_own_stay() {
        let mut own = serde_json::json!({"enabledPlugins": {"a@m": false, "mine@m": true}, "model": "x"});
        let host = serde_json::json!({"enabledPlugins": {"a@m": true, "b@m": true}, "theme": "dark"});
        assert!(merge_keys(&mut own, &host, &["enabledPlugins", "extraKnownMarketplaces"], true));
        assert_eq!(own, serde_json::json!({"enabledPlugins": {"a@m": false, "b@m": true, "mine@m": true}, "model": "x"}), "the project's own off stays, a new one comes");
        assert!(!merge_keys(&mut own, &host, &["enabledPlugins"], true), "nothing new the second time");
        let mut own = serde_json::json!({"plugins": {"p@m": [{"scope": "user", "installPath": "/cache/m/p/1.0"}, {"scope": "project", "installPath": "/cache/m/p/1.0", "projectPath": "/w"}, {"scope": "project", "installPath": "/w/own", "projectPath": "/w"}]}});
        let host = serde_json::json!({"plugins": {"p@m": [{"scope": "user", "installPath": "/cache/m/p/1.1"}]}});
        assert!(merge_installs(&mut own, &host, "plugins", Path::new("/cache")).0);
        assert_eq!(own["plugins"]["p@m"], serde_json::json!([{"scope": "user", "installPath": "/cache/m/p/1.1"}, {"scope": "project", "installPath": "/w/own", "projectPath": "/w"}]), "older copies of the person's go, the project's own stays");
        let mut own = serde_json::json!({"enabledPlugins": {"a@m": false, "b@m": true}});
        let seen = serde_json::json!({"enabledPlugins": {"a@m": false, "b@m": false}});
        let host = serde_json::json!({"enabledPlugins": {"a@m": true, "b@m": true, "c@m": true}});
        assert!(follow_person(&mut own, &host, &seen, "enabledPlugins"));
        assert_eq!(own["enabledPlugins"], serde_json::json!({"a@m": true, "b@m": true, "c@m": true}), "the person's turning on follows where the project left it as it was");
        let mut own = serde_json::json!({"enabledPlugins": {"a@m": false}});
        assert!(!follow_person(&mut own, &serde_json::json!({"enabledPlugins": {"a@m": true}}), &serde_json::json!({"enabledPlugins": {"a@m": true}}), "enabledPlugins"), "the project's own off stays");
        let mut own = serde_json::json!({"plugins": {"gone@m": [{"scope": "user", "installPath": "/cache/m/gone/1"}], "own@m": [{"scope": "user", "installPath": "/w/own"}]}});
        let (_, removed) = merge_installs(&mut own, &serde_json::json!({"plugins": {}}), "plugins", Path::new("/cache"));
        assert_eq!(removed, vec!["gone@m".to_string()]);
        assert!(own["plugins"].get("gone@m").is_none() && own["plugins"].get("own@m").is_some(), "the person's removed one goes, the project's own stays");
        let mut m = serde_json::json!({"mine": 1, "m": {"old": true}});
        assert!(merge_top(&mut m, &serde_json::json!({"m": {"new": true}}), &["m"]));
        assert_eq!(m, serde_json::json!({"mine": 1, "m": {"new": true}}));
    }

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
