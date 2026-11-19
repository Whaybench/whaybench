mod compositor;
mod xdg_shell;

use crate::App;

//
// Wl Seat
//

use smithay::input::dnd::{DnDGrab, DndGrabHandler, GrabType, Source};
use smithay::input::pointer::Focus;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::Serial;
use smithay::wayland::output::OutputHandler;
use smithay::wayland::pointer_constraints::PointerConstraintsHandler;
use smithay::wayland::selection::{SelectionHandler, SelectionSource, SelectionTarget};
use smithay::wayland::selection::data_device::{
    DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler, set_data_device_focus,
};

impl SeatHandler for App {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<App> {
        &mut self.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, _image: smithay::input::pointer::CursorImageStatus) {}

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let dh = &self.display_handle;
        let client = focused.and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client);
    }
}

impl PointerConstraintsHandler for App {}

//
// Wl Data Device
//

impl SelectionHandler for App {
    type SelectionUserData = ();

    // A panel's app copied text: it goes to the desktop's clipboard too. Asked for a moment later: while this runs the
    // seat still holds the selection before it, and the app's cannot be read yet.
    fn new_selection(&mut self, ty: SelectionTarget, source: Option<SelectionSource>, _seat: Seat<Self>) {
        if ty != SelectionTarget::Clipboard { return; }
        self.clip.pending = source.and_then(|s| crate::clipboard::text_type(&s.mime_types()));
    }

    // A panel's app pastes what came from the desktop.
    fn send_selection(&mut self, _ty: SelectionTarget, _mime: String, fd: std::os::fd::OwnedFd, _seat: Seat<Self>, _user_data: &()) {
        self.clip.send(fd);
    }
}

impl DataDeviceHandler for App {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

impl DndGrabHandler for App {}
impl WaylandDndGrabHandler for App {
    fn dnd_requested<S: Source>(
        &mut self,
        source: S,
        _icon: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        type_: GrabType,
    ) {
        match type_ {
            GrabType::Pointer => {
                let ptr = seat.get_pointer().unwrap();
                let start_data = ptr.grab_start_data().unwrap();

                // create a dnd grab to start the operation
                let grab = DnDGrab::new_pointer(&self.display_handle, start_data, source, seat);
                ptr.set_grab(self, grab, serial, Focus::Keep);
            }
            GrabType::Touch => {
                // Touch is not handled.
                source.cancel();
            }
        }
    }
}

//
// Decorations: the title bar is the product's, so apps are told not to draw their own.
//

impl smithay::wayland::shell::xdg::decoration::XdgDecorationHandler for App {
    fn new_decoration(&mut self, toplevel: smithay::wayland::shell::xdg::ToplevelSurface) {
        use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
        toplevel.with_pending_state(|s| s.decoration_mode = Some(Mode::ServerSide));
    }
    fn request_mode(&mut self, toplevel: smithay::wayland::shell::xdg::ToplevelSurface, _mode: smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode) {
        toplevel.send_pending_configure();
    }
    fn unset_mode(&mut self, _toplevel: smithay::wayland::shell::xdg::ToplevelSurface) {}
}

//
// Wl Output & Xdg Output
//

impl OutputHandler for App {}

//
// Input method (fcitx5 inside) and the virtual keyboard it gives keys back through
//

// shortcut: the input method's own popups (a candidate list, as for Hanja) are not drawn; Hangul composes in the app.
// Draw them over their parent when a language needs a list.
impl smithay::wayland::input_method::InputMethodHandler for App {
    fn new_popup(&mut self, _surface: smithay::wayland::input_method::PopupSurface) {}
    fn dismiss_popup(&mut self, _surface: smithay::wayland::input_method::PopupSurface) {}
    fn popup_repositioned(&mut self, _surface: smithay::wayland::input_method::PopupSurface) {}
    fn parent_geometry(&self, _parent: &WlSurface) -> smithay::utils::Rectangle<i32, smithay::utils::Logical> { Default::default() }
}

impl smithay::wayland::virtual_keyboard::VirtualKeyboardHandler for App {
    fn process_virtual_keyboard_event(&mut self, event: smithay::backend::input::InputEvent<smithay::wayland::virtual_keyboard::VirtualKeyboardBackend>) {
        use smithay::backend::input::{Event, InputEvent, KeyboardKeyEvent};
        use smithay::input::keyboard::FilterResult;
        use smithay::wayland::{input_method::InputMethodSeat, virtual_keyboard::VirtualKeyboardSpecialEvent as Special};
        let keyboard = self.seat.get_keyboard().unwrap();
        match event {
            // Its keycodes mean something only under its own keymap (fcitx5 sends back the seat's).
            InputEvent::DeviceAdded { device } | InputEvent::Special(Special::KeymapChanged { device }) => {
                if let Some(keymap) = device.keymap() { let _ = keyboard.set_keymap_from_keymap_file(self, keymap); }
            }
            InputEvent::Keyboard { event } => {
                let (serial, time) = (smithay::utils::SERIAL_COUNTER.next_serial(), Event::time(&event));
                let device = event.device();
                // A key the input method gives back goes past its own grab, or it would come round again for ever.
                let from_ime = self.seat.input_method().keyboard_grab_client().is_some_and(|c| Some(c) == device.client());
                if from_ime { keyboard.input_forward_bypassing_grab(self, event.key_code(), event.state(), serial, time, true); }
                else { keyboard.input_from_source::<(), _>(device.source(), self, event.key_code(), event.state(), serial, time, |_, _, _| FilterResult::Forward); }
            }
            InputEvent::Special(Special::Modifiers { mods_depressed, mods_latched, mods_locked, group, .. }) => {
                keyboard.with_xkb_state(self, |mut context| context.set_modifier_mask(mods_depressed, mods_latched, mods_locked, group));
            }
            _ => {}
        }
    }
}

smithay::delegate_dispatch2!(App);

//
// wlr-layer-shell: an app behind the panels
//

use smithay::desktop::{LayerSurface, layer_map_for_output};
use smithay::wayland::shell::wlr_layer::{Layer, LayerSurface as WlrLayerSurface, WlrLayerShellHandler, WlrLayerShellState};

/// Whether a layer is one Whaybench shows: those under the windows, drawn as the surface behind the panels.
pub fn under_windows(layer: Layer) -> bool { matches!(layer, Layer::Background | Layer::Bottom) }

/// Only the layers under the windows are taken (background, bottom): Whaybench draws them as the surface behind the
/// panels. A bar or a notice asking for a layer above the windows is refused at once, closed, as it would not be seen.
impl WlrLayerShellHandler for App {
    fn shell_state(&mut self) -> &mut WlrLayerShellState { &mut self.layer_shell_state }

    fn new_layer_surface(&mut self, surface: WlrLayerSurface, _output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>, layer: Layer, namespace: String) {
        if !under_windows(layer) { surface.send_close(); return; }
        let Some(output) = self.space.outputs().next().cloned() else { surface.send_close(); return };
        crate::record::detail("run", "background app", &[("namespace", namespace.clone())]);
        let _ = layer_map_for_output(&output).map_layer(&LayerSurface::new(surface, namespace));
        self.redraw = true;
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        for o in self.space.outputs() {
            let mut map = layer_map_for_output(o);
            let found = map.layers().find(|l| l.layer_surface() == &surface).cloned();
            if let Some(l) = found { map.unmap_layer(&l); }
        }
        self.redraw = true;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use wayland_client::{Connection, Dispatch, QueueHandle, globals::{GlobalListContents, registry_queue_init}, protocol::{wl_compositor::WlCompositor, wl_registry::WlRegistry, wl_surface::WlSurface}};
    use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1}, zwlr_layer_surface_v1::{self, Anchor, ZwlrLayerSurfaceV1}};

    /// What the test app heard, by the namespace of each of its layer surfaces.
    #[derive(Default)]
    struct Heard { configured: Vec<String>, closed: Vec<String> }

    impl Dispatch<WlRegistry, GlobalListContents> for Heard { fn event(_: &mut Self, _: &WlRegistry, _: wayland_client::protocol::wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {} }
    impl Dispatch<WlCompositor, ()> for Heard { fn event(_: &mut Self, _: &WlCompositor, _: wayland_client::protocol::wl_compositor::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {} }
    impl Dispatch<WlSurface, ()> for Heard { fn event(_: &mut Self, _: &WlSurface, _: wayland_client::protocol::wl_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {} }
    impl Dispatch<ZwlrLayerShellV1, ()> for Heard { fn event(_: &mut Self, _: &ZwlrLayerShellV1, _: wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {} }
    impl Dispatch<ZwlrLayerSurfaceV1, String> for Heard {
        fn event(heard: &mut Self, l: &ZwlrLayerSurfaceV1, e: zwlr_layer_surface_v1::Event, name: &String, _: &Connection, _: &QueueHandle<Self>) {
            match e {
                zwlr_layer_surface_v1::Event::Configure { serial, .. } => { l.ack_configure(serial); heard.configured.push(name.clone()); }
                zwlr_layer_surface_v1::Event::Closed => heard.closed.push(name.clone()),
                _ => {}
            }
        }
    }

    /// An app asking for the bottom layer is taken and told its size once; one asking for the top layer is closed at
    /// once; one that moves itself to the top layer later is closed then and drawn no more.
    #[test]
    fn background_apps_are_taken_and_others_closed() {
        let data = tempfile::tempdir().unwrap();
        let mut event_loop: smithay::reexports::calloop::EventLoop<crate::App> = smithay::reexports::calloop::EventLoop::try_new().unwrap();
        let display = smithay::reexports::wayland_server::Display::<crate::App>::new().unwrap();
        let mut state = crate::App::new(&mut event_loop, display, wb_core::paths::Data::new(data.path().to_path_buf()), Default::default());
        let output = smithay::output::Output::new("test".into(), smithay::output::PhysicalProperties { size: (0, 0).into(), subpixel: smithay::output::Subpixel::Unknown, make: "t".into(), model: "t".into(), serial_number: "1".into() });
        output.change_current_state(Some(smithay::output::Mode { size: (800, 600).into(), refresh: 60_000 }), None, None, Some((0, 0).into()));
        state.space.map_output(&output, (0, 0));
        let (server, client) = std::os::unix::net::UnixStream::pair().unwrap();
        state.display_handle.insert_client(server, std::sync::Arc::new(crate::state::ClientState::default())).unwrap();

        // The app runs in a thread of its own, as an app is a process of its own; this one serves it meanwhile.
        let (tx, rx) = mpsc::channel();
        // The app stays connected until the server has been looked at: when it leaves, its layers go.
        let (done, wait) = mpsc::channel::<()>();
        let app = std::thread::spawn(move || {
            let conn = Connection::from_socket(client).unwrap();
            let (globals, mut queue) = registry_queue_init::<Heard>(&conn).unwrap();
            let qh = queue.handle();
            let compositor: WlCompositor = globals.bind(&qh, 4..=6, ()).unwrap();
            let shell: ZwlrLayerShellV1 = globals.bind(&qh, 1..=4, ()).unwrap();
            let mut heard = Heard::default();
            let layer = |name: &str, on: Layer| {
                let s = compositor.create_surface(&qh, ());
                let l = shell.get_layer_surface(&s, None, on, name.into(), &qh, name.to_string());
                l.set_anchor(Anchor::all());
                s.commit();
                (s, l)
            };
            let _wall = layer("wall", Layer::Bottom);
            let _bar = layer("bar", Layer::Top);
            let (moving, moving_layer) = layer("moving", Layer::Background);
            queue.roundtrip(&mut heard).unwrap();
            queue.roundtrip(&mut heard).unwrap();
            moving_layer.set_layer(Layer::Overlay);
            moving.commit();
            queue.roundtrip(&mut heard).unwrap();
            tx.send((heard.configured, heard.closed)).unwrap();
            let _ = wait.recv();
        });
        let (configured, closed) = loop {
            event_loop.dispatch(Some(std::time::Duration::from_millis(5)), &mut state).unwrap();
            let _ = state.display_handle.flush_clients();
            if let Ok(heard) = rx.try_recv() { break heard; }
        };
        assert_eq!(configured, ["wall", "moving"], "the layers under the windows are told their size, once each");
        assert_eq!(closed, ["bar", "moving"], "a layer above the windows is closed, also one that moves there");
        let map = smithay::desktop::layer_map_for_output(&output);
        let names: Vec<&str> = map.layers().map(|l| l.namespace()).collect();
        assert_eq!(names, ["wall"], "only the one still under the windows is drawn");
        drop(map);
        done.send(()).unwrap();
        app.join().unwrap();
    }
}
