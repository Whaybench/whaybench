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
        Bridge { last: Arc::default(), offered: None, pending: None, inbox, watcher }
    }

    /// A new desktop text, if one came since the last look; one this side sent out is not taken back.
    pub fn from_desktop(&mut self) -> Option<()> {
        // A watcher that ended (a desktop without data-control, as WSLg) is waited for, or it stays a zombie.
        if let Some(status) = self.watcher.as_mut().and_then(|c| c.try_wait().ok().flatten()) {
            record::warn("clipboard", "desktop watch ended", "wl-paste --watch stopped; texts copied on the desktop do not come in", &[("status", status.to_string())]);
            self.watcher = None;
        }
        let text = fs::read_to_string(&self.inbox).ok()?;
        let _ = fs::remove_file(&self.inbox);
        let mut last = self.last.lock().unwrap();
        if text.is_empty() || *last == text { return None; }
        *last = text.clone();
        self.offered = Some(text);
        Some(())
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
        let mut b = Bridge { last: Arc::default(), offered: None, pending: None, inbox: data.cache().join("c.txt"), watcher: None };
        fs::write(&b.inbox, "desktop text").unwrap();
        assert!(b.from_desktop().is_some());
        assert_eq!(b.offered.as_deref(), Some("desktop text"));
        fs::write(&b.inbox, "desktop text").unwrap();
        assert!(b.from_desktop().is_none());
        assert!(!b.inbox.exists());
    }
}
