//! Everything the compositor keeps, and what connects the surface's layout to the apps' windows.
use std::{collections::HashMap, ffi::OsString, sync::Arc, time::{Duration, Instant}};

use smithay::{
    desktop::{PopupManager, Space, Window, WindowSurfaceType},
    input::{Seat, SeatState},
    reexports::{
        calloop::{EventLoop, Interest, LoopSignal, Mode, PostAction, generic::Generic},
        wayland_protocols::xdg::{decoration::zv1::server::zxdg_toplevel_decoration_v1, shell::server::xdg_toplevel},
        wayland_server::{
            Display, DisplayHandle,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::{
        compositor::{CompositorClientState, CompositorState, with_states},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::xdg::{ToplevelSurface, XdgShellState, XdgToplevelSurfaceData, decoration::XdgDecorationState},
        shm::ShmState,
        socket::ListeningSocketSource,
        input_method::InputMethodManagerState,
        text_input::{TextInputActivation, TextInputManagerState, TextInputSeat},
        virtual_keyboard::VirtualKeyboardManagerState,
    },
};
use wb_core::{log as record, paths::{self, Data}, theme::{self, Theme}};

use crate::{chrome::{Chrome, cut_name}, layout::{self, Edges, Guide, Metrics, Panel, Rect, SavedLayout, Side, Surface, Tab, Zone}, };

/// The status line: its app, and how high it is.
pub const STATUS_APP: &str = "dev.whaybench.status";
pub const STATUS_H: i32 = 26;
pub const MENU_APP: &str = "dev.whaybench.menu";
pub const MENU_H: i32 = 30;

/// `end_reason` of a restart: the program starts itself again after it has ended.
pub const RESTART: &str = "restart";

/// Which place of the first layout an app has; apps of one place share a panel as tabs.
pub fn slot_of(app: &str) -> Option<&'static str> {
    Some(match app {
        "dev.whaybench.map" | "dev.whaybench.map-branch" => "map",
        "dev.whaybench.sessions" | "dev.whaybench.files" => "sessions",
        "whaybench-term" => "term",
        "dev.whaybench.answer" => "answer",
        "dev.whaybench.input" => "input",
        "org.mozilla.firefox" | "firefox" | "dev.whaybench.view-term" | "dev.whaybench.notify" | "dev.whaybench.project" => "view",
        v if v == wb_core::view::VIEW_APP || v.starts_with("dev.whaybench.view.") => "view",
        v if v.starts_with("dev.whaybench.map.") => "map",
        _ => return None,
    })
}

/// Apps whose window is a settings window, not a panel.
const MODAL_APPS: [&str; 4] = ["dev.whaybench.theme", "dev.whaybench.mapstyle", "dev.whaybench.prefs", "dev.whaybench.inputset"];

fn front_of_group(key: &str) -> bool { key.starts_with("dev.whaybench.sessions#") || key.starts_with("dev.whaybench.view#") }

/// A first window when no size was saved.
pub const FIRST_WINDOW: (f64, f64) = (1500.0, 900.0);

/// What the pointer is doing, from press to release.
pub enum Act {
    Idle,
    /// The panel follows the pointer, stiffly; gx, gy is where it was picked up.
    Hold { pid: u64, gx: f64, gy: f64 },
    Resize { pid: u64, edges: Edges, partners: Vec<(u64, Side)> },
    /// A tab pressed: it may be pulled out of the bar.
    TabPull { pid: u64, tab: usize, x: f64, y: f64 },
    /// The bar's close button pressed, or anything else that the app must not see the release of.
    Swallow,
}

pub struct App {
    pub surface: Surface,
    pub windows: HashMap<u64, Window>,
    pub unmapped: Vec<ToplevelSurface>,
    next_tab: u64,
    pub act: Act,
    pub pointer: (f64, f64),
    pub hover: Option<(u64, Edges, Vec<(u64, Side)>)>,
    pub guides: Vec<Guide>,
    pub zone: Option<Zone>,
    pub band: (Option<(u64, Side)>, Instant),
    pub chrome: Chrome,
    /// The person's zoom of the title bars (Ctrl+wheel on one), over the theme's text size; in settings.json.
    pub title_zoom: f64,
    pub solid_ids: Vec<(smithay::backend::renderer::element::Id, [f32; 4])>,
    /// The shaded boxes of the last frame, with how each looked (render::Frame).
    /// An app asked to the front before its window appeared, and when.
    pub focus_when_open: Option<(String, std::time::Instant)>,
    pub shaded: Vec<(String, smithay::backend::renderer::gles::element::PixelShaderElement)>,
    pub shots: Vec<crate::ctl::Shot>,
    pub end_reason: String,
    pub theme: Theme,
    /// Which window is which conversation tab: window id to the relay's tab id.
    pub terms: HashMap<u64, String>,
    /// Which window goes in which place of the first layout.
    pub slots: HashMap<u64, String>,
    /// The status line: below the surface, not a panel, not movable.
    pub status: Option<Window>,
    /// The front tab and session last made the chosen session.
    pub focused_term: Option<(String, Option<String>)>,
    pub shaders: Option<crate::render::Shaders>,
    pub grabbed_popups: Vec<smithay::wayland::shell::xdg::PopupSurface>,
    data: Data,
    saved: SavedLayout,
    pub menu: Option<Window>,
    theme_seen: std::time::Instant,
    pub window_size: (i32, i32),
    save_due: bool,

    pub start_time: Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,
    pub space: Space<Window>,
    pub loop_signal: LoopSignal,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    /// linux-dmabuf, offered only when the window's GPU has a render node apps can use (not under WSLg).
    pub dmabuf: Option<smithay::wayland::dmabuf::DmabufState>,
    pub gpu: Option<std::rc::Rc<std::cell::RefCell<smithay::backend::winit::WinitGraphicsBackend<smithay::backend::renderer::gles::GlesRenderer>>>>,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<App>,
    pub data_device_state: DataDeviceState,
    pub clip: crate::clipboard::Bridge,
    pub popups: PopupManager,
    pub seat: Seat<Self>,
    /// Whether fcitx5 inside is the input method; else the desktop's, through the window.
    pub builtin_ime: bool,
}

pub fn read_layout(data: &Data) -> SavedLayout {
    let Ok(text) = std::fs::read_to_string(data.layout()) else { return SavedLayout::default() };
    match SavedLayout::parse(&text) {
        Some(l) => { record::decide("screen", "layout read", "the saved layout", &[("panels", l.panels.len().to_string())]); l }
        None => { record::warn("screen", "layout not read", "the file format differs; starting with the first layout", &[]); SavedLayout::default() }
    }
}

impl App {
    pub fn new(event_loop: &mut EventLoop<Self>, display: Display<Self>, data: Data, saved: SavedLayout) -> Self {
        let dh = display.handle();
        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        XdgDecorationState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let clip = crate::clipboard::Bridge::start(&data);
        let mut seat_state = SeatState::new();
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "whaybench");
        seat.add_keyboard(Default::default(), 200, 25).unwrap();
        seat.add_pointer();

        TextInputManagerState::new::<Self>(&dh);
        // The input method lives inside: fcitx5 attaches to this display (input-method-v2, with a virtual keyboard for
        // the keys it gives back), so typing works the same whatever the desktop offers. WSLg, for one, has only the
        // old text-input-v1 the window cannot use. Without fcitx5, or with `"input_method": "host"` in settings.json,
        // the desktop's input method reaches the apps through the window as before.
        let builtin_ime = crate::start::builtin_input_method(&data);
        if builtin_ime {
            InputMethodManagerState::new::<Self, _>(&dh, |_| true);
            VirtualKeyboardManagerState::new::<Self, _>(&dh, |_| true);
        } else {
            seat.text_input().set_compositor_input_method(true);
        }

        let socket_name = Self::init_wayland_listener(display, event_loop);
        let loop_signal = event_loop.get_signal();
        let now = Instant::now();
        let theme = theme::load_active(&data);
        let mut surface = Surface::new(FIRST_WINDOW.0, FIRST_WINDOW.1);
        surface.m = Metrics { outer: theme.outer, gap: theme.gap, pad: theme.pad };
        record::decide("screen", "theme", "the chosen theme", &[("theme", theme.name.clone())]);
        let title_zoom = title_zoom(&data);
        layout::set_title(theme.title_size * title_zoom);
        let chrome = Chrome::new(theme.title_size * title_zoom);
        Self {
            surface,
            theme,
            terms: HashMap::new(),
            slots: HashMap::new(),
            status: None,
            focused_term: None,
            shaders: None,
            grabbed_popups: Vec::new(),
            windows: HashMap::new(), unmapped: Vec::new(), next_tab: 1,
            act: Act::Idle, pointer: (0.0, 0.0), hover: None, guides: Vec::new(), zone: None, band: (None, now),
            chrome, solid_ids: Vec::new(), shaded: Vec::new(), focus_when_open: None,
            title_zoom,
            shots: Vec::new(), end_reason: String::new(), data, saved, menu: None, theme_seen: std::time::Instant::now(), window_size: (FIRST_WINDOW.0 as i32, FIRST_WINDOW.1 as i32), save_due: false,
            start_time: now, display_handle: dh, space: Space::default(), loop_signal, socket_name,
            compositor_state, xdg_shell_state, shm_state, dmabuf: None, gpu: None, output_manager_state, seat_state, data_device_state, clip,
            popups: PopupManager::default(), seat, builtin_ime,
        }
    }

    pub fn saved_window(&self) -> Option<(i32, i32)> { self.saved.window.filter(|(w, h)| *w >= 200 && *h >= 200) }

    fn init_wayland_listener(display: Display<App>, event_loop: &mut EventLoop<Self>) -> OsString {
        let listening_socket = ListeningSocketSource::new_auto().unwrap();
        let socket_name = listening_socket.socket_name().to_os_string();
        let loop_handle = event_loop.handle();
        loop_handle.insert_source(listening_socket, move |client_stream, _, state| {
            // One app that cannot connect does not stop the others.
            if let Err(e) = state.display_handle.insert_client(client_stream, Arc::new(ClientState::default())) { record::error("run", "app not connected", &[("error", e.to_string())]); }
        }).expect("Failed to init the wayland event source.");
        loop_handle.insert_source(Generic::new(display, Interest::READ, Mode::Level), |_, display, state| {
            // Safety: we don't drop the display
            unsafe { display.get_mut().dispatch_clients(state).unwrap(); }
            Ok(PostAction::Continue)
        }).unwrap();
        socket_name
    }

    /// Opens a conversation tab; it joins the other conversation tabs when its window appears.
    pub fn open_terminal(&mut self, open: &crate::start::TermOpen) {
        match crate::start::terminal_command(open, &self.data) {
            Ok(command) => {
                let run = record::run_id(&self.data);
                crate::start::spawn_app(&command, &self.socket_name, &self.data, &run);
            }
            Err(e) => {
                // Said, not only logged: otherwise the click seems to do nothing (an agent add-on deleted, for one).
                wb_core::status::say(&self.data, &wb_core::i18n::f("tab-not-opened", &[("e", e.clone())]), 8);
                record::error("session", "tab not opened", &[("error", e)]);
            }
        }
    }

    pub fn spawn(&self, command: &str) {
        crate::start::spawn_app(command, &self.socket_name, &self.data, &record::run_id(&self.data));
    }

    /// Brings the panel of an app to the front, its tab in front.
    pub fn focus_app(&mut self, app: &str) {
        let prefix = format!("{app}#");
        let found = self.surface.panels.iter().find_map(|p| p.tabs.iter().position(|t| t.key.starts_with(&prefix)).map(|i| (p.pid, i)));
        // Not open yet (a browser just being started): it comes to the front when its window appears.
        let Some((pid, i)) = found else { self.focus_when_open = Some((prefix, std::time::Instant::now())); return };
        if let Some(p) = self.surface.get_mut(pid) { p.active = i; }
        self.focus_panel(pid);
        self.save_due = true;
    }

    /// This app's tab comes up in its panel, and the keyboard stays where it is: what the AI shows must not take the
    /// keys from someone typing.
    pub fn front_app(&mut self, app: &str) {
        let prefix = format!("{app}#");
        let found = self.surface.panels.iter().find_map(|p| p.tabs.iter().position(|t| t.key.starts_with(&prefix)).map(|i| (p.pid, i)));
        let Some((pid, i)) = found else { return };
        if let Some(p) = self.surface.get_mut(pid) { p.active = i; }
        self.save_due = true;
    }

    /// A press outside the open menus closes them, newest first. Without this an app in another panel never hears
    /// of the press, and the menu stays on the screen.
    pub fn close_popups_unless_at(&mut self, pos: Point<f64, Logical>) {
        self.grabbed_popups.retain(|p| p.alive());
        if self.grabbed_popups.is_empty() { return; }
        let under = self.surface_under(pos).map(|h| h.0);
        if under.is_some_and(|s| self.grabbed_popups.iter().any(|p| p.wl_surface() == &s)) { return; }
        for p in self.grabbed_popups.drain(..).rev() { p.send_popup_done(); }
        let _ = self.display_handle.flush_clients();
    }

    /// The menu bar or one of its menus at this point. Its menus lie over the panels, title bars and borders included.
    pub fn menu_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        let w = self.menu.as_ref()?;
        let origin: Point<f64, Logical> = (0.0, -self.menu_height() as f64).into();
        w.surface_under(pos - origin, WindowSurfaceType::ALL).map(|(s, at)| (s, at.to_f64() + origin))
    }

    /// Pixels the menu bar takes at the top of the window. The surface starts below it: the output is mapped this far
    /// above the surface's origin, so pointer positions come out in surface coordinates and the menu bar is at y < 0.
    pub fn menu_height(&self) -> i32 { if self.menu.is_some() { MENU_H } else { 0 } }

    /// The surface takes what the menu bar and the status line leave.
    pub fn fit_strips(&mut self) {
        let (w, h) = self.window_size;
        let top = self.menu_height();
        let below = if self.status.is_some() { STATUS_H } else { 0 };
        let output = self.space.outputs().next().cloned();
        if let Some(output) = output { self.space.map_output(&output, (0, -top)); }
        self.surface.rescale(w as f64, (h - top - below) as f64);
    }

    /// The whole program again, over the same data: its tabs come back as after closing the window.
    pub fn restart(&mut self) {
        self.end_reason = RESTART.into();
        self.loop_signal.stop();
    }

    /// A panel of the panel menu: closed if it is open, opened if it is not. `key` is a tab's key or an app id.
    pub fn toggle_panel(&mut self, key: &str, command: Option<&str>) {
        let prefix = format!("{key}#");
        let found = self.surface.panels.iter().flat_map(|p| p.tabs.iter()).find(|t| t.key == key || t.key.starts_with(&prefix)).map(|t| t.id);
        match (found, command) {
            (Some(id), _) => { if let Some(w) = self.windows.get(&id) { w.toplevel().unwrap().send_close(); } }
            (None, Some(c)) => self.spawn(c),
            (None, None) => {}
        }
    }

    /// Every panel to its first place, groups as at first; branches and extra views close.
    pub fn reset_layout(&mut self) {
        let slots = self.slots.clone();
        let close: Vec<u64> = self.surface.panels.iter().flat_map(|p| p.tabs.iter())
            .filter(|t| t.key.starts_with("dev.whaybench.view.") || t.key.starts_with("dev.whaybench.map.") || t.key.starts_with("dev.whaybench.map#") && !t.key.ends_with("#1") || t.key.starts_with("dev.whaybench.map-branch"))
            .map(|t| t.id).collect();
        for id in close { if let Some(w) = self.windows.get(&id) { w.toplevel().unwrap().send_close(); } }
        self.saved = SavedLayout::default();
        self.surface.reset_places(|t| slots.get(&t.id).cloned(), front_of_group);
        self.save_due = true;
        record::decide("screen", "layout reset", "chosen from the menu", &[]);
    }

    /// Whether the status line is at this point.
    pub fn status_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        let w = self.status.as_ref()?;
        let top = self.surface.h;
        if pos.y < top { return None; }
        let origin: Point<f64, Logical> = (0.0, top).into();
        w.surface_under(pos - origin, WindowSurfaceType::ALL).map(|(s, at)| (s, at.to_f64() + origin))
    }

    /// Brings a conversation tab to the front.
    pub fn focus_term(&mut self, tab: &str) {
        let Some(id) = self.terms.iter().find(|(_, t)| t.as_str() == tab).map(|(i, _)| *i) else { return };
        let Some(pid) = self.surface.holding_tab(id).map(|p| p.pid) else { return };
        if let Some(p) = self.surface.get_mut(pid)
            && let Some(i) = p.tabs.iter().position(|t| t.id == id) { p.active = i; }
        self.focus_panel(pid);
    }

    /// Closes a conversation tab the way its close button does.
    pub fn close_term(&mut self, tab: &str) {
        let Some(id) = self.terms.iter().find(|(_, t)| t.as_str() == tab).map(|(i, _)| *i) else { return };
        if let Some(w) = self.windows.get(&id) { w.toplevel().unwrap().send_close(); }
    }

    /// A session asked from another tab has answered: back to the tab that asked, if the asked one is still in front
    /// (the person has not gone elsewhere meanwhile). A question left unanswered for an hour is let go.
    pub fn return_focus(&mut self) {
        let file = crate::ctl::return_file(&self.data);
        let Some(back) = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { return };
        let (Some(session), Some(tab), Some(at)) = (back["session"].as_str(), back["tab"].as_str(), back["at"].as_i64()) else { let _ = std::fs::remove_file(&file); return };
        if crate::ctl::now_ms() - at > 3_600_000 { let _ = std::fs::remove_file(&file); return; }
        let state = std::fs::read_to_string(wb_core::sessions::state_dir(&self.data).join(format!("{session}.json"))).ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).unwrap_or_default();
        if state["state"] != "done" || state["at"].as_i64().unwrap_or(0) <= at { return; }
        let _ = std::fs::remove_file(&file);
        if self.focused_term.as_ref().and_then(|(_, s)| s.as_deref()) == Some(session) {
            record::decide("session", "back to the asking tab", "the asked session answered", &[("session", session.to_string()), ("tab", tab.to_string())]);
            let tab = tab.to_string();
            self.focus_term(&tab);
        }
    }

    /// The conversation in front is the chosen session, so the map, answer and files panels follow it and requests go
    /// to it: the front tab of the conversation panel chosen last, whatever free panel lies above it, else any
    /// conversation tab left. With no conversation tab left, no session is chosen. Only a change counts.
    pub fn sync_focus(&mut self) {
        let front = self.surface.by_choice().find_map(|p| self.terms.get(&p.tab().id))
            .or_else(|| self.surface.by_choice().flat_map(|p| p.tabs.iter()).find_map(|t| self.terms.get(&t.id))).cloned();
        let Some(tab) = front else {
            if self.focused_term.take().is_some() { wb_core::sessions::clear_focus(&self.data); }
            return;
        };
        let session = std::fs::read_to_string(wb_core::sessions::tab_file(&self.data, &tab)).ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.get("session").and_then(|s| s.as_str()).map(String::from));
        let pair = (tab, session);
        if self.focused_term.as_ref() != Some(&pair) {
            match &pair.1 {
                Some(s) => wb_core::sessions::set_focus(&self.data, s, wb_core::i18n::t("input")),
                None => wb_core::sessions::set_focus_empty(&self.data, wb_core::i18n::t("input")),
            }
            self.focused_term = Some(pair);
        }
    }

    fn key_for(&self, app_id: &str) -> String {
        let open: Vec<&str> = self.surface.panels.iter().flat_map(|p| p.tabs.iter().map(|t| t.key.as_str())).collect();
        (1..).map(|n| format!("{app_id}#{n}")).find(|k| !open.contains(&k.as_str())).unwrap()
    }

    /// An app's window has committed for the first time, so its name and parent are known: it gets a panel.
    pub fn open_toplevel(&mut self, surface: ToplevelSurface) {
        let (title, app_id, parent) = with_states(surface.wl_surface(), |s| {
            let d = s.data_map.get::<XdgToplevelSurfaceData>().unwrap().lock().unwrap();
            (d.title.clone(), d.app_id.clone(), d.parent.clone())
        });
        let app_id = app_id.filter(|a| !a.is_empty()).unwrap_or_else(|| "app".into());
        // Ubuntu's Firefox is a snap, and a snap names its window <snap>_<app>: it is the view panel's browser all the same.
        let app_id = if app_id == "firefox_firefox" { wb_core::view::FIREFOX_APP.to_string() } else { app_id };
        if app_id == STATUS_APP || app_id == MENU_APP {
            let (w, _) = self.window_size;
            let high = if app_id == STATUS_APP { STATUS_H } else { MENU_H };
            surface.with_pending_state(|s| s.size = Some((w, high).into()));
            let window = Window::new_wayland_window(surface);
            self.space.map_element(window.clone(), (0, 0), false);
            if app_id == STATUS_APP { self.status = Some(window); } else { self.menu = Some(window); }
            self.fit_strips();
            return;
        }
        let id = self.next_tab;
        self.next_tab += 1;
        let name = cut_name(title.as_deref().filter(|t| !t.is_empty()).unwrap_or(&app_id));
        // A conversation tab's app id carries its tab id; its place in the layout does not depend on that.
        let term = app_id.strip_prefix(crate::start::TERM_APP).and_then(|t| t.strip_prefix('-')).map(String::from);
        let key_base = if term.is_some() { crate::start::TERM_APP.to_string() } else { app_id.clone() };
        let name = if term.is_some() && title.as_deref().is_none_or(|t| t.is_empty()) { wb_core::i18n::t("new-session").to_string() } else { name };
        let tab = Tab { id, key: self.key_for(&key_base), name };
        if let Some(t) = term.clone() { self.terms.insert(id, t); }
        let slot = slot_of(&key_base);
        if let Some(sl) = slot { self.slots.insert(id, sl.to_string()); }
        let key = tab.key.clone();

        // A settings window: centred over the surface, in front of everything until it closes.
        let modal = MODAL_APPS.contains(&app_id.as_str());
        // A dialog stays over its parent, inside it; a window without a parent is a panel of its own.
        let parent_panel = parent.as_ref().and_then(|ps| self.windows.iter().find(|(_, w)| w.toplevel().unwrap().wl_surface() == ps).map(|(i, _)| *i)).and_then(|i| self.surface.holding_tab(i)).map(|p| p.rect);
        let parent_modal = parent.as_ref().and_then(|ps| self.windows.iter().find(|(_, w)| w.toplevel().unwrap().wl_surface() == ps).map(|(i, _)| *i))
            .and_then(|i| self.surface.holding_tab(i)).is_some_and(|p| p.modal);
        // A dialog of a window that is not a panel (the menu bar, the status line) goes where a settings window goes.
        let pid = if modal || (parent.is_some() && parent_panel.is_none()) {
            let pid = self.surface.new_pid();
            let (w, h) = ((self.surface.w - 80.0).clamp(layout::MIN_W, 760.0), (self.surface.h - 80.0).max(layout::MIN_H));
            let mut p = Panel::new(pid, tab, Rect::new(((self.surface.w - w) / 2.0).max(0.0), 40.0_f64.min(self.surface.h / 10.0), w, h));
            p.transient = true;
            p.fitted = true;
            p.modal = modal;
            self.surface.panels.push(p);
            pid
        } else if let Some(over) = parent_panel {
            let pid = self.surface.new_pid();
            let mut p = Panel::new(pid, tab, Rect::new(over.x + 40.0, over.y + 40.0, (over.w - 80.0).max(layout::MIN_W), (over.h - 80.0).max(layout::MIN_H)));
            p.transient = true;
            p.fitted = true;
            // A dialog of a settings window is in front of it and takes the input until it closes.
            p.modal = parent_modal;
            self.surface.panels.push(p);
            pid
        } else {
            match self.surface.place_saved(&self.saved, tab) {
                Ok(pid) => {
                    // A member of this place that the layout does not know opened first and sat here alone: it comes in.
                    let strays: Vec<u64> = self.slots.iter().filter(|(k, v)| **k != id && Some(v.as_str()) == slot).map(|(k, _)| *k)
                        .filter(|k| self.surface.holding_tab(*k).is_some_and(|p| p.pid != pid && !p.tabs.iter().any(|t| self.saved.panels.iter().any(|s| s.tabs.contains(&t.key))))).collect();
                    if !strays.is_empty() {
                        for k in strays { self.surface.move_tab_to(k, pid); }
                        if let Some(p) = self.surface.get_mut(pid)
                            && !front_of_group(&p.tabs[p.active].key) && let Some(i) = p.tabs.iter().position(|t| front_of_group(&t.key)) { p.active = i; }
                    }
                    pid
                }
                Err(tab) => {
                    // Panels of one place stay together in one panel, as tabs.
                    let others: Vec<u64> = self.slots.iter().filter(|(k, v)| **k != id && Some(v.as_str()) == slot).map(|(k, _)| *k).collect();
                    let joined = others.iter().find_map(|k| self.surface.holding_tab(*k)).map(|p| p.pid);
                    if let Some(pid) = joined {
                        let p = self.surface.get_mut(pid).unwrap();
                        p.tabs.push(tab);
                        p.active = p.tabs.len() - 1;
                        // Whatever order they open in, these stay in front of their group.
                        if !front_of_group(&p.tabs[p.active].key) && let Some(i) = p.tabs.iter().position(|t| front_of_group(&t.key)) { p.active = i; }
                        pid
                    } else {
                        let pid = self.surface.new_pid();
                        let rect = slot.and_then(|sl| self.surface.first_place(sl)).unwrap_or_else(|| self.surface.free_place());
                        self.surface.panels.push(Panel::new(pid, tab, rect));
                        pid
                    }
                }
            }
        };
        let (fitted, rect) = self.surface.get(pid).map(|p| (p.fitted, p.rect)).unwrap();
        let r = self.surface.app_of(rect);
        surface.with_pending_state(|s| {
            // A dialog or settings window takes the size its content wants, up to its place (the bounds: what is wider
            // would be cut off, buttons and all); a panel takes its place.
            if fitted { s.bounds = Some((r.w as i32, r.h as i32).into()); } else { s.size = Some((r.w as i32, r.h as i32).into()); }
            s.decoration_mode = Some(zxdg_toplevel_decoration_v1::Mode::ServerSide);
            // Tiled: apps leave out their own shadows and rounded corners.
            for t in [xdg_toplevel::State::TiledLeft, xdg_toplevel::State::TiledRight, xdg_toplevel::State::TiledTop, xdg_toplevel::State::TiledBottom] { s.states.set(t); }
        });
        let window = Window::new_wayland_window(surface);
        self.space.map_element(window.clone(), (r.x as i32, r.y as i32), false);
        self.windows.insert(id, window);
        let asked = self.focus_when_open.take_if(|(prefix, at)| key.starts_with(prefix.as_str()) && at.elapsed() < std::time::Duration::from_secs(30));
        if asked.is_some()
            && let Some(p) = self.surface.get_mut(pid) && let Some(i) = p.tabs.iter().position(|t| t.id == id) { p.active = i; }
        record::decide("screen", "panel opened", if fitted { "a dialog or settings window, at its content's size" } else { "a new app" }, &[("panel", pid.to_string()), ("app", key)]);
        self.focus_panel(pid);
        self.save_due = true;
    }

    /// An app's window is gone.
    pub fn close_toplevel(&mut self, surface: &WlSurface) {
        if self.status.as_ref().is_some_and(|w| w.toplevel().unwrap().wl_surface() == surface) {
            if let Some(w) = self.status.take() { self.space.unmap_elem(&w); }
            self.fit_strips();
            return;
        }
        if self.menu.as_ref().is_some_and(|w| w.toplevel().unwrap().wl_surface() == surface) {
            if let Some(w) = self.menu.take() { self.space.unmap_elem(&w); }
            self.fit_strips();
            return;
        }
        let Some(id) = self.windows.iter().find(|(_, w)| w.toplevel().unwrap().wl_surface() == surface).map(|(i, _)| *i) else {
            self.unmapped.retain(|u| u.wl_surface() != surface);
            return;
        };
        if let Some(w) = self.windows.remove(&id) { self.space.unmap_elem(&w); }
        self.terms.remove(&id);
        self.slots.remove(&id);
        let holder = self.surface.holding_tab(id).map(|p| p.pid);
        self.surface.remove_tab(id);
        record::decide("screen", "panel closed", "the app ended", &[("panel", holder.map(|p| p.to_string()).unwrap_or_default())]);
        if let Act::Hold { pid, .. } | Act::Resize { pid, .. } | Act::TabPull { pid, .. } = self.act
            && self.surface.get(pid).is_none() { self.act = Act::Idle; self.zone = None; self.guides.clear(); }
        self.hover = None;
        if let Some(front) = self.surface.front() { self.focus_panel(front); }
        self.save_due = true;
    }

    /// Raises the panel; its front tab gets the keyboard. While a settings window is open, it keeps the keyboard.
    pub fn focus_panel(&mut self, pid: u64) {
        let pid = self.surface.modal().map_or(pid, |m| m.pid);
        self.surface.raise(pid);
        let Some(front) = self.surface.get(pid).map(|p| p.tab().id) else { return };
        for (id, w) in &self.windows {
            w.set_activated(*id == front);
            w.toplevel().unwrap().send_pending_configure();
        }
        if let Some(w) = self.windows.get(&front) {
            let target = w.toplevel().unwrap().wl_surface().clone();
            let keyboard = self.seat.get_keyboard().unwrap();
            keyboard.set_focus(self, Some(target), SERIAL_COUNTER.next_serial());
        }
        // The panels follow the conversation chosen now, not at the next tick.
        self.sync_focus();
    }

    /// Where the panel's front app has its origin on the surface.
    pub fn app_origin(&self, p: &Panel) -> Option<Point<f64, Logical>> {
        let g = self.windows.get(&p.tab().id)?.geometry().loc;
        let a = self.surface.app_of(p.rect);
        Some((a.x - g.x as f64, a.y - g.y as f64).into())
    }

    /// The app surface under the pointer, unless a title bar or a border is there.
    pub fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        // While a settings window is open, nothing else is reached.
        if self.surface.modal().is_some_and(|m| !m.rect.contains(pos.x, pos.y)) { return None; }
        if let Some(hit) = self.menu_under(pos) { return Some(hit); }
        if let Some(hit) = self.status_under(pos) { return Some(hit); }
        let p = self.surface.top_at(pos.x, pos.y)?;
        if p.rect.bar().contains(pos.x, pos.y) || self.surface.edges_at(pos.x, pos.y).is_some() { return None; }
        let w = self.windows.get(&p.tab().id)?;
        let origin = self.app_origin(p)?;
        w.surface_under(pos - origin, WindowSurfaceType::ALL).map(|(s, at)| (s, at.to_f64() + origin))
    }

    pub fn note_saved_soon(&mut self) { self.save_due = true; }

    /// Takes a theme at once: panels keep their places and what they hold on to.
    /// The title bars' text and height: the theme's size times the person's zoom.
    fn apply_title(&mut self) {
        let size = self.theme.title_size * self.title_zoom;
        self.chrome.set_size(size);
        layout::set_title(size);
    }

    /// Ctrl+wheel on a title bar: a tenth larger or smaller per notch, kept in settings.json.
    pub fn zoom_title(&mut self, notches: f64) {
        let z = ((self.title_zoom + notches * 0.1) * 10.0).round() / 10.0;
        let z = z.clamp(0.7, 2.0);
        if z == self.title_zoom { return; }
        self.title_zoom = z;
        self.apply_title();
        let file = self.data.root.join("settings.json");
        let mut all: serde_json::Map<String, serde_json::Value> = paths::read_kept(&file);
        if z == 1.0 { all.remove("title_zoom"); } else { all.insert("title_zoom".into(), z.into()); }
        if let Err(e) = paths::write_atomic(&file, serde_json::to_string_pretty(&serde_json::Value::Object(all)).unwrap_or_default()) { record::warn("screen", "title zoom not saved", &e.to_string(), &[]); }
        self.save_due = true;
    }

    pub fn set_theme(&mut self, name: &str) {
        let Some(t) = Theme::load(&self.data, name) else {
            record::warn("screen", "no theme", "no theme of that name", &[("theme", name.to_string())]);
            return;
        };
        self.surface.set_metrics(Metrics { outer: t.outer, gap: t.gap, pad: t.pad });
        self.theme = t;
        self.apply_title();
        if let Err(e) = theme::save_active(&self.data, name) { record::warn("screen", "theme not saved", &e.to_string(), &[]); }
        record::decide("screen", "theme", "requested", &[("theme", name.to_string())]);
        let _ = theme::write_terminal_config(&self.data);
        self.save_due = true;
    }

    /// Four times a second, as the panels do: the theme on file may have changed, or its folder may be gone.
    fn follow_theme(&mut self) {
        if self.theme_seen.elapsed() < std::time::Duration::from_millis(250) { return; }
        self.theme_seen = std::time::Instant::now();
        let now = theme::load_active(&self.data);
        if now != self.theme {
            record::decide("screen", "theme", "the file changed", &[("theme", now.name.clone())]);
            // Open terminals read their settings file again when it changes.
            let _ = theme::write_terminal_config(&self.data);
            self.surface.set_metrics(Metrics { outer: now.outer, gap: now.gap, pad: now.pad });
            self.theme = now;
            self.apply_title();
            self.save_due = true;
        }
    }

    /// One frame's work outside drawing: movement, sizes, names, the drop preview, saving.
    pub fn frame(&mut self, ms: f64) {
        self.follow_theme();
        self.surface.step(ms);
        self.surface.keep_order();
        self.update_hold();
        let pad = self.surface.m.pad;
        for p in &mut self.surface.panels {
            if !p.fitted { continue; }
            if let Some(w) = self.windows.get(&p.tab().id) {
                let g = w.geometry().size;
                if g.w > 0 && g.h > 0 {
                    let w = g.w as f64 + 2.0 * pad;
                    // A settings window that changes width keeps its middle where it was: placed in the middle of the
                    // screen, it stays there at the size its content gives it, and where the person moved it, there.
                    if p.modal && (w - p.rect.w).abs() > 0.5 { p.rect.x = (p.rect.x + (p.rect.w - w) / 2.0).max(0.0); }
                    p.rect.w = w;
                    p.rect.h = g.h as f64 + layout::title() + pad;
                }
            }
        }
        for p in &self.surface.panels {
            let a = self.surface.app_of(p.rect);
            for t in &p.tabs {
                let Some(w) = self.windows.get(&t.id) else { continue };
                self.space.map_element(w.clone(), (a.x as i32, a.y as i32), false);
                if !p.fitted {
                    let top = w.toplevel().unwrap();
                    top.with_pending_state(|s| s.size = Some((a.w as i32, a.h as i32).into()));
                    top.send_pending_configure();
                }
            }
        }
        for p in &mut self.surface.panels {
            for t in &mut p.tabs {
                let Some(w) = self.windows.get(&t.id) else { continue };
                let title = with_states(w.toplevel().unwrap().wl_surface(), |s| s.data_map.get::<XdgToplevelSurfaceData>().unwrap().lock().unwrap().title.clone());
                if let Some(title) = title.filter(|n| !n.is_empty()) {
                    let cut = cut_name(&title);
                    if cut != t.name { t.name = cut; }
                }
            }
        }
        let (width, _) = self.window_size;
        for (w, y, high) in [(self.status.clone(), self.surface.h as i32, STATUS_H), (self.menu.clone(), -self.menu_height(), MENU_H)] {
            let Some(w) = w else { continue };
            self.space.map_element(w.clone(), (0, y), false);
            let top = w.toplevel().unwrap();
            top.with_pending_state(|s| s.size = Some((width, high).into()));
            top.send_pending_configure();
        }
        if self.save_due && matches!(self.act, Act::Idle) { self.save_layout(); }
    }

    /// The preview of a drop can change without the pointer moving: the dwell in a band runs out.
    pub fn update_hold(&mut self) {
        let Act::Hold { pid, gx, gy } = self.act else { return };
        let (x, y) = self.pointer;
        // A dialog or a settings window moves freely: it never joins the panels.
        let free = self.seat.get_keyboard().is_some_and(|k| k.modifier_state().alt) || self.surface.get(pid).is_some_and(|p| p.transient);
        let band = self.surface.band_at(pid, x, y);
        if band != self.band.0 { self.band = (band, Instant::now()); }
        let dwell = if band.is_some() { self.band.1.elapsed() } else { Duration::ZERO };
        self.zone = if free { None } else { self.surface.drop_zone(pid, x, y, dwell) };
        let (tx, ty, guides) = if free || self.zone.is_some() {
            (x - gx, y - gy, Vec::new())
        } else {
            self.surface.snapped(pid, x - gx, y - gy)
        };
        self.guides = guides;
        if let Some(p) = self.surface.get_mut(pid) { p.tx = tx; p.ty = ty; }
    }

    pub fn save_layout(&mut self) {
        self.save_due = false;
        let saved = self.surface.to_saved(self.window_size);
        let done = serde_json::to_vec(&saved).map_err(|e| e.to_string()).and_then(|b| crate::ctl::write_atomic(&self.data.layout(), &b).map_err(|e| e.to_string()));
        match done {
            Ok(()) => record::detail("screen", "layout saved", &[("panels", saved.panels.len().to_string())]),
            Err(e) => record::warn("screen", "layout not saved", &e, &[]),
        }
    }

    /// The host's input method's text: composing shows in the app, finished text goes into it.
    pub fn handle_ime(&mut self, ime: smithay::reexports::winit::event::Ime) {
        use smithay::reexports::winit::event::Ime;
        let text_input = self.seat.text_input();
        match ime {
            Ime::Preedit(text, cursor) => {
                let (begin, end) = cursor.map_or((text.len() as i32, text.len() as i32), |(b, e)| (b as i32, e as i32));
                text_input.with_active_text_input(|t, _| t.preedit_string(Some(text.clone()), begin, end));
                text_input.done(false);
            }
            Ime::Commit(text) => {
                text_input.with_active_text_input(|t, _| {
                    t.preedit_string(None, 0, 0);
                    t.commit_string(Some(text.clone()));
                });
                text_input.done(false);
            }
            _ => {}
        }
    }
}

/// What each connected app has with the compositor.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl TextInputActivation for App {}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

/// The person's title bar zoom from settings.json, 1 when none.
fn title_zoom(data: &Data) -> f64 {
    let all: serde_json::Map<String, serde_json::Value> = paths::read_kept(&data.root.join("settings.json"));
    all.get("title_zoom").and_then(serde_json::Value::as_f64).unwrap_or(1.0).clamp(0.7, 2.0)
}
