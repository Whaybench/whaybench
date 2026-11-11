//! `whaybench mcp`: the conversation map tool over the Model Context Protocol (JSON-RPC 2.0, one message per line on
//! stdin and stdout), so every agent that speaks MCP keeps its map the same way. The agent starts it inside a
//! Whaybench tab; the tab says which session it is, since no agent tells an MCP server its session id.
use std::io::{self, BufRead, Write};

use serde_json::{Value, json};
use wb_core::{map_tool, paths::Data, sessions};

/// Protocol versions this server speaks; a client asking for another gets the newest.
const VERSIONS: [&str; 4] = ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];

/// The answer to one message, or None for a notification.
pub fn handle(d: &Data, tab: Option<&str>, msg: &Value) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let reply = |result: Value| Some(json!({"jsonrpc": "2.0", "id": id, "result": result}));
    match msg["method"].as_str().unwrap_or("") {
        "initialize" => {
            let asked = msg["params"]["protocolVersion"].as_str().unwrap_or("");
            let version = VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(VERSIONS[VERSIONS.len() - 1]);
            reply(json!({"protocolVersion": version, "capabilities": {"tools": {}}, "serverInfo": {"name": "whaybench", "version": env!("CARGO_PKG_VERSION")}}))
        }
        "ping" => reply(json!({})),
        "tools/list" => reply(json!({"tools": [map_tool::definition()]})),
        "tools/call" => {
            let p = &msg["params"];
            let text = if p["name"] != map_tool::NAME { Err(format!("unknown tool: {}", p["name"])) } else {
                match tab.and_then(|t| sessions::session_of_tab(d, t)) {
                    None => Err("This conversation is not in a Whaybench tab, so it has no map here.".to_string()),
                    Some(sid) => map_tool::call(&d.root, &sid, &p["arguments"]).map_err(|e| e.to_string()),
                }
            };
            let error = text.is_err();
            reply(json!({"content": [{"type": "text", "text": text.unwrap_or_else(|e| e)}], "isError": error}))
        }
        m => Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("method not found: {m}")}})),
    }
}

pub fn serve(d: &Data) -> Result<(), String> {
    let tab = std::env::var("WHAYBENCH_TAB").ok();
    let mut out = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() { continue; }
        let answer = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(d, tab.as_deref(), &msg),
            Err(e) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}})),
        };
        if let Some(a) = answer { writeln!(out, "{a}").and_then(|_| out.flush()).map_err(|e| e.to_string())?; }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_speaks_mcp_and_finds_the_session_by_tab() {
        let dir = tempfile::tempdir().unwrap();
        let d = Data::new(dir.path().to_path_buf());
        let init = handle(&d, None, &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}})).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert!(handle(&d, None, &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none(), "no answer to a notification");
        assert_eq!(handle(&d, None, &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap()["result"]["tools"][0]["name"], "context_map");
        let call = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "context_map", "arguments": {"upsert": [{"id": "a", "label": "A"}], "show": true}}});
        assert_eq!(handle(&d, None, &call).unwrap()["result"]["isError"], true, "no tab, no session");
        let sid = "aaaaaaaa-0000-0000-0000-000000000001";
        std::fs::create_dir_all(dir.path().join("sessions")).unwrap();
        std::fs::write(dir.path().join("sessions").join(format!("{sid}.json")), json!({"id": sid, "tab": "t1", "lastActive": 1}).to_string()).unwrap();
        let r = handle(&d, Some("t1"), &call).unwrap();
        assert_eq!((r["result"]["isError"].as_bool(), r["result"]["content"][0]["text"].as_str()), (Some(false), Some("◇ A [a]")));
        assert_eq!(handle(&d, None, &json!({"jsonrpc": "2.0", "id": 4, "method": "nope"})).unwrap()["error"]["code"], -32601);
    }
}
