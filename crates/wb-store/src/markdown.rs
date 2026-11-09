//! Markdown structure of a project document: headings, heading paths, sections.
use std::{collections::BTreeSet, sync::LazyLock};

use regex::Regex;
use similar::{Algorithm, DiffTag, capture_diff_slices};

static HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(#{1,6})\s+(.*?)\s*#*\s*$").unwrap());
/// Marks a path that was removed.
pub const DELETED: &str = " (deleted)";
pub fn without_mark(path: &str) -> String { path.replace(DELETED, "") }


pub type Head = (usize, usize, String);

/// (line index, level, title), outside code fences.
pub fn headings(text: &str) -> Vec<Head> {
    let (mut out, mut fence) = (Vec::new(), false);
    for (i, line) in text.split('\n').enumerate() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") { fence = !fence; continue; }
        if fence { continue; }
        if let Some(m) = HEAD.captures(line) { out.push((i, m[1].len(), m[2].to_string())); }
    }
    out
}

/// The heading path a line sits under, the document title left out.
pub fn path_at(heads: &[Head], line: usize) -> Vec<String> {
    let mut chain: Vec<(usize, &str)> = Vec::new();
    for (i, level, title) in heads {
        if *i > line { break; }
        chain.retain(|c| c.0 < *level);
        chain.push((*level, title));
    }
    chain.iter().filter(|c| c.0 > 1).map(|c| c.1.to_string()).collect()
}

fn joined(heads: &[Head], line: usize) -> String { path_at(heads, line).join(" > ") }

pub fn all_paths(text: &str) -> BTreeSet<String> {
    let heads = headings(text);
    heads.iter().filter(|h| h.1 > 1).map(|h| joined(&heads, h.0)).collect()
}

/// Heading paths touched by the change, in document order.
pub fn changed_paths(old: &str, new: &str) -> Vec<String> {
    let (a, b): (Vec<&str>, Vec<&str>) = (old.split('\n').collect(), new.split('\n').collect());
    let (ha, hb) = (headings(old), headings(new));
    let mut out: Vec<String> = Vec::new();
    let mut add = |p: String| if !out.contains(&p) { out.push(p); };
    for op in capture_diff_slices(Algorithm::Myers, &a, &b) {
        let (tag, r1, r2) = op.as_tag_tuple();
        if tag == DiffTag::Equal { continue; }
        if b[r2.clone()].iter().any(|l| !l.trim().is_empty()) {
            for j in r2.filter(|j| !b[*j].trim().is_empty()) { add(joined(&hb, j)); }
        } else if a[r1.clone()].iter().any(|l| !l.trim().is_empty()) {
            add(format!("{}{DELETED}", joined(&ha, r1.start)));
        }
    }
    out
}

fn section_at(text: &str, heads: &[Head], k: usize) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let (i, lv) = (heads[k].0, heads[k].1);
    let end = heads[k + 1..].iter().find(|h| h.1 <= lv).map(|h| h.0).unwrap_or(lines.len());
    lines[i..end].join("\n").trim().to_string()
}

/// The section at heading path 'A > B > C', its heading included, or '' when it is not there.
pub fn section_text(text: &str, path: &str) -> String {
    let heads = headings(text);
    (0..heads.len()).find(|k| joined(&heads, heads[*k].0) == path).map(|k| section_at(text, &heads, k)).unwrap_or_default()
}

/// A section without its heading line: what stays when only the heading is renamed.
fn body(text: &str, path: &str) -> String {
    section_text(text, path).split('\n').skip(1).collect::<Vec<_>>().join("\n").trim().to_string()
}

fn parent(path: &str) -> &str { path.rfind(" > ").map(|i| &path[..i]).unwrap_or("") }

/// (paths really gone, [old, new] renamed). A path that disappeared counts as renamed when a path that appeared
/// holds the same body; each new path stands for one old path.
pub fn removed_paths(old: &str, new: &str) -> (Vec<String>, Vec<(String, String)>) {
    let (pa, pb) = (all_paths(old), all_paths(new));
    let mut free: Vec<(String, String)> = pb.difference(&pa).map(|a| (a.clone(), body(new, a))).collect();
    let (mut really, mut renamed) = (Vec::new(), Vec::new());
    for g in pa.difference(&pb) {
        let b = body(old, g);
        let hit = free.iter().position(|(a, x)| *x == b && (!b.is_empty() || parent(g) == parent(a)));
        match hit {
            Some(k) => renamed.push((g.clone(), free.remove(k).0)),
            None => really.push(g.clone()),
        }
    }
    (really, renamed)
}

#[cfg(test)]
mod tests {
    use super::*;
    const DOC: &str = "# Title\n\n## 1 Chapter\ntext\n### 1.1 Section\ncontent\n```\n## in code\n```\n## 2 Next chapter ##\nend";
    #[test]
    fn headings_outside_code_with_paths() {
        let h = headings(DOC);
        assert_eq!(h.iter().map(|x| x.2.as_str()).collect::<Vec<_>>(), ["Title", "1 Chapter", "1.1 Section", "2 Next chapter"]);
        assert_eq!(path_at(&h, 5), ["1 Chapter", "1.1 Section"]);
        assert_eq!(path_at(&h, 0), Vec::<String>::new());
    }
    #[test]
    fn a_section_ends_at_the_next_of_its_level() {
        assert_eq!(section_text(DOC, "1 Chapter > 1.1 Section"), "### 1.1 Section\ncontent\n```\n## in code\n```");
        assert!(section_text(DOC, "1 Chapter").starts_with("## 1 Chapter\ntext\n### 1.1 Section"));
        assert_eq!(section_text(DOC, "none"), "");
    }
    #[test]
    fn changed_paths_name_the_place() {
        assert_eq!(changed_paths("# t\n## 1 a\nx\n", "# t\n## 1 a\ny\n"), ["1 a"]);
        assert_eq!(changed_paths("# t\n## 1 a\nx\n## 2 b\ny\n", "# t\n## 1 a\nx\n"), ["2 b (deleted)"]);
    }
    #[test]
    fn a_rename_is_not_a_removal() {
        let (gone, ren) = removed_paths("## a\nbody\n## b\ntwo\n", "## a2\nbody\n## b\ntwo\n");
        assert!(gone.is_empty());
        assert_eq!(ren, [("a".to_string(), "a2".to_string())]);
        assert_eq!(removed_paths("## a\nbody\n", "## b\nother text\n").0, ["a"]);
    }
}
