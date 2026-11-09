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
    pub fn layout(&self) -> PathBuf { self.root.join("layout.json") }
    /// The running compositor: its process, socket and run.
    pub fn instance(&self) -> PathBuf { self.cache().join("instance.json") }
    /// Requests to the running compositor (`whaybench ctl`).
    pub fn ctl(&self) -> PathBuf { self.cache().join("ctl") }
}

/// Where a new conversation starts: `work_dir` in the data directory's settings.json, else ~/work. Written with
/// `~/` so the setting survives moving to another computer. Not the home folder: Claude Code does not remember
/// trusting that, and would ask in every new session.
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
