//! Project document add-ons (`addons/project-docs/<id>/`): a real document is the template. Everything becomes Markdown
//! when it comes in, and from there on there is one way of working; the original stays beside it. A project starts
//! without documents, and they are added one at a time.
use std::{fs, path::{Path, PathBuf}, process::{Command, Stdio}};

use serde_json::json;

use crate::{Res, Store, arr, s, write_json};

const WORDS: [&str; 4] = ["docx", "doc", "odt", "rtf"];
const SHEETS: [&str; 3] = ["xlsx", "xls", "ods"];

#[derive(Clone, Debug, PartialEq)]
pub struct DocAddon { pub id: String, pub name: String, pub md: PathBuf, pub original: Option<PathBuf> }

use wb_core::addons::valid_id;
fn ext(p: &Path) -> String { p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default() }
fn convertible(p: &Path) -> bool { let e = ext(p); WORDS.contains(&e.as_str()) || SHEETS.contains(&e.as_str()) }

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
    v.sort();
    v
}

fn folders(data: &wb_core::paths::Data) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = fs::read_dir(data.root.join("addons/project-docs")).into_iter().flatten().flatten().filter(|e| e.path().is_dir())
        .map(|e| (e.file_name().to_string_lossy().to_string(), e.path())).filter(|(id, _)| valid_id(id)).collect();
    v.sort();
    v
}

/// The templates there are now: those with their Markdown.
pub fn list(data: &wb_core::paths::Data) -> Vec<DocAddon> {
    folders(data).into_iter().filter_map(|(id, dir)| {
        let all = files(&dir);
        let md = all.iter().find(|p| ext(p) == "md")?.clone();
        let name = wb_core::i18n::name_in_language(&format!("doc-{id}")).unwrap_or_else(|| md.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        Some(DocAddon { id, name, original: all.iter().find(|p| convertible(p)).cloned(), md })
    }).collect()
}

/// Originals that have no Markdown yet, and no failed attempt on file.
pub fn waiting(data: &wb_core::paths::Data) -> Vec<PathBuf> {
    folders(data).into_iter().filter_map(|(_, dir)| {
        let all = files(&dir);
        if all.iter().any(|p| ext(p) == "md" || p.to_string_lossy().ends_with(".error.txt")) { return None; }
        all.into_iter().find(|p| convertible(p))
    }).collect()
}

/// One sheet as a section with a table.
pub fn sheet_markdown(sheet: Option<&str>, rows: &[Vec<String>]) -> String {
    let rows: Vec<&Vec<String>> = rows.iter().filter(|r| r.iter().any(|c| !c.trim().is_empty())).collect();
    let mut out = String::new();
    if let Some(n) = sheet { out += &format!("## {n}\n\n"); }
    let Some(width) = rows.iter().map(|r| r.len()).max() else { return out };
    let line = |r: &Vec<String>| {
        let cells: Vec<String> = (0..width).map(|i| r.get(i).map_or(String::new(), |c| c.trim().replace('|', "\\|").replace('\n', " "))).collect();
        format!("| {} |\n", cells.join(" | "))
    };
    out += &line(rows[0]);
    out += &format!("|{}\n", "---|".repeat(width));
    for r in &rows[1..] { out += &line(r); }
    out + "\n"
}

/// A plain CSV reader: quotes, doubled quotes, line breaks inside quotes.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let (mut rows, mut row, mut cell, mut quoted) = (Vec::new(), Vec::new(), String::new(), false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => { cell.push('"'); chars.next(); }
            ('"', _) => quoted = !quoted,
            (',', false) => row.push(std::mem::take(&mut cell)),
            ('\n', false) => { row.push(std::mem::take(&mut cell)); rows.push(std::mem::take(&mut row)); }
            ('\r', false) => {}
            (o, _) => cell.push(o),
        }
    }
    if !cell.is_empty() || !row.is_empty() { row.push(cell); rows.push(row); }
    rows
}

/// Tables whose rows differ in length: what a merged cell turns into. Lines of those rows.
pub fn uneven_tables(md: &str) -> usize {
    let cells = |l: &str| l.replace("\\|", "").matches('|').count();
    let (mut bad, mut width) = (0, None);
    for line in md.lines() {
        if line.trim_start().starts_with('|') {
            let n = cells(line);
            match width { None => width = Some(n), Some(w) if w != n => bad += 1, _ => {} }
        } else { width = None; }
    }
    bad
}

fn soffice(profile: &Path, args: &[&str]) -> Result<(), String> {
    let done = Command::new("soffice").arg(format!("-env:UserInstallation=file://{}", profile.display())).arg("--headless").args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    match done { Ok(s) if s.success() => Ok(()), Ok(s) => Err(wb_core::i18n::f("libreoffice-failed", &[("s", (s).to_string())])), Err(e) => Err(wb_core::i18n::f("cannot-run-libreoffice", &[("e", (e).to_string())])) }
}

/// Makes the Markdown beside the original. The number of uneven table rows it has.
pub fn convert(data: &wb_core::paths::Data, original: &Path) -> Result<usize, String> {
    let dir = original.parent().ok_or(wb_core::i18n::t("no-folder"))?;
    let stem = original.file_stem().ok_or(wb_core::i18n::t("no-file-name"))?.to_string_lossy().to_string();
    let work = data.cache().join("doc-convert");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let (profile, out) = (work.join("profile"), work.join("out"));
    let (out_s, file_s) = (out.to_string_lossy().to_string(), original.to_string_lossy().to_string());
    let text = if WORDS.contains(&ext(original).as_str()) {
        soffice(&profile, &["--convert-to", "md", "--outdir", &out_s, &file_s])?;
        fs::read_to_string(out.join(format!("{stem}.md"))).map_err(|e| wb_core::i18n::f("no-conversion-output", &[("e", (e).to_string())]))?
    } else {
        // Every sheet to a file of its own, UTF-8.
        soffice(&profile, &["--convert-to", "csv:Text - txt - csv (StarCalc):44,34,76,1,,0,false,true,false,false,false,-1", "--outdir", &out_s, &file_s])?;
        let mut sheets: Vec<PathBuf> = files(&out).into_iter().filter(|p| ext(p) == "csv").collect();
        sheets.sort();
        if sheets.is_empty() { return Err(wb_core::i18n::t("no-conversion-output-2").into()); }
        let mut text = format!("# {stem}\n\n");
        for f in sheets {
            let sheet = f.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let sheet = sheet.strip_prefix(&format!("{stem}-")).unwrap_or(&sheet).to_string();
            text += &sheet_markdown(Some(&sheet), &parse_csv(&fs::read_to_string(&f).map_err(|e| e.to_string())?));
        }
        text
    };
    let _ = fs::remove_dir_all(&work);
    wb_core::paths::write_atomic(&dir.join(format!("{stem}.md")), &text).map_err(|e| e.to_string())?;
    Ok(uneven_tables(&text))
}

/// Records why a conversion failed, beside the original, so it is not tried at every look.
pub fn note_failure(original: &Path, why: &str) {
    let _ = fs::write(original.with_extension("error.txt"), why);
}

impl Store {
    /// Puts a template in the project as one of its documents. From here on only the AI writes it.
    pub fn add_doc(&self, pid: &str, addon: &DocAddon) -> Res<()> {
        let mut p = self.get_project(pid).ok_or("no project")?;
        if arr(&p, "docs").iter().any(|d| s(d, "id") == addon.id) { return Err(wb_core::i18n::f("document-already-exists", &[("a1", (addon.name).to_string())])); }
        let text = fs::read_to_string(&addon.md).map_err(|e| e.to_string())?;
        let mut doc = json!({"id": addon.id, "addon": addon.id, "name": addon.name});
        if let Some(o) = &addon.original { doc["original"] = json!(o.to_string_lossy()); }
        let mut docs = arr(&p, "docs").to_vec();
        docs.push(doc);
        p["docs"] = json!(docs);
        write_json(&self.pdir(pid).join("project.json"), &p)?;
        self.commit_docs(pid, vec![json!({"doc": addon.id, "what": wb_core::i18n::f("document-added", &[("a1", (addon.name).to_string())]), "text": text})], "", Some(json!({"system": "add-doc"})), "")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch;
    fn put(st: &Store, id: &str, file: &str, text: &str) -> PathBuf {
        let dir = st.data.root.join("addons/project-docs").join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(file), text).unwrap();
        dir.join(file)
    }
    #[test]
    fn a_folder_with_markdown_is_a_template_and_its_original_stays() {
        let st = scratch("da");
        put(&st, "wbs", "WBS.md", "# WBS\n");
        let word = put(&st, "req", "requirements.docx", "x");
        put(&st, "req", "requirements.md", "# Requirements\n");
        let only = put(&st, "only", "sheet.xlsx", "x");
        put(&st, "other", "notes.txt", "x");
        let all = list(&st.data);
        assert_eq!(all.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), vec!["req", "wbs"]);
        assert_eq!(all[0].name, "requirements");
        assert_eq!(all[0].original.as_ref(), Some(&word));
        assert_eq!(waiting(&st.data), vec![only.clone()], "an original with no Markdown waits for conversion");
        note_failure(&only, "failed");
        assert!(waiting(&st.data).is_empty(), "a failure is not retried at every look");
    }
    #[test]
    fn adding_makes_one_document_and_records_it() {
        let st = scratch("db");
        st.create_project("p", "Test", "", "", Vec::new(), false).unwrap();
        assert!(arr(&st.get_project("p").unwrap(), "docs").is_empty(), "a project starts with no documents");
        put(&st, "wbs", "WBS.md", "# WBS\n\n## 1. Scope\n");
        let a = list(&st.data).remove(0);
        st.add_doc("p", &a).unwrap();
        let p = st.get_project("p").unwrap();
        assert_eq!(arr(&p, "docs").len(), 1);
        assert_eq!(s(&arr(&p, "docs")[0], "name"), "WBS");
        assert!(st.document("p", "wbs").contains("## 1. Scope"));
        assert!(st.add_doc("p", &a).is_err(), "only once");
        fs::remove_dir_all(a.md.parent().unwrap()).unwrap();
        assert!(st.document("p", "wbs").contains("## 1. Scope"), "the document stays when its template goes");
        assert_eq!(st.recent("p", None).len(), 1);
    }
    #[test]
    fn sheets_become_tables() {
        let rows = parse_csv("no,task,\"ow\"\"ner\"\r\n1,\"a,b\",\r\n,,\r\n1.1,\"two\nlines\",x|y\r\n");
        assert_eq!(rows[1][1], "a,b");
        let md = sheet_markdown(Some("Sheet"), &rows);
        assert_eq!(md, "## Sheet\n\n| no | task | ow\"ner |\n|---|---|---|\n| 1 | a,b |  |\n| 1.1 | two lines | x\\|y |\n\n");
        assert_eq!(uneven_tables(&md), 0);
    }
    #[test]
    fn merged_cells_are_noticed() {
        assert_eq!(uneven_tables("| a | b | c |\n|---|---|---|\n| 1 | 2 | 3 |\n| merged | 4 |\n\n| x |\n|---|\n"), 1);
    }
}
