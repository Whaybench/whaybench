//! The notifications panel's History tab: every change to every map node, newest first, only to read (3.19).
//! Built only while the tab is on show, and again only when the memory's files changed.
use std::collections::HashMap;

use chrono::Datelike;
use serde_json::Value;
use wb_core::i18n::{f, t};

/// `nid` is the node the row is about, `label` its name there.
pub struct Row { pub day: String, pub line: String, pub tip: String, pub nid: String, pub label: String }

fn state_word(v: &Value) -> String { v.as_str().map_or_else(|| t("history-open").to_string(), |s| t(&format!("state-{s}")).to_string()) }

fn kind_word(v: &Value) -> String {
    v.as_str().map_or_else(|| t("history-topic").to_string(), |k| crate::mapdraw::kind_of(k).map_or(k.to_string(), |x| x.0.to_string()))
}

fn plain(v: &Value) -> String { v.as_str().map(String::from).unwrap_or_else(|| if v.is_null() { "—".into() } else { v.to_string() }) }

/// Each changed field in words, in a fixed order, and the long ones' before and after for the tooltip.
fn changed(data: &Value) -> (Vec<String>, Vec<String>) {
    let (mut said, mut tip) = (Vec::new(), Vec::new());
    let pair = |k: &str| data.get(k).and_then(Value::as_array).filter(|a| a.len() == 2).map(|a| (a[0].clone(), a[1].clone()));
    // Back from Deleted is told as that, not as a change of state.
    if let Some((a, b)) = pair("state") { said.push(if a == "removed" { t("history-restored").to_string() } else { format!("{} → {}", state_word(&a), state_word(&b)) }); }
    // Maps from before kept isOpen; it says what the state says.
    else if let Some((_, b)) = pair("isOpen") { said.push(format!("{} → {}", t("history-open"), if b == false { state_word(&Value::from("done")) } else { t("history-open").to_string() })); }
    if let Some((a, b)) = pair("kind") { said.push(format!("{} → {}", kind_word(&a), kind_word(&b))); }
    for (key, word) in [("label", "history-renamed"), ("detail", "history-detail"), ("reason", "history-reason"), ("parent", "history-moved")] {
        if let Some((a, b)) = pair(key) { said.push(t(word).to_string()); tip.push(format!("{} → {}", plain(&a), plain(&b))); }
    }
    if let Some((_, b)) = pair("mark") { said.push(t(if b.is_null() { "history-unmarked" } else { "history-marked" }).to_string()); }
    if let Some((_, b)) = pair("suggest") {
        said.push(match b.get("state") { Some(s) => f("history-suggested", &[("state", state_word(s))]), None => t("history-suggest-settled").to_string() });
        if let Some(why) = b.get("why").and_then(Value::as_str) { tip.push(why.to_string()); }
    }
    let conditions: Vec<&str> = ["at", "due", "requires", "assumes"].into_iter().filter(|k| pair(k).is_some()).collect();
    if !conditions.is_empty() {
        said.push(t("history-conditions").to_string());
        for k in conditions { if let Some((a, b)) = pair(k) { tip.push(format!("{k}: {} → {}", plain(&a), plain(&b))); } }
    }
    if said.is_empty() { said.push(t("history-changed").to_string()); }
    (said, tip)
}

/// The rows, newest first. `name` gives a session's name; `label` a node's name as it is now, for a node whose
/// earlier events are no longer kept.
pub fn rows(events: &[Value], mut name: impl FnMut(&str) -> String, mut label: impl FnMut(&str) -> Option<String>) -> Vec<Row> {
    let mut labels: HashMap<String, String> = HashMap::new();
    let mut out = Vec::new();
    let days = ["weekday-mon", "weekday-tue", "weekday-wed", "weekday-thu", "weekday-fri", "weekday-sat", "weekday-sun"];
    for e in events {
        let kind = e["type"].as_str().unwrap_or("");
        if kind == "file.touched" { continue; }
        let nid = e["nid"].as_str().unwrap_or("").to_string();
        let data = &e["data"];
        if let Some(l) = data["label"].as_str().or_else(|| data["label"][1].as_str()) { labels.insert(nid.clone(), l.to_string()); }
        let mut named = |nid: &str| { if !labels.contains_key(nid) { if let Some(l) = label(nid) { labels.insert(nid.to_string(), l); } } labels.get(nid).cloned() };
        let (what, tip) = match kind {
            "node.added" => (t("history-added").to_string(), data["detail"].as_str().unwrap_or("").to_string()),
            "node.changed" => { let (s, tip) = changed(data); (s.join(", "), tip.join("\n")) }
            "link.added" | "link.removed" => {
                let to = data["to"].as_str().unwrap_or("");
                let target = named(to).unwrap_or_else(|| "—".to_string());
                let rel = data["rel"].as_str().unwrap_or("");
                let word = t(&format!("history-rel-{rel}")).to_string();
                if kind == "link.added" { (format!("{word} → {target}"), String::new()) } else { (f("history-unlinked", &[("rel", word), ("to", target)]), String::new()) }
            }
            // Put back on a map by the person, or put on a map by a session (a map taken over).
            "session.linked" => (t(if e["source"] == "person" { "history-restored" } else { "history-on-map" }).to_string(), String::new()),
            "session.unlinked" => (t("history-off-map").to_string(), String::new()),
            _ => continue,
        };
        let source = e["source"].as_str().unwrap_or("");
        let who = match source.strip_prefix("session/") { Some(s) => name(s), None if source == "person" => t("history-person").to_string(), None => t("history-program").to_string() };
        let at = chrono::DateTime::parse_from_rfc3339(e["time"].as_str().unwrap_or("")).ok().map(|x| x.with_timezone(&chrono::Local));
        let day = at.map(|x| format!("{} ({})", x.format("%m-%d"), t(days[x.weekday().num_days_from_monday() as usize]))).unwrap_or_default();
        let clock = at.map(|x| x.format("%H:%M").to_string()).unwrap_or_default();
        let label = named(&nid).unwrap_or_else(|| "—".to_string());
        out.push(Row { day, line: format!("{clock}  {label} · {what} · {}", who.chars().take(30).collect::<String>()), tip, nid, label });
    }
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn ev(kind: &str, source: &str, nid: &str, time: &str, data: Value) -> Value { json!({"type": kind, "source": source, "nid": nid, "session": "s1", "time": time, "data": data}) }
    #[test]
    fn each_change_is_one_line_newest_first() {
        let events = vec![
            ev("node.added", "session/s1", "na", "2026-11-11T10:00:00Z", json!({"label": "Problem A", "detail": "why"})),
            ev("file.touched", "session/s1", "na", "2026-11-11T10:01:00Z", json!({"path": "/x"})),
            ev("node.changed", "session/s1", "na", "2026-11-12T10:00:00Z", json!({"state": [null, "done"], "label": ["Problem A", "Problem A2"]})),
            ev("node.changed", "person", "na", "2026-11-12T10:05:00Z", json!({"mark": [null, "important"]})),
            ev("link.added", "session/s1", "nb", "2026-11-12T10:06:00Z", json!({"to": "na", "rel": "resolves"})),
            ev("session.unlinked", "session/s2", "na", "2026-11-12T10:07:00Z", json!({})),
            ev("node.changed", "person", "nb", "2026-11-12T10:08:00Z", json!({"state": ["removed", "done"]})),
            ev("session.linked", "person", "na", "2026-11-12T10:09:00Z", json!({})),
        ];
        let rows = rows(&events, |s| format!("name of {s}"), |nid| (nid == "nb").then(|| "Decision B".to_string()));
        assert_eq!(rows.len(), 7, "file events are left out");
        assert!(rows[0].line.ends_with("Problem A2 · Restored · You") && rows[1].line.ends_with("Decision B · Restored · You"), "back on the map and back from Deleted read the same: {} / {}", rows[0].line, rows[1].line);
        let rows = &rows[2..];
        assert!(rows[0].line.ends_with("Problem A2 · Taken off the map · name of s2"), "{}", rows[0].line);
        assert_eq!((rows[0].nid.as_str(), rows[0].label.as_str()), ("na", "Problem A2"), "a row knows its node");
        let rows = &rows[1..];
        assert!(rows[0].line.ends_with("Decision B · Resolves → Problem A2 · name of s1"), "a node without a kept event is named as it is now: {}", rows[0].line);
        assert!(rows[1].line.ends_with("Problem A2 · Marked important · You"), "{}", rows[1].line);
        assert!(rows[2].line.contains("Problem A2 · Open → Done, Renamed · name of s1"), "{}", rows[2].line);
        assert_eq!(rows[2].tip, "Problem A → Problem A2");
        assert!(rows[3].line.contains("Problem A · Created"), "the name it had then: {}", rows[3].line);
        assert_eq!(rows[3].tip, "why");
    }
}
