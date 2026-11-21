//! Keeps the computer from going to sleep while an agent works (settings.json `keep_awake`, on unless false): a
//! turn being worked or waiting on the person (a command approved in the terminal runs while the state still says
//! waiting), or a subagent still running after the turn. The screen may still turn off and lock. On Linux a
//! `systemd-inhibit` block on sleep and the lid switch, under WSL Windows' `SetThreadExecutionState` held by
//! PowerShell (the sleep to stop is Windows'). The holder waits for its input to end: Whaybench closes it to let go,
//! and it closes by itself when Whaybench ends or dies. Runs on a thread of its own: asking Claude Code which
//! sessions run starts a program, and nothing here may hold up the compositor.
use std::{path::Path, process::{Child, Command, Stdio}, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};

use serde_json::Value;
use wb_core::{log as record, paths::Data, sessions};

/// A state older than this is a turn that ended without telling: it no longer keeps the computer up. Each tool call
/// writes the state again.
const STALE: Duration = Duration::from_secs(2 * 60 * 60);
/// After a holder fails, when to try again.
const RETRY: Duration = Duration::from_secs(5 * 60);
const EVERY: Duration = Duration::from_secs(10);

pub fn start(d: Data) {
    std::thread::spawn(move || {
        let mut holder: Option<Child> = None;
        let mut retry_at: Option<Instant> = None;
        loop {
            if let Some(c) = &mut holder && let Ok(Some(status)) = c.try_wait() {
                record::warn("awake", "sleep hold ended by itself", "the holder program ended or failed", &[("status", status.to_string())]);
                holder = None;
                retry_at = Some(Instant::now() + RETRY);
            }
            let want = enabled(&d) && busy(&d, SystemTime::now(), &|id| claude_runs(&d, id));
            if want && holder.is_none() && retry_at.is_none_or(|t| Instant::now() >= t) {
                match hold().stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
                    Ok(c) => { record::detail("awake", "sleep held", &[]); holder = Some(c); retry_at = None; }
                    Err(e) => {
                        record::warn("awake", "sleep not held", "the holder program did not start", &[("error", e.to_string())]);
                        retry_at = Some(Instant::now() + RETRY);
                    }
                }
            } else if !want && let Some(c) = holder.take() {
                let_go(c);
            }
            std::thread::sleep(EVERY);
        }
    });
}

/// Closes the holder's input, its way to end (killing the Linux side of a WSL program may leave the Windows one
/// running), and kills it only if it is still there a few seconds later.
fn let_go(mut c: Child) {
    drop(c.stdin.take());
    let until = Instant::now() + Duration::from_secs(5);
    while c.try_wait().is_ok_and(|s| s.is_none()) && Instant::now() < until { std::thread::sleep(Duration::from_millis(100)); }
    if c.try_wait().is_ok_and(|s| s.is_none()) { let _ = c.kill(); }
    let _ = c.wait();
    record::detail("awake", "sleep let go", &[]);
}

/// Read as it is: `paths::read_kept` would set aside a file it finds half written by the preferences window.
fn enabled(d: &Data) -> bool {
    std::fs::read(d.root.join("settings.json")).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .is_none_or(|v| v["keep_awake"] != Value::Bool(false))
}

/// Whether a Claude Code session still has its process, so a killed agent's last "working" does not hold for hours.
/// When Claude Code lists no session at all (it could not be asked), each is taken as running.
fn claude_runs(d: &Data, id: &str) -> bool {
    let addon = sessions::record_of(d, id).is_some_and(|r| r["agent"].as_str().is_some());
    addon || sessions::agents().is_empty() || sessions::agent(id).is_some()
}

/// Whether an agent works now: a session's state "working" or "waiting" written less than `STALE` ago whose agent
/// still runs, or a subagent still running. Only files changed in that time are read.
fn busy(d: &Data, now: SystemTime, runs: &dyn Fn(&str) -> bool) -> bool {
    let now_ms = now.duration_since(UNIX_EPOCH).map(|t| t.as_millis() as u64).unwrap_or(0);
    let fresh = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).is_ok_and(|t| now.duration_since(t).unwrap_or_default() < STALE);
    let turn = std::fs::read_dir(sessions::state_dir(d)).into_iter().flatten().flatten().any(|e| {
        let id = e.path().file_stem().unwrap_or_default().to_string_lossy().to_string();
        fresh(&e.path()) && std::fs::read(e.path()).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .is_some_and(|v| matches!(v["state"].as_str(), Some("working" | "waiting"))
                && now_ms.saturating_sub(v["at"].as_u64().unwrap_or(0)) < STALE.as_millis() as u64)
            && runs(&id)
    });
    turn || std::fs::read_dir(d.cache().join("subagents")).into_iter().flatten().flatten()
        .any(|e| fresh(&e.path()) && !sessions::subagents_running(d, &e.file_name().to_string_lossy()).is_empty())
}

/// The program that holds the computer awake until its input ends.
fn hold() -> Command {
    if crate::notify::wsl() {
        // ES_CONTINUOUS | ES_SYSTEM_REQUIRED in decimal (PowerShell reads 0x80000001 as a negative int). A failure
        // ends PowerShell, so it is seen and tried again. Sent encoded: no quote has to survive WSL's command line.
        let script = "$ErrorActionPreference = 'Stop'; \
            $k = Add-Type -Name Power -Namespace Whaybench -PassThru -MemberDefinition '[DllImport(\"kernel32.dll\")] public static extern uint SetThreadExecutionState(uint f);'; \
            if ($k::SetThreadExecutionState([uint32]2147483649) -eq 0) { exit 1 }; $null = [Console]::In.ReadToEnd()";
        let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut c = Command::new(crate::notify::POWERSHELL);
        c.args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &crate::notify::b64(&utf16)]);
        c
    } else {
        let mut c = Command::new("systemd-inhibit");
        c.args(["--what=sleep:handle-lid-switch", "--who=Whaybench", "--why=An agent is working", "--mode=block", "cat"]);
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_turn_of_a_running_agent_or_a_running_subagent_keeps_awake() {
        let root = std::env::temp_dir().join(format!("wb-awake-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = Data { root: root.clone() };
        let states = sessions::state_dir(&d);
        std::fs::create_dir_all(&states).unwrap();
        let now = SystemTime::now();
        let ms = |t: SystemTime| t.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        let state = |id: &str, s: &str, at: SystemTime| std::fs::write(states.join(format!("{id}.json")), format!(r#"{{"state":"{s}","at":{}}}"#, ms(at))).unwrap();
        let all = |_: &str| true;
        assert!(!busy(&d, now, &all), "no sessions");
        state("a", "done", now);
        assert!(!busy(&d, now, &all), "a finished turn");
        state("b", "working", now - STALE - Duration::from_secs(1));
        assert!(!busy(&d, now, &all), "a turn that never told its end");
        state("c", "waiting", now);
        assert!(busy(&d, now, &all), "waiting: a command approved in the terminal runs meanwhile");
        assert!(!busy(&d, now, &|id| id != "c"), "its agent is gone");
        assert!(!busy(&d, now + STALE + Duration::from_secs(1), &all), "the same, hours later");
        std::fs::remove_file(states.join("c.json")).unwrap();
        let sub = sessions::subagents_dir(&root, "a");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("x.json"), format!(r#"{{"at":{}}}"#, ms(now))).unwrap();
        assert!(busy(&d, now, &all), "a subagent still running after the turn");
        std::fs::write(sub.join("x.json"), format!(r#"{{"at":{},"done":{}}}"#, ms(now), ms(now))).unwrap();
        assert!(!busy(&d, now, &all), "the subagent stopped");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
