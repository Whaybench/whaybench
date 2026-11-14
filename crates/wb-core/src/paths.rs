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
/// For an installed release it is the stable path, ~/.local/lib/whaybench/<name>: that folder is a link to the version
/// in use (install.sh), so hooks, links and a restart name the version installed now, not the one this process began as.
pub fn own_exe() -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe().map(|p| PathBuf::from(p.to_string_lossy().trim_end_matches(" (deleted)")))?;
    Ok(stable(&exe, std::env::var_os("HOME").map(PathBuf::from).as_deref()).unwrap_or(exe))
}

/// `<home>/.local/lib/whaybench-versions/<version>/<name>` as `<home>/.local/lib/whaybench/<name>`.
fn stable(exe: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let lib = home?.join(".local/lib");
    let name = exe.file_name()?;
    (exe.parent()?.parent()? == lib.join("whaybench-versions")).then(|| lib.join("whaybench").join(name))
}

#[cfg(test)]
mod stable_tests {
    use super::*;
    #[test]
    fn an_installed_version_is_named_by_the_stable_path() {
        let home = Path::new("/h");
        assert_eq!(stable(Path::new("/h/.local/lib/whaybench-versions/0.6.1/whaybench-term"), Some(home)), Some(PathBuf::from("/h/.local/lib/whaybench/whaybench-term")));
        assert_eq!(stable(Path::new("/src/target/release/whaybench"), Some(home)), None, "a build of its own stays itself");
    }
}

pub fn work_dir(data: &Data, home: &Path) -> PathBuf {
    let set = std::fs::read_to_string(data.root.join("settings.json")).ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("work_dir").and_then(|w| w.as_str()).map(String::from))
        .filter(|w| !w.is_empty());
    let chosen = set.as_deref().map(|w| match w.strip_prefix("~/") { Some(rest) => home.join(rest), None => PathBuf::from(w) });
    // The folder an agent may write in (Codex's sandbox is the folder it starts in): never the home folder or one
    // holding Whaybench's data, where an agent could change its own settings and leave its sandbox.
    // Compared as the folders really are: `..` and links resolved, so `~/work/..` or a link to home is home.
    let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let safe = |p: &Path| p.is_absolute() && !p.components().any(|c| c == std::path::Component::ParentDir)
        && !real(home).starts_with(real(p)) && !real(&data.root).starts_with(real(p));
    match chosen {
        Some(p) if safe(&p) => p,
        Some(p) => {
            crate::log::warn("session", "work folder not used", "it holds the home folder or Whaybench's data", &[("folder", p.display().to_string())]);
            home.join("work")
        }
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

/// The last `max` bytes of a file, from its first whole line: a long log's end without reading all of it.
pub fn read_tail(path: &Path, max: u64) -> std::io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let from = f.metadata()?.len().saturating_sub(max);
    f.seek(SeekFrom::Start(from))?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes)?;
    // Cut in the middle of a line, the part before the first newline is not a line.
    let start = if from > 0 { bytes.iter().position(|b| *b == b'\n').map_or(bytes.len(), |i| i + 1) } else { 0 };
    Ok(String::from_utf8_lossy(&bytes[start..]).into_owned())
}

/// Reads a JSON file that is read, changed and written back. A file that is there but cannot be read as `T` is
/// moved aside to `<name>.unreadable` and logged, so the write that follows does not replace what it held.
/// Missing or empty is the default.
pub fn read_kept<T: serde::de::DeserializeOwned + Default>(path: &Path) -> T {
    // Bytes, not text: a file that is not UTF-8 is unreadable too and is set aside, not taken for a missing one.
    let Ok(bytes) = std::fs::read(path) else { return T::default() };
    if bytes.iter().all(u8::is_ascii_whitespace) { return T::default(); }
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        // Replaced whole by a writer since it was read: that is the one to take, not to set aside.
        if let Some(v) = std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()) { return v; }
        // Each one its own name: a second bad file does not replace the first one set aside.
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let aside = path.with_extension(format!("{}{ms}.unreadable", path.extension().map(|x| format!("{}.", x.to_string_lossy())).unwrap_or_default()));
        let moved = std::fs::rename(path, &aside).is_ok();
        crate::log::warn("data", "unreadable file set aside", &e.to_string(), &[("file", path.display().to_string()), ("moved", moved.to_string())]);
        T::default()
    })
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
        for unsafe_dir in [r#""~/""#, r#""/h""#, r#""/""#, r#""relative""#, r#""~/work/..""#] {
            std::fs::write(d.join("settings.json"), format!(r#"{{"work_dir":{unsafe_dir}}}"#)).unwrap();
            assert_eq!(work_dir(&data, home), Path::new("/h/work"), "{unsafe_dir}: the home folder, above it, or not a full path");
        }
        let parent = d.parent().unwrap().to_string_lossy().to_string();
        std::fs::write(d.join("settings.json"), format!(r#"{{"work_dir":"{parent}"}}"#)).unwrap();
        assert_eq!(work_dir(&data, home), Path::new("/h/work"), "a folder holding the data folder");
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
    #[test]
    fn a_tail_starts_at_a_whole_line() {
        let dir = std::env::temp_dir().join(format!("wb-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("t.jsonl");
        std::fs::write(&file, "first line\nsecond\nthird\n").unwrap();
        assert_eq!(read_tail(&file, 100).unwrap(), "first line\nsecond\nthird\n", "small: all of it");
        assert_eq!(read_tail(&file, 10).unwrap(), "third\n", "cut: from the first whole line");
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn an_unreadable_file_is_set_aside_not_lost() {
        let dir = std::env::temp_dir().join(format!("wb-kept-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.json");
        assert_eq!(read_kept::<Vec<String>>(&file), Vec::<String>::new(), "missing is empty");
        std::fs::write(&file, "[\"x\"]").unwrap();
        assert_eq!(read_kept::<Vec<String>>(&file), vec!["x".to_string()]);
        std::fs::write(&file, "[\"x\", cut").unwrap();
        assert_eq!(read_kept::<Vec<String>>(&file), Vec::<String>::new());
        let not_text = dir.join("b.json");
        std::fs::write(&not_text, [0xff, 0xfe]).unwrap();
        assert_eq!(read_kept::<Vec<String>>(&not_text), Vec::<String>::new());
        assert!(!not_text.exists(), "not UTF-8: set aside too");
        let aside = std::fs::read_dir(&dir).unwrap().flatten().find(|e| { let n = e.file_name().to_string_lossy().to_string(); n.starts_with("a.json.") && n.ends_with(".unreadable") }).unwrap();
        assert_eq!(std::fs::read_to_string(aside.path()).unwrap(), "[\"x\", cut", "kept beside");
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
