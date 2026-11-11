//! The agent side of the command: `whaybench map` (the conversation map, for every agent),
//! `whaybench hook codex` and `whaybench --agent codex`.
use std::{io, path::PathBuf};

use wb_core::{codex, i18n, map, paths::{self, Data}, session_record as rec};

pub fn map(data: &Data, words: &[String]) -> Result<(), String> {
    let input = || serde_json::from_reader::<_, serde_json::Value>(io::stdin().lock()).map_err(|e| e.to_string());
    let map = match words {
        [id] => map::command(&data.root, id, None),
        [id, w] if w == "--write" => map::command(&data.root, id, Some(input()?)),
        [id, w] if w == "--take-over" => map::take_over(&data.root, id, &input()?),
        [id, w] if w == "--start-from" => map::start_from(&data.root, id, &input()?),
        [id, w] if w == "--end-turn" => map::end_turn(&data.root, id),
        _ => return Err("map SESSION [--write | --take-over | --start-from | --end-turn]".into()),
    };
    println!("{}", map.map_err(|e| e.to_string())?);
    Ok(())
}

/// `whaybench record SESSION ...`: the session's record, state, heartbeat and answers (wb_core::session_record).
pub fn record(data: &Data, words: &[String]) -> Result<(), String> {
    let usage = "record SESSION touch [--prompt TEXT] [--tab TAB] [--cwd DIR] | state working|waiting|done [--note TEXT] | alive | off | answer < ANSWER.json | moved TO [--empty-carried ID] | carry-from OLD | file read|edit|attach PATH [--node ID]";
    let (Some(id), Some(op)) = (words.first(), words.get(1)) else { return Err(usage.into()) };
    let rest = &words[2..];
    // --name VALUE pairs after the words an operation takes.
    let opt = |from: usize, name: &str| -> Result<Option<&str>, String> {
        let mut it = rest[from.min(rest.len())..].iter();
        let mut found = None;
        while let Some(k) = it.next() {
            if !k.starts_with("--") { return Err(usage.into()); }
            let v = it.next().ok_or(usage)?;
            if k == name { found = Some(v.as_str()); }
        }
        Ok(found)
    };
    let root = &data.root;
    let done = match op.as_str() {
        "touch" => rec::touch(root, id, rec::Touch { prompt: opt(0, "--prompt")?, tab: opt(0, "--tab")?, cwd: opt(0, "--cwd")?, agent: None }).map(|_| ()),
        "state" => { let s = rest.first().ok_or(usage)?; rec::state(root, id, s, opt(1, "--note")?) }
        "alive" | "off" if rest.is_empty() => rec::alive(root, id, op == "alive"),
        "answer" if rest.is_empty() => {
            let input = serde_json::from_reader(io::stdin().lock()).map_err(|e| e.to_string())?;
            rec::answer(root, id, &input).map(|_| ())
        }
        "moved" => { let to = rest.first().ok_or(usage)?; rec::moved(root, id, to, opt(1, "--empty-carried")?) }
        "carry-from" if rest.len() == 1 => rec::carry_from(root, id, &rest[0]),
        "file" if rest.len() >= 2 => wb_core::memory::file_touched(root, id, &rest[0], &rest[1], opt(2, "--node")?.filter(|n| !n.is_empty())),
        _ => return Err(usage.into()),
    };
    done.map_err(|e| e.to_string())
}

pub fn hook(data: &Data, words: &[String]) -> Result<(), String> {
    if words != ["codex"] { return Err("hook codex".into()); }
    let event = serde_json::from_reader(io::stdin().lock()).map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let tab = std::env::var("WHAYBENCH_TAB").ok();
    println!("{}", codex::hook(&data.root, &event, tab.as_deref(), &exe).map_err(|e| e.to_string())?);
    Ok(())
}

/// Before anything starts: Codex's folder is made ready and everything this run starts is told the agent.
pub fn start_codex(data: &Data) -> Result<(), String> {
    let source = std::env::var_os("CODEX_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".codex"))).ok_or("HOME is not set")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let home = codex::prepare(&data.root, &source, &exe).map_err(|e| e.to_string())?;
    // No other thread exists yet.
    unsafe {
        std::env::set_var("CODEX_HOME", home);
        std::env::set_var(codex::ENV_AGENT, "codex");
        std::env::set_var(paths::ENV_DATA_DIR, &data.root);
    }
    println!("{}", i18n::t("codex-hooks"));
    Ok(())
}
