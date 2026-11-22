//! Projects: create, find by folder, and the tree of projects.
use std::{collections::HashSet, fs, path::Path};

use serde_json::{Value, json};

use crate::{Res, Store, arr, now_ms, s, write_json};

impl Store {
    pub fn create_project(&self, pid: &str, name: &str, desc: &str, parent: &str, paths: Vec<String>, always: bool) -> Res<Value> {
        if self.get_project(pid).is_some() { return Err(format!("project exists: {pid}")); }
        if !parent.is_empty() && self.get_project(parent).is_none() { return Err(format!("no parent project: {parent}")); }
        let p = json!({"id": pid, "name": name, "desc": desc, "parent": parent, "docs": [], "paths": paths, "always": always, "created": now_ms()});
        write_json(&self.pdir(pid).join("project.json"), &p)?;
        Ok(p)
    }

    /// Notes the project folder and whether it is a subvolume (spec 3.22).
    pub fn set_folder(&self, pid: &str, folder: &str, subvolume: bool) -> Res<Value> {
        let mut p = self.get_project(pid).ok_or_else(|| format!("no project: {pid}"))?;
        p["folder"] = json!(folder);
        p["subvolume"] = json!(subvolume);
        // A project made with its own folder; one without `kind` is the older kind, working as before.
        p["kind"] = json!("briefsafe");
        write_json(&self.pdir(pid).join("project.json"), &p)?;
        Ok(p)
    }

    /// Makes a Briefsafe project (spec 3.22): its folder (a subvolume on btrfs, `from` copied in), its data root
    /// `<folder>/.whaybench` laid out like a data folder with the project in it and a copy of every document
    /// template, the host's stub, and the first snapshot. Everything that can refuse is asked before the folder is
    /// made; a failure after it removes what was made. Answers the project and the first snapshot.
    #[allow(clippy::too_many_arguments)]
    pub fn create_in_folder(&self, pid: &str, name: &str, desc: &str, parent: &str, mut paths: Vec<String>, always: bool, folder: &str, from: Option<&Path>) -> Res<(Value, Option<crate::folder::Taken>)> {
        use wb_core::roots::{self, Stub};
        if self.get_project(pid).is_some() || roots::stubs(&self.data).iter().any(|s| s.id == pid) { return Err(format!("project exists: {pid}")); }
        if !parent.is_empty() && self.get_project(parent).is_none() { return Err(format!("no parent project: {parent}")); }
        // Its records are in its own folder, where the host's projects are not: it stands on its own.
        if !parent.is_empty() { return Err("a Briefsafe project (--folder) has no parent project".into()); }
        let inside = |a: &str, b: &str| a == b || Path::new(a).starts_with(b) || Path::new(b).starts_with(a);
        let taken: Vec<(String, String)> = roots::stubs(&self.data).into_iter().map(|s| (s.id, s.folder.to_string_lossy().to_string()))
            .chain(self.list_projects().iter().map(|p| (s(p, "id").to_string(), s(p, "folder").to_string()))).filter(|(_, f)| !f.is_empty()).collect();
        if let Some((other, _)) = taken.iter().find(|(_, f)| inside(folder, f)) {
            return Err(format!("the folder is inside or around the folder of project {other} (a snapshot would not hold the inner one)"));
        }
        // podman takes a mount as `from:to[:options]`: a folder with a colon in its path could not be mounted.
        if folder.contains(':') { return Err(format!("a project folder's path cannot hold ':' ({folder})")); }
        let dir = Path::new(folder);
        let subvolume = crate::folder::make(dir, from)?;
        let stub = Stub::new(&self.data, pid, name, dir);
        let made = (|| -> Res<Value> {
            // A copied folder's own Whaybench data (another project's, or a link to one) is not this project's.
            let data = dir.join(roots::PROJECT_DATA);
            match fs::symlink_metadata(&data) {
                Ok(m) if m.is_dir() => fs::remove_dir_all(&data).map_err(|e| e.to_string())?,
                Ok(_) => fs::remove_file(&data).map_err(|e| e.to_string())?,
                Err(_) => {}
            }
            let inner = Store::new(stub.stored());
            roots::write_marker(&inner.data, pid).map_err(|e| e.to_string())?;
            if !paths.iter().any(|p| p == folder) { paths.insert(0, folder.to_string()); }
            inner.create_project(pid, name, desc, parent, paths, always)?;
            let p = inner.set_folder(pid, folder, subvolume)?;
            // The templates as they are now: later changes to the add-ons do not reach this project.
            let templates = self.data.root.join("addons/project-docs");
            if templates.is_dir() {
                let to = inner.data.root.join("addons/project-docs");
                fs::create_dir_all(&to).map_err(|e| e.to_string())?;
                crate::folder::copy_into(&templates, &to)?;
            }
            exclude_from_git(dir)?;
            roots::write_stub(&self.data, &stub).map_err(|e| e.to_string())?;
            Ok(p)
        })();
        let p = match made {
            Ok(p) => p,
            Err(e) => {
                roots::remove_stub(&self.data, pid);
                return Err(match crate::folder::discard(dir, subvolume) { Ok(()) => e, Err(e2) => format!("{e}; the folder made is left at {folder}: {e2}") });
            }
        };
        // The first state, before anything is changed in it.
        // The project is made either way: a first snapshot that failed is said, not taken for a failed project.
        let first = subvolume.then(|| crate::folder::snapshot(dir).unwrap_or_else(|e| crate::folder::Taken { name: String::new(), warning: Some(format!("no first snapshot: {e}")) }));
        Ok((p, first))
    }

    pub fn project_for_cwd(&self, cwd: &str) -> Option<Value> {
        let mut best: Option<(Value, usize)> = None;
        for p in self.list_projects() {
            for path in arr(&p, "paths").iter().filter_map(Value::as_str) {
                let path = path.trim_end_matches('/');
                if (cwd == path || cwd.starts_with(&format!("{path}/"))) && best.as_ref().is_none_or(|b| path.len() > b.1) { best = Some((p.clone(), path.len())); }
            }
        }
        best.map(|b| b.0)
    }

    pub fn children(&self, pid: &str) -> Vec<Value> { self.list_projects().into_iter().filter(|p| s(p, "parent") == pid).collect() }

    pub fn ancestors(&self, pid: &str) -> Vec<Value> {
        let (mut out, mut seen) = (Vec::new(), HashSet::new());
        let mut p = self.get_project(pid);
        while let Some(cur) = p {
            let up = s(&cur, "parent").to_string();
            if up.is_empty() || !seen.insert(up.clone()) { break; }
            p = self.get_project(&up);
            if let Some(a) = &p { out.insert(0, a.clone()); }
        }
        out
    }

    pub fn tree(&self) -> Vec<(Value, usize)> {
        fn walk(ps: &[Value], ids: &HashSet<String>, parent: &str, depth: usize, out: &mut Vec<(Value, usize)>) {
            for p in ps {
                let par = s(p, "parent");
                if par == parent || (parent.is_empty() && !par.is_empty() && !ids.contains(par)) {
                    out.push((p.clone(), depth));
                    walk(ps, ids, s(p, "id"), depth + 1, out);
                }
            }
        }
        let ps = self.list_projects();
        let ids: HashSet<String> = ps.iter().map(|p| s(p, "id").to_string()).collect();
        let mut out = Vec::new();
        walk(&ps, &ids, "", 0, &mut out);
        out
    }

    pub fn move_project(&self, pid: &str, parent: &str) -> Res<Value> {
        let mut p = self.get_project(pid).ok_or_else(|| format!("no project: {pid}"))?;
        if !parent.is_empty() {
            if parent == pid || self.get_project(parent).is_none() { return Err(format!("bad parent: {parent}")); }
            if self.ancestors(parent).iter().any(|a| s(a, "id") == pid) { return Err("a project cannot go under its own descendant".into()); }
        }
        p["parent"] = json!(parent);
        write_json(&self.pdir(pid).join("project.json"), &p)?;
        Ok(p)
    }
}

/// Tests of the project store.
/// The project's data stays out of a git repository in its folder: `.whaybench/` in `.git/info/exclude`, so a
/// commit of everything does not take it along. The repository's own files are not changed.
fn exclude_from_git(dir: &Path) -> Res<()> {
    // The repository the folder is in: its own, or one around it.
    let Some(top) = dir.ancestors().find(|a| a.join(".git").is_dir()) else { return Ok(()) };
    let git = top.join(".git");
    let file = git.join("info/exclude");
    // Missing is empty; one that cannot be read is not overwritten as if it were.
    let now = match fs::read_to_string(&file) { Ok(t) => t, Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(), Err(e) => return Err(format!("{}: {e}", file.display())) };
    let rel = dir.strip_prefix(top).map(|r| r.to_string_lossy().to_string()).unwrap_or_default();
    let mut lines = vec![if rel.is_empty() { format!("/{}/", wb_core::roots::PROJECT_DATA) } else { format!("/{rel}/{}/", wb_core::roots::PROJECT_DATA) }];
    // In a repository around the folder its snapshots are inside too, each with a copy of the data.
    if !rel.is_empty() {
        let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let parent = Path::new(&rel).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
        lines.push(if parent.is_empty() { format!("/.{name}.snapshots/") } else { format!("/{parent}/.{name}.snapshots/") });
    }
    lines.retain(|l| !now.lines().any(|x| x.trim() == l));
    if lines.is_empty() { return Ok(()); }
    fs::create_dir_all(git.join("info")).map_err(|e| e.to_string())?;
    let sep = if now.is_empty() || now.ends_with('\n') { "" } else { "\n" };
    fs::write(&file, format!("{now}{sep}{}\n", lines.join("\n"))).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::*;
    use crate::{docaddons::DocAddon, read, scratch, text};

    /// A project with one document, added from a template as the person would.
    fn setup() -> Store {
        let st = scratch("t");
        st.create_project("t", "Test", "", "", vec![], false).unwrap();
        let md = st.data.root.join("spec.md");
        fs::write(&md, "# Spec\n\n## 1 Overview\n").unwrap();
        st.add_doc("t", &DocAddon { id: "spec".into(), name: "Spec".into(), md, original: None }).unwrap();
        st
    }
    #[test]
    fn a_project_with_its_own_folder_is_of_the_briefsafe_kind() {
        let st = setup();
        assert!(st.get_project("t").unwrap().get("kind").is_none(), "made without a folder: the older kind");
        let p = st.set_folder("t", "/w/t", true).unwrap();
        assert_eq!((s(&p, "kind"), s(&p, "folder"), p["subvolume"].as_bool()), ("briefsafe", "/w/t", Some(true)));
        assert_eq!(st.get_project("t").unwrap(), p);
    }

    #[test]
    fn a_briefsafe_project_keeps_its_data_in_its_folder_and_only_a_stub_on_the_host() {
        let host = scratch("bs");
        let md = host.data.root.join("addons/project-docs/wbs/WBS.md");
        fs::create_dir_all(md.parent().unwrap()).unwrap();
        fs::write(&md, "# WBS\n").unwrap();
        let work = host.data.root.join("work");
        let src = work.join("old");
        fs::create_dir_all(src.join(".git/info")).unwrap();
        fs::write(src.join("a.txt"), "hi").unwrap();
        // The source's own Whaybench data, here a link to somewhere else: not copied in as this project's.
        let elsewhere = work.join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, src.join(".whaybench")).unwrap();
        let folder = work.join("up").to_string_lossy().to_string();
        assert!(host.create_in_folder("up", "Up", "", "nosuch", vec![], false, &folder, Some(&src)).is_err());
        assert!(!Path::new(&folder).exists(), "a refusal makes nothing");
        host.create_project("top", "Top", "", "", vec![], false).unwrap();
        assert!(host.create_in_folder("up", "Up", "", "top", vec![], false, &folder, Some(&src)).is_err(), "no parent for a Briefsafe project");
        assert!(!Path::new(&folder).exists());
        let (p, _) = host.create_in_folder("up", "Up", "", "", vec![], false, &folder, Some(&src)).unwrap();
        assert_eq!(s(&p, "kind"), "briefsafe");
        assert!(host.get_project("up").is_none(), "the project is not in the host's store");
        let stubs = wb_core::roots::stubs(&host.data);
        assert_eq!((stubs.len(), stubs[0].open()), (1, false), "made closed");
        wb_core::roots::open_plain(&stubs[0]).unwrap();
        assert!(stubs[0].open());
        let inner = Store::new(stubs[0].root());
        assert_eq!(s(&inner.get_project("up").unwrap(), "folder"), folder);
        assert_eq!(fs::read_to_string(inner.data.root.join("addons/project-docs/wbs/WBS.md")).unwrap(), "# WBS\n");
        assert_eq!(fs::read_to_string(Path::new(&folder).join(".git/info/exclude")).unwrap(), "/.whaybench/\n");
        assert!(fs::symlink_metadata(Path::new(&folder).join(".whaybench")).unwrap().is_dir() && fs::read_dir(&elsewhere).unwrap().count() == 0, "its own data, nothing written through the link");
        assert!(host.create_in_folder("up", "Up", "", "", vec![], false, &format!("{folder}2"), None).is_err(), "the id is taken");
        assert!(host.create_in_folder("in", "In", "", "", vec![], false, &format!("{folder}/in"), None).is_err(), "not inside another");
        // Snapshots of a test folder on btrfs are read-only: clean up the way the program prunes.
        let dir = Path::new(&folder);
        // Once each: a snapshot that cannot be deleted must not keep the test going round.
        for n in crate::folder::snapshots(dir) {
            let at = dir.with_file_name(".up.snapshots").join(&n);
            let _ = std::process::Command::new("btrfs").args(["property", "set", "-ts"]).arg(&at).args(["ro", "false"]).status();
            let _ = std::process::Command::new("btrfs").args(["subvolume", "delete"]).arg(&at).output();
        }
        let _ = crate::folder::discard(dir, true);
        let _ = fs::remove_dir_all(&host.data.root);
    }

    fn change(p: &Path, f: impl Fn(&str) -> String) { let t = text(p); fs::write(p, f(&t)).unwrap(); }
    fn edit(st: &Store, f: impl Fn(&str) -> String, what: &str) -> Value {
        let p = st.checkout("t", "spec", "s1").unwrap();
        change(&p, f);
        st.put("t", "spec", "s1", what, None, "").unwrap()
    }
    fn append(p: &Path, t: &str) { change(p, |x| format!("{x}{t}")); }

    #[test]
    fn bad_new_projects_are_refused() {
        let st = setup();
        assert!(st.create_project("t", "x", "", "", vec![], false).is_err(), "same id");
        assert!(st.create_project("n", "x", "", "nope", vec![], false).is_err(), "no parent");
    }

    #[test]
    fn put_records_changed_section() {
        let st = setup();
        let r = edit(&st, |t| format!("{t}\n## 9 new section\ncontent\n"), "new section");
        assert_eq!(r["paths"], json!(["9 new section"]));
        assert!(st.document("t", "spec").contains("## 9 new section"));
    }

    #[test]
    fn put_needs_a_summary_and_an_unchanged_copy_records_nothing() {
        let st = setup();
        let p = st.checkout("t", "spec", "s1").unwrap();
        append(&p, "\nadded\n");
        assert!(st.put("t", "spec", "s1", "", None, "").is_err());
        let q = st.checkout("t", "spec", "s2").unwrap();
        assert!(q.exists());
        assert_eq!(st.put("t", "spec", "s2", "as it is", None, "").unwrap()["unchanged"], json!(true));
    }

    #[test]
    fn put_fails_when_document_changed_since_checkout() {
        let st = setup();
        let late = st.checkout("t", "spec", "s1").unwrap();
        let first = st.checkout("t", "spec", "s2").unwrap();
        append(&first, "\nother session\n");
        st.put("t", "spec", "s2", "other session", None, "").unwrap();
        append(&late, "\nlate fix\n");
        assert!(st.put("t", "spec", "s1", "late fix", None, "").is_err());
    }

    #[test]
    fn removing_and_renaming_go_through_at_once() {
        let st = setup();
        edit(&st, |t| format!("{t}\n## 9 new chapter\n### 9.1 first section\nbody one\n### 9.2 second section\nbody two\n"), "new chapter");
        assert!(edit(&st, |t| t.replace("### 9.2 second section", "### 9.3 fixed name"), "name only changed").get("committed").is_some());
        assert!(edit(&st, |t| t.replace("### 9.1 first section\nbody one\n", ""), "section removed").get("committed").is_some());
        assert!(!st.document("t", "spec").contains("9.1 first section"));
    }

    #[test]
    fn the_project_map_is_recorded() {
        let st = setup();
        st.map_put("t", &json!({"areas":[{"id":"ar","title":"Area"}],"items":[{"id":"i1","area":"ar","text":"item","stage":"idea","origin":"node"}]}), "s1").unwrap();
        assert_eq!(st.project_map("t")["items"][0]["origin"], json!([{"label":"node"}]));
        assert!(st.recent("t", None).iter().any(|c| c.get("itemEvent").is_some() && s(c, "doc") == "map"));
    }

    #[test]
    fn map_rules() {
        let st = setup();
        assert!(st.map_put("t", &json!({"areas":[{"id":"x"}]}), "s1").is_err(), "a new area needs a title");
        st.map_put("t", &json!({"areas":[{"id":"ar","title":"Area"}],"items":[{"id":"a","area":"ar","text":"upper","stage":"idea"},{"id":"b","area":"ar","parent":"a","text":"lower","stage":"doing"}]}), "s1").unwrap();
        assert!(st.map_put("t", &json!({"items":[{"id":"c","area":"ar","text":"text","stage":"nope"}]}), "s1").is_err());
        assert!(st.map_put("t", &json!({"items":[{"id":"a","include":"nope"}]}), "s1").is_err());
        let r = st.map_put("t", &json!({"remove":["a"]}), "s1").unwrap();
        assert_eq!(r["removed"], json!(2), "children go along");
    }

    #[test]
    fn change_detail_shows_before_and_after() {
        let st = setup();
        edit(&st, |t| format!("{t}\n## 9 new section\nfirst\n"), "new section");
        edit(&st, |t| t.replace("first", "later"), "fixed");
        let last = st.recent("t", None)[0].clone();
        let d = st.change_detail("t", s(&last, "id"), "spec").unwrap();
        assert!(s(&d[0], "before").contains("first"));
        assert!(s(&d[0], "after").contains("later"));
    }

    #[test]
    fn a_project_goes_under_another() {
        let st = setup();
        st.create_project("c", "Child", "", "", vec![], false).unwrap();
        st.move_project("c", "t").unwrap();
        assert_eq!(st.children("t").iter().map(|p| s(p, "id").to_string()).collect::<Vec<_>>(), ["c"]);
        assert_eq!(st.tree().iter().map(|(p, d)| (s(p, "id").to_string(), *d)).collect::<Vec<_>>(), [("t".to_string(), 0), ("c".to_string(), 1)]);
        assert!(st.move_project("t", "c").is_err(), "not under its own descendant");
        assert!(st.move_project("c", "c").is_err());
        st.move_project("c", "").unwrap();
        assert_eq!(s(&st.get_project("c").unwrap(), "parent"), "");
    }

    #[test]
    fn a_project_is_found_by_the_longest_folder() {
        let st = scratch("cwd");
        st.create_project("a", "A", "", "", vec!["/w".into()], false).unwrap();
        st.create_project("b", "B", "", "", vec!["/w/x/".into()], false).unwrap();
        assert_eq!(s(&st.project_for_cwd("/w/x/y").unwrap(), "id"), "b");
        assert_eq!(s(&st.project_for_cwd("/w/z").unwrap(), "id"), "a");
        assert!(st.project_for_cwd("/wz").is_none());
    }

    #[test]
    fn work_item_is_taken_once() {
        let st = setup();
        let q = st.queue_add("t", "reflect", "node", "s1").unwrap();
        assert_eq!(st.queue_take("t", &q, "s1").unwrap(), "taken");
        assert_eq!(st.queue_take("t", &q, "s1").unwrap(), "taken", "the same session asking again");
        assert_eq!(st.queue_take("t", &q, "s2").unwrap(), "already");
        assert!(st.queue_done("t", &q, "s2", "").is_err(), "not its item");
        st.queue_done("t", &q, "s1", "reflected").unwrap();
        assert_eq!(st.queue_take("t", &q, "s3").unwrap(), "already", "a replayed request after it was done");
        assert_eq!(s(&st.queue("t", None)[0], "status"), "done");
        assert_eq!(st.queue_take("t", "none", "s3").unwrap(), "missing");
    }

    #[test]
    fn stale_take_can_be_taken_again() {
        let st = setup();
        let q = st.queue_add("t", "reflect", "node", "s1").unwrap();
        st.queue_take("t", &q, "s1").unwrap();
        let path = st.pdir("t").join("queue").join(format!("{q}.json"));
        let mut item = read(&path).unwrap();
        item["takenAt"] = json!(item["takenAt"].as_i64().unwrap() - (crate::queue::TAKE_STALE_MIN + 1) * 60_000);
        write_json(&path, &item).unwrap();
        assert_eq!(st.queue_take("t", &q, "s2").unwrap(), "taken");
    }
}
