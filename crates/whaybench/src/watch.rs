//! Folders watched by the event loop with inotify: Whaybench is told when a file there is written or laid, and reads
//! it then; nothing is looked for on a clock.
use std::path::{Path, PathBuf};

use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction, RegistrationToken, generic::Generic};
use wb_core::log as record;

/// A folder watched by inotify, as an event source of the loop.
pub struct Watch { fd: std::os::fd::OwnedFd }

impl std::os::fd::AsFd for Watch { fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> { self.fd.as_fd() } }

impl Watch {
    pub fn new(dir: &Path) -> Option<Watch> {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
        // Safety: plain system calls on a descriptor of our own and a NUL-ended path.
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if fd < 0 { return None; }
        // Safety: a descriptor just made, ours alone.
        let w = Watch { fd: unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd) } };
        if unsafe { libc::inotify_add_watch(fd, path.as_ptr(), libc::IN_CLOSE_WRITE | libc::IN_MOVED_TO | libc::IN_CREATE | libc::IN_DELETE | libc::IN_MOVED_FROM) } < 0 {
            record::warn("screen", "folder not watched", &std::io::Error::last_os_error().to_string(), &[("folder", dir.display().to_string())]);
            return None;
        }
        Some(w)
    }

    /// The names of the files written, laid or removed in the folder since last asked. Reads only what is waiting.
    pub fn names(&self) -> Vec<String> {
        let mut buf = [0u8; 4096];
        let mut names = Vec::new();
        loop {
            // Safety: reads into a buffer of our own, of its size.
            let n = unsafe { libc::read(std::os::fd::AsRawFd::as_raw_fd(&self.fd), buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 { return names; }
            let (mut at, head) = (0, std::mem::size_of::<libc::inotify_event>());
            while at + head <= n as usize {
                // Safety: the kernel lays whole events one after another; read unaligned from the bytes.
                let e: libc::inotify_event = unsafe { std::ptr::read_unaligned(buf[at..].as_ptr().cast()) };
                let named = &buf[at + head..(at + head + e.len as usize).min(n as usize)];
                if let Some(n) = named.split(|&b| b == 0).next() && !n.is_empty() { names.push(String::from_utf8_lossy(n).into_owned()); }
                at += head + e.len as usize;
            }
        }
    }
}

/// The theme's files: its folder, the file naming the theme in force, the default theme's settings. A change there
/// tells Whaybench to read the theme again at the next frame.
#[derive(Default)]
pub struct ThemeWatch(Option<(String, Vec<RegistrationToken>)>);

impl ThemeWatch {
    pub fn sync(&mut self, handle: &LoopHandle<'_, crate::App>, state: &mut crate::App) {
        // Again for another theme, or when its folder was made anew (a watch holds the folder, not its name).
        if self.0.as_ref().is_some_and(|(n, _)| *n == state.theme.name) && !std::mem::take(&mut state.theme_folder_new) { return; }
        for token in self.0.take().map(|(_, t)| t).unwrap_or_default() { handle.remove(token); }
        let root = state.data.root.clone();
        // (folder, the names that matter there; none: any)
        let folders: Vec<(PathBuf, Option<&'static [&'static str]>)> = vec![
            (root.clone(), Some(&["theme.json"])),
            (root.join("addons"), Some(&["themes-default-settings.json"])),
            (root.join("addons/themes").join(&state.theme.name), None),
        ];
        let mut tokens = Vec::new();
        // The folder holding the themes: the one in force made, removed or put back under its name.
        let name = state.theme.name.clone();
        if let Some(watch) = Watch::new(&root.join("addons/themes")) {
            let made = handle.insert_source(Generic::new(watch, Interest::READ, Mode::Level), move |_, w, state: &mut crate::App| {
                if w.names().contains(&name) { state.theme_folder_new = true; state.theme_dirty = true; state.redraw = true; }
                Ok(PostAction::Continue)
            });
            if let Ok(t) = made { tokens.push(t); }
        }
        for (dir, only) in folders {
            if !dir.is_dir() || dir == root.join("addons/themes").join(wb_core::theme::DEFAULT) { continue; }
            let Some(watch) = Watch::new(&dir) else { continue };
            let made = handle.insert_source(Generic::new(watch, Interest::READ, Mode::Level), move |_, w, state: &mut crate::App| {
                if w.names().iter().any(|n| only.is_none_or(|o| o.contains(&n.as_str()))) { state.theme_dirty = true; state.redraw = true; }
                Ok(PostAction::Continue)
            });
            match made { Ok(t) => tokens.push(t), Err(e) => record::warn("screen", "theme files not watched", &e.to_string(), &[]) }
        }
        self.0 = Some((state.theme.name.clone(), tokens));
        // What changed before the watch was there is read once now.
        state.theme_dirty = true;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_theme_folder_watch_hears_settings_reset_and_a_folder_made_anew() {
        let dir = std::env::temp_dir().join(format!("wb-theme-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("settings.json"), "{}").unwrap();
        let w = super::Watch::new(&dir).unwrap();
        let _ = w.names();
        std::fs::remove_file(dir.join("settings.json")).unwrap();
        assert!(w.names().contains(&"settings.json".to_string()), "a reset removes the settings: heard");
        std::fs::create_dir(dir.join("a-theme")).unwrap();
        assert!(w.names().contains(&"a-theme".to_string()), "a theme folder made anew: heard");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
