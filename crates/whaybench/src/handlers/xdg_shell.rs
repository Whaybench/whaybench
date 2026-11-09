use smithay::{
    desktop::{PopupKind, find_popup_root_surface, get_popup_toplevel_coords},
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::protocol::{wl_seat, wl_surface::WlSurface},
    },
    utils::{Rectangle, Serial},
    wayland::shell::xdg::{PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState},
};

use crate::App;

impl XdgShellHandler for App {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    // The name and the parent come with the first commit: the panel opens then.
    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        self.unmapped.push(surface);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.close_toplevel(surface.wl_surface());
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|state| {
            let geometry = positioner.get_geometry();
            state.geometry = geometry;
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    // An app cannot move or resize its panel: that is done on the product's title bar and borders.
    fn move_request(&mut self, _surface: ToplevelSurface, _seat: wl_seat::WlSeat, _serial: Serial) {}

    fn resize_request(&mut self, _surface: ToplevelSurface, _seat: wl_seat::WlSeat, _serial: Serial, _edges: xdg_toplevel::ResizeEdge) {}

    // A menu that asked for the pointer closes when the person presses anywhere outside it. Only those: a tooltip or
    // other popup told to close that never asked comes out wrong in GTK.
    fn grab(&mut self, surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        if !self.grabbed_popups.contains(&surface) { self.grabbed_popups.push(surface); }
    }
}

/// Should be called on `WlSurface::commit`
pub fn handle_commit(state: &mut App, surface: &WlSurface) {
    if let Some(i) = state.unmapped.iter().position(|u| u.wl_surface() == surface) {
        let top = state.unmapped.remove(i);
        state.open_toplevel(top.clone());
        top.send_configure();
    }
    state.popups.commit(surface);
    if let Some(PopupKind::Xdg(xdg)) = state.popups.find_popup(surface) {
        if !xdg.is_initial_configure_sent() {
            // NOTE: This should never fail as the initial configure is always allowed.
            xdg.send_configure().expect("initial configure failed");
        }
    }
}

impl App {
    /// Menus and tooltips stay inside the panel's area.
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else { return };
        let Some(id) = self.windows.iter().find(|(_, w)| w.toplevel().unwrap().wl_surface() == &root).map(|(i, _)| *i) else { return };
        let Some(app) = self.surface.holding_tab(id).map(|p| self.surface.app_of(p.rect)) else { return };
        let coords = get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        let target = Rectangle::new((-coords.x, -coords.y).into(), (app.w as i32, app.h as i32).into());
        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}
