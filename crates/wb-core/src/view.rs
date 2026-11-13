//! What is shown. The view panel shows what the AI makes and what the person opens from the file
//! panel, web pages and PDF too, and goes with a session: each session has its own history, and a view can follow the chosen session, stay on
//! one session, or stay on one thing. Firefox is a separate app beside it, for the person's own browsing. Files that
//! need a terminal tool open in a terminal in the same place.
use std::{path::{Path, PathBuf}, time::{SystemTime, UNIX_EPOCH}};

use serde_json::{json, Value};

use crate::{paths::Data, status};

pub const FIREFOX_APP: &str = "org.mozilla.firefox";
pub const TERM_APP: &str = "dev.whaybench.view-term";
pub const VIEW_APP: &str = "dev.whaybench.view";
/// The first view: always there, cannot be closed.
pub const FIRST_VIEW: &str = "picture";
const HISTORY_KEEP: usize = 50;

/// The app id of a view; panels of views share a place, as tabs.
pub fn app_of(name: &str) -> String { if name == FIRST_VIEW { VIEW_APP.to_string() } else { format!("{VIEW_APP}.{}", name.replace('-', "_")) } }

/// What a view is tied to.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Link {
    /// Whichever session is chosen.
    #[default]
    Follow,
    Session(String),
    /// One page or file, under its title. Nothing else comes in.
    Target(String, String),
}

/// For an agent that reads a view: whose page it shows, as every session reads the same view. A view kept on one
/// session names the number that session goes on under now.
pub fn link_note(d: &Data, link: &Link, followed: Option<&str>) -> String {
    let label = |s: &str| crate::agents::session_label(&d.root, &crate::sessions::current_id(d, s));
    match link {
        Link::Follow => match followed {
            Some(s) => crate::i18n::ai_f("view-link-follow", &[("session", label(s))]),
            None => crate::i18n::ai("view-link-follow-none").to_string(),
        },
        Link::Session(s) => crate::i18n::ai_f("view-link-session", &[("session", label(s))]),
        Link::Target(..) => crate::i18n::ai("view-link-page").to_string(),
    }
}

impl Link {
    pub fn to_json(&self) -> Value {
        match self {
            Link::Follow => json!({"mode": "follow"}),
            Link::Session(s) => json!({"mode": "session", "session": s}),
            Link::Target(p, t) => json!({"mode": "target", "target": [p, t]}),
        }
    }
    pub fn from_json(v: &Value) -> Link {
        match v.get("mode").and_then(Value::as_str) {
            Some("session") => v.get("session").and_then(Value::as_str).map_or(Link::Follow, |s| Link::Session(s.to_string())),
            Some("target") => match v.get("target").and_then(Value::as_array).map(Vec::as_slice) {
                Some([p, t]) => Link::Target(p.as_str().unwrap_or("").to_string(), t.as_str().unwrap_or("").to_string()),
                _ => Link::Follow,
            },
            _ => Link::Follow,
        }
    }
}

fn views_file(d: &Data) -> PathBuf { d.root.join("view/views.json") }

/// The open views and what each is tied to; kept over a restart. The first view is always among them.
pub fn views(d: &Data) -> Vec<(String, Link)> {
    let text = std::fs::read_to_string(views_file(d)).unwrap_or_default();
    let list: Vec<Value> = serde_json::from_str(&text).unwrap_or_default();
    let mut out: Vec<(String, Link)> = list.iter().filter_map(|v| Some((v.get("name")?.as_str()?.to_string(), Link::from_json(v)))).collect();
    if !out.iter().any(|(n, _)| n == FIRST_VIEW) { out.insert(0, (FIRST_VIEW.to_string(), Link::Follow)); }
    out
}

fn write_views(d: &Data, list: &[(String, Link)]) {
    let v: Vec<Value> = list.iter().map(|(n, l)| { let mut j = l.to_json(); j["name"] = json!(n); j }).collect();
    let f = views_file(d);
    if let Some(dir) = f.parent() { let _ = std::fs::create_dir_all(dir); }
    let _ = std::fs::write(f, Value::Array(v).to_string());
}

pub fn set_view(d: &Data, name: &str, link: &Link) {
    let mut list = views(d);
    match list.iter_mut().find(|(n, _)| n == name) { Some(e) => e.1 = link.clone(), None => list.push((name.to_string(), link.clone())) }
    write_views(d, &list);
}

pub fn drop_view(d: &Data, name: &str) {
    let list: Vec<_> = views(d).into_iter().filter(|(n, _)| n != name).collect();
    write_views(d, &list);
}

/// `view-<n>`, one above the largest there is.
pub fn next_view_name(d: &Data) -> String {
    let top = views(d).iter().filter_map(|(n, _)| n.strip_prefix("view-")?.parse::<u32>().ok()).max().unwrap_or(1);
    format!("view-{}", top + 1)
}

/// One more view, in its panel. Its name.
pub fn new_view(d: &Data, panel_program: &Path, link: &Link) -> std::io::Result<String> {
    let name = next_view_name(d);
    set_view(d, &name, link);
    status::post(d, &json!({"cmd": "spawn", "command": format!("exec '{}' view '{name}'", panel_program.display())}))?;
    Ok(name)
}

pub fn panel_title(name: &str) -> String {
    match name.strip_prefix("view-") { Some(n) => crate::i18n::f("view", &[("n", (n).to_string())]), None => crate::i18n::t("view-2").to_string() }
}

/// The history of one view for one session. The first view keeps `s-<id>`, others their own, so two views
/// on one session do not write the same list.
pub fn history_dir(d: &Data, view: &str, session: Option<&str>) -> PathBuf {
    let root = d.root.join("view/history");
    match session {
        None => root,
        Some(s) if view == FIRST_VIEW => root.join(format!("s-{s}")),
        Some(s) => root.join(view).join(format!("s-{s}")),
    }
}

/// What a view has shown for a session, with where it is now. `[[path or address, title], …]` on disk.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History { pub dir: PathBuf, pub items: Vec<(String, String)>, pub pos: usize }

impl History {
    /// Reads the list; what no longer exists falls out. It stands at the newest.
    pub fn load(dir: PathBuf) -> Self {
        let text = std::fs::read_to_string(dir.join("index.json")).unwrap_or_default();
        let list: Vec<(String, String)> = serde_json::from_str(&text).unwrap_or_default();
        let items: Vec<_> = list.into_iter().filter(|(p, _)| is_web(p) || Path::new(p).exists()).collect();
        let pos = items.len().saturating_sub(1);
        History { dir, items, pos }
    }
    /// One thing alone, for a view held on it. Not saved.
    pub fn only(path: &str, title: &str) -> Self { History { dir: PathBuf::new(), items: vec![(path.to_string(), title.to_string())], pos: 0 } }
    pub fn current(&self) -> Option<&(String, String)> { self.items.get(self.pos) }
    fn save(&self) {
        if self.dir.as_os_str().is_empty() { return; }
        let _ = std::fs::create_dir_all(&self.dir);
        let _ = std::fs::write(self.dir.join("index.json"), json!(self.items).to_string());
    }
    /// Shows this now. The thing on screen again is not a new entry.
    pub fn push(&mut self, path: &str, title: &str) {
        if self.current().is_none_or(|c| c.0 != path) {
            self.items.push((path.to_string(), title.to_string()));
            if self.items.len() > HISTORY_KEEP { self.items.remove(0); }
        }
        self.pos = self.items.len() - 1;
        self.save();
    }
    /// False: there is nothing that way.
    pub fn go(&mut self, step: i32) -> bool {
        let to = self.pos as i32 + step;
        if to < 0 || to as usize >= self.items.len() { return false; }
        self.pos = to as usize;
        true
    }
    /// Takes the entry on screen off the list; no file is deleted.
    pub fn remove_current(&mut self) {
        if self.pos < self.items.len() { self.items.remove(self.pos); }
        self.pos = self.pos.min(self.items.len().saturating_sub(1));
        self.save();
    }
}

fn requests_dir(d: &Data, to: &str) -> PathBuf { d.root.join("view/requests").join(to) }
/// A session's requests wait here until a view is on that session.
pub fn for_session(session: &str) -> String { format!("s-{session}") }
/// A view's own requests.
pub fn for_view(name: &str) -> String { format!("v-{name}") }

pub fn ask(d: &Data, to: &str, request: &Value) -> std::io::Result<()> {
    let dir = requests_dir(d, to);
    std::fs::create_dir_all(&dir)?;
    let name = SystemTime::now().duration_since(UNIX_EPOCH).map(|t| t.as_nanos()).unwrap_or(0);
    let tmp = dir.join(format!(".{name}-{}", std::process::id()));
    std::fs::write(&tmp, request.to_string())?;
    std::fs::rename(tmp, dir.join(format!("{name}-{}.json", std::process::id())))
}

/// The waiting requests, oldest first. Whoever removes a file has it, so two views never take the same one.
pub fn take_requests(d: &Data, to: &str) -> Vec<Value> {
    let Ok(rd) = std::fs::read_dir(requests_dir(d, to)) else { return Vec::new() };
    let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    files.into_iter().filter_map(|f| {
        let text = std::fs::read_to_string(&f).ok();
        std::fs::remove_file(&f).ok()?;
        serde_json::from_str(&text?).ok()
    }).collect()
}

/// A session goes on under a new number (/background, a restart): its views and what waits for them go with it, older
/// entries first when it already has some.
pub fn session_moved(d: &Data, old: &str, new: &str) {
    let root = d.root.join("view/history");
    let mut dirs = vec![(root.join(format!("s-{old}")), root.join(format!("s-{new}")))];
    if let Ok(rd) = std::fs::read_dir(&root) {
        for e in rd.flatten() {
            if e.path().is_dir() { dirs.push((e.path().join(format!("s-{old}")), e.path().join(format!("s-{new}")))); }
        }
    }
    for (from, to) in dirs {
        if !from.is_dir() { continue; }
        if !to.exists() {
            if let Some(parent) = to.parent() { let _ = std::fs::create_dir_all(parent); }
            if std::fs::rename(&from, &to).is_ok() {
                // The list names its copies by full path, and they have just moved.
                let list = to.join("index.json");
                let (old_dir, new_dir) = (format!("{}/", from.display()), format!("{}/", to.display()));
                let items: Vec<(String, String)> = serde_json::from_str(&std::fs::read_to_string(&list).unwrap_or_default()).unwrap_or_default();
                let moved: Vec<(String, String)> = items.into_iter().map(|(p, t)| (p.strip_prefix(&old_dir).map_or_else(|| p.clone(), |rest| format!("{new_dir}{rest}")), t)).collect();
                let _ = std::fs::write(&list, json!(moved).to_string());
            }
            continue;
        }
        let read = |dir: &Path| -> Vec<(String, String)> { serde_json::from_str(&std::fs::read_to_string(dir.join("index.json")).unwrap_or_default()).unwrap_or_default() };
        let mut joined = read(&from);
        for item in read(&to) { if !joined.contains(&item) { joined.push(item); } }
        if joined.len() > HISTORY_KEEP { joined.drain(..joined.len() - HISTORY_KEEP); }
        if std::fs::write(to.join("index.json"), json!(joined).to_string()).is_ok() { let _ = std::fs::remove_file(from.join("index.json")); }
    }
    let (from, to) = (requests_dir(d, &for_session(old)), requests_dir(d, &for_session(new)));
    if from.is_dir() {
        let _ = std::fs::create_dir_all(&to);
        for e in std::fs::read_dir(&from).into_iter().flatten().flatten() { let _ = std::fs::rename(e.path(), to.join(e.file_name())); }
    }
}

/// What a view shows, as text. Asks the panel and waits for its answer.
/// Where the view panel writes a read's reply: in cache/, which no agent sandbox may write, so a request can name only
/// a file in it, never a path (an agent writes requests in view/; with a path it could make the panel overwrite any file).
pub fn reply_dir(d: &Data) -> PathBuf { d.cache().join("view-read") }

/// A reply's file in reply_dir, from the name a request gives; None for anything but a plain `.json` name.
pub fn reply_file(d: &Data, name: &str) -> Option<PathBuf> {
    let ok = name.ends_with(".json") && !name.starts_with('.') && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
    ok.then(|| reply_dir(d).join(name))
}

pub fn read_panel(d: &Data, name: &str, all: bool) -> Result<Value, String> {
    let file = format!("read-{}-{}.json", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).map(|t| t.as_nanos()).unwrap_or(0));
    let reply = reply_dir(d).join(&file);
    ask(d, &for_view(name), &json!({"do": "read", "all": all, "reply": file})).map_err(|e| e.to_string())?;
    for _ in 0..60 {
        if let Ok(text) = std::fs::read_to_string(&reply) {
            let _ = std::fs::remove_file(&reply);
            return serde_json::from_str(&text).map_err(|e| e.to_string());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(crate::i18n::f("view-not-responding-whaybench-closed-or", &[("name", (name).to_string())]))
}

/// The browsers' sign-ins and settings: outside view/, which an agent's sandbox may write.
pub fn profile(d: &Data) -> PathBuf { d.root.join("browser/firefox") }
pub fn webkit_store(d: &Data) -> PathBuf { d.root.join("browser/webkit") }

/// A Firefox still running on this profile (its `lock` link names host:+pid): it is moved at a later start.
fn in_use(profile: &Path) -> bool {
    std::fs::read_link(profile.join("lock")).ok()
        .and_then(|t| t.to_string_lossy().rsplit('+').next().and_then(|p| p.parse::<u32>().ok()))
        .is_some_and(|pid| Path::new(&format!("/proc/{pid}")).exists())
}

/// Once: the browser data of versions before 0.6.1 moves out of view/. Before any browser starts.
pub fn move_browser_data(d: &Data) {
    for (old, new) in [(d.root.join("view/firefox"), profile(d)), (d.root.join("view/webkit"), webkit_store(d))] {
        if old.is_dir() && !new.exists() && !in_use(&old) {
            if let Some(p) = new.parent() { let _ = std::fs::create_dir_all(p); }
            let _ = std::fs::rename(&old, &new);
        }
    }
}

/// The panel's Firefox shows the page only: no tab strip, and the address bar only while it is in use (Ctrl+L). Its
/// profile is the program's own, so these files are written at every start.
const USER_JS: &str = "user_pref(\"toolkit.legacyUserProfileCustomizations.stylesheets\", true);\n";
const USER_CHROME: &str = "#TabsToolbar { visibility: collapse !important; }\n#navigator-toolbox:not(:focus-within) { visibility: collapse !important; }\n";

fn prepare_profile(d: &Data) {
    let p = profile(d);
    let _ = std::fs::create_dir_all(p.join("chrome"));
    let _ = std::fs::write(p.join("user.js"), USER_JS);
    let _ = std::fs::write(p.join("chrome/userChrome.css"), USER_CHROME);
}

/// The panel's Firefox, as a shell command. Accessibility is on for it alone: the system setting stays.
pub fn firefox_command(d: &Data, target: &str) -> String {
    prepare_profile(d);
    format!("mkdir -p '{p}'; MOZ_ENABLE_WAYLAND=1 GNOME_ACCESSIBILITY=1 exec firefox --profile '{p}' {target}", p = profile(d).display())
}

/// The running panel Firefox: its main process, not a content process.
pub fn firefox_pid(d: &Data) -> Option<u32> {
    let want = profile(d);
    let want = want.to_string_lossy();
    for e in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = e.file_name().to_string_lossy().parse::<u32>().ok() else { continue };
        let Ok(raw) = std::fs::read(e.path().join("cmdline")) else { continue };
        let args: Vec<String> = raw.split(|b| *b == 0).map(|a| String::from_utf8_lossy(a).to_string()).collect();
        let exe = args.first().map(|a| a.rsplit('/').next().unwrap_or("").to_string()).unwrap_or_default();
        if exe != "firefox" && exe != "firefox-bin" { continue; }
        if args.iter().any(|a| a == "-contentproc") { continue; }
        if args.windows(2).any(|w| w[0] == "--profile" && w[1] == *want) { return Some(pid); }
    }
    None
}

pub fn spawn(d: &Data, command: &str) -> std::io::Result<()> {
    status::post(d, &serde_json::json!({"cmd": "spawn", "command": command}))
}

/// A terminal app in the view place.
pub fn terminal(d: &Data, argv: &[String]) -> std::io::Result<()> {
    let config = crate::theme::write_terminal_config(d)?;
    let quoted: Vec<String> = argv.iter().map(|a| format!("'{}'", a.replace('\'', "'\\''"))).collect();
    spawn(d, &format!("exec alacritty --class {TERM_APP} --config-file '{}' -e {}", config.display(), quoted.join(" ")))?;
    status::focus_app(d, TERM_APP);
    Ok(())
}

pub fn is_web(t: &str) -> bool { t.starts_with("http://") || t.starts_with("https://") || t.starts_with("file://") }

/// The address Firefox gets for a file or address, at a page for a PDF.
pub fn address(target: &str, page: Option<u32>) -> String {
    let base = if is_web(target) { target.to_string() } else { format!("file://{target}") };
    match page { Some(p) if target.to_lowercase().ends_with(".pdf") => format!("{base}#page={p}"), _ => base }
}

/// Opens it as a new tab of the panel's Firefox; if Firefox is not running, it starts in the panel.
pub fn open_in_firefox(d: &Data, target: &str, page: Option<u32>) -> std::io::Result<()> {
    let url = address(target, page).replace('\'', "%27");
    if firefox_pid(d).is_some() {
        spawn(d, &format!("MOZ_ENABLE_WAYLAND=1 GNOME_ACCESSIBILITY=1 exec firefox --profile '{}' --new-tab '{url}'", profile(d).display()))?;
    } else {
        spawn(d, &firefox_command(d, &format!("'{url}'")))?;
    }
    status::focus_app(d, FIREFOX_APP);
    Ok(())
}

const PACKED: [&str; 18] = [".gz", ".xz", ".bz2", ".zst", ".lz4", ".lzma", ".z", ".tar", ".tgz", ".tbz2", ".txz", ".zip", ".jar", ".rpm", ".deb", ".7z", ".cpio", ".iso"];
const AS_IS: [&str; 11] = [".pdf", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".html", ".htm", ".mp4", ".webm"];

/// The command that shows this file in a terminal, or None when Firefox shows it.
pub fn system_viewer(path: &Path) -> Option<Vec<String>> {
    let text = path.to_string_lossy().to_string();
    let low = text.to_lowercase();
    let word = |w: &str| w.to_string();
    if text.contains("/share/man/") && path.parent().and_then(|p| p.file_name()).is_some_and(|n| n.to_string_lossy().starts_with("man")) {
        return Some(vec![word("man"), word("-l"), text]);
    }
    if AS_IS.iter().any(|e| low.ends_with(e)) || !path.is_file() { return None; }
    if PACKED.iter().any(|e| low.ends_with(e)) { return Some(vec![word("env"), word("LESSOPEN=||/usr/bin/lesspipe.sh %s"), word("less"), text]); }
    if low.ends_with(".log") || text.starts_with("/var/log/") { return Some(vec![word("less"), word("+G"), text]); }
    let head = std::fs::read(path).ok()?;
    let head = &head[..head.len().min(8192)];
    let binary = match std::str::from_utf8(head) {
        Ok(t) => t.contains('\0'),
        Err(e) => e.valid_up_to() + 4 < head.len(), // a cut through the last character is still text
    };
    binary.then(|| vec![word("sh"), word("-c"), word("{ printf \"%s\\n\\n\" \"$1\"; file -- \"$1\"; echo; ls -l -- \"$1\"; } | less"), word("sh"), text])
}

/// Picked by the person: the first view shows it, or its proper tool.
pub fn open(d: &Data, target: &str, page: Option<u32>) -> std::io::Result<()> { open_to(d, &for_view(FIRST_VIEW), target, page) }

/// Shown by a session: it goes with that session, and appears when a view is on that session.
pub fn open_for_session(d: &Data, session: &str, target: &str, page: Option<u32>) -> std::io::Result<()> { open_to(d, &for_session(session), target, page) }

/// Into a named view.
pub fn open_in_view(d: &Data, name: &str, target: &str, page: Option<u32>) -> std::io::Result<()> { open_to(d, &for_view(name), target, page) }

fn open_to(d: &Data, to: &str, target: &str, page: Option<u32>) -> std::io::Result<()> {
    if !is_web(target) {
        if let Some(argv) = system_viewer(Path::new(target)) { return terminal(d, &argv); }
    }
    ask(d, to, &json!({"do": "show", "path": target, "page": page}))
}

pub fn vim(d: &Data, path: &str) -> std::io::Result<()> { terminal(d, &["vim".to_string(), path.to_string()]) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_read_reply_is_a_plain_name_in_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let d = Data::new(dir.path().to_path_buf());
        assert_eq!(reply_file(&d, "read-1-2.json"), Some(d.cache().join("view-read/read-1-2.json")));
        for bad in ["/etc/profile", "../ctl/a.json", "a/b.json", ".x.json", "a.json.tmp", "", "a b.json"] { assert!(reply_file(&d, bad).is_none(), "{bad}"); }
    }

    #[test]
    fn browser_data_moves_out_of_view_once() {
        let dir = tempfile::tempdir().unwrap();
        let d = Data::new(dir.path().to_path_buf());
        std::fs::create_dir_all(dir.path().join("view/firefox")).unwrap();
        std::fs::write(dir.path().join("view/firefox/prefs.js"), "x").unwrap();
        std::os::unix::fs::symlink(format!("host:+{}", std::process::id()), dir.path().join("view/firefox/lock")).unwrap();
        move_browser_data(&d);
        assert!(!profile(&d).exists(), "not while a Firefox runs on it");
        std::fs::remove_file(dir.path().join("view/firefox/lock")).unwrap();
        move_browser_data(&d);
        assert!(profile(&d).join("prefs.js").exists() && !dir.path().join("view/firefox").exists());
    }

    #[test]
    fn a_read_says_whose_page_and_follows_a_moved_session() {
        let dir = tempfile::tempdir().unwrap();
        let d = Data::new(dir.path().to_path_buf());
        std::fs::create_dir_all(dir.path().join("sessions")).unwrap();
        crate::addons::install_bundled(&d);
        std::fs::write(dir.path().join("sessions/old11111.json"), r#"{"id":"old11111","movedTo":"new22222","agent":"codex"}"#).unwrap();
        std::fs::write(dir.path().join("sessions/new22222.json"), r#"{"id":"new22222","agent":"codex"}"#).unwrap();
        assert!(link_note(&d, &Link::Session("old11111".into()), None).contains("Codex new22222"), "the number it goes on under");
        assert!(link_note(&d, &Link::Follow, Some("new22222")).contains("follows the conversation the person chose, now Codex new22222"));
        assert!(link_note(&d, &Link::Follow, None).contains("none is chosen"));
        assert!(link_note(&d, &Link::Target("/p".into(), "p".into()), None).contains("pinned"));
    }

    #[test]
    fn addresses() {
        assert_eq!(address("https://a.b", Some(3)), "https://a.b");
        assert_eq!(address("/x/a.PDF", Some(3)), "file:///x/a.PDF#page=3");
        assert_eq!(address("/x/a.txt", Some(3)), "file:///x/a.txt");
        assert_eq!(address("/x/a.pdf", None), "file:///x/a.pdf");
    }
    #[test]
    fn each_file_has_its_tool() {
        let d = std::env::temp_dir().join(format!("wb-view-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = |n: &str, b: &[u8]| { let p = d.join(n); std::fs::write(&p, b).unwrap(); p };
        assert_eq!(system_viewer(&f("a.txt", "plain text é".as_bytes())), None);
        assert_eq!(system_viewer(&f("a.png", &[0, 1, 2])), None, "shown as it is");
        assert_eq!(system_viewer(&f("a.log", b"x")).unwrap()[0], "less");
        assert_eq!(system_viewer(&f("a.gz", b"x")).unwrap()[2], "less");
        assert_eq!(system_viewer(&f("a.bin", &[0, 1, 2, 3])).unwrap()[0], "sh");
        assert_eq!(system_viewer(Path::new("/usr/share/man/man1/ls.1.gz")).unwrap()[0], "man");
        assert_eq!(system_viewer(Path::new("/no/such/file")), None);
    }

    fn scratch(tag: &str) -> Data {
        let dir = std::env::temp_dir().join(format!("wb-view-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Data::new(dir)
    }

    #[test]
    fn links_survive_a_round_trip() {
        for l in [Link::Follow, Link::Session("abc".into()), Link::Target("/a b.html".into(), "title".into())] {
            assert_eq!(Link::from_json(&l.to_json()), l);
        }
        assert_eq!(Link::from_json(&json!({"mode": "session"})), Link::Follow, "a session link without a session");
    }

    #[test]
    fn views_are_kept_and_the_first_is_always_there() {
        let d = scratch("views");
        assert_eq!(views(&d), vec![("picture".to_string(), Link::Follow)]);
        assert_eq!(next_view_name(&d), "view-2");
        set_view(&d, "view-2", &Link::Session("s1".into()));
        assert_eq!(next_view_name(&d), "view-3");
        set_view(&d, "picture", &Link::Target("/x".into(), "x".into()));
        assert_eq!(views(&d).len(), 2);
        assert_eq!(views(&d)[0].1, Link::Target("/x".into(), "x".into()));
        drop_view(&d, "view-2");
        assert_eq!(views(&d).len(), 1);
        assert_eq!(app_of("picture"), VIEW_APP);
        assert_eq!(app_of("view-2"), "dev.whaybench.view.view_2");
        assert_eq!(panel_title("view-2"), "View 2");
        let _ = std::fs::remove_dir_all(&d.root);
    }

    #[test]
    fn each_session_has_its_own_history() {
        let d = scratch("hist");
        let (a, b) = (history_dir(&d, "picture", Some("A")), history_dir(&d, "picture", Some("B")));
        let mut h = History::load(a.clone());
        h.push("https://a.example", "web");
        h.push("https://b.example", "web");
        h.push("https://b.example", "web");
        assert_eq!(h.items.len(), 2, "the same thing again is not a new entry");
        assert!(History::load(b).items.is_empty());
        let back = History::load(a);
        assert_eq!((back.items.len(), back.pos), (2, 1), "stands at the newest");
        assert_ne!(history_dir(&d, "view-2", Some("A")), history_dir(&d, "picture", Some("A")));
        let _ = std::fs::remove_dir_all(&d.root);
    }

    #[test]
    fn history_walks_forgets_and_drops_what_vanished() {
        let d = scratch("walk");
        let dir = history_dir(&d, "picture", Some("A"));
        let file = d.root.join("f.html");
        std::fs::write(&file, "x").unwrap();
        let mut h = History::load(dir.clone());
        h.push("https://a.example", "1");
        h.push(file.to_str().unwrap(), "2");
        h.push("https://c.example", "3");
        assert!(!h.go(1));
        assert!(h.go(-2));
        assert_eq!(h.current().unwrap().1, "1");
        assert!(!h.go(-1));
        h.remove_current();
        assert_eq!(h.current().unwrap().1, "2");
        assert!(file.exists(), "no file is deleted");
        std::fs::remove_file(&file).unwrap();
        assert_eq!(History::load(dir).items.len(), 1, "gone from disk, gone from the list");
        for n in 0..60 { h.push(&format!("https://{n}.example"), "x"); }
        assert_eq!(h.items.len(), HISTORY_KEEP);
        let _ = std::fs::remove_dir_all(&d.root);
    }

    #[test]
    fn requests_wait_for_their_session_and_are_taken_once() {
        let d = scratch("req");
        open_for_session(&d, "A", "https://a.example", None).unwrap();
        open_for_session(&d, "A", "https://b.example", None).unwrap();
        assert!(take_requests(&d, &for_session("B")).is_empty());
        let got = take_requests(&d, &for_session("A"));
        assert_eq!(got.len(), 2);
        assert_eq!(got[0]["path"], "https://a.example", "oldest first");
        assert!(take_requests(&d, &for_session("A")).is_empty());
        let _ = std::fs::remove_dir_all(&d.root);
    }

    #[test]
    fn a_moved_session_keeps_its_views() {
        let d = scratch("move");
        let mut h = History::load(history_dir(&d, "picture", Some("old")));
        h.push("https://a.example", "web");
        let mut h2 = History::load(history_dir(&d, "view-2", Some("old")));
        h2.push("https://b.example", "web");
        open_for_session(&d, "old", "https://c.example", None).unwrap();
        session_moved(&d, "old", "new");
        assert_eq!(History::load(history_dir(&d, "picture", Some("new"))).items.len(), 1);
        assert_eq!(History::load(history_dir(&d, "view-2", Some("new"))).items.len(), 1);
        assert_eq!(take_requests(&d, &for_session("new")).len(), 1);
        // A copy kept in the history folder moves with it and stays on the list.
        let dir = history_dir(&d, "picture", Some("c1"));
        std::fs::create_dir_all(&dir).unwrap();
        let copy = dir.join("1.html");
        std::fs::write(&copy, "x").unwrap();
        let mut h4 = History::load(dir);
        h4.push(copy.to_str().unwrap(), "picture");
        session_moved(&d, "c1", "c2");
        let kept = History::load(history_dir(&d, "picture", Some("c2")));
        assert_eq!(kept.items.len(), 1, "the copy is still found");
        assert!(Path::new(&kept.items[0].0).exists());
        // It moves again and already has something of its own there: older entries first.
        let mut h3 = History::load(history_dir(&d, "picture", Some("newer")));
        h3.push("https://z.example", "web");
        session_moved(&d, "new", "newer");
        let got: Vec<String> = History::load(history_dir(&d, "picture", Some("newer"))).items.into_iter().map(|i| i.0).collect();
        assert_eq!(got, vec!["https://a.example", "https://z.example"]);
        session_moved(&d, "new", "newer");
        assert_eq!(History::load(history_dir(&d, "picture", Some("newer"))).items.len(), 2, "doing it again changes nothing");
        let _ = std::fs::remove_dir_all(&d.root);
    }
    #[test]
    fn the_panel_firefox_shows_the_page_only() {
        let dir = tempfile::tempdir().unwrap();
        let d = Data::new(dir.path().to_path_buf());
        assert!(firefox_command(&d, "").contains("--profile"));
        assert!(std::fs::read_to_string(profile(&d).join("user.js")).unwrap().contains("legacyUserProfileCustomizations.stylesheets\", true"));
        assert!(std::fs::read_to_string(profile(&d).join("chrome/userChrome.css")).unwrap().contains("#TabsToolbar"));
    }

}
