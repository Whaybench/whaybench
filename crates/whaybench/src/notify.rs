//! Desktop notifications: a session starts waiting on the person while Whaybench is not the window in front. On a
//! Linux desktop through its notification service, under WSL as a Windows toast (WSLg passes no notifications on).
use std::{collections::HashMap, process::{Command, Stdio}, time::{Duration, SystemTime, UNIX_EPOCH}};

use wb_core::{log as record, paths::Data};

pub const POWERSHELL: &str = "/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe";

/// Shows a notification and waits until it is handed over; a desktop without a notification service shows nothing.
fn show(title: &str, body: &str) {
    let mut c = if wb_core::input::wsl() {
        // The texts go as base64, so nothing in them is read as PowerShell. Sent as PowerShell, which has a
        // registered app id; shortcut: the toast names PowerShell, until the installer gives Whaybench's shortcut one.
        let script = format!("$d={{param($s)[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($s))}}; \
            $m=[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType=WindowsRuntime]; \
            $x=$m::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); $t=$x.GetElementsByTagName('text'); \
            $null=$t.Item(0).AppendChild($x.CreateTextNode((& $d '{}'))); $null=$t.Item(1).AppendChild($x.CreateTextNode((& $d '{}'))); \
            $m::CreateToastNotifier('{{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}}\\WindowsPowerShell\\v1.0\\powershell.exe').Show([Windows.UI.Notifications.ToastNotification]::new($x))",
            b64(title.as_bytes()), b64(body.as_bytes()));
        let mut c = Command::new(POWERSHELL);
        c.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        c
    } else {
        let mut c = Command::new("gdbus");
        c.args(["call", "--session", "--dest", "org.freedesktop.Notifications", "--object-path", "/org/freedesktop/Notifications",
            "--method", "org.freedesktop.Notifications.Notify", "--", "'Whaybench'", "0", "''", &quoted(title), &quoted(body), "[]", "{}", "-1"]);
        c
    };
    let _ = c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

/// A string in GVariant text, as gdbus reads its arguments.
fn quoted(s: &str) -> String { format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")) }

pub fn b64(s: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in s.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..4 { out.push(if i <= chunk.len() { A[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' }); }
    }
    out
}

fn ms(t: SystemTime) -> u64 { t.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0) }

/// Each session's state as last seen, to tell a session that has just started waiting from one still waiting.
pub struct Waiting {
    /// Per session: the state and when it was written. A new wait is a new write, even right after another.
    seen: HashMap<String, (String, u64)>,
    /// States written before Whaybench started (waiting from before) are not news.
    since: u64,
    /// The last look: only state files changed after it are read, however long ago it was (a suspend).
    looked: SystemTime,
}

impl Waiting {
    pub fn new() -> Self { let now = SystemTime::now(); Waiting { seen: HashMap::new(), since: ms(now), looked: now } }

    fn newly(&mut self, session: &str, state: &str, at: u64) -> bool {
        let before = self.seen.insert(session.to_string(), (state.to_string(), at));
        state == "waiting" && before.is_none_or(|(s, a)| s != "waiting" || a != at) && at >= self.since
    }

    /// Reads the session states written since the last look and notifies each new wait while Whaybench is not in
    /// front. The name and the notification are made off the compositor's loop: finding a name can ask Claude Code.
    pub fn poll(&mut self, d: &Data, focused: bool) {
        // A second of overlap, so a file written during the last look is read again; a wait already seen is no news.
        let from = self.looked - Duration::from_secs(1);
        self.looked = SystemTime::now();
        for e in wb_core::roots::all_of(d).iter().flat_map(|r| std::fs::read_dir(wb_core::sessions::state_dir(r)).into_iter().flatten().flatten()) {
            if e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < from) { continue; }
            let Some(v) = std::fs::read_to_string(e.path()).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { continue };
            let session = e.path().file_stem().unwrap_or_default().to_string_lossy().to_string();
            if !self.newly(&session, v["state"].as_str().unwrap_or(""), v["at"].as_u64().unwrap_or(0)) || focused { continue; }
            record::detail("session", "waiting told on the desktop", &[("session", session.clone())]);
            let (d, note) = (d.clone(), v["note"].as_str().unwrap_or("").to_string());
            std::thread::spawn(move || {
                let body = [wb_core::sessions::session_name(&d, &session), note].into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" · ");
                show(wb_core::i18n::t("notify-waiting"), &body);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_and_quotes() {
        assert_eq!([b64(b""), b64(b"f"), b64(b"fo"), b64(b"foo"), b64("\u{d55c}".as_bytes())], ["", "Zg==", "Zm8=", "Zm9v", "7ZWc"]);
        assert_eq!(quoted("it's a\\b"), "'it\\'s a\\\\b'");
    }

    #[test]
    fn only_the_start_of_a_wait_is_news() {
        let mut w = Waiting::new();
        w.since = 100;
        assert!(!w.newly("old", "waiting", 50), "waiting from before the start");
        assert!(w.newly("s", "waiting", 200));
        assert!(!w.newly("s", "waiting", 200), "the same wait read again");
        assert!(w.newly("s", "waiting", 250), "the next request, its working state never seen in between");
        assert!(!w.newly("s", "working", 400));
        assert!(w.newly("s", "waiting", 500), "a new wait");
    }
}
