//! A display of its own for each open Briefsafe project (spec 3.22): a socket in a folder its container sees. What
//! connects there is that project's, whatever it says of itself: its windows are the project's browser, and it gets no
//! way to type, to be the input method or to draw as Whaybench's own parts (`state::host_client`).
use std::{collections::HashMap, os::unix::net::UnixListener, path::PathBuf, sync::Arc};

use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction, RegistrationToken, generic::Generic};
use wb_core::{container, log as record, paths::Data, roots};

use crate::{App, state::ClientState};

/// The sockets by project, with the file each is (made again when it is gone, as after another start's clean-up).
#[derive(Default)]
pub struct Displays(HashMap<String, (RegistrationToken, PathBuf)>);

impl Displays {
    /// A socket for each project open in its container now, none for the others.
    pub fn sync(&mut self, handle: &LoopHandle<'_, App>, data: &Data) {
        let open: HashMap<String, (PathBuf, String)> = roots::stubs(data).into_iter().filter(|s| s.open() && container::opened_isolated(s))
            .map(|s| (container::display_dir(&s), s)).filter(|(d, _)| d.is_dir()).map(|(d, s)| (s.id, (d.join(container::DISPLAY_SOCKET), s.name))).collect();
        self.0.retain(|id, (token, file)| {
            let stays = open.get(id).is_some_and(|(f, _)| f == file) && file.exists();
            if !stays { handle.remove(*token); let _ = std::fs::remove_file(&*file); }
            stays
        });
        for (id, (file, name)) in open {
            if self.0.contains_key(&id) { continue; }
            // Left by a run before: the folder is the host's, its container sees it read-only.
            let _ = std::fs::remove_file(&file);
            let token = UnixListener::bind(&file).and_then(|l| { l.set_nonblocking(true)?; Ok(l) }).map_err(|e| e.to_string()).and_then(|l| {
                let project = id.clone();
                handle.insert_source(Generic::new(l, Interest::READ, Mode::Level), move |_, listener, state| {
                    while let Ok((stream, _)) = listener.accept() {
                        state.redraw = true;
                        let client = Arc::new(ClientState { project: Some(project.clone()), project_name: name.clone(), ..Default::default() });
                        if let Err(e) = state.display_handle.insert_client(stream, client) { record::error("run", "app not connected", &[("error", e.to_string())]); }
                    }
                    Ok(PostAction::Continue)
                }).map_err(|e| e.to_string())
            });
            match token {
                Ok(t) => { self.0.insert(id, (t, file)); }
                Err(e) => record::warn("run", "no display for the project", &e, &[("project", id)]),
            }
        }
    }
}
