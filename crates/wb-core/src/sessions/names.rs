//! What a session is called: from its map's topics, told apart when two sessions share one.
use super::*;

/// A node label as a name: the part before ' → ' (the topic, not its conclusion).
fn short(label: &str, n: usize) -> String {
    let head = label.split(" → ").next().unwrap_or("").trim();
    if head.chars().count() <= n { head.to_string() } else { format!("{}…", head.chars().take(n - 1).collect::<String>()) }
}

/// (main topic, what it is on now): the top branch with the most nodes, and the step under it holding the current one.
pub fn map_topics(map: &Value) -> (String, String) {
    let nodes = map.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
    if nodes.is_empty() { return Default::default(); }
    let sid = |n: &Value| n.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let parent = |n: &Value| n.get("parent").and_then(Value::as_str).unwrap_or("").to_string();
    let by: std::collections::HashMap<String, &Value> = nodes.iter().map(|n| (sid(n), n)).collect();
    let top_of = |start: &Value| -> String {
        let (mut x, mut seen) = (start, Vec::new());
        while let Some(up) = by.get(&parent(x)) {
            if seen.contains(&sid(x)) { break; }
            seen.push(sid(x));
            x = up;
        }
        sid(x)
    };
    let mut counts: Vec<(String, usize)> = Vec::new();
    for n in &nodes {
        let t = top_of(n);
        match counts.iter_mut().find(|c| c.0 == t) { Some(c) => c.1 += 1, None => counts.push((t, 1)) }
    }
    // Most nodes; of equals, the one that comes last in the map.
    let position = |id: &str| nodes.iter().position(|n| sid(n) == id).unwrap_or(0);
    let main = counts.iter().max_by_key(|c| (c.1, position(&c.0))).map(|c| c.0.clone()).unwrap_or_default();
    let label = |id: &str| by.get(id).and_then(|n| n.get("label")).and_then(Value::as_str).unwrap_or("").to_string();
    let current = map.get("current").and_then(Value::as_str).unwrap_or("");
    let mut now = String::new();
    if let Some(cur) = by.get(current) {
        if top_of(cur) != main {
            now = label(&top_of(cur));
        } else {
            let mut step = (*cur).clone();
            let mut guard = 0;
            let mut last = String::new();
            while sid(&step) != main && guard < 200 {
                last = sid(&step);
                match by.get(&parent(&step)) { Some(up) => step = (*up).clone(), None => break }
                guard += 1;
            }
            if !last.is_empty() { now = label(&last); }
        }
    }
    (short(&label(&main), 26), short(&now, 22))
}

/// Names from the maps, made distinct: main topic, then with what it is on, then with the date of last activity.
pub fn map_titles(d: &Data) -> std::collections::HashMap<String, String> {
    use std::collections::HashMap;
    let mut rows: Vec<(String, String, String, i64)> = Vec::new();
    for info in all(d) {
        if info.get("movedTo").and_then(Value::as_str).is_some_and(|m| !m.is_empty()) { continue; }
        let id = info["id"].as_str().unwrap_or("").to_string();
        let Some(map) = map_of(d, &id) else { continue };
        let (main, now) = map_topics(&map);
        if !main.is_empty() { rows.push((id, main, now, info.get("lastActive").and_then(Value::as_i64).unwrap_or(0))); }
    }
    let mut names: HashMap<String, String> = HashMap::new();
    for (id, main, now, _) in &rows {
        let same = rows.iter().filter(|r| r.1 == *main).count();
        names.insert(id.clone(), if same == 1 || now.is_empty() { main.clone() } else { format!("{main} · {now}") });
    }
    let snapshot = names.clone();
    for (id, _, _, at) in &rows {
        let name = &snapshot[id];
        if snapshot.values().filter(|v| *v == name).count() > 1 {
            let stamp = chrono::DateTime::from_timestamp_millis(*at).map(|t| t.with_timezone(&chrono::Local).format("%m-%d %H:%M").to_string()).unwrap_or_default();
            names.insert(id.clone(), format!("{name} · {stamp}"));
        }
    }
    names
}

static TITLES: Mutex<Option<(Instant, PathBuf, std::collections::HashMap<String, String>)>> = Mutex::new(None);

/// `map_titles`, kept for a second: it reads every record and every map, and a list of sessions asks for
/// each session's name in turn.
fn titles_kept(d: &Data) -> std::collections::HashMap<String, String> {
    let mut g = TITLES.lock().unwrap();
    if let Some((at, root, titles)) = g.as_ref()
        && at.elapsed() < Duration::from_secs(1) && *root == d.root { return titles.clone(); }
    let titles = map_titles(d);
    *g = Some((Instant::now(), d.root.clone(), titles.clone()));
    titles
}

/// The session's name: its main topic from its map; without a map, the name Claude Code gives it
/// (without the " (n)" it adds per /background), remembered after the session stops; else the first prompt.
pub fn session_name(d: &Data, id: &str) -> String {
    let file = d.root.join("names.json");
    let mut names: Value = crate::paths::read_kept::<Option<Value>>(&file).filter(Value::is_object).unwrap_or_else(|| serde_json::json!({}));
    let running = strip_count(agent(id).as_ref().and_then(|a| a.get("name")).and_then(Value::as_str).unwrap_or(""));
    if !running.is_empty() && names.get(id).and_then(Value::as_str) != Some(&running) {
        names[id] = Value::String(running.clone());
        let _ = write_atomic(&file, names.to_string());
    }
    if let Some(n) = titles_kept(d).get(id) { return n.clone(); }
    if !running.is_empty() { return running; }
    if let Some(n) = names.get(id).and_then(Value::as_str) { return n.to_string(); }
    record_of(d, id).and_then(|r| r.get("firstPrompt").and_then(Value::as_str).map(String::from)).unwrap_or_default()
}

pub fn tab_title(d: &Data, id: &str) -> String { session_name(d, id) }

fn strip_count(name: &str) -> String {
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
    fn a_map_names_a_session_by_its_biggest_branch() {
        let m = serde_json::json!({"current":"c","nodes":[{"id":"a","label":"small branch"},{"id":"b","label":"big topic → conclusion"},{"id":"c","parent":"b","label":"first step"},{"id":"d","parent":"c","label":"detail"}]});
        assert_eq!(map_topics(&m), ("big topic".to_string(), "first step".to_string()));
        assert_eq!(map_topics(&serde_json::json!({"nodes":[]})), Default::default());
    }
    #[test]
    fn the_same_topic_in_two_sessions_is_told_apart() {
        let d = data("titles");
        rec(&d, "s1", serde_json::json!({"id":"s1","lastActive":1000}));
        rec(&d, "s2", serde_json::json!({"id":"s2","lastActive":2000}));
        rec(&d, "s3", serde_json::json!({"id":"s3","firstPrompt":"first question"}));
        let map = |cur: &str| Some(serde_json::json!({"current":cur,"nodes":[{"id":"a","label":"same topic"},{"id":"x","parent":"a","label":"one"},{"id":"y","parent":"a","label":"two"}]}));
        crate::map::command(&d.root, "s1", map("x")).unwrap();
        crate::map::command(&d.root, "s2", map("y")).unwrap();
        let t = map_titles(&d);
        assert_eq!(t["s1"], "same topic · one");
        assert_eq!(t["s2"], "same topic · two");
        assert_eq!(session_name(&d, "s3"), "first question", "without a map, the first prompt");
    }
    #[test]
    fn the_count_claude_code_adds_is_removed() {
        assert_eq!(strip_count("tidy up (3)"), "tidy up");
        assert_eq!(strip_count("tidy up"), "tidy up");
        assert_eq!(strip_count("tidy (up)"), "tidy (up)");
        assert_eq!(strip_count(""), "");
    }
}
