//! The clipboard between the apps in the panels and the person's desktop, both ways (text). The panels'
//! apps share this compositor's own clipboard; the desktop's is reached with wl-clipboard, whose
//! `wl-paste --watch` and `wl-copy` use the desktop's data-control protocol and need no window. This
//! process keeps the desktop's WAYLAND_DISPLAY (start::prepare), so both run against the desktop.
use std::{
    fs,
    io::{Read, Write},
    os::fd::OwnedFd,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
};

use wb_core::{log as record, paths::Data};

/// Types offered for text from the desktop; apps ask for one of them.
pub const TEXT_TYPES: [&str; 5] = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "TEXT", "STRING"];
// shortcut: text only, up to 16 MiB; images and rich text stay on their side until someone needs them across.
const LIMIT: u64 = 16 << 20;

pub struct Bridge {
    /// The text both sides hold now: what crosses is not sent back to where it came from.
    last: Arc<Mutex<String>>,
    /// The desktop's text, offered to the panels' apps while it is this compositor's selection.
    pub offered: Option<String>,
    /// A panel's app copied text of this type; it is read on the next tick.
    pub pending: Option<String>,
    /// Where `wl-paste --watch` leaves each new desktop text.
    inbox: PathBuf,
    watcher: Option<Child>,
    /// When the desktop's text was last read once (where it cannot be watched).
    read_at: Mutex<Option<std::time::Instant>>,
}

impl Bridge {
    pub fn start(data: &Data) -> Self {
        let inbox = data.cache().join("clipboard-desktop.txt");
        let _ = fs::remove_file(&inbox);
        let script = r#"cat > "$0.tmp" && mv "$0.tmp" "$0""#;
        let watcher = Command::new("wl-paste").args(["--no-newline", "--type", "text", "--watch", "sh", "-c", script])
            .arg(&inbox).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
            .map_err(|e| record::warn("clipboard", "desktop not reached", "install wl-clipboard", &[("error", e.to_string())]))
            .ok();
        Bridge { last: Arc::default(), offered: None, pending: None, inbox, watcher, read_at: Mutex::default() }
    }

    /// A new desktop text, if one came since the last look; one this side sent out is not taken back.
    pub fn from_desktop(&mut self) -> Option<()> {
        // A watcher that ended (a desktop without data-control, as WSLg) is waited for, or it stays a zombie.
        if let Some(status) = self.watcher.as_mut().and_then(|c| c.try_wait().ok().flatten()) {
            record::warn("clipboard", "desktop watch ended", "wl-paste --watch stopped; the desktop's text is read when the window gets the keyboard back", &[("status", status.to_string())]);
            self.watcher = None;
            self.read_once();
        }
        let text = fs::read_to_string(&self.inbox).ok()?;
        let _ = fs::remove_file(&self.inbox);
        let mut last = self.last.lock().unwrap();
        if text.is_empty() || *last == text { return None; }
        *last = text.clone();
        self.offered = Some(text);
        Some(())
    }

    /// Reads the desktop's text once, when the desktop cannot be watched (WSLg has no data-control): called when the
    /// window gets the keyboard back, which is when something copied on the desktop is wanted inside. The text
    /// arrives like a watched one, on the next tick.
    pub fn read_once(&self) {
        if self.watcher.is_some() { return; }
        // Not again within seconds: a wl-paste without data-control shows a small window of its own to read, which
        // takes the keyboard from this one and gives it back, and that return must not start the next read.
        {
            let mut at = self.read_at.lock().unwrap();
            if at.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(3)) { return; }
            *at = Some(std::time::Instant::now());
        }
        let inbox = self.inbox.clone();
        std::thread::spawn(move || {
            let Some(text) = desktop_text() else { return };
            if text.is_empty() || text.len() as u64 > LIMIT { return; }
            // Its own name: a read on focus and one when the watch ends may run at once.
            let tmp = inbox.with_extension(format!("once.{:?}", std::thread::current().id()).replace(['(', ')'], ""));
            if fs::write(&tmp, &text).is_ok() { let _ = fs::rename(tmp, inbox); }
        });
    }

    /// A panel's app copied: its text is read from `pipe` (the app writes it there) and given to the desktop.
    pub fn to_desktop(&self, pipe: std::io::PipeReader) {
        let last = self.last.clone();
        std::thread::spawn(move || {
            let mut text = String::new();
            if pipe.take(LIMIT).read_to_string(&mut text).is_err() || text.is_empty() { return; }
            {
                let mut last = last.lock().unwrap();
                if *last == text { return; }
                *last = text.clone();
            }
            let copy = Command::new("wl-copy").args(["--type", "text/plain;charset=utf-8"]).stdin(Stdio::piped()).stderr(Stdio::null()).spawn();
            match copy {
                Ok(mut c) => { if let Some(mut i) = c.stdin.take() { let _ = i.write_all(text.as_bytes()); } let _ = c.wait(); }
                Err(e) => record::warn("clipboard", "not given to the desktop", "install wl-clipboard", &[("error", e.to_string())]),
            }
        });
    }

    /// A panel's app pastes the desktop's text.
    pub fn send(&self, fd: OwnedFd) {
        let text = self.offered.clone().unwrap_or_default();
        std::thread::spawn(move || { let _ = fs::File::from(fd).write_all(text.as_bytes()); });
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        if let Some(c) = &mut self.watcher { let _ = c.kill(); let _ = c.wait(); }
    }
}

/// The desktop's text now. Under WSL, from Windows itself through PowerShell: a wl-paste there would show a window
/// for a moment (WSLg has no data-control), take the keyboard and make keys repeat.
fn desktop_text() -> Option<Vec<u8>> {
    let wsl = std::path::Path::new("/proc/sys/fs/binfmt_misc/WSLInterop").exists() || std::env::var_os("WSL_DISTRO_NAME").is_some();
    let out = if wsl {
        Command::new("/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Get-Clipboard -Raw"])
            .stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?
    } else {
        Command::new("wl-paste").args(["--no-newline", "--type", "text"]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?
    };
    if !out.status.success() { return None; }
    if !wsl { return Some(out.stdout); }
    // Windows ends lines with CR LF and Get-Clipboard adds one at the end.
    let text = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
    Some(text.strip_suffix('\n').unwrap_or(&text).as_bytes().to_vec())
}

/// The text type to ask an app for, among the ones it offers.
pub fn text_type(offered: &[String]) -> Option<String> {
    TEXT_TYPES.iter().find(|t| offered.iter().any(|o| o == *t)).map(|t| t.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_text_is_preferred_and_other_types_are_left() {
        let o = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(text_type(&o(&["text/html", "text/plain", "text/plain;charset=utf-8"])).as_deref(), Some("text/plain;charset=utf-8"));
        assert_eq!(text_type(&o(&["UTF8_STRING"])).as_deref(), Some("UTF8_STRING"));
        assert_eq!(text_type(&o(&["image/png"])), None);
    }

    #[test]
    fn a_text_that_crossed_is_not_taken_back() {
        let dir = tempfile::tempdir().unwrap();
        let data = Data::new(dir.path().to_path_buf());
        fs::create_dir_all(data.cache()).unwrap();
        let mut b = Bridge { last: Arc::default(), offered: None, pending: None, inbox: data.cache().join("c.txt"), watcher: None, read_at: Mutex::default() };
        fs::write(&b.inbox, "desktop text").unwrap();
        assert!(b.from_desktop().is_some());
        assert_eq!(b.offered.as_deref(), Some("desktop text"));
        fs::write(&b.inbox, "desktop text").unwrap();
        assert!(b.from_desktop().is_none());
        assert!(!b.inbox.exists());
    }
}
