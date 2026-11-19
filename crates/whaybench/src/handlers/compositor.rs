use crate::{App, state::ClientState};
use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    reexports::wayland_server::{
        Client,
        protocol::{wl_buffer, wl_surface::WlSurface},
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            CompositorClientState, CompositorHandler, CompositorState, get_parent, is_sync_subsurface,
        },
        shm::{ShmHandler, ShmState},
    },
};

use smithay::{
    backend::{allocator::dmabuf::Dmabuf, renderer::ImportDma},
    wayland::dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
};

use super::xdg_shell;

impl CompositorHandler for App {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self
                .space
                .elements()
                .find(|w| w.toplevel().unwrap().wl_surface() == &root)
            {
                window.on_commit();
            }
        };

        xdg_shell::handle_commit(self, surface);
        layer_commit(self, surface);
    }
}

/// A background app's surface committed: the layers are laid out again, and a new one is told its size, as Smithay's
/// anvil does.
fn layer_commit(state: &mut App, surface: &WlSurface) {
    use smithay::desktop::{WindowSurfaceType, layer_map_for_output};
    use smithay::wayland::{compositor::with_states, shell::wlr_layer::LayerSurfaceData};
    let Some(output) = state.space.outputs().find(|o| layer_map_for_output(o).layer_for_surface(surface, WindowSurfaceType::TOPLEVEL).is_some()).cloned() else { return };
    let sent = with_states(surface, |s| s.data_map.get::<LayerSurfaceData>().is_some_and(|d| d.lock().unwrap().initial_configure_sent));
    let mut map = layer_map_for_output(&output);
    map.arrange();
    if !sent && let Some(layer) = map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL) { layer.layer_surface().send_configure(); }
}

impl BufferHandler for App {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for App {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl DmabufHandler for App {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        self.dmabuf.as_mut().expect("the dmabuf global exists only with its state")
    }

    fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: Dmabuf, notifier: ImportNotifier) {
        // A test import, as anvil does, so a buffer the GPU cannot take is refused now and not when drawn.
        let ok = match self.gpu.as_ref().and_then(|b| b.try_borrow_mut().ok()) {
            Some(mut backend) => backend.renderer().import_dmabuf(&dmabuf, None).is_ok(),
            None => true,
        };
        if ok { let _ = notifier.successful::<App>(); } else { notifier.failed(); }
    }
}
