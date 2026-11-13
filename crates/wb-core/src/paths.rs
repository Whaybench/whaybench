//! The one user data directory. No fixed path anywhere else: everything is built from this.
use std::path::{Path, PathBuf};

/// Passed to the panel apps the compositor starts, so they use the same directory.
pub const ENV_DATA_DIR: &str = "WHAYBENCH_DATA_DIR";

/// `--data-dir`, then WHAYBENCH_DATA_DIR, then $XDG_DATA_HOME/whaybench, then ~/.local/share/whaybench.
/// The location is not stored inside the directory: that would be circular.
pub fn resolve(arg: Option<&Path>, get: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let set = |k: &str| get(k).filter(|v| !v.is_empty());
    if let Some(a) = arg { return Some(a.to_path_buf()); }
    if let Some(d) = set(ENV_DATA_DIR) { return Some(d.into()); }
    if let Some(x) = set("XDG_DATA_HOME") { return Some(Path::new(&x).join("whaybench")); }
    set("HOME").map(|h| Path::new(&h).join(".local/share/whaybench"))
}

/// The folders under the data directory.
#[derive(Clone, Debug)]
pub struct Data { pub root: PathBuf }

impl Data {
    pub fn new(root: PathBuf) -> Self { Data { root } }
    pub fn logs(&self) -> PathBuf { self.root.join("logs") }
    /// One folder per run: one record file per program (compositor and panel apps).
    pub fn log_runs(&self) -> PathBuf { self.logs().join("run") }
    /// One file per session, written through `whaybench record`.
    pub fn log_sessions(&self) -> PathBuf { self.logs().join("sessions") }
    pub fn log_detail(&self) -> PathBuf { self.logs().join("detail.json") }
    /// Things that can be made again. Deleting this loses nothing of the user's.
    pub fn cache(&self) -> PathBuf { self.root.join("cache") }
    /// Saved conversation maps; made at start, so opening a map finds it even before anything was saved.
    pub fn maps(&self) -> PathBuf { self.root.join("maps") }
    pub fn layout(&self) -> PathBuf { self.root.join("layout.json") }
    /// The running compositor: its process, socket and run.
    pub fn instance(&self) -> PathBuf { self.cache().join("instance.json") }
    /// Requests to the running compositor (`whaybench ctl`).
    pub fn ctl(&self) -> PathBuf { self.cache().join("ctl") }
}

/// Where a new conversation starts: `work_dir` in the data directory's settings.json, else ~/work. Written with
/// `~/` so the setting survives moving to another computer. Not the home folder: Claude Code does not remember
/// trusting that, and would ask in every new session.
/// This program's file. After an update replaced it while it runs, Linux names the old one "<path> (deleted)"; the
/// path is still where the program is, now the new one.
pub fn own_exe() -> std::io::Result<PathBuf> {
    std::env::current_exe().map(|p| PathBuf::from(p.to_string_lossy().trim_end_matches(" (deleted)")))
}

pub fn work_dir(data: &Data, home: &Path) -> PathBuf {
    let set = std::fs::read_to_string(data.root.join("settings.json")).ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("work_dir").and_then(|w| w.as_str()).map(String::from))
        .filter(|w| !w.is_empty());
    match set.as_deref() {
        Some(w) => match w.strip_prefix("~/") { Some(rest) => home.join(rest), None => PathBuf::from(w) },
        None => home.join("work"),
    }
}

/// Writes a whole file or none of it: readers poll these files, and several programs write the same ones.
/// The temporary name has the process number, so two writers do not share it.
pub fn write_atomic(path: &Path, content: impl AsRef<[u8]>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, content)?;
    std::fs::rename(tmp, path)
}

/// Holds the umask the person's programs had, so a program started for them inside keeps theirs.
const ENV_UMASK: &str = "WHAYBENCH_UMASK";

/// First thing in every Whaybench program, before any thread: what it writes (conversations, answers, maps, the
/// memory) is for this account only, as Claude Code keeps its conversations: files 600, folders 700. The person's
/// own umask is kept for the programs started for them (see `user_umask`).
pub fn private_files() {
    // SAFETY: umask only sets this process's mask.
    let old = unsafe { libc::umask(0o077) };
    // SAFETY: called first in main, while no other thread can read the environment.
    if std::env::var_os(ENV_UMASK).is_none() { unsafe { std::env::set_var(ENV_UMASK, format!("{old:o}")) } }
}

/// The person's umask, for a program started for them (an app in a panel, Claude Code in a tab), so what they make
/// in their own folders is as they would make it outside Whaybench.
pub fn user_umask() -> libc::mode_t {
    std::env::var(ENV_UMASK).ok().and_then(|v| libc::mode_t::from_str_radix(&v, 8).ok()).unwrap_or(0o022)
}

/// Files written before `private_files` (Whaybench up to 0.5.4) are made private too: folders 700, files 600,
/// a script 700. Links are left alone.
pub fn make_private(root: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let Ok(meta) = std::fs::symlink_metadata(e.path()) else { continue };
        let mode = meta.permissions().mode() & 0o777;
        let want = if meta.is_dir() || (meta.is_file() && mode & 0o100 != 0) { 0o700 } else if meta.is_file() { 0o600 } else { continue };
        if mode != want { let _ = std::fs::set_permissions(e.path(), std::fs::Permissions::from_mode(want)); }
        if meta.is_dir() { make_private(&e.path()); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_files_are_made_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sessions");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("a.json"), "{}").unwrap();
        std::fs::write(dir.path().join("open"), "#!/bin/sh").unwrap();
        for (p, m) in [(&sub, 0o755), (&sub.join("a.json"), 0o644), (&dir.path().join("open"), 0o755)] { std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap(); }
        make_private(dir.path());
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&sub), 0o700);
        assert_eq!(mode(&sub.join("a.json")), 0o600);
        assert_eq!(mode(&dir.path().join("open")), 0o700);
    }
    fn env<'a>(v: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| v.iter().find(|(n, _)| *n == k).map(|(_, x)| x.to_string())
    }
    #[test]
    fn order_is_arg_env_xdg_home() {
        let all = [("WHAYBENCH_DATA_DIR", "/e"), ("XDG_DATA_HOME", "/x"), ("HOME", "/h")];
        assert_eq!(resolve(Some(Path::new("/a")), env(&all)).unwrap(), Path::new("/a"));
        assert_eq!(resolve(None, env(&all)).unwrap(), Path::new("/e"));
        assert_eq!(resolve(None, env(&all[1..])).unwrap(), Path::new("/x/whaybench"));
        assert_eq!(resolve(None, env(&all[2..])).unwrap(), Path::new("/h/.local/share/whaybench"));
    }
    #[test]
    fn the_work_folder_is_a_setting() {
        let d = std::env::temp_dir().join(format!("wb-work-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let data = Data::new(d.clone());
        let home = Path::new("/h");
        assert_eq!(work_dir(&data, home), Path::new("/h/work"));
        std::fs::write(d.join("settings.json"), r#"{"work_dir":"~/code"}"#).unwrap();
        assert_eq!(work_dir(&data, home), Path::new("/h/code"));
        std::fs::write(d.join("settings.json"), r#"{"work_dir":"/srv/x"}"#).unwrap();
        assert_eq!(work_dir(&data, home), Path::new("/srv/x"));
        std::fs::write(d.join("settings.json"), "{ broken").unwrap();
        assert_eq!(work_dir(&data, home), Path::new("/h/work"));
    }
    #[test]
    fn empty_values_do_not_count_and_no_home_is_none() {
        assert_eq!(resolve(None, env(&[("WHAYBENCH_DATA_DIR", ""), ("HOME", "/h")])).unwrap(), Path::new("/h/.local/share/whaybench"));
        assert!(resolve(None, env(&[])).is_none());
    }
    #[test]
    fn a_file_is_written_whole_into_a_folder_that_was_missing() {
        let dir = std::env::temp_dir().join(format!("wb-atomic-{}", std::process::id()));
        let file = dir.join("deep/a.json");
        write_atomic(&file, "one").unwrap();
        write_atomic(&file, "two").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "two");
        assert_eq!(std::fs::read_dir(dir.join("deep")).unwrap().count(), 1, "no temporary file is left");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
