//! Agents other than Claude Code are add-ons, `addons/agents/<id>/` (addons/agents/README.md). They run no Claude
//! Code mod, so their hooks call `whaybench hook <id>`, which writes the same shared files the mod writes: the session
//! record, its state, the heartbeat, the answer, and the map turn.
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use serde::Deserialize;
use serde_json::Value;

use crate::map::{Result, write_json};

/// Set for everything a run with an add-on agent starts: the terminal relay, the panels and the hooks.
pub const ENV_AGENT: &str = "WHAYBENCH_AGENT";
/// The agent in the program, not an add-on.
pub const CLAUDE: &str = "claude";

#[derive(Deserialize)]
pub struct Agent {
    #[serde(skip)]
    pub id: String,
    #[serde(skip)]
    dir: PathBuf,
    pub name: String,
    pub command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    resume: Vec<String>,
    home: Option<Home>,
    #[serde(default)]
    merge: BTreeMap<String, String>,
    #[serde(default)]
    write: BTreeMap<String, String>,
    context: Option<String>,
    pub trust: Option<String>,
    pub usage: Option<String>,
}

#[derive(Deserialize)]
struct Home {
    env: String,
    from: String,
    #[serde(default)]
    copy: Vec<String>,
    #[serde(default)]
    copy_keys: BTreeMap<String, Vec<String>>,
}

/// The add-on agent this run was started with; none is Claude Code.
pub fn active() -> Option<String> {
    std::env::var(ENV_AGENT).ok().filter(|a| !a.is_empty() && a != CLAUDE)
}

/// Whether this run's agent is an add-on: none of them has Claude Code's background sessions, `claude agents` or transcripts.
pub fn other() -> bool { active().is_some() }

/// The agents a new conversation can start with: Claude Code, then the add-ons that load, as (id, name).
pub fn list(data: &Path) -> Vec<(String, String)> {
    let mut ids: Vec<String> = fs::read_dir(data.join("addons/agents")).into_iter().flatten().flatten()
        .filter(|e| e.path().is_dir()).filter_map(|e| e.file_name().into_string().ok()).collect();
    ids.sort();
    std::iter::once((CLAUDE.to_string(), CLAUDE_NAME.to_string()))
        .chain(ids.into_iter().filter_map(|id| load(data, &id).ok().map(|a| (id, a.name)))).collect()
}

const CLAUDE_NAME: &str = "Claude Code";

/// The name people see for a session's agent (its record's `agent`); an add-on since deleted shows its id.
pub fn name_of(data: &Path, agent: Option<&str>) -> String {
    match agent {
        None => CLAUDE_NAME.into(),
        Some(id) => load(data, id).map(|a| a.name).unwrap_or_else(|_| id.into()),
    }
}

/// A session as people tell sessions apart: its agent's name and the start of its number, `Codex 01a1d60c`.
pub fn session_label(data: &Path, sid: &str) -> String {
    let agent = crate::map::read_json(&data.join("sessions").join(format!("{sid}.json")), Value::Null).ok()
        .and_then(|r| r["agent"].as_str().map(String::from));
    format!("{} {}", name_of(data, agent.as_deref()), sid.chars().take(8).collect::<String>())
}

pub fn load(data: &Path, id: &str) -> Result<Agent> {
    if id == CLAUDE || !crate::addons::valid_id(id) { return Err(format!("not an agent add-on: {id}").into()); }
    let dir = data.join("addons/agents").join(id);
    let text = fs::read_to_string(dir.join("agent.json")).map_err(|e| format!("agent add-on {id}: {e}"))?;
    let mut agent: Agent = serde_json::from_str(&text).map_err(|e| format!("agent add-on {id}: {e}"))?;
    // Files an add-on names stay inside its folder and its settings folder.
    let names = agent.merge.iter().chain(&agent.write).flat_map(|(a, b)| [a, b])
        .chain(&agent.context).chain(agent.home.iter().flat_map(|h| h.copy.iter().chain(h.copy_keys.keys())));
    for n in names {
        if n.is_empty() || n.contains('/') || n.starts_with('.') { return Err(format!("agent add-on {id}: bad file name {n}").into()); }
    }
    agent.id = id.into();
    agent.dir = dir;
    Ok(agent)
}

pub fn shell_quote(path: &Path) -> Result<String> {
    let path = path.to_str().ok_or("command path is not UTF-8")?;
    Ok(format!("'{}'", path.replace('\'', "'\\''")))
}

impl Agent {
    /// The program's arguments; a session coming back (`--resume ID` or `attach ID`) is the add-on's `resume`.
    pub fn args(&self, args: &[String]) -> Vec<String> {
        let mut out = self.args.clone();
        match args {
            [flag, id] if !self.resume.is_empty() && (flag == "--resume" || flag == "attach") =>
                out.extend(self.resume.iter().map(|a| a.replace("{session}", id))),
            _ => out.extend(args.iter().cloned()),
        }
        out
    }

    /// What the agent is told at session start and with each prompt.
    pub fn context(&self, session: &str) -> Result<Option<String>> {
        let Some(file) = &self.context else { return Ok(None) };
        Ok(Some(fs::read_to_string(self.dir.join(file))?.trim_end().replace("{session}", session)))
    }

    /// The agent's own settings folder `<data>/<id>`, made ready before anything starts. Logins and choices are copied
    /// from the person's folder once; the templates are written each time. Returns the variable and folder to set.
    pub fn prepare(&self, data: &Path, exe: &Path) -> Result<Option<(String, PathBuf)>> {
        self.prepare_with(data, exe, self.home.as_ref().and_then(|h| std::env::var_os(&h.env)).map(PathBuf::from))
    }

    /// `own`: the person's folder as the variable names it, when set.
    fn prepare_with(&self, data: &Path, exe: &Path, own: Option<PathBuf>) -> Result<Option<(String, PathBuf)>> {
        let Some(h) = &self.home else { return Ok(None) };
        let home = data.join(&self.id);
        fs::DirBuilder::new().recursive(true).mode(0o700).create(&home)?;
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700))?;
        let source = own.map_or_else(|| expand(&h.from), Ok)?;
        for file in &h.copy {
            if home.join(file).exists() { continue; }
            match fs::read(source.join(file)) {
                Ok(bytes) => create_private(&home.join(file), &bytes)?,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {},
                Err(e) => return Err(e.into()),
            }
        }
        for (file, keys) in &h.copy_keys {
            if !home.join(file).exists() { create_private(&home.join(file), picked(&source.join(file), keys)?.as_bytes())?; }
        }
        let fill = Fill { exe: exe.to_str().ok_or("command path is not UTF-8")?, data: data.to_str().ok_or("data path is not UTF-8")?, hook: format!("{} hook {}", shell_quote(exe)?, self.id) };
        for (file, template) in &self.merge {
            if !file.ends_with(".toml") { return Err(format!("agent add-on {}: merge takes TOML", self.id).into()); }
            let mut add: toml::Value = fs::read_to_string(self.dir.join(template))?.parse::<toml::Table>()?.into();
            fill.toml(&mut add);
            let target = home.join(file);
            if !target.exists() { create_private(&target, b"")?; }
            let mut table: toml::Value = fs::read_to_string(&target)?.parse::<toml::Table>()?.into();
            let before = table.clone();
            merge(&mut table, add);
            if table != before { fs::write(&target, toml::to_string(&table)?)?; }
        }
        for (file, template) in &self.write {
            let text = fs::read_to_string(self.dir.join(template))?;
            if template.ends_with(".toml") {
                let mut v: toml::Value = text.parse::<toml::Table>()?.into();
                fill.toml(&mut v);
                fs::write(home.join(file), toml::to_string(&v)?)?;
            } else {
                let mut v: Value = serde_json::from_str(&text)?;
                fill.json(&mut v);
                write_json(&home.join(file), &v)?;
            }
        }
        Ok(Some((h.env.clone(), home)))
    }
}

fn expand(path: &str) -> Result<PathBuf> {
    match path.strip_prefix("~/") {
        Some(rest) => Ok(PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?).join(rest)),
        None => Ok(PathBuf::from(path)),
    }
}

/// The named keys of a TOML file, as a new file; a missing file gives an empty one.
fn picked(source: &Path, keys: &[String]) -> Result<String> {
    let text = match fs::read_to_string(source) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let source: toml::Table = text.parse()?;
    let config: toml::Table = keys.iter().filter_map(|k| source.get(k).map(|v| (k.clone(), v.clone()))).collect();
    Ok(toml::to_string(&config)?)
}

/// Tables merge key by key; anything else from the template replaces what was there.
fn merge(into: &mut toml::Value, add: toml::Value) {
    match (into, add) {
        (toml::Value::Table(a), toml::Value::Table(b)) => for (k, v) in b {
            match a.get_mut(&k) { Some(old) if old.is_table() && v.is_table() => merge(old, v), _ => { a.insert(k, v); } }
        },
        (into, add) => *into = add,
    }
}

/// Filled after parsing, so a path with quotes cannot break the file.
struct Fill<'a> { exe: &'a str, data: &'a str, hook: String }

impl Fill<'_> {
    fn text(&self, s: &str) -> String { s.replace("{exe}", self.exe).replace("{data}", self.data).replace("{hook}", &self.hook) }
    fn toml(&self, v: &mut toml::Value) {
        match v {
            toml::Value::String(s) => *s = self.text(s),
            toml::Value::Array(a) => a.iter_mut().for_each(|v| self.toml(v)),
            toml::Value::Table(t) => t.iter_mut().for_each(|(_, v)| self.toml(v)),
            _ => {}
        }
    }
    fn json(&self, v: &mut Value) {
        match v {
            Value::String(s) => *s = self.text(s),
            Value::Array(a) => a.iter_mut().for_each(|v| self.json(v)),
            Value::Object(o) => o.values_mut().for_each(|v| self.json(v)),
            _ => {}
        }
    }
}

fn create_private(path: &Path, bytes: &[u8]) -> Result<()> {
    match OpenOptions::new().write(true).create_new(true).mode(0o600).open(path) {
        Ok(mut file) => { file.write_all(bytes)?; Ok(()) },
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::read_json;
    use serde_json::json;

    fn event(name: &str) -> Value {
        json!({"session_id":"codex-1","hook_event_name":name,"cwd":"work"})
    }

    fn call(data: &Path, event: &Value) -> Result<Value> {
        crate::hook::run(data, &crate::hook::Agent { name: "codex", tab: Some("tab1"), inside: true }, event)
    }

    fn with_addons(data: &Path) { crate::addons::install_bundled(&crate::paths::Data::new(data.to_path_buf())); }

    #[test]
    fn invalid_events_and_ids_do_not_write_records() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for id in ["../original", "/tmp/file", "", "a/b", "a.b"] {
            let mut e = event("SessionStart");
            e["session_id"] = json!(id);
            assert!(call(dir.path(), &e).is_err());
        }
        assert!(call(dir.path(), &event("UnknownEvent")).is_err());
        assert!(call(dir.path(), &event("UserPromptSubmit")).is_err());
        assert_eq!(fs::read_dir(dir.path())?.count(), 0);
        Ok(())
    }

    #[test]
    fn preparation_writes_hooks_and_keeps_the_private_login() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let data = dir.path().join("data ' é");
        with_addons(&data);
        let source = dir.path().join("source");
        fs::create_dir_all(&source)?;
        fs::write(source.join("auth.json"), "test-login")?;
        fs::write(source.join("config.toml"), "model = 'chosen-model'\nmodel_reasoning_effort = 'high'\n[plugins.other]\nenabled = true\n")?;
        let exe = dir.path().join("bin/whaybench");
        let codex = load(&data, "codex")?;
        // The person's own CODEX_HOME is where the login is copied from.
        let (var, home) = codex.prepare_with(&data, &exe, Some(source.clone()))?.unwrap();
        assert_eq!((var.as_str(), &home), ("CODEX_HOME", &data.join("codex")));
        assert_eq!(fs::metadata(&home)?.permissions().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(home.join("auth.json"))?.permissions().mode() & 0o777, 0o600);
        let config: toml::Table = fs::read_to_string(home.join("config.toml"))?.parse()?;
        assert_eq!(config["model"].as_str(), Some("chosen-model"));
        let mcp = &config["mcp_servers"]["whaybench"];
        assert_eq!(mcp["command"].as_str(), exe.to_str(), "a path with a quote stays one value");
        assert_eq!((mcp["args"][0].as_str(), mcp["args"][2].as_str(), mcp["env_vars"][0].as_str()), (Some("mcp"), data.to_str(), Some("WHAYBENCH_TAB")), "the map tool is Codex's MCP server");
        assert_eq!(mcp["tools"]["context_map"]["approval_mode"].as_str(), Some("approve"), "kept every turn without asking");
        let roots: Vec<&str> = config["sandbox_workspace_write"]["writable_roots"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
        assert!(roots.contains(&data.join("view").to_str().unwrap()), "Whaybench's commands can write their requests");
        assert!(!roots.iter().any(|r| *r == data.to_str().unwrap() || r.ends_with("/codex") || r.ends_with("/addons") || r.ends_with("/cache")), "not its own settings, the add-ons or other tabs' input");
        assert_eq!(fs::metadata(home.join("config.toml"))?.permissions().mode() & 0o777, 0o600, "still private after the server is added");
        assert_eq!(config.len(), 4, "model, effort, the MCP server and the sandbox; nothing else is copied");
        fs::write(home.join("auth.json"), "refreshed-login")?;
        codex.prepare_with(&data, &exe, Some(source.clone()))?;
        assert_eq!(fs::read_to_string(home.join("auth.json"))?, "refreshed-login");
        let hooks = read_json(&home.join("hooks.json"), Value::Null)?;
        let events = hooks["hooks"].as_object().unwrap();
        assert_eq!(events.len(), 8);
        for (_, h) in events {
            assert_eq!(h[0]["hooks"][0]["command"], format!("{} hook codex", shell_quote(&exe)?));
        }
        Ok(())
    }

    #[test]
    fn corrupt_existing_records_are_reported_and_preserved() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("sessions/codex-1.json");
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, "{broken")?;
        assert!(call(dir.path(), &event("SessionStart")).is_err());
        assert_eq!(fs::read_to_string(path)?, "{broken");
        Ok(())
    }

    #[test]
    fn codex_arguments_keep_the_sandbox_and_resume_natively() -> Result<()> {
        let dir = tempfile::tempdir()?;
        with_addons(dir.path());
        let codex = load(dir.path(), "codex")?;
        let a = codex.args(&["--resume".into(), "sid".into()]);
        assert_eq!(a, ["--no-daemon", "--sandbox", "workspace-write", "resume", "sid"]);
        assert_eq!(codex.args(&[]).len(), 3);
        assert!(!a.iter().any(|s| s.contains("bypass")));
        assert!(codex.context("sid")?.unwrap().contains("Whaybench session sid."));
        Ok(())
    }

    #[test]
    fn claude_and_unsafe_names_are_not_add_ons() {
        let dir = tempfile::tempdir().unwrap();
        for id in ["claude", "../x", "", "a/b"] { assert!(load(dir.path(), id).is_err()); }
        assert_eq!(list(dir.path()), [("claude".to_string(), "Claude Code".to_string())]);
        with_addons(dir.path());
        assert_eq!(list(dir.path()).iter().map(|(i, _)| i.as_str()).collect::<Vec<_>>(), ["claude", "codex"]);
        fs::create_dir_all(dir.path().join("sessions")).unwrap();
        fs::write(dir.path().join("sessions/01a1d60c-6.json"), r#"{"agent":"codex"}"#).unwrap();
        assert_eq!((session_label(dir.path(), "01a1d60c-6"), session_label(dir.path(), "a3cf3125-f")), ("Codex 01a1d60c".into(), "Claude Code a3cf3125".into()));
        assert_eq!((name_of(dir.path(), None), name_of(dir.path(), Some("codex")), name_of(dir.path(), Some("gone"))), ("Claude Code".into(), "Codex".into(), "gone".into()));
    }
}
