//! The project map: JSON the AI writes, each write recording what changed.
use serde_json::{Value, json};

use crate::{Res, Store, arr, by_id, history::diff_objs, new_id, now_ms, read, s, write_json};

pub const STAGES: [&str; 7] = ["idea", "decided", "doing", "unverified", "done", "resolved", "dropped"];
pub const INCLUDE: [&str; 3] = ["include", "hold", "exclude"];

fn merge(into: &mut Value, from: &Value) {
    if let (Some(a), Some(b)) = (into.as_object_mut(), from.as_object()) {
        for (k, v) in b { a.insert(k.clone(), v.clone()); }
    }
}

fn empty(v: Option<&Value>) -> bool { v.is_none_or(|x| x.is_null() || x.as_str() == Some("")) }

impl Store {
    pub fn project_map(&self, pid: &str) -> Value {
        read(&self.pdir(pid).join("map.json")).filter(|m| m.get("version") == Some(&json!(2))).unwrap_or_else(|| json!({"version": 2, "areas": [], "items": []}))
    }

    /// Upserts areas and items by id; `remove` drops items and their children.
    pub fn map_put(&self, pid: &str, data: &Value, session: &str) -> Res<Value> {
        if self.get_project(pid).is_none() { return Err(format!("no project: {pid}")); }
        // Read, merge and write as one step: two sessions putting at once would each write the map without the other's
        // change. Released when the file closes, at the end of this call.
        let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(self.pdir(pid).join("map.lock")).map_err(|e| e.to_string())?;
        lock.lock().map_err(|e| e.to_string())?;
        let m = self.project_map(pid);
        let (old_items, old_areas) = (by_id(arr(&m, "items")), by_id(arr(&m, "areas")));
        let mut areas = old_areas.clone();
        for a in arr(data, "areas") {
            let id = s(a, "id");
            if id.is_empty() || (s(a, "title").is_empty() && !areas.contains_key(id)) { return Err(format!("an area needs id and title: {a}")); }
            merge(areas.entry(id.to_string()).or_insert_with(|| json!({"id": id})), a);
        }
        let mut area_list: Vec<Value> = areas.values().cloned().collect();
        area_list.sort_by(|x, y| {
            let key = |v: &Value| (v.get("order").and_then(Value::as_i64).unwrap_or(999), s(v, "title").to_string());
            key(x).cmp(&key(y))
        });
        let mut items = old_items.clone();
        for it in arr(data, "items") {
            let mut it = it.clone();
            let stage_ok = STAGES.contains(&s(&it, "stage"));
            if !items.contains_key(s(&it, "id")) {
                if s(&it, "text").is_empty() || !areas.contains_key(s(&it, "area")) || !stage_ok { return Err(format!("a new item needs id, text, an existing area and a stage {STAGES:?}: {it}")); }
                if s(&it, "id").is_empty() { it["id"] = json!(new_id(8)); }
            }
            if !empty(it.get("stage")) && !stage_ok { return Err(format!("stage must be one of {STAGES:?}")); }
            if !empty(it.get("include")) && !INCLUDE.contains(&s(&it, "include")) { return Err(format!("include must be one of {INCLUDE:?}")); }
            if let Some(o) = it.get("origin").cloned() {
                // Always a list of {session, node, label}; a bare string becomes its label.
                let list = if let Value::Array(v) = o { v } else { vec![o] };
                it["origin"] = Value::Array(list.into_iter().map(|x| if x.is_object() { x } else { json!({"label": x.as_str().map(String::from).unwrap_or_else(|| x.to_string())}) }).collect());
            }
            let cur = items.entry(s(&it, "id").to_string()).or_insert_with(|| json!({}));
            merge(cur, &it);
            cur["at"] = json!(now_ms());
            cur["by"] = json!(session);
        }
        let mut gone: Vec<String> = arr(data, "remove").iter().filter_map(Value::as_str).map(String::from).collect();
        loop {
            let more: Vec<String> = items.values().filter(|i| gone.iter().any(|g| g == s(i, "parent")) && !gone.iter().any(|g| g == s(i, "id"))).map(|i| s(i, "id").to_string()).collect();
            if more.is_empty() { break; }
            gone.extend(more);
        }
        let item_list: Vec<Value> = items.values().filter(|i| !gone.iter().any(|g| g == s(i, "id"))).cloned().collect();
        let out = json!({"version": 2, "areas": area_list, "items": item_list});
        write_json(&self.pdir(pid).join("map.json"), &out)?;
        let mut changes = diff_objs("area", &old_areas, &by_id(arr(&out, "areas")));
        changes.extend(diff_objs("item", &old_items, &by_id(arr(&out, "items"))));
        self.log_items(pid, "map", changes, session, s(data, "note"))?;
        Ok(json!({"areas": arr(&out, "areas").len(), "items": arr(&out, "items").len(), "removed": gone.len()}))
    }

    /// Starts a project's map from a conversation map: an area, its nodes as items in their tree, each pointing back at
    /// the copy they came from. The AI goes on from there.
    pub fn map_from_nodes(&self, pid: &str, nodes: &[Value], title: &str, file: &str) -> Res<Value> {
        let ids: Vec<&str> = nodes.iter().map(|n| s(n, "id")).collect();
        let items: Vec<Value> = nodes.iter().map(|n| {
            let open = wb_core::map::is_open(n);
            let stage = match s(n, "state") { "doing" => "doing", "plan" if open => "decided", "done" | "changed" => "done", "dropped" => "dropped", _ if !open => "resolved", _ => "idea" };
            let mut item = json!({"id": s(n, "id"), "area": "imported", "text": s(n, "label"), "stage": stage, "origin": [{"imported": file, "node": s(n, "id"), "label": s(n, "label"), "title": title}]});
            if ids.contains(&s(n, "parent")) { item["parent"] = json!(s(n, "parent")); }
            if !s(n, "reason").is_empty() { item["reason"] = json!(s(n, "reason")); }
            item
        }).collect();
        self.map_put(pid, &json!({"areas": [{"id": "imported", "title": title}], "items": items}), "")
    }
}

#[cfg(test)]
mod map_from_nodes_tests {
    use serde_json::json;
    use crate::{arr, s, scratch};
    #[test]
    fn sessions_writing_the_map_at_once_lose_nothing() {
        let st = scratch("race");
        st.create_project("p", "Test", "", "", Vec::new(), false).unwrap();
        st.map_put("p", &json!({"areas": [{"id": "ar", "title": "Area"}]}), "").unwrap();
        let root = st.data.root.clone();
        let threads: Vec<_> = (0..8).map(|t| { let root = root.clone(); std::thread::spawn(move || {
            let st = crate::Store::new(wb_core::paths::Data::new(root));
            for i in 0..15 { st.map_put("p", &json!({"items": [{"id": format!("{t}-{i}"), "area": "ar", "text": "x", "stage": "idea"}]}), &format!("s{t}")).unwrap(); }
        }) }).collect();
        for t in threads { t.join().unwrap(); }
        assert_eq!(arr(&st.project_map("p"), "items").len(), 8 * 15);
    }
    #[test]
    fn a_conversation_map_starts_a_project_map() {
        let st = scratch("mf");
        st.create_project("p", "Test", "", "", Vec::new(), false).unwrap();
        let nodes = vec![json!({"id":"a","label":"top","isOpen":true}), json!({"id":"b","parent":"a","label":"bottom","state":"dropped","reason":"why"}), json!({"id":"c","parent":"outside","label":"apart","state":"doing"})];
        st.map_from_nodes("p", &nodes, "title", "/x.json").unwrap();
        let m = st.project_map("p");
        assert_eq!(s(&arr(&m, "areas")[0], "title"), "title");
        let items = arr(&m, "items");
        assert_eq!(items.len(), 3);
        let of = |id: &str| items.iter().find(|i| s(i, "id") == id).unwrap();
        assert_eq!((s(of("a"), "stage"), s(of("b"), "stage"), s(of("c"), "stage")), ("idea", "dropped", "doing"));
        assert_eq!(s(of("b"), "parent"), "a");
        assert_eq!(s(of("c"), "parent"), "", "a parent outside the map is dropped");
        assert_eq!(s(&arr(of("a"), "origin")[0], "imported"), "/x.json");
    }
}
