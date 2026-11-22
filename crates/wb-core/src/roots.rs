//! Data roots (spec 3.22). The host's data folder, and one more per Briefsafe project, laid out like a data folder.
//! A project's data rests in `<project folder>/.whaybench` (later sealed in a vault there). Open, it is seen at one
//! place, `<host data>/cache/open/<id>`: the same path for the host's programs and inside the project's container, so
//! paths in requests mean the same on both sides. Closed, that place is gone and nothing of the project is seen.
//! The host's folder keeps only a stub per project, `projects/<id>.json`. Host programs (compositor, panels,
//! terminal) look in the host root and in every open project's.
use std::{fs, path::{Path, PathBuf}};

use serde_json::{Value, json};

use crate::paths::{Data, write_atomic};

/// The folder in a project folder that holds the project's data.
pub const PROJECT_DATA: &str = ".whaybench";

/// Claude Code's folder of a project's agents, in its data root: their settings and transcripts.
pub const CLAUDE_DIR: &str = "claude";

/// The file that names, where a project's data rests, the project it belongs to. Checked before opening, so a moved
/// or replaced folder is not opened as the project. Whether a data folder is a project's is told by its path.
pub const MARKER: &str = "briefsafe.json";

/// A host stub: what the host knows of a Briefsafe project, open or not.
#[derive(Clone, Debug, PartialEq)]
pub struct Stub { pub id: String, pub name: String, pub folder: PathBuf, pub open_at: PathBuf }

impl Stub {
    pub fn new(host: &Data, id: &str, name: &str, folder: &Path) -> Self {
        Stub { id: id.into(), name: name.into(), folder: folder.to_path_buf(), open_at: open_dir(host, id) }
    }
    /// Where the project's data rests.
    pub fn stored(&self) -> Data { Data::new(self.folder.join(PROJECT_DATA)) }
    /// The project's data root as programs see it while it is open.
    pub fn root(&self) -> Data { Data::new(self.open_at.clone()) }
    /// Open: the host's own link at its open place points to where its data rests. Nothing the project's agents can
    /// write decides it.
    pub fn open(&self) -> bool {
        fs::symlink_metadata(&self.open_at).is_ok_and(|m| m.file_type().is_symlink())
            && fs::read_link(&self.open_at).is_ok_and(|t| t == self.stored().root)
    }
    /// Where its data rests is this project's (its marker names it).
    pub fn intact(&self) -> bool {
        fs::read_to_string(self.stored().root.join(MARKER)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .is_some_and(|v| v["id"] == self.id.as_str())
    }
}

/// Where an open project's data is seen: the same path on the host and in its container.
pub fn open_dir(host: &Data, id: &str) -> PathBuf { host.cache().join("open").join(id) }

/// Opens a project whose data is not sealed: its data is seen at its open place (a link to where it rests).
pub fn open_plain(s: &Stub) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    if s.open() { return Ok(()); }
    if !s.intact() { return Err(Error::new(ErrorKind::NotFound, format!("{} does not hold project {}'s data (moved or deleted?)", s.stored().root.display(), s.id))); }
    if let Some(parent) = s.open_at.parent() { fs::create_dir_all(parent)?; }
    match fs::symlink_metadata(&s.open_at) {
        Ok(m) if m.file_type().is_symlink() => fs::remove_file(&s.open_at)?,
        // A folder left at the open place (written to after a close) goes only if empty; anything in it is shown.
        Ok(m) if m.is_dir() => fs::remove_dir(&s.open_at).map_err(|e| Error::new(e.kind(), format!("{} is in the way: {e}", s.open_at.display())))?,
        Ok(_) => return Err(Error::new(ErrorKind::AlreadyExists, format!("{} is in the way", s.open_at.display()))),
        Err(_) => {}
    }
    std::os::unix::fs::symlink(&s.stored().root, &s.open_at)
}

/// Closes a project opened by `open_plain`: its open place goes; where the data rests is untouched.
pub fn close_plain(s: &Stub) -> std::io::Result<()> {
    if fs::symlink_metadata(&s.open_at).is_ok_and(|m| m.file_type().is_symlink()) { fs::remove_file(&s.open_at)?; }
    Ok(())
}

fn stub_path(host: &Data, id: &str) -> PathBuf { host.root.join("projects").join(format!("{id}.json")) }

pub fn write_stub(host: &Data, s: &Stub) -> std::io::Result<()> {
    write_atomic(&stub_path(host, &s.id), serde_json::to_string_pretty(&json!({"id": s.id, "name": s.name, "kind": "briefsafe", "folder": s.folder})).unwrap_or_default())
}

/// Marks a data folder as a project's own (written where the project's data rests).
pub fn write_marker(stored: &Data, id: &str) -> std::io::Result<()> {
    write_atomic(&stored.root.join(MARKER), json!({"id": id}).to_string())
}

pub fn remove_stub(host: &Data, id: &str) { let _ = fs::remove_file(stub_path(host, id)); }

/// The host's stubs, by id. A folder `projects/<id>/` beside them is a project of the older kind.
pub fn stubs(host: &Data) -> Vec<Stub> {
    // Read many times a second (the window's request loop, the panels): the files are read again only when the
    // folder's names or time changed (a stub is written by rename; two changes can fall in one tick of the clock).
    use std::sync::Mutex;
    type Kept = (PathBuf, std::time::SystemTime, Vec<std::ffi::OsString>, Vec<Stub>);
    static KEPT: Mutex<Vec<Kept>> = Mutex::new(Vec::new());
    let dir = host.root.join("projects");
    let Ok(changed) = fs::metadata(&dir).and_then(|m| m.modified()) else { return Vec::new() };
    let mut names: Vec<std::ffi::OsString> = fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.file_name()).collect();
    names.sort();
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((.., all)) = kept.iter().find(|(d, t, n, _)| *d == dir && *t == changed && *n == names) { return all.clone(); }
    let all = read_stubs(host);
    kept.retain(|(d, ..)| *d != dir);
    kept.push((dir, changed, names, all.clone()));
    all
}

fn read_stubs(host: &Data) -> Vec<Stub> {
    let mut all: Vec<Stub> = fs::read_dir(host.root.join("projects")).into_iter().flatten().flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json") && e.path().is_file())
        .filter_map(|e| serde_json::from_str::<Value>(&fs::read_to_string(e.path()).ok()?).ok())
        .filter(|v| v["kind"] == "briefsafe")
        .filter_map(|v| Some(Stub::new(host, v["id"].as_str()?, v["name"].as_str().unwrap_or_default(), Path::new(v["folder"].as_str()?))))
        .collect();
    all.sort_by(|a, b| a.id.cmp(&b.id));
    all
}

/// The roots host programs look in.
#[derive(Clone, Debug)]
pub struct Roots { pub host: Data }

impl Roots {
    pub fn new(host: Data) -> Self { Roots { host } }

    /// The open projects' roots, with their ids.
    pub fn projects(&self) -> Vec<(String, Data)> {
        stubs(&self.host).into_iter().filter(Stub::open).map(|s| { let r = s.root(); (s.id, r) }).collect()
    }

    /// The host root first, then each open project's.
    pub fn all(&self) -> Vec<Data> { std::iter::once(self.host.clone()).chain(self.projects().into_iter().map(|p| p.1)).collect() }

    /// The project a session belongs to; none: a host session.
    pub fn project_of(&self, session: &str) -> Option<String> {
        self.projects().into_iter().find(|(_, d)| has_session(d, session)).map(|p| p.0)
    }

    /// The root of a session: the project's that has its record, else the host's (a session not written yet is
    /// the host's until its record appears).
    pub fn of_session(&self, session: &str) -> Data { self.known(session).unwrap_or_else(|| self.host.clone()) }

    /// The root that holds a session's record, if any does. Writers go by this: a closed project's session is in no
    /// root, and its writes are not sent to the host's.
    pub fn known(&self, session: &str) -> Option<Data> {
        if has_session(&self.host, session) { return Some(self.host.clone()); }
        self.projects().into_iter().find(|(_, d)| has_session(d, session)).map(|p| p.1)
    }
}

/// Whether this data folder is a project's own (its agents' and the project's, not the host's).
/// Told by its path (`.../cache/open/<id>`), the same on the host and in the container: nothing inside it decides.
pub fn is_project_root(d: &Data) -> bool {
    d.root.parent().is_some_and(|p| p.file_name().is_some_and(|n| n == "open") && p.parent().is_some_and(|c| c.file_name().is_some_and(|n| n == "cache")))
}

/// The root a session's files are in, for a caller holding `d`: a project's programs hold their own root, the
/// host's hold the host's and find a project session in its project.
pub fn for_session(d: &Data, session: &str) -> Data { if is_project_root(d) { d.clone() } else { Roots::new(d.clone()).of_session(session) } }

/// The roots to list sessions from, for a caller holding `d`.
pub fn all_of(d: &Data) -> Vec<Data> { if is_project_root(d) { vec![d.clone()] } else { Roots::new(d.clone()).all() } }

/// The root holding a session's record for a caller holding `d`, if any: for writes about an existing session.
pub fn known_session(d: &Data, session: &str) -> Option<Data> {
    if is_project_root(d) { has_session(d, session).then(|| d.clone()) } else { Roots::new(d.clone()).known(session) }
}

fn has_session(d: &Data, session: &str) -> bool { d.root.join("sessions").join(format!("{session}.json")).is_file() }

/// The project data root a folder is in, if it is inside a Briefsafe project folder.
pub fn project_root_of(host: &Data, dir: &Path) -> Option<Stub> {
    // Compared as real paths, so a folder reached through a link is still found; the deepest project wins.
    let real = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let dir = real(dir);
    stubs(host).into_iter().filter(|s| dir.starts_with(real(&s.folder))).max_by_key(|s| s.folder.components().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wb-roots-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn the_kept_list_of_projects_follows_new_and_removed_stubs() {
        let dir = tempfile::tempdir().unwrap();
        let host = Data::new(dir.path().join("host"));
        fs::create_dir_all(host.root.join("projects")).unwrap();
        assert!(stubs(&host).is_empty());
        write_stub(&host, &Stub::new(&host, "a", "A", &dir.path().join("a"))).unwrap();
        assert_eq!(stubs(&host).len(), 1);
        write_stub(&host, &Stub::new(&host, "b", "B", &dir.path().join("b"))).unwrap();
        assert_eq!(stubs(&host).iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        remove_stub(&host, "a");
        assert_eq!(stubs(&host).iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["b"]);
    }

    #[test]
    fn a_project_session_is_found_in_its_projects_root_and_others_in_the_hosts() {
        let b = base("find");
        let host = Data::new(b.join("host"));
        let stub = Stub::new(&host, "up", "Up", &b.join("work/up"));
        write_stub(&host, &stub).unwrap();
        // A folder of the older kind beside the stubs is not a stub.
        fs::create_dir_all(host.root.join("projects/old")).unwrap();
        fs::write(host.root.join("projects/old/project.json"), "{}").unwrap();
        assert_eq!(stubs(&host), vec![stub.clone()]);
        let roots = Roots::new(host.clone());
        assert!(roots.projects().is_empty(), "not open: its data root has no project yet");

        let st = stub.stored();
        fs::create_dir_all(st.root.join("projects/up")).unwrap();
        fs::write(st.root.join("projects/up/project.json"), "{}").unwrap();
        write_marker(&st, "up").unwrap();
        assert!(roots.projects().is_empty(), "closed: not seen");
        open_plain(&stub).unwrap();
        let pr = stub.root();
        for (d, id) in [(&host, "h1"), (&pr, "p1")] {
            fs::create_dir_all(d.root.join("sessions")).unwrap();
            fs::write(d.root.join(format!("sessions/{id}.json")), "{}").unwrap();
        }
        assert_eq!(roots.all().iter().map(|d| d.root.clone()).collect::<Vec<_>>(), vec![host.root.clone(), pr.root.clone()]);
        assert_eq!(roots.of_session("p1").root, pr.root);
        assert_eq!(roots.of_session("h1").root, host.root);
        assert_eq!(roots.of_session("new").root, host.root);
        assert_eq!((roots.project_of("p1"), roots.project_of("h1")), (Some("up".to_string()), None));
        assert_eq!(project_root_of(&host, &b.join("work/up/src")).map(|s| s.id), Some("up".into()));
        assert!(project_root_of(&host, &b.join("work/upper")).is_none(), "a folder that only starts with the name");

        // Its sessions' transcripts are in its own Claude folder, not the person's.
        assert_eq!(pr.claude_dir(), Some(pr.root.join(CLAUDE_DIR)));
        let t = pr.root.join("claude/projects/-w-up");
        fs::create_dir_all(&t).unwrap();
        fs::write(t.join("p1.jsonl"), "{\"type\":\"user\",\"message\":{\"content\":\"hi\"}}\n{\"type\":\"assistant\",\"message\":{\"content\":\"ok\"}}\n").unwrap();
        assert!(crate::sessions::has_conversation(&pr, "p1"));
        assert_ne!(host.claude_dir(), pr.claude_dir());

        // What a project's programs say and ask for reaches the host's readers.
        crate::status::say(&pr, "from the project", 30);
        assert_eq!(crate::status::current(&host).as_deref(), Some("from the project"));
        crate::view::ask(&pr, &crate::view::for_session("p1"), &json!({"open": "x"})).unwrap();
        assert_eq!(crate::view::take_requests(&host, &crate::view::for_session("p1")), vec![json!({"open": "x"})]);

        // Open is the host's link, not a file the project's agents can delete.
        fs::remove_file(pr.root.join("projects/up/project.json")).unwrap();
        assert!(stub.open());
        close_plain(&stub).unwrap();
        assert_eq!(roots.of_session("p1").root, host.root, "a closed project's sessions are not found");
        assert!(known_session(&host, "p1").is_none(), "and nothing is written for them anywhere");
        assert!(write_atomic(&stub.open_at.join("view/x.json"), "late").is_err(), "a late write does not make the place again");
        assert!(fs::symlink_metadata(&stub.open_at).is_err());
        // Something left at the open place: an empty folder goes, anything else is in the way and said so.
        fs::create_dir_all(&stub.open_at).unwrap();
        open_plain(&stub).unwrap();
        close_plain(&stub).unwrap();
        fs::create_dir_all(stub.open_at.join("x")).unwrap();
        assert!(open_plain(&stub).is_err());
        fs::remove_dir_all(&stub.open_at).unwrap();
        // A folder that is not this project's (moved, replaced) is not opened.
        fs::remove_file(st.root.join(MARKER)).unwrap();
        assert!(open_plain(&stub).is_err() && !stub.open());
        write_marker(&st, "up").unwrap();
        assert!(st.root.join("sessions/p1.json").exists(), "closing leaves the data where it rests");
        remove_stub(&host, "up");
        fs::remove_dir_all(&b).unwrap();
    }
}
