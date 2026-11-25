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
    /// Arguments that start the agent with a first prompt, `{prompt}` being its text; for an agent that makes its
    /// session, and runs its start hook, only once it has a prompt.
    #[serde(default)]
    prompt: Vec<String>,
    home: Option<Home>,
    #[serde(default)]
    merge: BTreeMap<String, String>,
    #[serde(default)]
    write: BTreeMap<String, String>,
    context: Option<String>,
    /// What each prompt gets instead of `context`; without it, `context` goes with every prompt too.
    prompt_context: Option<String>,
    pub trust: Option<String>,
    /// What the agent writes to its terminal when it waits for input, so never at a start screen (sign-in, trust).
    pub ready: Option<String>,
    pub usage: Option<String>,
    /// Arguments that sign the agent in where it has no browser (a Briefsafe project's container), and the page the
    /// person opens for it.
    #[serde(default)]
    pub login: Vec<String>,
    pub login_page: Option<String>,
}

#[derive(Deserialize)]
struct Home {
    env: String,
    from: String,
    #[serde(default)]
    copy: Vec<String>,
    #[serde(default)]
    copy_keys: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    link: Vec<String>,
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
    format!("{} {}", session_agent(data, sid), sid.chars().take(8).collect::<String>())
}

fn session_agent(data: &Path, sid: &str) -> String {
    let at = crate::roots::for_session(&crate::paths::Data::new(data.to_path_buf()), sid).root;
    let agent = crate::map::read_json(&at.join("sessions").join(format!("{sid}.json")), Value::Null).ok()
        .and_then(|r| r["agent"].as_str().map(String::from));
    name_of(data, agent.as_deref())
}

/// A session's line where a conversation is named: its title, the start of its number and its agent. The title is cut,
/// not what comes after it: two conversations of one title are told apart by that.
pub fn session_line(data: &Path, sid: &str) -> String { line_of(data, sid, &crate::sessions::session_name(&crate::paths::Data::new(data.to_path_buf()), sid)) }

/// The same for a conversation that only starts: its name is not asked for yet (that can take the agent's answer).
pub fn new_session_line(data: &Path, sid: &str) -> String { line_of(data, sid, "") }

fn line_of(data: &Path, sid: &str, name: &str) -> String {
    let title: String = name.lines().next().unwrap_or_default().to_string();
    let title = if title.is_empty() { crate::i18n::t("new-session").to_string() } else if title.chars().count() > LINE_TITLE { format!("{}…", title.chars().take(LINE_TITLE).collect::<String>()) } else { title };
    format!("{title} · {} · {}", sid.chars().take(8).collect::<String>(), session_agent(data, sid))
}
const LINE_TITLE: usize = 24;

/// Where a project's container finds the host's agent add-ons.
pub const ENV_ADDONS: &str = "WHAYBENCH_AGENT_ADDONS";
pub const ADDONS_IN_CONTAINER: &str = "/opt/whaybench/agents";

pub fn load(data: &Path, id: &str) -> Result<Agent> {
    if id == CLAUDE || !crate::addons::valid_id(id) { return Err(format!("not an agent add-on: {id}").into()); }
    // In a project's container the host's add-ons are mounted read-only apart from the project's data (`ADDONS_IN_CONTAINER`).
    // Only inside a container (podman's /run/.containerenv): on the host the add-ons are the data folder's, checked.
    let in_container = Path::new("/run/.containerenv").exists();
    let base = std::env::var_os(ENV_ADDONS).filter(|v| in_container && !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| data.join("addons/agents"));
    let dir = base.join(id);
    let text = fs::read_to_string(dir.join("agent.json")).map_err(|e| format!("agent add-on {id}: {e}"))?;
    let mut agent: Agent = serde_json::from_str(&text).map_err(|e| format!("agent add-on {id}: {e}"))?;
    // Files an add-on names stay inside its folder and its settings folder.
    let names = agent.merge.iter().chain(&agent.write).flat_map(|(a, b)| [a, b])
        .chain(&agent.context).chain(&agent.prompt_context).chain(agent.home.iter().flat_map(|h| h.copy.iter().chain(h.copy_keys.keys())));
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

fn trust_file(data: &Path) -> PathBuf { data.join("addon-trust.json") }

impl Agent {
    /// Every file the add-on brings, in a fixed order: what the person confirms before it runs.
    fn fingerprint(&self) -> Result<String> {
        use sha1::{Digest, Sha1};
        let mut names: Vec<&String> = self.merge.values().chain(self.write.values()).chain(&self.context).chain(&self.prompt_context).collect();
        names.sort();
        let mut h = Sha1::new();
        for n in std::iter::once(&"agent.json".to_string()).chain(names) {
            h.update(n.as_bytes());
            h.update([0]);
            h.update(fs::read(self.dir.join(n))?);
            h.update([0]);
        }
        Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }

    /// An add-on agent runs programs as the person: it runs only as the program brings it, or once the person has
    /// confirmed exactly these files (map styles' instructions are confirmed the same way).
    pub fn trusted(&self, data: &Path) -> bool {
        crate::addons::bundled_as_is(&crate::paths::Data::new(data.to_path_buf()), &format!("agents/{}", self.id))
            || self.fingerprint().ok().is_some_and(|f| crate::map::read_json(&trust_file(data), Value::Null).ok().is_some_and(|t| t[format!("agents/{}", self.id)] == f))
    }

    pub fn trust(&self, data: &Path) -> Result<()> {
        let mut t = crate::map::read_json(&trust_file(data), serde_json::json!({}))?;
        t[format!("agents/{}", self.id)] = Value::String(self.fingerprint()?);
        write_json(&trust_file(data), &t)
    }

    /// What the person reads before confirming: the program and its arguments, the settings it writes, what it is told.
    pub fn summary(&self) -> (String, Vec<String>, String) {
        let program = std::iter::once(self.command.as_str()).chain(self.args.iter().map(String::as_str)).collect::<Vec<_>>().join(" ");
        let files = self.merge.keys().chain(self.write.keys()).cloned().collect();
        // Both texts the agent is told: at the start, and with each prompt.
        let told = self.context.iter().chain(&self.prompt_context).filter_map(|c| fs::read_to_string(self.dir.join(c)).ok()).collect::<Vec<_>>().join("\n\n");
        (program, files, told)
    }

    pub fn folder(&self) -> &Path { &self.dir }

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

    /// The arguments that give a new session its first prompt, if the add-on has them.
    pub fn first_prompt(&self, text: &str) -> Option<Vec<String>> {
        (!self.prompt.is_empty()).then(|| self.prompt.iter().map(|a| a.replace("{prompt}", text)).collect())
    }

    /// What the agent is told at session start (`start`) and with each prompt.
    pub fn context(&self, session: &str, start: bool) -> Result<Option<String>> {
        let Some(file) = (if start { None } else { self.prompt_context.as_ref() }).or(self.context.as_ref()) else { return Ok(None) };
        Ok(Some(fs::read_to_string(self.dir.join(file))?.trim_end().replace("{session}", session)))
    }

    /// The agent's own settings folder `<data>/<id>`, made ready before anything starts. Logins and choices are copied
    /// from the person's folder once; the templates are written each time. Returns the variable and folder to set.
    /// The variable naming its settings folder (CODEX_HOME), if it has one.
    pub fn home_var(&self) -> Option<&str> { self.home.as_ref().map(|h| h.env.as_str()) }

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
        for file in &h.link { link_shared(&source, &home, file)?; }
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
            // A writable folder of the sandbox that is not there yet is not writable (Codex binds what exists).
            let roots = table.get("sandbox_workspace_write").and_then(|s| s.get("writable_roots")).and_then(toml::Value::as_array);
            for r in roots.into_iter().flatten().filter_map(toml::Value::as_str).filter(|r| Path::new(r).starts_with(data)) { fs::create_dir_all(r)?; }
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

/// A file the agent shares with the person's own folder, such as its login: a link to the person's, so signing in or
/// out on either side holds on both. Signing out here removes only the link; the person's file is then that same,
/// revoked login, or a new one from a sign-in there since: it is never removed, and linked again only once it is newer
/// than the sign-out. A file written here in the meantime (signing in here) is moved to the person's folder and linked.
fn link_shared(source: &Path, home: &Path, file: &str) -> Result<()> {
    fs::DirBuilder::new().recursive(true).mode(0o700).create(source)?;
    // Whole paths: a link is read from its own folder, so a relative CODEX_HOME would point elsewhere.
    let source = fs::canonicalize(source)?;
    // The person's folder is this one: the file is already theirs.
    if source == fs::canonicalize(home)? { return Ok(()); }
    let (here, there) = (home.join(file), source.join(file));
    let (mark, out) = (home.join(format!(".{file}.linked")), home.join(format!(".{file}.signed-out")));
    match fs::symlink_metadata(&here) {
        Ok(m) if m.file_type().is_symlink() => {
            if fs::read_link(&here)? == there { return Ok(()); }
            fs::remove_file(&here)?;
        }
        Ok(m) => {
            let theirs = fs::metadata(&there).and_then(|t| t.modified()).ok();
            if theirs.is_none_or(|t| m.modified().is_ok_and(|ours| ours > t)) { move_file(&here, &there)?; } else { fs::remove_file(&here)?; }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // Noticed now: anything the person's file got before this is older than the sign-out.
            if mark.exists() { create_private(&out, b"")?; fs::remove_file(&mark)?; }
            let since = fs::metadata(&out).and_then(|o| o.modified()).ok();
            let newer = |since| fs::metadata(&there).and_then(|t| t.modified()).is_ok_and(|t| t > since);
            if since.is_some_and(|since| !newer(since)) { return Ok(()); }
        }
        Err(e) => return Err(e.into()),
    }
    // The mark first: a link without it would make a later sign-out here go unnoticed.
    create_private(&mark, b"")?;
    std::os::unix::fs::symlink(&there, &here)?;
    if let Err(e) = fs::remove_file(&out) && e.kind() != io::ErrorKind::NotFound { return Err(e.into()); }
    Ok(())
}

/// A rename, or where the two are on different file systems a copy beside `to` renamed over it: a copy cut short (a
/// full disk) never takes the place of a whole file.
fn move_file(from: &Path, to: &Path) -> Result<()> {
    if fs::rename(from, to).is_ok() { return Ok(()); }
    let part = to.with_file_name(format!(".{}.part", to.file_name().and_then(|n| n.to_str()).unwrap_or("file")));
    fs::copy(from, &part)?;
    fs::rename(&part, to)?;
    fs::remove_file(from)?;
    Ok(())
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
                let config: toml::Table = fs::read_to_string(home.join("config.toml"))?.parse()?;
        assert_eq!(config["model"].as_str(), Some("chosen-model"));
        let mcp = &config["mcp_servers"]["whaybench"];
        assert_eq!(mcp["command"].as_str(), exe.to_str(), "a path with a quote stays one value");
        assert_eq!((mcp["args"][0].as_str(), mcp["args"][2].as_str(), mcp["env_vars"][0].as_str()), (Some("mcp"), data.to_str(), Some("WHAYBENCH_TAB")), "the map tool is Codex's MCP server");
        assert_eq!(mcp["tools"]["context_map"]["approval_mode"].as_str(), Some("approve"), "kept every turn without asking");
        let roots: Vec<&str> = config["sandbox_workspace_write"]["writable_roots"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
        assert!(roots.contains(&data.join("view").to_str().unwrap()), "Whaybench's commands can write their requests");
        assert!(roots.contains(&data.join("cache/subagents").to_str().unwrap()), "and the tasks a program reports, but no more of cache");
        assert!(data.join("cache/subagents").is_dir(), "made beforehand: a folder that is not there cannot be written in the sandbox");
        assert!(!roots.iter().any(|r| *r == data.to_str().unwrap() || r.ends_with("/codex") || r.ends_with("/addons") || r.ends_with("/cache")), "not its own settings, the add-ons or other tabs' input");
        assert_eq!(fs::metadata(home.join("config.toml"))?.permissions().mode() & 0o777, 0o600, "still private after the server is added");
        assert_eq!(config.len(), 4, "model, effort, the MCP server and the sandbox; nothing else is copied");
        // The login is the person's file, linked: a refresh, a sign-out or a sign-in on either side holds on both.
        assert_eq!(fs::read_link(home.join("auth.json"))?, source.join("auth.json"));
        fs::write(home.join("auth.json"), "refreshed-login")?;
        assert_eq!(fs::read_to_string(source.join("auth.json"))?, "refreshed-login");
        fs::remove_file(source.join("auth.json"))?;
        assert!(fs::read(home.join("auth.json")).is_err(), "signed out outside: no revoked login stays here");
        fs::write(source.join("auth.json"), "revoked-login")?;
        codex.prepare_with(&data, &exe, Some(source.clone()))?;
        // Signed out here: the link goes. The person's file, the same revoked login, is kept but not linked again.
        fs::remove_file(home.join("auth.json"))?;
        codex.prepare_with(&data, &exe, Some(source.clone()))?;
        assert!(fs::symlink_metadata(home.join("auth.json")).is_err(), "no link: the agent asks to sign in");
        assert_eq!(fs::read_to_string(source.join("auth.json"))?, "revoked-login", "the person's file is never removed");
        codex.prepare_with(&data, &exe, Some(source.clone()))?;
        assert!(fs::symlink_metadata(home.join("auth.json")).is_err(), "still not linked at the next start");
        // Signed in outside after that: linked again.
        let later = fs::metadata(home.join(".auth.json.signed-out"))?.modified()? + std::time::Duration::from_secs(1);
        fs::write(source.join("auth.json"), "signed-in-outside")?;
        fs::File::options().write(true).open(source.join("auth.json"))?.set_modified(later)?;
        codex.prepare_with(&data, &exe, Some(source.clone()))?;
        assert_eq!(fs::read_to_string(home.join("auth.json"))?, "signed-in-outside");
        // Signed out here again, then in here: the new login goes to the person's folder and is linked.
        fs::remove_file(home.join("auth.json"))?;
        codex.prepare_with(&data, &exe, Some(source.clone()))?;
        fs::write(home.join("auth.json"), "signed-in-here")?;
        fs::File::options().write(true).open(home.join("auth.json"))?.set_modified(later + std::time::Duration::from_secs(1))?;
        codex.prepare_with(&data, &exe, Some(source.clone()))?;
        assert_eq!(fs::read_link(home.join("auth.json"))?, source.join("auth.json"));
        assert_eq!(fs::read_to_string(source.join("auth.json"))?, "signed-in-here");
        Ok(())
    }

    #[test]
    fn a_failed_move_keeps_the_source() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (from, to) = (dir.path().join("a"), dir.path().join("b/auth.json"));
        fs::write(&from, "login")?;
        // The folder of `to` is missing, so the rename and then the copy fail: the source stays.
        assert!(move_file(&from, &to).is_err());
        assert_eq!(fs::read_to_string(&from)?, "login");
        fs::create_dir_all(dir.path().join("b"))?;
        move_file(&from, &to)?;
        assert_eq!(fs::read_to_string(&to)?, "login");
        assert!(!dir.path().join("b/.auth.json.part").exists() && !from.exists());
        Ok(())
    }

    #[test]
    fn the_persons_folder_being_the_agents_own_is_left_alone() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let data = dir.path().join("data");
        with_addons(&data);
        fs::create_dir_all(data.join("codex"))?;
        fs::write(data.join("codex/auth.json"), "login")?;
        load(&data, "codex")?.prepare_with(&data, &dir.path().join("whaybench"), Some(data.join("codex")))?;
        assert_eq!(fs::read_to_string(data.join("codex/auth.json"))?, "login");
        assert!(!fs::symlink_metadata(data.join("codex/auth.json"))?.file_type().is_symlink());
        Ok(())
    }

    #[test]
    fn a_login_copied_before_the_link_is_kept() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let data = dir.path().join("data");
        with_addons(&data);
        let source = dir.path().join("source");
        fs::create_dir_all(data.join("codex"))?;
        fs::write(data.join("codex/auth.json"), "copied-login")?;
        let codex = load(&data, "codex")?;
        codex.prepare_with(&data, &dir.path().join("whaybench"), Some(source.clone()))?;
        assert_eq!(fs::read_to_string(source.join("auth.json"))?, "copied-login", "moved to the person's folder, which did not exist");
        assert_eq!(fs::read_to_string(data.join("codex/auth.json"))?, "copied-login", "and read through the link");
        Ok(())
    }

    #[test]
    fn preparation_writes_hooks() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let data = dir.path().join("data");
        with_addons(&data);
        let exe = dir.path().join("bin/whaybench");
        let codex = load(&data, "codex")?;
        let (_, home) = codex.prepare_with(&data, &exe, Some(dir.path().join("source")))?.unwrap();
        let hooks = read_json(&home.join("hooks.json"), Value::Null)?;
        let events = hooks["hooks"].as_object().unwrap();
        assert_eq!(events.len(), 10);
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
        assert!(codex.context("sid", true)?.unwrap().contains("Its description has the rules"), "the whole note at the start");
        let each = codex.context("sid", false)?.unwrap();
        assert!(each.starts_with("Whaybench session sid.") && each.len() < 200, "one line with each prompt: {each}");
        // The person confirms the per-prompt text too: changing it asks again.
        let before = codex.fingerprint()?;
        assert!(codex.summary().2.contains("Whaybench session {session}. Conversation map"));
        fs::write(codex.folder().join("prompt.md"), "something else")?;
        assert_ne!(codex.fingerprint()?, before);
        Ok(())
    }

    #[test]
    fn an_add_on_runs_as_brought_or_once_confirmed() -> Result<()> {
        let dir = tempfile::tempdir()?;
        with_addons(dir.path());
        let codex = load(dir.path(), "codex")?;
        assert!(codex.trusted(dir.path()), "the program's own, unchanged");
        fs::write(dir.path().join("addons/agents/codex/context.md"), "changed")?;
        let codex = load(dir.path(), "codex")?;
        assert!(!codex.trusted(dir.path()), "changed: confirm first");
        codex.trust(dir.path())?;
        assert!(codex.trusted(dir.path()));
        fs::write(dir.path().join("addons/agents/codex/agent.json"), fs::read_to_string(dir.path().join("addons/agents/codex/agent.json"))?.replace("--no-daemon", "--other"))?;
        assert!(!load(dir.path(), "codex")?.trusted(dir.path()), "a confirmation holds for those files only");
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
        // A title is cut before what tells two conversations apart, and one without a title says so.
        assert!(session_line(dir.path(), "a3cf3125-f").ends_with(" · a3cf3125 · Claude Code"));
        crate::paths::write_atomic(&dir.path().join("names.json"), serde_json::json!({"a3cf3125-f": "x".repeat(40)}).to_string()).unwrap();
        assert_eq!(session_line(dir.path(), "a3cf3125-f"), format!("{}… · a3cf3125 · Claude Code", "x".repeat(24)));
        assert_eq!((name_of(dir.path(), None), name_of(dir.path(), Some("codex")), name_of(dir.path(), Some("gone"))), ("Claude Code".into(), "Codex".into(), "gone".into()));
    }
}
