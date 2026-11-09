//! Projects: create, find by folder, and the tree of projects.
use std::collections::HashSet;

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
