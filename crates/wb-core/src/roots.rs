//! Data roots (spec 3.22). The host's data folder, and one more per Briefsafe project: `<project folder>/.whaybench`,
//! laid out like a data folder. A project's sessions, maps, answers and documents are written there, by programs
//! that run with it as their data folder. The host's folder keeps only a stub per project, `projects/<id>.json`.
//! Host programs (compositor, panels, terminal) look in the host root and in every project root that is open.
use std::{fs, path::{Path, PathBuf}};

use serde_json::{Value, json};

use crate::paths::{Data, write_atomic};

/// The folder in a project folder that holds the project's data.
pub const PROJECT_DATA: &str = ".whaybench";

/// Claude Code's folder of a project's agents, in its data root: their settings, sign-in link and transcripts.
pub const CLAUDE_DIR: &str = "claude";

/// A host stub: what the host knows of a Briefsafe project before it is opened.
#[derive(Clone, Debug, PartialEq)]
pub struct Stub { pub id: String, pub name: String, pub folder: PathBuf }

impl Stub {
    /// The project's data root.
    pub fn root(&self) -> Data { Data::new(self.folder.join(PROJECT_DATA)) }
    /// Open: its data root can be read (later: the vault is mounted).
    pub fn open(&self) -> bool { self.root().root.join("projects").join(&self.id).join("project.json").is_file() }
}

fn stub_path(host: &Data, id: &str) -> PathBuf { host.root.join("projects").join(format!("{id}.json")) }

pub fn write_stub(host: &Data, s: &Stub) -> std::io::Result<()> {
    write_atomic(&stub_path(host, &s.id), serde_json::to_string_pretty(&json!({"id": s.id, "name": s.name, "kind": "briefsafe", "folder": s.folder})).unwrap_or_default())
}

pub fn remove_stub(host: &Data, id: &str) { let _ = fs::remove_file(stub_path(host, id)); }

/// The host's stubs, by id. A folder `projects/<id>/` beside them is a project of the older kind.
pub fn stubs(host: &Data) -> Vec<Stub> {
    let mut all: Vec<Stub> = fs::read_dir(host.root.join("projects")).into_iter().flatten().flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json") && e.path().is_file())
        .filter_map(|e| serde_json::from_str::<Value>(&fs::read_to_string(e.path()).ok()?).ok())
        .filter(|v| v["kind"] == "briefsafe")
        .filter_map(|v| Some(Stub { id: v["id"].as_str()?.to_string(), name: v["name"].as_str().unwrap_or_default().to_string(), folder: PathBuf::from(v["folder"].as_str()?) }))
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
    pub fn of_session(&self, session: &str) -> Data {
        if session.is_empty() || has_session(&self.host, session) { return self.host.clone(); }
        self.projects().into_iter().find(|(_, d)| has_session(d, session)).map(|p| p.1).unwrap_or_else(|| self.host.clone())
    }
}

/// Whether this data folder is a project's own (its agents' and the project's, not the host's).
pub fn is_project_root(d: &Data) -> bool { d.root.file_name().is_some_and(|n| n == PROJECT_DATA) }

/// The root a session's files are in, for a caller holding `d`: a project's programs hold their own root, the
/// host's hold the host's and find a project session in its project.
pub fn for_session(d: &Data, session: &str) -> Data { if is_project_root(d) { d.clone() } else { Roots::new(d.clone()).of_session(session) } }

/// The roots to list sessions from, for a caller holding `d`.
pub fn all_of(d: &Data) -> Vec<Data> { if is_project_root(d) { vec![d.clone()] } else { Roots::new(d.clone()).all() } }

fn has_session(d: &Data, session: &str) -> bool { d.root.join("sessions").join(format!("{session}.json")).is_file() }

/// The project data root a folder is in, if it is inside a Briefsafe project folder.
pub fn project_root_of(host: &Data, dir: &Path) -> Option<Stub> {
    stubs(host).into_iter().find(|s| dir.starts_with(&s.folder))
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
    fn a_project_session_is_found_in_its_projects_root_and_others_in_the_hosts() {
        let b = base("find");
        let host = Data::new(b.join("host"));
        let stub = Stub { id: "up".into(), name: "Up".into(), folder: b.join("work/up") };
        write_stub(&host, &stub).unwrap();
        // A folder of the older kind beside the stubs is not a stub.
        fs::create_dir_all(host.root.join("projects/old")).unwrap();
        fs::write(host.root.join("projects/old/project.json"), "{}").unwrap();
        assert_eq!(stubs(&host), vec![stub.clone()]);
        let roots = Roots::new(host.clone());
        assert!(roots.projects().is_empty(), "not open: its data root has no project yet");

        let pr = stub.root();
        fs::create_dir_all(pr.root.join("projects/up")).unwrap();
        fs::write(pr.root.join("projects/up/project.json"), "{}").unwrap();
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

        remove_stub(&host, "up");
        assert_eq!(roots.of_session("p1").root, host.root, "a closed or removed project's sessions are not found");
        fs::remove_dir_all(&b).unwrap();
    }
}
