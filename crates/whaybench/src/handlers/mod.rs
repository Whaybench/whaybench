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
