//! A Briefsafe project's container (spec 3.22): rootless podman, one per project. In it the project's agents and the
//! Whaybench commands they call. It gets the project folder and the project's open data place (the same paths as on
//! the host), the host's programs read-only, and nothing of the host's data or settings: no writable way back.
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{paths::Data, roots::{CLAUDE_DIR, PROJECT_DATA, Stub}};

pub fn name(id: &str) -> String { format!("whaybench-{id}") }

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

pub fn running(id: &str) -> bool {
    Command::new("podman").args(["container", "inspect", "-f", "{{.State.Running}}", &name(id)]).stderr(Stdio::null()).output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
}

/// The folder a program is in, followed through links: the host's programs are mounted read-only from there.
fn real_dir(program: &Path) -> Option<PathBuf> { std::fs::canonicalize(program).ok()?.parent().map(Path::to_path_buf) }

/// What the container is started with (spec 3.22). `home` is the person's home folder, `programs` the folders of the
/// programs the agents run (Whaybench's own, Claude Code's, Codex's).
pub fn run_args(s: &Stub, image: &str, home: &Path, programs: &[PathBuf]) -> Vec<String> {
    let path = |p: &Path| p.to_string_lossy().to_string();
    let data = path(&s.open_at);
    let mut a: Vec<String> = ["run", "-d", "--init", "--name", &name(&s.id), "--userns=keep-id", "--security-opt", "label=disable"].map(String::from).to_vec();
    let mut bind = |from: &Path, to: &Path, ro: bool| { a.push("-v".into()); a.push(format!("{}:{}{}", path(from), path(to), if ro { ":ro" } else { "" })); };
    bind(&s.folder, &s.folder, false);
    // The data where it rests is not for the agents to touch: they see it at its open place only. An empty place
    // (notmpcopyup: podman would otherwise copy what is under it into the tmpfs).
    let resting = s.folder.join(PROJECT_DATA);
    let real_data = std::fs::canonicalize(&s.open_at).unwrap_or_else(|_| s.open_at.clone());
    bind(&real_data, &s.open_at, false);
    for p in ["/usr", "/etc/alternatives", "/etc/pki", "/etc/ssl", "/etc/ca-certificates"].map(Path::new).iter().filter(|p| p.exists()) { bind(p, p, true); }
    // Plugins are code: their installed copies are read where Claude Code installed them.
    for p in [home.join(".local/bin"), home.join(".local/lib"), home.join(".claude/plugins/cache")].iter().chain(programs).filter(|p| p.exists()) { bind(p, p, true); }
    a.extend(["--tmpfs".into(), format!("{}:ro,notmpcopyup,size=64k", path(&resting))]);
    for (k, v) in [
        ("HOME", path(home)),
        ("PATH", format!("{}:/usr/bin:/bin", path(&home.join(".local/bin")))),
        ("WHAYBENCH_DATA_DIR", data.clone()),
        ("CLAUDE_CONFIG_DIR", format!("{data}/{CLAUDE_DIR}")),
        ("CLAUDE_CODE_TMPDIR", format!("{data}/cache/tmp")),
        // The programs are the host's, read-only: they are updated there, not from in here.
        ("DISABLE_AUTOUPDATER", "1".into()),
    ] { a.push("-e".into()); a.push(format!("{k}={v}")); }
    a.extend(["-w".into(), path(&s.folder), image.into(), "sleep".into(), "infinity".into()]);
    a
}

/// The project's own Claude Code folder, made once from the person's: their settings and plugin lists (not their
/// sign-in, conversations or history), and the project folder already trusted, since Whaybench made it.
pub fn prepare_claude(s: &Stub, home: &Path) -> Result<(), String> {
    let own = s.open_at.join(CLAUDE_DIR);
    let them = home.join(".claude");
    std::fs::create_dir_all(own.join("plugins")).map_err(|e| e.to_string())?;
    for f in ["settings.json", "plugins/installed_plugins.json", "plugins/known_marketplaces.json"] {
        if !own.join(f).exists() && them.join(f).is_file() { std::fs::copy(them.join(f), own.join(f)).map_err(|e| e.to_string())?; }
    }
    let state = own.join(".claude.json");
    if !state.exists() {
        let folder = s.folder.to_string_lossy().to_string();
        let v = serde_json::json!({"hasCompletedOnboarding": true, "projects": {folder: {"hasTrustDialogAccepted": true}}});
        crate::paths::write_atomic(&state, v.to_string()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Starts the project's container if it is not running. The project must be open (its data at its open place).
pub fn start(s: &Stub, programs: &[PathBuf]) -> Result<(), String> {
    if running(&s.id) { return Ok(()); }
    if !s.open() { return Err(format!("project {} is not open", s.id)); }
    let image = host_image().ok_or("no container image for this distribution: the project runs without isolation")?;
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or("no home folder")?;
    for dir in ["cache/tmp", CLAUDE_DIR] { std::fs::create_dir_all(s.open_at.join(dir)).map_err(|e| e.to_string())?; }
    prepare_claude(s, &home)?;
    // The plugins' marketplaces are where Claude Code loads them from: read-only, as the programs are.
    let mut read = programs.to_vec();
    read.extend(marketplaces(&s.open_at.join(CLAUDE_DIR).join("plugins/known_marketplaces.json")));
    // A container left from before (stopped) is made again with today's mounts.
    let _ = Command::new("podman").args(["rm", "-f", &name(&s.id)]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    let out = Command::new("podman").args(run_args(s, &image, &home, &read)).output().map_err(|e| format!("podman: {e}"))?;
    if out.status.success() { Ok(()) } else { Err(format!("podman: {}", String::from_utf8_lossy(&out.stderr).trim())) }
}

fn marketplaces(list: &Path) -> Vec<PathBuf> {
    let v: serde_json::Value = std::fs::read_to_string(list).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    v.as_object().into_iter().flatten().filter_map(|(_, m)| m["installLocation"].as_str().map(PathBuf::from)).collect()
}

pub fn stop(id: &str) {
    let _ = Command::new("podman").args(["rm", "-f", "-t", "5", &name(id)]).stdout(Stdio::null()).stderr(Stdio::null()).status();
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
pub fn exec_args(id: &str, cwd: &Path, pass: &[String], program: &str, args: &[String]) -> Vec<String> {
    let mut a: Vec<String> = vec!["exec".into(), "-it".into(), "-w".into(), cwd.to_string_lossy().to_string()];
    for k in pass { a.push("-e".into()); a.push(k.clone()); }
    a.push(name(id));
    a.push(program.into());
    a.extend(args.iter().cloned());
    a
}

/// The project a folder belongs to, for a host program about to start an agent there.
pub fn project_at(host: &Data, cwd: &Path) -> Option<Stub> { crate::roots::project_root_of(host, cwd) }

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
        assert_eq!(exec_args("up", Path::new("/w/up"), &["TERM".into(), "CLAUDE_CODE_OAUTH_TOKEN".into()], "claude", &["--resume".into(), "x".into()]).join(" "),
            "exec -it -w /w/up -e TERM -e CLAUDE_CODE_OAUTH_TOKEN whaybench-up claude --resume x");
    }
}
