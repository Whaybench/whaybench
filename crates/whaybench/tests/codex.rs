//! The real `whaybench map` and `whaybench hook codex` processes, with their stdin/stdout contract.
use std::{fs, io::Write, path::{Path, PathBuf}, process::{Command, Stdio}};

use serde_json::{Value, json};
use wb_core::{paths::Data, sessions};

fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn adapter(data: &Path, args: &[&str], input: Option<&Value>, lang: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_whaybench"))
        .args(args)
        .env("WHAYBENCH_DATA_DIR", data)
        .env("WHAYBENCH_TAB", "tab1")
        .env("WHAYBENCH_LANG", lang)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().unwrap();
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(serde_json::to_string(input).unwrap().as_bytes()).unwrap();
    } else {
        drop(child.stdin.take());
    }
    child.wait_with_output().unwrap()
}

fn run(data: &Path, args: &[&str], input: Option<&Value>) -> Value {
    let output = adapter(data, args, input, "en");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn event(name: &str) -> Value {
    json!({"session_id":"codex-map-test", "hook_event_name":name, "cwd":"work"})
}

fn read(path: PathBuf) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn first_question_two_turns_and_resume_keep_the_map_visible() {
    let dir = scratch();
    let data = dir.path();
    run(data, &["hook", "codex"], Some(&event("SessionStart")));
    let latest = read(data.join("latest.json"));
    assert_eq!(latest["nodes"], json!([]));
    assert!(latest["current"].is_null());
    let d = Data::new(data.to_path_buf());
    assert_eq!(sessions::target(&d).as_deref(), Some("codex-map-test"));

    let mut prompt = event("UserPromptSubmit");
    prompt["prompt"] = json!("check the map connection");
    let context = run(data, &["hook", "codex"], Some(&prompt));
    let text = context["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
    assert!(text.contains("automatically from the first question"));
    assert!(text.contains(" map codex-map-test --write"));

    let first = json!({"upsert":[{"id":"map", "label":"map connection", "isOpen":true, "state":"doing"}], "current":"map"});
    let first = run(data, &["map", "codex-map-test", "--write"], Some(&first));
    assert_eq!(read(data.join("latest.json")), first);
    assert_eq!(first["version"], 2);
    assert_eq!(first["nodes"][0]["opened"]["turn"], 0);
    let mut stop = event("Stop");
    stop["turn_id"] = json!("turn1");
    stop["last_assistant_message"] = json!("first turn checked");
    run(data, &["hook", "codex"], Some(&stop));
    let first = read(data.join("codex-map-test.json"));
    assert_eq!(first["turn"], 1);
    prompt["prompt"] = json!("check the update in the next turn too");
    run(data, &["hook", "codex"], Some(&prompt));
    assert_eq!(run(data, &["map", "codex-map-test"], None), first);
    assert_eq!(read(data.join("latest.json")), first);

    let second = json!({"upsert":[
        {"id":"turn", "parent":"map", "label":"update per turn → checked (map kept)", "isOpen":false, "state":"done"}
    ], "current":"turn"});
    let second = run(data, &["map", "codex-map-test", "--write"], Some(&second));
    assert_eq!(second["sessionId"], "codex-map-test");
    assert_eq!(read(data.join("latest.json")), second);
    assert_eq!(second["nodes"][0], first["nodes"][0]);
    assert_eq!(second["nodes"][1]["opened"]["turn"], 1);
    assert_eq!(second["nodes"][1]["closed"]["turn"], 1);

    stop["turn_id"] = json!("turn2");
    stop["last_assistant_message"] = json!("map update in the next turn checked");
    run(data, &["hook", "codex"], Some(&stop));
    run(data, &["hook", "codex"], Some(&stop));
    let answer = read(data.join("answers/codex-map-test.json"));
    assert_eq!(answer["prompt"], prompt["prompt"]);
    let finished = run(data, &["map", "codex-map-test"], None);
    assert_eq!(finished["turn"], 2);
    assert_eq!(finished["nodes"], second["nodes"]);
    assert_eq!(fs::read_to_string(data.join("answer-log/codex-map-test.jsonl")).unwrap().lines().count(), 2);

    // Another session may become latest. Resuming restores this map as the panel's target.
    run(data, &["map", "other", "--write"], Some(&json!({"nodes":[]})));
    assert_eq!(sessions::target(&d).as_deref(), Some("other"));
    let mut resume = event("SessionStart");
    resume["source"] = json!("resume");
    run(data, &["hook", "codex"], Some(&resume));
    assert_eq!(read(data.join("latest.json")), finished);
    assert_eq!(sessions::target(&d).as_deref(), Some("codex-map-test"));
}



#[test]
fn startup_and_every_prompt_provide_localized_map_instructions() {
    for lang in ["en", "ko"] {
        let dir = scratch();
        for name in ["SessionStart", "UserPromptSubmit"] {
            let mut e = event(name);
            e["prompt"] = json!("first question");
            let output = adapter(dir.path(), &["hook", "codex"], Some(&e), lang);
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            let output: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(output["hookSpecificOutput"]["hookEventName"], name);
            let context = output["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
            assert_eq!(context.contains("automatically from the first question"), lang == "en", "the text is in the session's language");
            assert!(context.contains("state") && !context.contains("isOpen"));
            assert!(context.contains(" map codex-map-test --write"));
            assert!(!context.contains("{ $"));
        }
    }
}
