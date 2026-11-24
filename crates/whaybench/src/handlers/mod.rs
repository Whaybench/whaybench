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
    use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
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

    impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Heard { fn event(_: &mut Self, w: &xdg_wm_base::XdgWmBase, e: xdg_wm_base::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) { if let xdg_wm_base::Event::Ping { serial } = e { w.pong(serial); } } }
    impl Dispatch<xdg_surface::XdgSurface, ()> for Heard { fn event(_: &mut Self, s: &xdg_surface::XdgSurface, e: xdg_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) { if let xdg_surface::Event::Configure { serial } = e { s.ack_configure(serial); } } }
    impl Dispatch<xdg_toplevel::XdgToplevel, ()> for Heard { fn event(_: &mut Self, _: &xdg_toplevel::XdgToplevel, _: xdg_toplevel::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {} }

    type Loop = smithay::reexports::calloop::EventLoop<'static, crate::App>;

    fn start(data: &wb_core::paths::Data) -> (Loop, crate::App) {
        let mut events = Loop::try_new().unwrap();
        let display = smithay::reexports::wayland_server::Display::<crate::App>::new().unwrap();
        let state = crate::App::new(&mut events, display, data.clone(), Default::default());
        (events, state)
    }

    fn serve(state: &mut crate::App, events: &mut Loop) {
        events.dispatch(Some(std::time::Duration::from_millis(5)), state).unwrap();
        let _ = state.display_handle.flush_clients();
    }

    /// Serves the app until it says the step is done (an app that failed ends the test, not hangs it).
    fn until<T>(rx: &mpsc::Receiver<T>, state: &mut crate::App, events: &mut Loop) -> T {
        loop {
            serve(state, events);
            match rx.try_recv() { Ok(v) => return v, Err(mpsc::TryRecvError::Disconnected) => panic!("the app failed"), Err(_) => {} }
        }
    }

    /// An app in a thread of its own, with a window, and one more as a dialog of it when asked.
    struct OneWindow { go: mpsc::Sender<bool>, done: mpsc::Receiver<Vec<String>>, app: std::thread::JoinHandle<()> }
    impl OneWindow {
        /// With the globals it was offered.
        fn seen(state: &mut crate::App, events: &mut Loop, project: Option<&str>, app_id: &str) -> (Self, Vec<String>) {
            let (server, client) = std::os::unix::net::UnixStream::pair().unwrap();
            let who = crate::state::ClientState { project: project.map(String::from), ..Default::default() };
            state.display_handle.insert_client(server, std::sync::Arc::new(who)).unwrap();
            let ((tx, done), (go, wait)) = (mpsc::channel(), mpsc::channel::<bool>());
            let id = app_id.to_string();
            let app = std::thread::spawn(move || {
                let conn = Connection::from_socket(client).unwrap();
                let (globals, mut queue) = registry_queue_init::<Heard>(&conn).unwrap();
                let names: Vec<String> = globals.contents().with_list(|l| l.iter().map(|g| g.interface.clone()).collect());
                let qh = queue.handle();
                let compositor: WlCompositor = globals.bind(&qh, 4..=6, ()).unwrap();
                let base: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=2, ()).unwrap();
                let window = |of: Option<&xdg_toplevel::XdgToplevel>| {
                    let surface = compositor.create_surface(&qh, ());
                    let top = base.get_xdg_surface(&surface, &qh, ()).get_toplevel(&qh, ());
                    top.set_app_id(id.clone());
                    top.set_parent(of);
                    surface.commit();
                    top
                };
                let first = window(None);
                queue.roundtrip(&mut Heard::default()).unwrap();
                tx.send(names).unwrap();
                while wait.recv() == Ok(true) {
                    let _dialog = window(Some(&first));
                    queue.roundtrip(&mut Heard::default()).unwrap();
                    tx.send(Vec::new()).unwrap();
                }
            });
            let names = until(&done, state, events);
            (OneWindow { go, done, app }, names)
        }
        fn open(state: &mut crate::App, events: &mut Loop, project: Option<&str>, app_id: &str) -> Self { Self::seen(state, events, project, app_id).0 }
        fn dialog(&self, state: &mut crate::App, events: &mut Loop) { self.go.send(true).unwrap(); until(&self.done, state, events); }
        fn close(self, state: &mut crate::App, events: &mut Loop) {
            self.go.send(false).unwrap();
            self.app.join().unwrap();
            for _ in 0..5 { serve(state, events); }
        }
    }

    /// What a project's program connects with: the same window system as the host's apps, without what types, composes
    /// or draws as Whaybench's own parts; and its window is the project's browser, whatever it calls itself.
    #[test]
    fn a_projects_program_gets_a_window_and_no_more() {
        let dir = tempfile::tempdir().unwrap();
        let (mut events, mut state) = start(&wb_core::paths::Data::new(dir.path().to_path_buf()));
        let (_up, names) = OneWindow::seen(&mut state, &mut events, Some("up"), crate::state::STATUS_APP);
        for kept in ["zwlr_layer_shell_v1", "zwp_virtual_keyboard_manager_v1", "zwp_input_method_manager_v2"] { assert!(!names.iter().any(|n| n == kept), "{kept}"); }
        assert!(names.iter().any(|n| n == "xdg_wm_base") && names.iter().any(|n| n == "zwp_text_input_manager_v3"), "a window, and text composed for it: {names:?}");
        let keys: Vec<String> = state.surface.panels.iter().flat_map(|p| p.tabs.iter().map(|t| t.key.clone())).collect();
        assert_eq!(keys, ["org.mozilla.firefox.p-up#1"], "the project's browser, not the status line it named itself");
        assert!(state.status.is_none());
        let (_host, names) = OneWindow::seen(&mut state, &mut events, None, crate::state::STATUS_APP);
        assert!(names.iter().any(|n| n == "zwlr_layer_shell_v1") && state.status.is_some(), "an app of the host's as before");
        // (Without fcitx5 nobody is offered the two that type: what withholds them is asked itself.)
        let of = |state: &mut crate::App, project: Option<&str>| {
            let (server, _client) = std::os::unix::net::UnixStream::pair().unwrap();
            state.display_handle.insert_client(server, std::sync::Arc::new(crate::state::ClientState { project: project.map(String::from), ..Default::default() })).unwrap()
        };
        assert!(!crate::state::host_client(&of(&mut state, Some("up"))) && crate::state::host_client(&of(&mut state, None)));
    }

    const HOST: &str = "org.mozilla.firefox#1";
    const UP: &str = "org.mozilla.firefox.p-up#1";
    const TERM: &str = "whaybench-term#1";
    const VIEW: &str = "dev.whaybench.view#1";

    /// A desk with one conversation tab, the project of which can be changed.
    struct Desk { _dir: tempfile::TempDir, data: wb_core::paths::Data, events: Loop, state: crate::App, apps: Vec<OneWindow> }
    impl Desk {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let data = wb_core::paths::Data::new(dir.path().to_path_buf());
            std::fs::create_dir_all(dir.path().join("cache/term")).unwrap();
            // Two projects open in their containers, and one that runs without one.
            for id in ["up", "two", "plain"] {
                let s = wb_core::roots::Stub::new(&data, id, id, &dir.path().join(id));
                wb_core::roots::write_stub(&data, &s).unwrap();
                std::fs::create_dir_all(s.open_at.parent().unwrap()).unwrap();
                if id != "plain" { wb_core::container::note_isolated(&s, true).unwrap(); }
            }
            let (events, state) = start(&data);
            Desk { _dir: dir, data, events, state, apps: Vec::new() }
        }
        fn open(&mut self, project: Option<&str>, app: &str) -> usize { let w = OneWindow::open(&mut self.state, &mut self.events, project, app); self.apps.push(w); self.apps.len() - 1 }
        fn host(&mut self) { self.open(None, wb_core::view::FIREFOX_APP); }
        fn up(&mut self) -> usize { self.open(Some("up"), "x") }
        fn term(&mut self) { self.open(None, "whaybench-term-t1"); }
        /// The conversation is one of this project's; none: the host's.
        fn choose(&mut self, project: Option<&str>) { assert!(wb_core::sessions::set_tab_project(&self.data, "t1", project)); self.state.sync_focus(); }
        /// The person presses on the panel that shows this tab.
        fn press(&mut self, key: &str) {
            let pid = self.state.surface.panels.iter().find(|p| p.tab().key == key).map(|p| p.pid).unwrap_or_else(|| panic!("{key} is not in front"));
            self.state.press_panel(pid);
        }
        /// The person presses this tab in its title bar.
        fn press_tab(&mut self, key: &str) {
            let Some(pid) = self.state.surface.panels.iter().find(|p| p.tabs.iter().any(|t| t.key == key)).map(|p| p.pid) else { return };
            let p = self.state.surface.get_mut(pid).unwrap();
            p.active = p.tabs.iter().position(|t| t.key == key).unwrap();
            self.state.tab_pressed(pid);
            self.state.press_panel(pid);
        }
        fn close(&mut self, i: usize) { let w = self.apps.remove(i); w.close(&mut self.state, &mut self.events); }
        /// Whether the tab is the front one of its panel.
        fn shows(&self, key: &str) -> bool { self.state.surface.panels.iter().any(|p| p.tab().key == key) }
        fn keys(&self) -> String {
            let id = self.state.keys_tab();
            self.state.surface.panels.iter().flat_map(|p| p.tabs.iter()).find(|t| Some(t.id) == id).map(|t| t.key.clone()).unwrap_or_default()
        }
        fn asked(&mut self, ago: u64) { self.state.browsers.ask_at(&wb_core::view::project_browser_app("up"), std::time::Instant::now().checked_sub(std::time::Duration::from_secs(ago)).unwrap()); }
    }

    #[test]
    fn a_projects_browser_comes_up_with_its_conversation() {
        let mut d = Desk::new();
        d.host();
        d.term();
        d.up();
        assert!(d.shows(HOST) && d.keys() == TERM, "another place's conversation is chosen: it waits behind");
        d.choose(Some("up"));
        assert!(d.shows(UP) && d.keys() == TERM, "its conversation is chosen: it comes up, the keys stay");
        d.choose(None);
        assert!(d.shows(HOST) && d.keys() == TERM, "and the host's with a host conversation");
    }

    #[test]
    fn what_the_person_types_in_is_not_changed_and_what_waited_comes_up_after() {
        let mut d = Desk::new();
        d.host();
        d.term();
        let up = d.up();
        d.press(HOST);
        d.choose(Some("up"));
        assert!(d.shows(HOST) && d.keys() == HOST, "not followed under the hands");
        d.press(TERM);
        assert!(d.shows(UP) && d.keys() == TERM, "but once the keyboard is elsewhere");
        // A window of the chosen project that opens while the person types in the host's browser.
        d.close(up);
        d.choose(None);
        d.choose(Some("up"));
        d.press(HOST);
        d.up();
        assert!(d.shows(HOST) && d.keys() == HOST, "a window that opens waits");
        d.state.front_project_browser("up");
        assert!(d.shows(HOST) && d.keys() == HOST, "and what its agents ask for");
        d.press(TERM);
        assert!(d.shows(UP) && d.keys() == TERM, "both come up once the keyboard is elsewhere");
        // A window that is there already, asked for while the person types in the host's browser beside it.
        let pid = d.state.surface.panels.iter().find(|p| p.tab().key == UP).map(|p| p.pid).unwrap();
        let p = d.state.surface.get_mut(pid).unwrap();
        p.active = p.tabs.iter().position(|t| t.key == HOST).unwrap();
        d.press(HOST);
        d.state.front_project_browser("up");
        assert!(d.shows(HOST) && d.keys() == HOST, "asked for, it waits");
        d.press(TERM);
        assert!(d.shows(UP) && d.keys() == TERM, "and comes up");
    }

    #[test]
    fn a_projects_window_opens_in_front_of_the_view_without_the_keys() {
        let mut d = Desk::new();
        d.open(None, "dev.whaybench.view");
        d.term();
        d.choose(Some("up"));
        d.up();
        assert!(d.shows(UP) && d.keys() == TERM, "the page that was asked for shows");
        // The view is looked at, not typed in: the page shows, and nothing typed goes to the one hidden.
        let mut d = Desk::new();
        d.term();
        d.open(None, "dev.whaybench.view");
        d.choose(Some("up"));
        d.press(VIEW);
        d.up();
        assert!(d.shows(UP) && d.keys().is_empty(), "{}", d.keys());
        // The tab it would cover is the one typed in.
        let mut d = Desk::new();
        d.term();
        d.open(None, "dev.whaybench.project");
        d.choose(Some("up"));
        d.press("dev.whaybench.project#1");
        d.up();
        assert!(d.shows("dev.whaybench.project#1") && d.keys() == "dev.whaybench.project#1", "never over the tab typed in");
        d.press(TERM);
        assert!(d.shows(UP) && d.keys() == TERM, "after it");
    }

    #[test]
    fn only_what_the_person_asked_for_takes_the_keys() {
        let mut d = Desk::new();
        d.host();
        d.term();
        d.choose(Some("up"));
        d.press(HOST);
        d.asked(1);
        d.up();
        assert!(d.shows(UP) && d.keys() == UP, "by the menu, also from the host's browser");
        let mut d = Desk::new();
        d.host();
        d.term();
        d.choose(Some("up"));
        d.asked(31);
        d.up();
        assert!(d.shows(UP) && d.keys() == TERM, "a start that failed long ago asks for nothing");
        d.choose(None);
        d.press(HOST);
        d.asked(1);
        d.up();
        assert!(d.shows(HOST) && d.keys() == HOST, "nor one of a conversation that is not chosen now");
    }

    #[test]
    fn a_dialog_of_the_browser_the_person_is_in_is_seen_and_one_of_another_waits_with_it() {
        let mut d = Desk::new();
        d.term();
        let up = d.up();
        d.press(UP);
        let w = d.apps.remove(up);
        w.dialog(&mut d.state, &mut d.events);
        assert_eq!(d.keys(), "org.mozilla.firefox.p-up#2", "the file window the person opened, whatever conversation is chosen");
        let mut d = Desk::new();
        d.host();
        d.term();
        let up = d.up();
        let panels = d.state.surface.panels.len();
        let w = d.apps.remove(up);
        w.dialog(&mut d.state, &mut d.events);
        assert!(d.shows(HOST) && d.keys() == TERM && d.state.surface.panels.len() == panels, "not over the panels of another place");
        d.apps.push(w);
        d.choose(Some("up"));
        assert!(d.shows("org.mozilla.firefox.p-up#2"), "the dialog, not the window that waits for it");
    }

    #[test]
    fn a_dialog_of_a_projects_browser_does_not_cover_the_screen() {
        for all in [800.0, 1500.0] { assert!(crate::browsers::room(true, 5000, all) <= all - 80.0); }
        assert_eq!(crate::browsers::room(false, 5000, 800.0), 5000.0, "a host's as it asks");
    }

    #[test]
    fn a_browser_that_closes_hands_the_keys_to_no_other_side() {
        let mut d = Desk::new();
        d.term();
        let up = d.up();
        d.open(Some("two"), "x");
        d.choose(Some("up"));
        d.press(UP);
        assert_eq!(d.keys(), UP);
        d.close(up);
        assert_eq!(d.keys(), "", "the next tab is another project's: what is typed goes nowhere");
        // With the view in the panel it comes up, and may have them.
        let mut d = Desk::new();
        d.open(None, "dev.whaybench.view");
        d.term();
        d.choose(Some("up"));
        d.asked(1);
        let up = d.up();
        d.open(Some("two"), "x");
        assert_eq!(d.keys(), UP);
        d.close(up);
        assert!(d.shows(VIEW) && d.keys() == VIEW);
        // One that had not the keyboard changes nothing, and a conversation that ends gives none to a project's page.
        let mut d = Desk::new();
        d.host();
        let up = d.up();
        d.press(HOST);
        d.close(up);
        assert!(d.shows(HOST) && d.keys() == HOST);
        let mut d = Desk::new();
        d.up();
        d.press(UP);
        d.term();
        assert_eq!(d.keys(), TERM);
        d.close(1);
        assert_ne!(d.keys(), UP, "typed for the conversation, not for the page");
    }

    #[test]
    fn where_the_host_has_no_browser_the_view_stands_in_and_the_projects_comes_back() {
        let mut d = Desk::new();
        d.open(None, "dev.whaybench.view");
        d.term();
        d.choose(Some("up"));
        d.up();
        assert!(d.shows(UP));
        d.choose(None);
        assert!(d.shows(VIEW), "not the project's page beside a host conversation");
        d.choose(Some("up"));
        assert!(d.shows(UP), "back with its conversation");
    }

    #[test]
    fn a_floating_place_saved_for_a_projects_browser_is_not_taken_over_another_place() {
        let mut d = Desk::new();
        d.host();
        d.term();
        let mut s = crate::layout::Surface::new(d.state.surface.w, d.state.surface.h);
        s.panels.push(crate::layout::Panel::new(1, crate::layout::Tab { id: 1, key: UP.into(), name: "x".into() }, crate::layout::Rect::new(200.0, 200.0, 400.0, 300.0)));
        d.state.saved = s.to_saved((1500, 900));
        let panels = d.state.surface.panels.len();
        d.up();
        assert!(d.state.surface.panels.len() == panels && d.shows(HOST), "behind, with its group");
    }

    #[test]
    fn the_menu_is_told_which_browser_its_line_is_of() {
        let mut d = Desk::new();
        d.term();
        d.choose(Some("plain"));
        assert_eq!(wb_core::view::chosen_browser(&d.data), wb_core::view::FIREFOX_APP, "a project without a container has none of its own");
        assert!(!d.state.toggle_project_browser());
        d.choose(Some("up"));
        assert_eq!(wb_core::view::chosen_browser(&d.data), wb_core::view::project_browser_app("up"));
        d.choose(None);
        assert_eq!(wb_core::view::chosen_browser(&d.data), wb_core::view::FIREFOX_APP);
    }

    #[test]
    fn a_settings_window_keeps_the_keys_and_gives_them_back() {
        let mut d = Desk::new();
        d.term();
        d.choose(Some("up"));
        d.up();
        d.press(UP);
        let other = d.open(None, "dev.whaybench.notify");
        d.press_tab(UP);
        let theme = d.open(None, "dev.whaybench.theme");
        assert_eq!(d.keys(), "dev.whaybench.theme#1");
        d.close(other);
        assert_eq!(d.keys(), "dev.whaybench.theme#1", "whatever closes meanwhile");
        d.close(theme - 1);
        assert_eq!(d.keys(), UP, "back where the person was");
    }

    #[test]
    fn a_project_without_a_container_leaves_the_hosts_browser_alone() {
        let mut d = Desk::new();
        d.open(None, "dev.whaybench.view");
        d.host();
        d.term();
        d.press_tab(HOST);
        d.press(TERM);
        d.choose(Some("plain"));
        assert!(d.shows(HOST));
    }

    #[test]
    fn a_tab_that_closes_brings_up_no_other_sides_page() {
        let mut d = Desk::new();
        d.open(None, "dev.whaybench.view");
        let notify = d.open(None, "dev.whaybench.notify");
        d.term();
        d.up();
        d.press_tab("dev.whaybench.notify#1");
        d.press(TERM);
        d.close(notify);
        assert!(!d.shows(UP) && d.shows(VIEW));
        // A closes: the view comes up, and B's browser with B's conversation.
        let mut d = Desk::new();
        d.open(None, "dev.whaybench.view");
        d.term();
        d.choose(Some("up"));
        let up = d.up();
        d.open(Some("two"), "x");
        d.close(up);
        assert!(d.shows(VIEW));
        d.choose(Some("two"));
        assert!(d.shows("org.mozilla.firefox.p-two#1"));
    }

    #[test]
    fn a_dropped_panel_keeps_its_front_tab() {
        let mut d = Desk::new();
        d.up();
        d.host();
        d.term();
        d.press_tab(HOST);
        let held = d.state.surface.panels.iter().find(|p| p.tab().key == HOST).map(|p| p.pid).unwrap();
        let target = d.state.surface.panels.iter().find(|p| p.tab().key == TERM).map(|p| p.pid).unwrap();
        d.state.surface.apply_drop(held, crate::layout::Zone { target, how: crate::layout::How::Tab, preview: Default::default() });
        d.state.focus_panel(target);
        assert!(d.shows(HOST) && d.keys() == HOST);
    }

    #[test]
    fn a_view_the_person_chose_and_a_window_in_front_stay() {
        let mut d = Desk::new();
        d.open(None, "dev.whaybench.view");
        d.term();
        d.choose(Some("up"));
        d.up();
        d.choose(None);
        assert!(d.shows(VIEW), "stands in");
        d.press_tab(VIEW);
        d.press(TERM);
        d.choose(Some("up"));
        assert!(d.shows(VIEW), "the person chose it meanwhile");
        // Two windows of one project: the one in front stays when its agents open a page.
        d.press_tab(UP);
        d.up();
        d.press_tab(UP);
        d.press(TERM);
        d.state.front_project_browser("up");
        assert!(d.shows(UP));
    }

    #[test]
    fn a_dialog_that_waited_comes_up_with_its_conversation() {
        let mut d = Desk::new();
        d.term();
        let up = d.up();
        let w = d.apps.remove(up);
        w.dialog(&mut d.state, &mut d.events);
        d.apps.push(w);
        assert!(d.shows(UP), "behind its own window");
        d.choose(Some("up"));
        assert!(d.shows("org.mozilla.firefox.p-up#2"));
    }

    #[test]
    fn the_hosts_browser_waits_while_the_person_types_for_a_project() {
        let mut d = Desk::new();
        d.host();
        d.term();
        d.choose(Some("up"));
        d.up();
        d.press(UP);
        d.state.focus_app(wb_core::view::FIREFOX_APP);
        assert!(d.shows(UP) && d.keys() == UP, "a host's agent opens a page");
        // A panel opened by a program has had the keys since: the host's page takes them no more by itself than the project's.
        let pid = d.state.surface.panels.iter().find(|p| p.tab().key == UP).map(|p| p.pid).unwrap();
        let at = d.state.surface.get(pid).unwrap().tabs.iter().position(|t| t.key == HOST).unwrap();
        let alone = d.state.surface.tear_out(pid, at, 300.0, 300.0).unwrap();
        d.open(None, "dev.whaybench.answer");
        assert_eq!(d.keys(), "dev.whaybench.answer#1");
        d.state.focus_panel(alone);
        assert_eq!(d.keys(), "dev.whaybench.answer#1");
        d.press(HOST);
        assert_eq!(d.keys(), HOST, "but by the person's press");
    }

    #[test]
    fn what_the_person_asked_for_comes_no_more_once_they_went_elsewhere() {
        let mut d = Desk::new();
        d.host();
        d.term();
        d.choose(Some("up"));
        d.asked(1);
        d.press(HOST);
        d.up();
        assert!(d.shows(HOST) && d.keys() == HOST);
    }

    #[test]
    fn no_page_takes_what_is_typed_in_a_projects_conversation() {
        // A host's agent opens a page, in a browser that is there, and in one that starts.
        let mut d = Desk::new();
        d.host();
        d.term();
        d.choose(Some("up"));
        d.press(TERM);
        d.state.focus_app(wb_core::view::FIREFOX_APP);
        assert_eq!(d.keys(), TERM);
        d.choose(None);
        assert!(d.shows(HOST), "it comes up with a conversation of the host's");
        let mut d = Desk::new();
        d.open(None, "dev.whaybench.view");
        d.term();
        d.choose(Some("up"));
        d.press(TERM);
        d.host();
        assert_eq!(d.keys(), TERM);
        d.choose(None);
        d.press(TERM);
        d.state.focus_app(wb_core::view::FIREFOX_APP);
        assert_eq!(d.keys(), HOST, "in a conversation of the host's as always");
        // Beside the project's own page, and by whatever else would give the host's the keys.
        let mut d = Desk::new();
        d.host();
        d.term();
        d.choose(Some("up"));
        d.up();
        d.press(TERM);
        assert!(d.shows(UP));
        d.state.focus_app(wb_core::view::FIREFOX_APP);
        assert!(d.shows(UP) && d.keys() == TERM, "not over the project's page");
        let pid = d.state.surface.panels.iter().find(|p| p.tab().key == UP).map(|p| p.pid).unwrap();
        let at = d.state.surface.get(pid).unwrap().tabs.iter().position(|t| t.key == HOST).unwrap();
        let alone = d.state.surface.tear_out(pid, at, 300.0, 300.0).unwrap();
        d.state.focus_panel(alone);
        assert_eq!(d.keys(), TERM);
        // The conversation ends under the hands.
        let mut d = Desk::new();
        d.host();
        d.press(HOST);
        d.term();
        d.choose(Some("up"));
        d.press(TERM);
        d.close(1);
        assert_eq!(d.keys(), "");
    }

    #[test]
    fn a_page_asked_for_from_a_panel_stays_when_its_terminal_opens() {
        let mut d = Desk::new();
        d.host();
        d.open(None, "dev.whaybench.project");
        d.term();
        d.choose(Some("up"));
        d.up();
        d.press_tab("dev.whaybench.project#1");
        d.state.focus_app(wb_core::view::FIREFOX_APP);
        assert!(d.shows(HOST) && d.keys() == HOST, "inner login's page");
        d.open(None, "dev.whaybench.map");
        assert!(d.shows(HOST), "beside its terminal");
        // The same when the host's browser starts for it.
        let mut d = Desk::new();
        d.open(None, "dev.whaybench.project");
        d.term();
        d.choose(Some("up"));
        d.up();
        d.press_tab("dev.whaybench.project#1");
        d.host();
        assert!(d.shows(HOST) && d.keys() == HOST);
        d.open(None, "dev.whaybench.map");
        assert!(d.shows(HOST));
    }

    #[test]
    fn a_dialog_of_a_hidden_browser_waits_and_a_tab_the_person_chose_stays() {
        let mut d = Desk::new();
        d.host();
        d.term();
        let up = d.up();
        d.choose(Some("up"));
        d.press_tab(HOST);
        let panels = d.state.surface.panels.len();
        let w = d.apps.remove(up);
        w.dialog(&mut d.state, &mut d.events);
        d.apps.push(w);
        assert!(d.state.surface.panels.len() == panels && d.shows(HOST) && d.keys() == HOST, "not over the page typed in");
        let mut d = Desk::new();
        d.host();
        d.term();
        d.up();
        d.choose(Some("up"));
        d.press_tab(HOST);
        d.press(TERM);
        assert!(d.shows(HOST), "chosen by the person beside a project's conversation");
        d.choose(None);
        d.choose(Some("up"));
        assert!(d.shows(UP), "until another conversation is chosen");
    }

    #[test]
    fn a_browser_opens_where_a_browser_is_else_with_the_view() {
        for _ in 0..8 {
            let mut d = Desk::new();
            d.open(None, "dev.whaybench.notify");
            let pid = d.state.surface.panels[0].pid;
            d.state.surface.panels[0].tabs.push(crate::layout::Tab { id: 900, key: "dev.whaybench.answer#1".into(), name: "a".into() });
            d.open(None, "dev.whaybench.view");
            let view = d.state.surface.holding_tab(d.state.surface.panels.iter().flat_map(|p| p.tabs.iter()).find(|t| t.key == VIEW).unwrap().id).unwrap().pid;
            let at = d.state.surface.get(view).unwrap().tabs.iter().position(|t| t.key == VIEW).unwrap();
            let view = if view == pid { d.state.surface.tear_out(pid, at, 700.0, 100.0).unwrap() } else { view };
            d.host();
            let of = |d: &Desk, key: &str| d.state.surface.panels.iter().find(|p| p.tabs.iter().any(|t| t.key == key)).map(|p| p.pid);
            assert_eq!(of(&d, HOST), Some(view), "with the view, not with what else shares its group");
            let i = d.state.surface.get(view).unwrap().tabs.iter().position(|t| t.key == HOST).unwrap();
            let alone = d.state.surface.tear_out(view, i, 300.0, 300.0).unwrap();
            d.term();
            d.choose(Some("up"));
            d.up();
            assert_eq!(of(&d, UP), Some(alone), "where the person put the other browser");
            assert!(d.shows(UP));
            d.choose(None);
            assert!(d.shows(HOST), "they take turns there");
        }
    }

    /// What opens, closes, is dragged, pressed and chosen, in any order: the three things hold after each step.
    #[test]
    fn the_rules_hold_whatever_happens() {
        let steps = 60;
        for seed in 1..=std::env::var("WB_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(12u64) {
            let mut d = Desk::new();
            let mut n = seed.wrapping_mul(0x9e3779b97f4a7c15);
            let mut pick = move |of: usize| { n ^= n << 13; n ^= n >> 7; n ^= n << 17; (n >> 11) as usize % of.max(1) };
            d.term();
            // The conversation's window stays; `went`: the side whose browser the person went to, or asked for, last.
            let (mut names, mut log) = (vec!["term".to_string()], Vec::new());
            let mut went: Option<Option<String>> = None;
            for _ in 0..steps {
                let before = (d.state.keys_tab(), d.keys());
                let was_front = |d: &Desk, id: Option<u64>| id.is_some_and(|i| d.state.surface.holding_tab(i).is_some_and(|p| p.tab().id == i));
                let mut program = true;
                let modal = d.state.surface.modal().is_some();
                let typing = d.state.typing_for_project();
                let what = match pick(15) {
                    0 => { d.host(); names.push("host".into()); program = typing; "host opens".to_string() }
                    1 | 2 => { let p = ["up", "two"][pick(2)]; d.open(Some(p), "x"); names.push(p.into()); format!("{p} opens") }
                    3 => {
                        let Some(i) = (0..names.len()).filter(|i| ["up", "two"].contains(&names[*i].as_str())).nth(pick(3)) else { continue };
                        let w = d.apps.remove(i);
                        w.dialog(&mut d.state, &mut d.events);
                        d.apps.insert(i, w);
                        format!("dialog of {}", names[i])
                    }
                    4 => { let a = ["dev.whaybench.view", "dev.whaybench.notify", "dev.whaybench.project"][pick(3)]; d.open(None, a); names.push(a.into()); program = false; format!("{a} opens") }
                    5 => { if modal { continue; } d.open(None, "dev.whaybench.theme"); names.push("theme".into()); program = false; "settings open".into() }
                    6 | 7 => {
                        if names.len() < 2 { continue; }
                        let i = 1 + pick(names.len() - 1);
                        let w = names.remove(i);
                        program = false;
                        d.close(i);
                        format!("{w} closes")
                    }
                    8 | 9 => {
                        let keys: Vec<String> = d.state.surface.panels.iter().flat_map(|p| p.tabs.iter().map(|t| t.key.clone())).collect();
                        let k = keys[pick(keys.len())].clone();
                        if modal { continue; }
                        d.press_tab(&k);
                        if crate::browsers::is_browser(&k) { went = Some(wb_core::view::browser_project(&k).map(String::from)); } else { went = None; }
                        program = false;
                        format!("press {k}")
                    }
                    10 => { let c = [None, Some("up"), Some("two"), Some("plain")][pick(4)]; d.choose(c); format!("choose {c:?}") }
                    11 => { let p = ["up", "two"][pick(2)]; d.state.front_project_browser(p); format!("{p} asks") }
                    12 => {
                        let pids: Vec<u64> = d.state.surface.panels.iter().filter(|p| !p.transient).map(|p| p.pid).collect();
                        if pids.len() < 2 || modal { continue; }
                        let (a, b) = (pids[pick(pids.len())], pids[pick(pids.len())]);
                        if a == b { continue; }
                        // Held by its title bar, as a person does.
                        d.state.press_panel(a);
                        let k = d.keys();
                        went = crate::browsers::is_browser(&k).then(|| wb_core::view::browser_project(&k).map(String::from));
                        d.state.surface.apply_drop(a, crate::layout::Zone { target: b, how: crate::layout::How::Tab, preview: Default::default() });
                        d.state.focus_panel(b);
                        program = false;
                        "drop".into()
                    }
                    13 => { d.state.focus_app(wb_core::view::FIREFOX_APP); program = typing; "host asks".into() }
                    _ => {
                        // The conversation ends, and another tab is opened in its place.
                        if modal { continue; }
                        d.close(0);
                        let k = d.keys();
                        assert!(!typing || !crate::browsers::is_browser(&k), "{k} goes on with what was typed: seed {seed}: {}; the conversation ends", log.join("; "));
                        d.term();
                        let w = d.apps.pop().unwrap();
                        d.apps.insert(0, w);
                        if typing { went = Some(Some("-".into())); }
                        program = false;
                        "the conversation ends and another opens".into()
                    }
                };
                d.state.sync_focus();
                log.push(what);
                let at = || format!("seed {seed}: {}", log.join("; "));
                let keys = d.keys();
                // Settings keep the keyboard.
                if let Some(m) = d.state.surface.modal() { assert_eq!(keys, m.tab().key, "{}", at()); continue; }
                // A project's page has it only where the person went; what was typed for a project goes to no other page.
                if crate::browsers::is_browser(&keys) {
                    let side = wb_core::view::browser_project(&keys).map(String::from);
                    if side.is_some() { assert_eq!(went, Some(side.clone()), "{keys} has the keys: {}", at()); }
                    else if let Some(Some(p)) = &went { panic!("the host's page has the keys of {p}: {}", at()); }
                }
                // From a project's page to what a program opened since: no page has it by itself.
                else if !keys.is_empty() && matches!(went, Some(Some(_))) { went = Some(Some("-".into())); }
                // It is never in a tab that is hidden.
                if let Some(id) = d.state.keys_tab() { assert!(was_front(&d, Some(id)), "keys in a hidden {keys}: {}", at()); }
                // What a program does changes nothing under the hands (the view is looked at).
                if program && before.0.is_some() && !before.1.starts_with(wb_core::view::VIEW_APP) && d.state.windows.contains_key(&before.0.unwrap()) {
                    let dialog = d.state.keys_tab() != before.0 && keys.split('#').next() == before.1.split('#').next();
                    assert!(dialog || d.state.keys_tab() == before.0 && was_front(&d, before.0), "{} lost the keys to {keys}: {}", before.1, at());
                }
                // No page of another side is left in front, but one the person pressed or types in.
                let want = wb_core::view::chosen_browser(&d.data);
                for p in &d.state.surface.panels {
                    let (k, app) = (p.tab().key.clone(), p.tab().key.split('#').next().unwrap_or_default().to_string());
                    let alone = p.tabs.iter().all(|t| crate::browsers::is_browser(&t.key)) && !p.tabs.iter().any(|t| t.key.split('#').next() == Some(&want));
                    assert!(!crate::browsers::is_browser(&k) || app == want || k == keys || alone || d.state.browsers.is_pinned(p.tab().id), "{k} is left in front of {want}: {}", at());
                }
            }
        }
    }
}
