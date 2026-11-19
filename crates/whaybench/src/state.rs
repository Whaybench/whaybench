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
/// The window's own buttons at the right of the menu bar (minimise, maximise, close), each this wide, as Windows has them.
pub const CONTROL_W: f64 = 46.0;
/// How close to the window's edge a press resizes the window. The top edge is thinner: the menu is there.
const WINDOW_EDGE: f64 = 5.0;
const WINDOW_TOP_EDGE: f64 = 3.0;

/// The window's button `CONTROL_W` wide from the right of a window `width` wide at x: close, maximise, minimise.
fn control_in(width: f64, x: f64) -> Option<Control> {
    let right = width - x;
    [(Control::Close, 1.0), (Control::Maximize, 2.0), (Control::Minimize, 3.0)].into_iter().find(|(_, n)| right > (n - 1.0) * CONTROL_W && right <= n * CONTROL_W).map(|(c, _)| c)
}

/// The edge of a window `w` by `h` at (x, wy), window coordinates, to resize it from there.
fn edge_in(w: f64, h: f64, x: f64, wy: f64) -> Option<smithay::reexports::winit::window::ResizeDirection> {
    use smithay::reexports::winit::window::ResizeDirection as D;
    let (l, r, t, b) = (x < WINDOW_EDGE, x > w - WINDOW_EDGE, wy < WINDOW_TOP_EDGE, wy > h - WINDOW_EDGE);
    Some(match (l, r, t, b) {
        (true, _, true, _) => D::NorthWest, (_, true, true, _) => D::NorthEast,
        (true, _, _, true) => D::SouthWest, (_, true, _, true) => D::SouthEast,
        (true, ..) => D::West, (_, true, ..) => D::East, (_, _, true, _) => D::North, (_, _, _, true) => D::South,
        _ => return None,
    })
}

/// What the window's own frame asks of the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WindowCmd { Move, Resize(smithay::reexports::winit::window::ResizeDirection), Minimize, Maximize, Close }

/// A button of the window's own frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Control { Minimize, Maximize, Close }

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
/// The browser is on its way out or in: ended here and not yet started there. Meanwhile it runs nowhere, and that is
/// not the person closing it.
static BROWSER_MOVING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Ends the move when the thread that does it ends, however it ends.
struct MovingDone;
impl Drop for MovingDone { fn drop(&mut self) { BROWSER_MOVING.store(false, std::sync::atomic::Ordering::Relaxed); } }

/// How long the ↩ of the last move stays on a title bar.
pub const UNDO_FOR: Duration = Duration::from_secs(10);

/// A move that can be undone for a while: the moved panel's front tab (its title bar shows the ↩), the whole layout
/// when the first of its drags began (moves of one panel one after another are one), and the panels it changed, as
/// they were.
pub struct Undo { pub at: Instant, pub mark: u64, pub start: Vec<Panel>, pub was: Vec<Panel> }

/// A panel moved out to a window of its own on the desktop: its app id and name, its command line (run again out
/// there, and back here), where it comes back to, and its process out there (0 until it is started).
/// What a window coming back is known by.
#[derive(Clone, PartialEq, Debug)]
pub enum Comer { Process(u32), App(String) }

pub struct Outside { pub app: String, pub name: String, pub command: String, pub home: Option<layout::Home>, pub pid: u32 }

pub enum Act {
    Idle,
    /// The panel follows the pointer, stiffly; gx, gy is where it was picked up. `dock`: held by the title bar's dot
    /// (or a tab pulled out), so it can go onto a target circle of another panel; else it only moves.
    Hold { pid: u64, gx: f64, gy: f64, dock: bool },
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
    /// Something happened that may change the picture (an app's request, input, a command): draw the next frame.
    /// Without it, and with nothing moving, the screen is looked at only a few times a second.
    pub redraw: bool,
    /// Something drawn this frame moves by itself (a panel gliding, a theme's motion): the next frame is drawn at once,
    /// not when something happens. Set while drawing, through a shared borrow.
    pub moving: std::cell::Cell<bool>,
    pub pointer: (f64, f64),
    pub hover: Option<(u64, Edges, Vec<(u64, Side)>)>,
    /// What the window's own frame was asked to do (move, resize, minimise, maximise, close); the window does it.
    pub window_cmd: Option<WindowCmd>,
    /// The last press on the window's empty title strip and where, for a double click.
    pub strip_pressed: Option<(Instant, f64, f64)>,
    pub maximized: bool,
    pub guides: Vec<Guide>,
    pub zone: Option<Zone>,
    /// The held browser's dot is in the band along the window's edge: dropped, the browser goes out to the desktop.
    pub out_zone: bool,
    pub chrome: Chrome,
    /// The person's zoom of the title bars (Ctrl+wheel on one), over the theme's text size; in settings.json.
    pub title_zoom: f64,
    /// Whether Whaybench's own window is the one in front on the desktop. False until the desktop says so, so a
    /// Whaybench started behind other windows still notifies.
    pub host_focused: bool,
    /// Sessions' states as last seen, for the notification when one starts waiting.
    waiting: crate::notify::Waiting,
    /// Panels of the panel menu started and not yet open, by key: a click again while one starts (Firefox takes
    /// seconds) would start a second, which finds the first one's profile locked.
    pub starting: HashMap<String, Instant>,
    /// Where windows coming back from the desktop go: the panel they were a tab of, or the place they had. By the
    /// process started for it (two map branches share an app id), or by app id for the browser (Firefox's own start).
    pub homes: Vec<(Comer, layout::Home)>,
    /// The panels out on the desktop, but the browser (its own way): the input panel shows the way back for each.
    pub outside: Vec<Outside>,
    /// The layout when a drag began, and the last move's, to go back to for a while (the ↩ on a title bar).
    pub drag_from: Option<Vec<Panel>>,
    pub undos: Vec<Undo>,
    /// Whether the browser is out on the desktop, looked at once a second (it reads /proc): its title bar label.
    pub browser_away: (bool, Instant),
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
    /// Apps that ask to be the surface behind the panels (wlr-layer-shell: swaybg, mpvpaper and the like).
    pub layer_shell_state: smithay::wayland::shell::wlr_layer::WlrLayerShellState,
    pub shm_state: ShmState,
    /// linux-dmabuf, offered only when the window's GPU has a render node apps can use (not under WSLg).
    pub dmabuf: Option<smithay::wayland::dmabuf::DmabufState>,
    pub gpu: Option<std::rc::Rc<std::cell::RefCell<smithay::backend::winit::WinitGraphicsBackend<smithay::backend::renderer::gles::GlesRenderer>>>>,
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
        let layer_shell_state = smithay::wayland::shell::wlr_layer::WlrLayerShellState::new::<Self>(&dh);
        XdgDecorationState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        // The globals stay registered: the manager only holds their ids and has nothing to do when dropped.
        let _ = OutputManagerState::new_with_xdg_output::<Self>(&dh);
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
        let chrome = Chrome::new(theme.title_size * title_zoom, &theme.title_font, theme.title_weight, theme.title_spacing);
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
            act: Act::Idle, redraw: true, moving: Default::default(), pointer: (0.0, 0.0), hover: None, window_cmd: None, strip_pressed: None, maximized: false, guides: Vec::new(), zone: None, out_zone: false,
            chrome, solid_ids: Vec::new(), shaded: Vec::new(), focus_when_open: None,
            title_zoom,
            host_focused: false,
            waiting: crate::notify::Waiting::new(),
            starting: HashMap::new(),
            homes: Vec::new(),
            outside: Vec::new(),
            drag_from: None,
            undos: Vec::new(),
            browser_away: (false, now),
            shots: Vec::new(), end_reason: String::new(), data, saved, menu: None, theme_seen: std::time::Instant::now(), window_size: (FIRST_WINDOW.0 as i32, FIRST_WINDOW.1 as i32), save_due: false,
            start_time: now, display_handle: dh, space: Space::default(), loop_signal, socket_name,
            compositor_state, xdg_shell_state, layer_shell_state, shm_state, dmabuf: None, gpu: None, seat_state, data_device_state, clip,
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
            state.redraw = true;
            if let Err(e) = state.display_handle.insert_client(client_stream, Arc::new(ClientState::default())) { record::error("run", "app not connected", &[("error", e.to_string())]); }
        }).expect("Failed to init the wayland event source.");
        loop_handle.insert_source(Generic::new(display, Interest::READ, Mode::Level), |_, display, state| {
            // An app's request (a new picture, a new title) may change what shows.
            state.redraw = true;
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

    /// When a drag of the panel showing `tab` begins: the layout to go back to. A move of the same panel while its ↩
    /// is there continues it, so ↩ goes back to before the first (a tab pulled out, then put somewhere).
    pub fn drag_begins(&mut self, tab: Option<u64>) {
        let going = self.undos.iter().position(|u| Some(u.mark) == tab && u.at.elapsed() < UNDO_FOR);
        self.drag_from = Some(match going { Some(i) => self.undos.remove(i).start, None => self.surface.panels.clone() });
    }

    /// After a drag of the panel showing `tab`: its move can be undone for a while, if it changed the layout. A move
    /// that changes a panel another one changed takes that one's ↩ away: undone, it would not fit any more.
    pub fn moved(&mut self, tab: u64) {
        let Some(start) = self.drag_from.take() else { return };
        let was = layout::touched(&start, &self.surface.panels);
        if was.is_empty() { return; }
        let ids = |ps: &[Panel]| ps.iter().flat_map(|p| p.tabs.iter().map(|t| t.id)).collect::<Vec<_>>();
        let mine = ids(&was);
        self.undos.retain(|u| u.at.elapsed() < UNDO_FOR && !ids(&u.was).iter().any(|t| mine.contains(t)));
        self.undos.push(Undo { at: Instant::now(), mark: tab, start, was });
    }

    /// The ↩ this panel shows: the last move of a panel whose tab it holds, while it lasts.
    pub fn undo_of(&self, pid: u64) -> Option<usize> {
        self.undos.iter().rposition(|u| u.at.elapsed() < UNDO_FOR && self.surface.holding_tab(u.mark).is_some_and(|p| p.pid == pid))
    }

    /// The ↩ of a panel: the panels its last move changed go back to how they were; the rest stays.
    pub fn undo_move(&mut self, pid: u64) {
        let Some(i) = self.undo_of(pid) else { return };
        let u = self.undos.remove(i);
        self.surface.panels = layout::undo_one(&self.surface.panels, &u.was, |id| self.windows.contains_key(&id));
        record::decide("screen", "move undone", "the ↩ on a title bar", &[("panels", u.was.len().to_string())]);
        self.save_due = true;
    }

    /// Moves a panel (its one tab) out to a window of its own on the desktop: the same program, started again out
    /// there with the same command line (its content is read from the data folder, so it shows the same). `home`:
    /// where it comes back to.
    pub fn send_out(&mut self, tab: u64, home: Option<layout::Home>) {
        let Some(w) = self.windows.get(&tab) else { return };
        let top = w.toplevel().unwrap();
        let pid = smithay::reexports::wayland_server::Resource::client(top.wl_surface()).and_then(|c| c.get_credentials(&self.display_handle).ok()).map(|c| c.pid as u32);
        let Some(pid) = pid else { return record::warn("screen", "panel not moved out", "no process for its window", &[]) };
        let args: Vec<String> = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default().split(|b| *b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).to_string()).collect();
        if args.is_empty() { return record::warn("screen", "panel not moved out", "its command line is gone", &[]); }
        let command = crate::start::exec_line(&args);
        let Some((key, name)) = self.surface.holding_tab(tab).and_then(|p| p.tabs.iter().find(|t| t.id == tab)).map(|t| (t.key.clone(), t.name.clone())) else { return };
        let app = key.split('#').next().unwrap_or(&key).to_string();
        // Ended, not closed: closed, a view or a map branch would be gone for good, and the first view does not close.
        crate::start::end_panel(pid);
        // The compositor runs with the desktop's display (start.rs).
        let display = std::env::var_os("WAYLAND_DISPLAY").unwrap_or_default();
        // At the size it had here, with a bar of its own: the desktop gives neither.
        let size = self.surface.holding_tab(tab).map(|p| self.surface.app_of(p.rect)).map(|r| format!("{}x{}", r.w.round(), r.h.round())).unwrap_or_default();
        let outside = format!("export {}={size}; {command}", wb_core::view::ENV_OUTSIDE);
        let out = crate::start::spawn_outside(&outside, &display, &self.data, &record::run_id(&self.data)).unwrap_or(0);
        record::decide("screen", "panel moved out", "dropped at the window's edge", &[("app", app.clone())]);
        self.outside.push(Outside { app, name, command, home, pid: out });
    }

    /// The way back of a panel out on the desktop: it ends out there and starts here again, where it was.
    pub fn bring_back(&mut self, i: usize) {
        if i >= self.outside.len() { return; }
        let o = self.outside.remove(i);
        // Only while it is still the one started out there: its number may belong to another program since.
        if o.pid != 0 && crate::start::is_outside(o.pid) { crate::start::forget_outside(o.pid); unsafe { libc::kill(o.pid as i32, libc::SIGTERM); } }
        record::decide("screen", "panel moved in", "its way back on the input panel", &[("app", o.app)]);
        let pid = crate::start::spawn_panel(&o.command, &self.socket_name, &self.data, &record::run_id(&self.data));
        if let (Some(h), Some(pid)) = (o.home, pid) { self.homes.push((Comer::Process(pid), h)); }
    }

    /// The panel that shows the way back for the browser while it is out: the one with the conversations.
    pub fn browser_back_panel(&self) -> Option<u64> {
        if !self.browser_away.0 { return None; }
        self.surface.panels.iter().find(|p| p.tabs.iter().any(|t| t.key.starts_with(crate::start::TERM_APP))).map(|p| p.pid)
    }

    /// Moves the panel's Firefox out to a window of its own on the desktop, or back into a panel. It is a restart of
    /// Firefox on the other display: it saves its session when it ends and restores it when it starts
    /// (browser.startup.page), so the tabs, sign-ins and history go along. Out, its panel closes and the others take
    /// the room, as when any panel closes; back, it takes a place as any panel opened (or, by ↩, its own place).
    pub fn move_browser(&mut self, out: bool) {
        let data = self.data.clone();
        if if out { browser_out(&data) } else { !wb_core::view::detached(&data) } { return; }
        wb_core::view::set_detached(&data, out);
        self.browser_away = (out, Instant::now());
        record::decide("browser", if out { "moved out" } else { "moved in" }, "asked", &[]);
        self.relaunch_browser(out);
    }

    /// Ends the panel's Firefox where it is and starts it on the other display, off the event loop.
    fn relaunch_browser(&mut self, out: bool) {
        let (data, run) = (self.data.clone(), record::run_id(&self.data));
        // The compositor runs with the desktop's display (start.rs); its apps get this one.
        let display: OsString = if out { std::env::var_os("WAYLAND_DISPLAY").unwrap_or_default() } else { self.socket_name.clone() };
        if out {
            // Its windows close: with the last one Firefox ends, as it does when the person closes it.
            self.close_app(wb_core::view::FIREFOX_APP);
        } else if let Some(pid) = wb_core::view::firefox_pid(&data) {
            // Out there it has no window of ours to close: asked to end, as on a desktop's log out.
            crate::start::forget_outside(pid);
            unsafe { libc::kill(pid as i32, libc::SIGTERM); }
        }
        BROWSER_MOVING.store(true, std::sync::atomic::Ordering::Relaxed);
        std::thread::spawn(move || {
            let _done = MovingDone;
            if !wb_core::view::firefox_gone(&data, Duration::from_secs(15)) {
                return record::warn("browser", "not moved", "Firefox did not end in 15 s", &[]);
            }
            // Out there it ends with Whaybench, as the panels moved out do; the next start finds its profile free.
            let command = wb_core::view::firefox_command(&data, "");
            if out { crate::start::spawn_outside(&command, &display, &data, &run); } else { crate::start::spawn_app(&command, &display, &data, &run); }
            // Still moving until it runs (its start takes seconds): meanwhile it runs nowhere, not closed by the person.
            let started = std::time::Instant::now();
            while wb_core::view::firefox_pid(&data).is_none() && started.elapsed() < Duration::from_secs(15) { std::thread::sleep(Duration::from_millis(200)); }
        });
    }

    /// Asks every window of the app to close.
    fn close_app(&mut self, app: &str) {
        let mine = format!("{app}#");
        let ids: Vec<u64> = self.surface.panels.iter().flat_map(|p| p.tabs.iter()).filter(|t| t.key == app || t.key.starts_with(&mine)).map(|t| t.id).collect();
        for id in ids { if let Some(w) = self.windows.get(&id) { w.toplevel().unwrap().send_close(); } }
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

    /// How wide the menu bar is: what its menus need, leaving the window's title strip and buttons the rest.
    pub fn menu_width(&self) -> i32 {
        let width = self.window_size.0;
        let room = (width as f64 - 3.0 * CONTROL_W).max(0.0) as i32;
        let need = self.menu.as_ref().and_then(|w| w.toplevel()).map(|t| with_states(t.wl_surface(), |s| s.cached_state.get::<smithay::wayland::shell::xdg::SurfaceCachedState>().current().min_size.w)).unwrap_or(0);
        if need > 0 { need.min(room) } else { room }
    }

    /// The window's button at this point (surface coordinates: the menu bar is at y < 0).
    pub fn control_at(&self, x: f64, y: f64) -> Option<Control> {
        if self.menu.is_none() || y >= 0.0 || y < -(MENU_H as f64) { return None; }
        control_in(self.window_size.0 as f64, x)
    }

    /// The window's edge at this point, to resize it from there.
    pub fn window_edge_at(&self, x: f64, y: f64) -> Option<smithay::reexports::winit::window::ResizeDirection> {
        if self.maximized { return None; }
        edge_in(self.window_size.0 as f64, self.window_size.1 as f64, x, y + self.menu_height() as f64)
    }

    /// What a left press on the window's own frame does: resize from an edge, a button, or move from the empty strip
    /// (a double click maximises). None when the press is not on the frame.
    pub fn frame_press(&mut self, x: f64, y: f64) -> Option<WindowCmd> {
        if let Some(d) = self.window_edge_at(x, y) { return Some(WindowCmd::Resize(d)); }
        if let Some(c) = self.control_at(x, y) {
            return Some(match c { Control::Minimize => WindowCmd::Minimize, Control::Maximize => WindowCmd::Maximize, Control::Close => WindowCmd::Close });
        }
        if self.menu.is_none() || y >= 0.0 || x < self.menu_width() as f64 { return None; }
        // Two presses close in time and place, as a double click; two far apart are two moves.
        let double = self.strip_pressed.replace((Instant::now(), x, y)).is_some_and(|(t, px, py)| t.elapsed() < Duration::from_millis(400) && (px - x).hypot(py - y) < 6.0);
        if double { self.strip_pressed = None; }
        Some(if double { WindowCmd::Maximize } else { WindowCmd::Move })
    }

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
            (None, Some(_)) if self.starting.get(key).is_some_and(|at| at.elapsed() < Duration::from_secs(30)) => {
                record::decide("screen", "panel not started again", "it is still starting", &[("app", key.to_string())]);
            }
            // The browser moved out comes back into its panel, rather than a second window out there (or a second
            // panel beside the one that stands in for it, when the person closed it out there).
            (None, Some(_)) if key == wb_core::view::FIREFOX_APP && wb_core::view::detached(&self.data) => self.move_browser(false),
            // One out on the desktop comes back, rather than a second one here.
            (None, Some(_)) if let Some(i) = self.outside.iter().position(|o| key.split('#').next() == Some(o.app.as_str())) => self.bring_back(i),
            (None, Some(c)) => {
                if key == wb_core::view::FIREFOX_APP { wb_core::view::set_detached(&self.data, false); }
                self.starting.insert(key.to_string(), Instant::now());
                self.spawn(c);
            }
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

    /// Notifies the desktop of sessions that have just started waiting on the person (see `notify::Waiting`).
    pub fn tell_waiting(&mut self) { self.waiting.poll(&self.data, self.host_focused); }

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
        let pid = smithay::reexports::wayland_server::Resource::client(surface.wl_surface()).and_then(|c| c.get_credentials(&self.display_handle).ok()).map(|c| c.pid as u32);
        let comer = |c: &Comer| match c { Comer::Process(p) => Some(*p) == pid, Comer::App(a) => *a == app_id };
        let home = self.homes.iter().position(|(c, _)| comer(c)).map(|i| self.homes.remove(i).1);
        // A dialog of a window that is not a panel (the menu bar, the status line) goes where a settings window goes.
        let pid = if let Some(home) = home {
            // The browser back by ↩: where it was before it went out (the others are back where they were too).
            match home {
                layout::Home::Join(pid) if let Some(p) = self.surface.get_mut(pid) => { p.tabs.push(tab); p.active = p.tabs.len() - 1; pid }
                layout::Home::Join(_) => { let pid = self.surface.new_pid(); let r = self.surface.free_place(); self.surface.panels.push(Panel::new(pid, tab, r)); pid }
                layout::Home::Place(rect) => { let pid = self.surface.new_pid(); self.surface.panels.push(Panel::new(pid, tab, rect)); pid }
            }
        } else if modal || (parent.is_some() && parent_panel.is_none()) {
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
        self.starting.retain(|k, _| key != *k && !key.starts_with(&format!("{k}#")));
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
        self.chrome.set_style(size, &self.theme.title_font, self.theme.title_weight, self.theme.title_spacing);
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
        if self.browser_away.1.elapsed() > Duration::from_secs(1) {
            self.browser_away = (browser_out(&self.data), Instant::now());
            // One the person closed out there has no way back to show.
            self.outside.retain(|o| o.pid == 0 || crate::start::is_outside(o.pid));
            // The browser closed out there (not on its way in or out): no place waits for it any more.
            if wb_core::view::detached(&self.data) && !self.browser_away.0 && !BROWSER_MOVING.load(std::sync::atomic::Ordering::Relaxed) {
                wb_core::view::set_detached(&self.data, false);
                self.homes.retain(|(c, _)| *c != Comer::App(wb_core::view::FIREFOX_APP.into()));
            }
        }
        if self.surface.step(ms) { self.moving.set(true); }
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
                    // Its target moves too: the panel glides to its target each frame and would go back.
                    if p.modal && (w - p.rect.w).abs() > 0.5 { p.rect.x = recentred(p.rect.x, p.rect.w, w, self.surface.w); p.tx = p.rect.x; }
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
        let menu_w = self.menu_width();
        for (w, y, high, wide) in [(self.status.clone(), self.surface.h as i32, STATUS_H, width), (self.menu.clone(), -self.menu_height(), MENU_H, menu_w)] {
            let Some(w) = w else { continue };
            self.space.map_element(w.clone(), (0, y), false);
            let top = w.toplevel().unwrap();
            top.with_pending_state(|s| s.size = Some((wide, high).into()));
            top.send_pending_configure();
        }
        if self.save_due && matches!(self.act, Act::Idle) { self.save_layout(); }
    }

    /// Where the held panel goes: on a target circle when it is held by its dot, else it moves and snaps.
    pub fn update_hold(&mut self) {
        let Act::Hold { pid, gx, gy, dock } = self.act else { return };
        let (x, y) = self.pointer;
        // A dialog or a settings window moves freely: it never joins the panels.
        let free = self.seat.get_keyboard().is_some_and(|k| k.modifier_state().alt) || self.surface.get(pid).is_some_and(|p| p.transient);
        // The dot is drawn where the title bar is now (the panel follows the pointer with a lag), and it is what goes
        // onto a target: what shows and what counts are the same.
        let dot = self.surface.get(pid).map(dot_at);
        self.zone = match dot { Some((dx, dy)) if dock && !free => self.surface.drop_zone(pid, dx, dy, self.theme.dock_ring_size), _ => None };
        // The browser's dot at the window's edge, not on a target: it goes out.
        self.out_zone = dock && !free && self.zone.is_none() && self.surface.get(pid).is_some_and(can_go_out)
            && dot.is_some_and(|(dx, dy)| out_edge(self.surface.w, self.surface.h, self.theme.dock_ring_size, dx, dy).is_some());
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
        self.redraw = true;
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

/// Where the docking dot of a panel is: the middle of its title bar as drawn now.
pub fn dot_at(p: &Panel) -> (f64, f64) { (p.rect.x + p.rect.w / 2.0, p.rect.y + layout::title() / 2.0) }

/// Whether the panel's Firefox is out on the desktop now: moved out, and still running (the person may have closed it
/// out there, or Whaybench started again since).
fn browser_out(data: &wb_core::paths::Data) -> bool { wb_core::view::detached(data) && wb_core::view::firefox_pid(data).is_some() }

/// Whether the panel can go out to the desktop: one tab, the browser or a panel of Whaybench's that shows what the data
/// folder holds (not the conversations, the input panel, the bars or a dialog).
pub fn can_go_out(p: &Panel) -> bool {
    !p.transient && p.tabs.len() == 1 && (holds_browser(p) || goes_out(&p.tabs[0].key))
}

/// Whether a tab of this key is a panel that can go out: one of these apps, or one named under it (a view
/// `<view>.<name>`, an imported map `<map>.i<n>`), its number after `#`; not a terminal in the view's place
/// (`dev.whaybench.view-term`) or the map style settings (`dev.whaybench.mapstyle`).
pub fn goes_out(key: &str) -> bool {
    const OUT: [&str; 7] = ["dev.whaybench.map", "dev.whaybench.sessions", "dev.whaybench.files", "dev.whaybench.answer", "dev.whaybench.notify", "dev.whaybench.project", wb_core::view::VIEW_APP];
    let app = key.split('#').next().unwrap_or(key);
    OUT.iter().any(|a| app == *a || app.strip_prefix(a).is_some_and(|rest| rest.starts_with('.')))
}

/// Whether the panel holds the browser.
pub fn holds_browser(p: &Panel) -> bool {
    let app = wb_core::view::FIREFOX_APP;
    p.tabs.iter().any(|t| t.key == app || t.key.starts_with(&format!("{app}#")))
}

/// The edge of the window whose band (`band` deep) the point is in, the nearest if in two.
pub fn out_edge(w: f64, h: f64, band: f64, x: f64, y: f64) -> Option<Side> {
    [(Side::Left, x), (Side::Right, w - x), (Side::Top, y), (Side::Bottom, h - y)].into_iter()
        .filter(|(_, d)| *d <= band).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(s, _)| s)
}

/// Where a settings window that was `was` wide at `x` goes at `w` wide: its middle kept, inside the screen when it fits.
fn recentred(x: f64, was: f64, w: f64, screen: f64) -> f64 { (x + (was - w) / 2.0).min(screen - w).max(0.0) }

#[cfg(test)]
mod tests {
    #[test]
    fn a_settings_window_that_grows_stays_on_the_screen() {
        use super::recentred;
        assert_eq!(recentred(580.0, 760.0, 1000.0, 1920.0), 460.0, "its middle kept");
        assert_eq!(recentred(1500.0, 400.0, 800.0, 1920.0), 1120.0, "moved by the person near the edge: pulled back in");
        assert_eq!(recentred(100.0, 760.0, 2400.0, 1920.0), 0.0, "wider than the screen: from its left edge");
    }

    #[test]
    fn the_window_buttons_and_edges_meet_without_a_gap() {
        use super::{Control, control_in, edge_in};
        use smithay::reexports::winit::window::ResizeDirection as D;
        // Each button is 46 pixels from the right: close, then maximise, then minimise; left of them, none.
        assert_eq!(control_in(1000.0, 999.0), Some(Control::Close));
        assert_eq!(control_in(1000.0, 954.0), Some(Control::Close));
        assert_eq!(control_in(1000.0, 953.9), Some(Control::Maximize));
        assert_eq!(control_in(1000.0, 908.0), Some(Control::Maximize));
        assert_eq!(control_in(1000.0, 907.9), Some(Control::Minimize));
        assert_eq!(control_in(1000.0, 862.0), Some(Control::Minimize));
        assert_eq!(control_in(1000.0, 861.9), None);
        // Corners take both sides; the middle and just inside the edges are no edge.
        assert_eq!(edge_in(800.0, 600.0, 1.0, 1.0), Some(D::NorthWest));
        assert_eq!(edge_in(800.0, 600.0, 799.0, 599.0), Some(D::SouthEast));
        assert_eq!(edge_in(800.0, 600.0, 400.0, 599.0), Some(D::South));
        assert_eq!(edge_in(800.0, 600.0, 400.0, 3.0), None, "the top edge is thin: the menu is there");
        assert_eq!(edge_in(800.0, 600.0, 5.0, 300.0), None);
    }

    use super::*;

    #[test]
    fn only_the_panels_that_show_the_data_folder_go_out() {
        for yes in ["dev.whaybench.map#1", "dev.whaybench.map#3", "dev.whaybench.map.i0a1b2c3d#1", "dev.whaybench.view#1", "dev.whaybench.view.notes#1", "dev.whaybench.files#1"] { assert!(goes_out(yes), "{yes}"); }
        for no in ["dev.whaybench.view-term#1", "whaybench-term#1", "dev.whaybench.input#1", "dev.whaybench.mapstyle#1", "dev.whaybench.prefs#1", "dev.whaybench.viewer#1"] { assert!(!goes_out(no), "{no}"); }
    }

    #[test]
    fn the_band_at_the_window_edge_is_the_nearest_edge_within_its_depth() {
        assert_eq!(out_edge(1000.0, 800.0, 48.0, 10.0, 400.0), Some(Side::Left));
        assert_eq!(out_edge(1000.0, 800.0, 48.0, 500.0, 790.0), Some(Side::Bottom));
        assert_eq!(out_edge(1000.0, 800.0, 48.0, 995.0, 5.0), Some(Side::Right), "in two bands: the nearer edge");
        assert_eq!(out_edge(1000.0, 800.0, 48.0, 500.0, 400.0), None);
    }
}
