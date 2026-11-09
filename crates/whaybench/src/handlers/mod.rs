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

smithay::delegate_dispatch2!(App);
