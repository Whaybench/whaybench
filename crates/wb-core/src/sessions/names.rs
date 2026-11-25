//! What a session is called: by its own map.
use super::*;

/// A node label as a name: the part before ' → ' (the topic, not its conclusion).
pub(crate) fn short(label: &str, n: usize) -> String {
    let head = label.split(" → ").next().unwrap_or("").trim();
    if head.chars().count() <= n { head.to_string() } else { format!("{}…", head.chars().take(n - 1).collect::<String>()) }
}

/// The topic of a map without a name: the top branch most of its latest work is in (`map::latest_work`); of equals, the
/// one that comes last in the map.
pub fn main_topic(map: &Value) -> String {
    let nodes = map.get("nodes").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    let by: std::collections::HashMap<&str, &Value> = nodes.iter().filter_map(|n| Some((n["id"].as_str()?, n))).collect();
    let top_of = |start: &Value| {
        let mut x = start;
        for _ in 0..nodes.len() { match x["parent"].as_str().and_then(|p| by.get(p)) { Some(up) => x = up, None => break } }
        x["id"].as_str().unwrap_or("").to_string()
    };
    let mut counts: Vec<(String, usize)> = Vec::new();
    for (_, group) in crate::map::latest_work(nodes) {
        for t in group.iter().map(|n| top_of(n)).collect::<std::collections::BTreeSet<_>>() {
            match counts.iter_mut().find(|c| c.0 == t) { Some(c) => c.1 += 1, None => counts.push((t, 1)) }
        }
    }
    let position = |id: &str| nodes.iter().position(|n| n["id"] == id).unwrap_or(0);
    let main = counts.iter().max_by_key(|c| (c.1, position(&c.0))).map(|c| c.0.as_str()).unwrap_or("");
    short(by.get(main).and_then(|n| n["label"].as_str()).unwrap_or(""), 26)
}

/// What a session's own map calls it: the name the agent gave, else its topic. No other session is looked at. Kept by
/// the map file's identity and time, so a map that did not change is not read again (a node renamed or deleted
/// from another conversation, or by the person, shows once this one's map changes).
pub fn map_name(d: &Data, id: &str) -> Option<String> {
    type Kept = std::collections::HashMap<PathBuf, ((SystemTime, u64), Option<String>)>;
    static NAMES: Mutex<Option<Kept>> = Mutex::new(None);
    if !plain_id(id) { return None; }
    let root = at(d, id).root;
    let file = root.join(format!("{id}.json"));
    let m = crate::paths::meta(&file).ok()?;
    let key = (m.modified().ok()?, std::os::unix::fs::MetadataExt::ino(&m));
    let mut names = NAMES.lock().unwrap();
    let names = names.get_or_insert_with(Default::default);
    if let Some((k, n)) = names.get(&file) && *k == key { return n.clone(); }
    // A map that could not be read is not kept as one without a name: it is tried again.
    let name = match crate::map::title(&root, id) { Some(t) => Some(t), None => Some(main_topic(&map_of(d, id)?)).filter(|t| !t.is_empty()) };
    names.insert(file, (key, name.clone()));
    name
}

/// The session's name: the one its own map gives it; without a map, the name Claude Code gives it
/// (without the " (n)" it adds per /background), remembered after the session stops; else the first prompt.
pub fn session_name(d: &Data, id: &str) -> String {
    // A name the person gave in Claude Code (/rename, --name) comes first: the program does not undo it.
    if let Some(n) = record_of(d, id).and_then(|r| r.get("titleByPerson")?.as_str().map(String::from)).filter(|n| !n.is_empty()) { return n; }
    // A project session's name is the project's: it says what the conversation is about.
    let file = crate::roots::known_session(d, id).unwrap_or_else(|| d.clone()).root.join("names.json");
    let mut names: Value = crate::paths::read_kept::<Option<Value>>(&file).filter(Value::is_object).unwrap_or_else(|| serde_json::json!({}));
    let running = strip_count(agent(id).as_ref().and_then(|a| a.get("name")).and_then(Value::as_str).unwrap_or(""));
    if !running.is_empty() && names.get(id).and_then(Value::as_str) != Some(&running) {
        names[id] = Value::String(running.clone());
        let _ = write_atomic(&file, names.to_string());
    }
    if let Some(n) = map_name(d, id) { return n; }
    if !running.is_empty() { return running; }
    if let Some(n) = names.get(id).and_then(Value::as_str) { return n.to_string(); }
    record_of(d, id).and_then(|r| r.get("firstPrompt").and_then(Value::as_str).map(String::from)).unwrap_or_default()
}

pub(crate) fn strip_count(name: &str) -> String {
    let t = name.trim();
    if let Some(open) = t.rfind(" (") {
        let inner = &t[open + 2..];
        if inner.ends_with(')') && inner[..inner.len() - 1].chars().all(|c| c.is_ascii_digit()) && inner.len() > 1 { return t[..open].trim().to_string(); }
    }
    t.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::tests::{data, rec};

    #[test]
    fn a_map_names_a_session_by_the_branch_of_its_latest_work() {
        let n = |id: &str, parent: &str, at: i64, state: &str| serde_json::json!({"id":id,"parent":parent,"label":format!("{id} → x"),"state":state,"opened":{"at":at}});
        assert_eq!(main_topic(&serde_json::json!({"nodes":[]})), "");
        // Twelve options of one call count once; the dropped ones not at all.
        let mut nodes = vec![n("old", "", 1, "done"), n("new", "", 2, "")];
        nodes.extend((0..12).map(|i| n(&format!("o{i}"), "old", 10, if i < 6 { "dropped" } else { "done" })));
        nodes.extend((0..3).map(|i| n(&format!("w{i}"), "new", 20 + i, "doing")));
        assert_eq!(main_topic(&serde_json::json!({"nodes":nodes})), "new");
        // Only the ten latest times count.
        nodes.extend((0..10).map(|i| n(&format!("z{i}"), "old", 100 + i, "")));
        assert_eq!(main_topic(&serde_json::json!({"nodes":nodes})), "old");
        // Nodes that name each other as parent do not loop.
        assert!(!main_topic(&serde_json::json!({"nodes":[n("a", "b", 1, ""), n("b", "a", 1, "")]})).is_empty());
    }
    #[test]
    fn a_session_is_named_by_its_own_map_alone() {
        let d = data("titles");
        rec(&d, "s1", serde_json::json!({"id":"s1","lastActive":1000}));
        rec(&d, "s2", serde_json::json!({"id":"s2","lastActive":2000}));
        rec(&d, "s3", serde_json::json!({"id":"s3","firstPrompt":"first question"}));
        for s in ["s1", "s2"] { crate::map::command(&d.root, s, Some(serde_json::json!({"upsert":[{"id":"a","label":"same topic"}]}))).unwrap(); }
        assert_eq!((session_name(&d, "s1"), session_name(&d, "s2")), ("same topic".into(), "same topic".into()));
        assert_eq!(session_name(&d, "s3"), "first question", "without a map, the first prompt");
        crate::map::set_title(&d.root, "s1", "Own\nname").unwrap();
        assert_eq!(session_name(&d, "s1"), "Own name", "the agent's name, on one line");
        assert_eq!(session_name(&d, "s2"), "same topic");
        // A change of the map, a turn's end and a move to another number keep it; a map taken over does not.
        crate::map::command(&d.root, "s1", Some(serde_json::json!({"upsert":[{"id":"b","label":"more"}]}))).unwrap();
        crate::map::end_turn(&d.root, "s1").unwrap();
        assert_eq!(session_name(&d, "s1"), "Own name");
        crate::map::take_over(&d.root, "s4", &crate::map::view(&d.root, "s1").unwrap()).unwrap();
        assert_eq!(crate::map::title(&d.root, "s4"), None);
        adopt(&d, "s1", "s5");
        assert_eq!(session_name(&d, "s5"), "Own name");
        rec(&d, "s1", serde_json::json!({"id":"s1","titleByPerson":"the person's"}));
        assert_eq!(session_name(&d, "s1"), "the person's");
    }
    #[test]
    fn the_count_claude_code_adds_is_removed() {
        assert_eq!(strip_count("tidy up (3)"), "tidy up");
        assert_eq!(strip_count("tidy up"), "tidy up");
        assert_eq!(strip_count("tidy (up)"), "tidy (up)");
        assert_eq!(strip_count(""), "");
    }
}
