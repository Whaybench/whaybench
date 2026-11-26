//! The answer panel said stuck by a hook (`wb_core::ask::stuck_panel`): each root's ask folder watched by inotify in
//! the event loop, the host's and each open project's (whose hooks may run in a container that sees no process of the
//! host). Watched again when a project opens or closes (a link made or removed in cache/open). Nothing is looked for
//! on a clock: `sync` does nothing until a watch has said something changed.

use std::{os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd}, path::Path};

use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction, RegistrationToken, generic::Generic};

/// One folder watched for names laid, made, moved in or removed.
struct Dir(OwnedFd);

impl AsFd for Dir { fn as_fd(&self) -> BorrowedFd<'_> { self.0.as_fd() } }

impl Dir {
    fn new(dir: &Path) -> Option<Dir> {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
        // Safety: plain system calls on a descriptor of our own and a NUL-ended path.
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if fd < 0 { return None; }
        // Safety: a descriptor just made, ours alone.
        let d = Dir(unsafe { OwnedFd::from_raw_fd(fd) });
        let events = libc::IN_CLOSE_WRITE | libc::IN_MOVED_TO | libc::IN_CREATE | libc::IN_DELETE | libc::IN_MOVED_FROM | libc::IN_DELETE_SELF | libc::IN_MOVE_SELF;
        // Safety: the descriptor above and a NUL-ended path.
        if unsafe { libc::inotify_add_watch(fd, path.as_ptr(), events) } < 0 { return None; }
        Some(d)
    }

    /// The names told since last asked; "" for the folder itself (removed, moved, unmounted). Never blocks.
    fn names(&self) -> Vec<String> {
        let (mut buf, mut out) = ([0u8; 4096], Vec::new());
        loop {
            // Safety: reads into a buffer of our own, of its size.
            let n = unsafe { libc::read(self.0.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 { return out; }
            let (mut at, head) = (0, std::mem::size_of::<libc::inotify_event>());
            while at + head <= n as usize {
                // Safety: the kernel lays whole events one after another; read unaligned from the bytes.
                let e: libc::inotify_event = unsafe { std::ptr::read_unaligned(buf[at..].as_ptr().cast()) };
                let named = &buf[at + head..(at + head + e.len as usize).min(n as usize)];
                out.push(named.split(|&b| b == 0).next().map(|n| String::from_utf8_lossy(n).into_owned()).unwrap_or_default());
                at += head + e.len as usize;
            }
        }
    }
}

#[derive(Default)]
pub struct AskWatch { armed: bool, tokens: Vec<RegistrationToken> }

impl AskWatch {
    pub fn sync(&mut self, handle: &LoopHandle<'_, crate::App>, state: &mut crate::App) {
        if self.armed && !std::mem::take(&mut state.ask_roots_new) { return; }
        self.armed = true;
        for t in self.tokens.drain(..) { handle.remove(t); }
        let host = state.data.clone();
        // Projects opening and closing: the roots to watch change.
        self.watch(handle, &host.cache().join("open"), None);
        for r in wb_core::roots::all_of(&host) { self.watch(handle, &r.cache().join("ask"), Some(r.root.clone())); }
    }

    /// `dir` watched: for `root`, its "stuck" said; for none, any change means the roots are watched again. A folder
    /// not there is waited for at its nearest folder that is, by the one name on the way to it.
    fn watch(&mut self, handle: &LoopHandle<'_, crate::App>, dir: &Path, root: Option<std::path::PathBuf>) {
        let (at, wanted) = if dir.is_dir() { (dir.to_path_buf(), None) } else {
            let Some(a) = dir.ancestors().skip(1).find(|a| a.is_dir()) else { return };
            let Some(name) = dir.strip_prefix(a).ok().and_then(|r| r.components().next()).map(|c| c.as_os_str().to_string_lossy().into_owned()) else { return };
            (a.to_path_buf(), Some(name))
        };
        let Some(w) = Dir::new(&at) else { return };
        let token = handle.insert_source(Generic::new(w, Interest::READ, Mode::Level), move |_, w, state: &mut crate::App| {
            let names = w.names();
            if let Some(name) = &wanted {
                if names.iter().any(|n| n == name) { state.ask_roots_new = true; }
            } else if names.iter().any(String::is_empty) || root.is_none() {
                state.ask_roots_new = true;
            } else if let Some(root) = &root && names.iter().any(|n| n == "stuck") && let Some(pid) = wb_core::ask::stuck_panel(root) {
                state.restart_answer(pid);
            }
            Ok(PostAction::Continue)
        });
        if let Ok(t) = token { self.tokens.push(t); }
    }
}
