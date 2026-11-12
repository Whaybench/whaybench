//! Component versions: not pinned, recorded. A difference from the release record is shown
//! and never stops the program. Everything that reads an outside program's version is in this file.
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

pub const MISSING: &str = "missing";
pub const UNKNOWN: &str = "unknown";

/// (name, program, arguments): what to run to read a component's version.
const PROBES: &[(&str, &str, &[&str])] = &[
    ("Claude Code", "claude", &["--version"]),
    ("Alacritty", "alacritty", &["--version"]),
    ("Firefox", "firefox", &["--version"]),
    ("wl-clipboard", "wl-paste", &["--version"]),
    ("GTK", "pkg-config", &["--modversion", "gtk4"]),
    ("fcitx5", "fcitx5", &["--version"]),
    ("KDE", "plasmashell", &["--version"]),
];

/// A command's version line: the last one with a dotted number in it. Some programs print warnings before the
/// version, others a copyright and licence after it.
fn last_line(s: &str) -> String {
    let dotted = |l: &&str| l.as_bytes().windows(3).any(|w| w[0].is_ascii_digit() && w[1] == b'.' && w[2].is_ascii_digit());
    s.lines().rev().find(dotted).or_else(|| s.lines().rev().find(|l| !l.trim().is_empty())).unwrap_or("").trim().to_string()
}

/// A program by name: PATH, then the user's own bin folder, where Claude Code installs itself.
pub fn find_program(name: &str, path: &str, home: Option<&Path>) -> Option<PathBuf> {
    let extra = home.map(|h| h.join(".local/bin"));
    std::env::split_paths(path).chain(extra).map(|d| d.join(name)).find(|p| p.is_file())
}

fn probe(program: &str, args: &[&str]) -> String {
    // The apps find a program next to this one first (start::spawn_app), such as the Alacritty a release brings.
    let here = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_string_lossy().to_string())).unwrap_or_default();
    let path = format!("{here}:{}", std::env::var("PATH").unwrap_or_default());
    let home = std::env::var("HOME").ok();
    let Some(p) = find_program(program, &path, home.as_deref().map(Path::new)) else { return MISSING.into() };
    match Command::new(p).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output() {
        Ok(o) if o.status.success() => { let t = last_line(&String::from_utf8_lossy(&o.stdout)); if t.is_empty() { UNKNOWN.into() } else { t } }
        _ => UNKNOWN.into(),
    }
}

/// Reads every probed component, each on its own thread, and hands back what has come in by `first`.
/// The rest arrive through the returned function, which waits at most `limit` in total: a slow program
/// must not hold up the window, and a program that hangs is reported as unknown.
pub fn read_components(first: Duration, limit: Duration) -> (Vec<(String, String)>, impl FnOnce() -> Vec<(String, String)>) {
    let began = Instant::now();
    let (tx, rx) = mpsc::channel();
    for (i, (_, program, args)) in PROBES.iter().enumerate() {
        let tx = tx.clone();
        thread::spawn(move || { let _ = tx.send((i, probe(program, args))); });
    }
    drop(tx);
    let mut got: Vec<Option<String>> = vec![None; PROBES.len()];
    while let Ok((i, v)) = rx.recv_timeout(first.saturating_sub(began.elapsed())) { got[i] = Some(v); }
    let early = PROBES.iter().zip(&got).filter_map(|((n, _, _), v)| v.clone().map(|v| (n.to_string(), v))).collect();
    let rest = move || {
        let mut late = Vec::new();
        while let Ok((i, v)) = rx.recv_timeout(limit.saturating_sub(began.elapsed())) { late.push((PROBES[i].0.to_string(), v)); got[i] = Some(String::new()); }
        for ((n, _, _), v) in PROBES.iter().zip(&got) { if v.is_none() { late.push((n.to_string(), UNKNOWN.to_string())); } }
        late
    };
    (early, rest)
}

/// The `name = "value"` lines of a release record's [components] table.
pub fn parse_release(text: &str) -> Vec<(String, String)> {
    let mut inside = false;
    let mut out = Vec::new();
    for l in text.lines().map(str::trim) {
        if l.starts_with('[') { inside = l == "[components]"; continue; }
        if !inside || l.starts_with('#') { continue; }
        if let Some((k, v)) = l.split_once('=') {
            out.push((k.trim().trim_matches('"').to_string(), v.trim().trim_matches('"').to_string()));
        }
    }
    out
}

/// What differs from the release record: (name, tested, now). No record: nothing to compare.
pub fn differences(release: &[(String, String)], now: &[(String, String)]) -> Vec<(String, String, String)> {
    release.iter().filter_map(|(k, tested)| {
        let current = now.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_else(|| MISSING.into());
        (&current != tested).then(|| (k.clone(), tested.clone(), current))
    }).collect()
}


/// The socket of the person's desktop session. WAYLAND_DISPLAY when it is a live socket; otherwise the
/// one the desktop shell itself was given. Never the newest socket: that can be a nested compositor's.
pub fn host_socket(env: Option<&str>, runtime: &Path, shell_env: impl Fn() -> Option<String>) -> Option<String> {
    let live = |s: &str| {
        use std::os::unix::fs::FileTypeExt;
        std::fs::metadata(runtime.join(s)).is_ok_and(|m| m.file_type().is_socket())
    };
    env.filter(|s| live(s)).map(String::from).or_else(|| shell_env().filter(|s| live(s)))
}

/// WAYLAND_DISPLAY of the first running desktop shell of this user.
pub fn desktop_shell_socket() -> Option<String> {
    let uid = unsafe { libc::getuid() };
    for e in std::fs::read_dir("/proc").ok()?.flatten() {
        use std::os::unix::fs::MetadataExt;
        if e.metadata().map(|m| m.uid()).ok() != Some(uid) { continue; }
        let comm = std::fs::read_to_string(e.path().join("comm")).unwrap_or_default();
        if !["plasmashell", "kwin_wayland", "gnome-shell"].contains(&comm.trim()) { continue; }
        let env = std::fs::read(e.path().join("environ")).unwrap_or_default();
        for kv in env.split(|b| *b == 0) {
            if let Some(v) = kv.strip_prefix(b"WAYLAND_DISPLAY=") { return Some(String::from_utf8_lossy(v).to_string()); }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_record_is_read_and_compared() {
        let r = parse_release("version = \"0.1.0\"\n[components]\n# tested\n\"Claude Code\" = \"2.1.318\"\nGTK = \"4.22.5\"\n[other]\nx = \"y\"\n");
        assert_eq!(r.len(), 2);
        let now = vec![("Claude Code".to_string(), "2.1.319".to_string()), ("GTK".to_string(), "4.22.5".to_string())];
        assert_eq!(differences(&r, &now), vec![("Claude Code".to_string(), "2.1.318".to_string(), "2.1.319".to_string())]);
        assert!(differences(&[], &now).is_empty());
        assert_eq!(differences(&r, &[]).len(), 2);
        assert_eq!(differences(&r, &[]).first().unwrap().2, MISSING);
    }
    #[test]
    fn every_component_is_reported_early_or_late() {
        let (early, rest) = read_components(Duration::ZERO, Duration::from_secs(30));
        let late = rest();
        assert_eq!(early.len() + late.len(), PROBES.len());
    }
    #[test]
    fn warnings_before_a_version_are_skipped() {
        assert_eq!(last_line("restorecon: noise\nMozilla Firefox 159.0\n\n"), "Mozilla Firefox 159.0");
        assert_eq!(last_line("wl-clipboard 2.2.1\nCopyright (C) 2018-2025 Someone\nThere is NO WARRANTY.\n"), "wl-clipboard 2.2.1");
    }
    #[test]
    fn a_missing_program_is_reported_not_fatal() {
        assert_eq!(probe("no-such-program-whaybench", &[]), MISSING);
    }
    #[test]
    fn user_bin_is_searched() {
        let d = std::env::temp_dir().join(format!("wb-home-{}", std::process::id()));
        std::fs::create_dir_all(d.join(".local/bin")).unwrap();
        std::fs::write(d.join(".local/bin/claude"), "").unwrap();
        assert!(find_program("claude", "/no/such", Some(&d)).is_some());
        assert!(find_program("claude", "/no/such", None).is_none());
    }
    #[test]
    fn a_dead_socket_is_not_used() {
        let d = std::env::temp_dir().join(format!("wb-sock-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let _l = std::os::unix::net::UnixListener::bind(d.join("wayland-5")).ok();
        std::fs::write(d.join("wayland-9"), "").unwrap();
        assert_eq!(host_socket(Some("wayland-5"), &d, || None).as_deref(), Some("wayland-5"));
        assert_eq!(host_socket(Some("wayland-9"), &d, || Some("wayland-5".into())).as_deref(), Some("wayland-5"));
        assert_eq!(host_socket(None, &d, || Some("wayland-7".into())), None);
    }
}
