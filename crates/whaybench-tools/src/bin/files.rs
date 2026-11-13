//! Attaches files to a map node, so the node lists them even if Claude has not touched them yet.
use std::{collections::BTreeSet, path::Path};

use serde_json::{Map, Value, json};
use wb_core::{paths::{self, Data}, sessions};

const HELP: &str = "Attaches files to a map node, so the node lists them even if Claude has not touched them yet.

  whaybench-files attach NODE PATTERN...    add paths or glob patterns (** for subfolders) to NODE
  whaybench-files detach NODE [PATTERN...]  remove those patterns, or all of NODE's
  whaybench-files list [NODE]               patterns per node, with how many files each matches
  --session ID   work on that session's map instead of the current one";

fn absolute(p: &str) -> String {
    if p.starts_with('/') { return p.to_string(); }
    std::env::current_dir().unwrap_or_default().join(p).to_string_lossy().to_string()
}

fn matches(pattern: &str) -> usize {
    if !pattern.contains(['*', '?', '[']) { return usize::from(Path::new(pattern).exists()); }
    glob::glob(pattern).map(|g| g.flatten().count()).unwrap_or(0)
}

fn run(mut args: Vec<String>, data: &Data) -> Result<(), String> {
    let mut session = None;
    if let Some(i) = args.iter().position(|a| a == "--session") {
        let id = args.get(i + 1).ok_or("--session needs an id")?.clone();
        session = Some(sessions::current_id(data, &id));
        args.drain(i..=i + 1);
    }
    let session = session.or_else(|| sessions::target(data)).ok_or(wb_core::i18n::t("no-session"))?;
    let path = data.root.join("node-files").join(format!("{session}.json"));
    let mut table: Map<String, Value> = paths::read_kept(&path);
    let listed = |v: Option<&Value>| -> Vec<String> { v.and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default() };
    match args.first().map(String::as_str) {
        Some("attach") if args.len() >= 3 => {
            let mut set: BTreeSet<String> = listed(table.get(&args[1])).into_iter().collect();
            set.extend(args[2..].iter().map(|p| absolute(p)));
            for p in &args[2..] { let _ = wb_core::memory::file_touched(&data.root, &session, "attach", &absolute(p), Some(&args[1])); }
            table.insert(args[1].clone(), json!(set));
        }
        Some("detach") if args.len() >= 2 => {
            let drop = &args[2..];
            let kept: Vec<String> = listed(table.get(&args[1])).into_iter().filter(|p| !drop.is_empty() && !drop.contains(p)).collect();
            if kept.is_empty() { table.shift_remove(&args[1]); } else { table.insert(args[1].clone(), json!(kept)); }
        }
        Some("list") => {
            for (node, patterns) in &table {
                if args.get(1).is_some_and(|n| n != node) { continue; }
                println!("{node}");
                for p in listed(Some(patterns)) { println!("  {p}  ({})", matches(&p)); }
            }
            return Ok(());
        }
        _ => return Err(HELP.into()),
    }
    paths::write_atomic(&path, serde_json::to_string_pretty(&Value::Object(table)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

fn main() {
    paths::private_files();
    let Some(root) = paths::resolve(None, |k| std::env::var(k).ok()) else { eprintln!("error: no data directory"); std::process::exit(1) };
    if let Err(e) = run(std::env::args().skip(1).collect(), &Data::new(root)) { eprintln!("{e}"); std::process::exit(1); }
}
