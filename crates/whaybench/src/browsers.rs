//! Which browser is in front, and whether one may have the keyboard (spec 3.22). Three things hold, whatever opens,
//! closes, moves or is chosen; they are kept in two places, so no case has to be thought of:
//! - a browser gets the keyboard only where the person is already, or where they went or asked (`may_have_keys`, which
//!   every program-made focus passes; a press does not);
//! - a settings window that is open has it (`App::focus_panel`);
//! - a browser of another side than the chosen conversation's is not left in front, unless the person chose its tab or
//!   types in it (`settle_browsers`, after every change and each second).
use std::{collections::{HashMap, HashSet}, time::{Duration, Instant}};

use smithay::utils::SERIAL_COUNTER;
use wb_core::{container, log as record, roots, view};

use crate::{App, layout::Panel, state::project_of};

/// How long a window the person asked for by the menu may take to come.
const ASKED: Duration = Duration::from_secs(30);
/// A second press of the menu line meanwhile starts nothing.
const PRESSED: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct Browsers {
    /// The isolated project whose conversation is chosen; none: the host.
    side: Option<String>,
    /// Where the person went last.
    last: Went,
    /// Browser tabs the person pressed since this conversation's side was chosen: left in front.
    pinned: HashSet<u64>,
    /// Dialogs put behind, as tabs beside the window they belong to.
    dialogs: HashSet<u64>,
    /// Browsers asked to be shown while the person typed there: each comes up when it may, with its own side chosen.
    front_due: HashSet<String>,
    /// Browsers the person asked for by the menu and has not gone elsewhere since.
    asked: HashMap<String, Instant>,
    /// Tabs brought up where the wanted browser has none, by panel: there the browser comes back with its conversation.
    stand_in: HashMap<u64, u64>,
}

/// Where the person went last: to a window of the host's, to a project's browser, or from one to what a program put under
/// their hands since (a tab that opened): then no browser takes the keyboard by itself.
#[derive(Clone, Default, PartialEq, Debug)]
enum Went { #[default] Host, Project(String), Away }

#[cfg(test)]
impl Browsers {
    pub fn is_pinned(&self, id: u64) -> bool { self.pinned.contains(&id) }
    pub fn ask_at(&mut self, app: &str, at: Instant) { self.asked.insert(app.to_string(), at); }
}

/// Where a browser's window goes when it opens.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Opens { WithKeys, InFront, Elsewhere }

fn app_of(key: &str) -> &str { key.split('#').next().unwrap_or(key) }

/// Whether a tab of this key is a browser: the host's or a project's.
pub fn is_browser(key: &str) -> bool { app_of(key) == view::FIREFOX_APP || view::browser_project(key).is_some() }

/// The room a dialog gets of what it asks: a project's never the whole screen, whatever size it takes.
pub fn room(of_project: bool, asks: i32, all: f64) -> f64 { if of_project { (asks as f64).min((all - 80.0).max(1.0)) } else { asks as f64 } }

fn app_of_side(side: Option<&str>) -> String { side.map_or(view::FIREFOX_APP.to_string(), view::project_browser_app) }

/// The newest tab of the app (a dialog of it comes after it).
fn newest(p: &Panel, app: &str) -> Option<usize> { p.tabs.iter().rposition(|t| app_of(&t.key) == app) }

/// Instead of a browser that has no place here: the view, else the first tab that is none.
fn instead(p: &Panel) -> Option<usize> {
    p.tabs.iter().position(|t| app_of(&t.key) == view::VIEW_APP).or_else(|| p.tabs.iter().position(|t| !is_browser(&t.key)))
}

impl App {
    /// The project whose conversation the person has chosen; none: a conversation of the host, or no conversation.
    pub fn chosen_project(&self) -> Option<String> { self.focused_term.as_ref().and_then(|(tab, _)| wb_core::sessions::tab_project(&self.data, tab)) }

    /// That project, when it has a browser of its own (it runs in its container).
    fn chosen_isolated(&self) -> Option<roots::Stub> {
        let p = self.chosen_project()?;
        roots::stubs(&self.data).into_iter().find(|s| s.id == p && container::opened_isolated(s))
    }

    /// The tab the keyboard is in.
    pub fn keys_tab(&self) -> Option<u64> {
        let focus = self.seat.get_keyboard()?.current_focus()?;
        self.windows.iter().find(|(_, w)| *w.toplevel().unwrap().wl_surface() == focus).map(|(id, _)| *id)
    }

    /// The project the window with the keyboard is of.
    fn keys_project(&self) -> Option<String> { self.seat.get_keyboard()?.current_focus().and_then(|s| project_of(&s)).map(|(id, _)| id) }

    /// Whether the person asked for the browser of this side by the menu just now.
    fn asked(&self, side: Option<&str>) -> bool { self.browsers.asked.get(&app_of_side(side)).is_some_and(|at| at.elapsed() < ASKED) }

    /// The person asked for this browser by the menu: its window may come with the keyboard, until they go elsewhere.
    pub fn ask_browser(&mut self, app: &str) { self.browsers.asked.insert(app.to_string(), Instant::now()); }

    /// Whether the person types for an isolated project: in its conversation, or in the input panel with it chosen.
    pub fn typing_for_project(&self) -> bool {
        let Some(id) = self.keys_tab() else { return false };
        let input = self.surface.holding_tab(id).is_some_and(|p| p.tabs.iter().any(|t| t.id == id && app_of(&t.key) == "dev.whaybench.input"));
        (input || self.terms.contains_key(&id)) && self.chosen_isolated().is_some()
    }

    /// The window they typed for a project in is closing: no browser takes what they go on typing.
    pub fn closing_under_typing(&mut self, id: u64) { if self.keys_tab() == Some(id) && self.typing_for_project() { self.browsers.last = Went::Away; } }

    /// Whether a program may give the keyboard to this tab. Any that is no browser; a browser only where the person is
    /// or was last, or asked to be: what they type for one side never goes on into another's page.
    pub fn may_have_keys(&self, id: u64) -> bool {
        let Some(key) = self.surface.holding_tab(id).and_then(|p| p.tabs.iter().find(|t| t.id == id)).map(|t| t.key.as_str()) else { return true };
        if !is_browser(key) { return true; }
        let side = view::browser_project(key);
        let there = match &self.browsers.last { Went::Host => side.is_none() && !self.typing_for_project(), Went::Project(p) => side == Some(p.as_str()), Went::Away => false };
        there || self.asked(side) && (side.is_none() || self.browsers.side.as_deref() == side)
    }

    /// Notes who has the keyboard now, by the person's going there or not. A settings window (`modal`) has it only for a
    /// while: where they were stays noted, and gets it back.
    pub fn keys_given(&mut self, id: u64, person: bool, modal: bool) {
        if modal { return; }
        if person { self.browsers.asked.clear(); }
        let project = self.surface.holding_tab(id).and_then(|p| p.tabs.iter().find(|t| t.id == id)).and_then(|t| view::browser_project(&t.key)).map(String::from);
        self.browsers.last = match (project, &self.browsers.last) {
            (Some(p), _) => Went::Project(p),
            (None, Went::Project(_) | Went::Away) if !person => Went::Away,
            (None, _) => Went::Host,
        };
    }

    /// A browser that may not have the keyboard: it stays where it is, while that is a window in front; else nobody has it.
    pub fn refuse_keys(&mut self) {
        match self.keys_tab() { Some(id) if self.surface.holding_tab(id).is_some_and(|p| p.tab().id == id) => {}, _ => self.no_keys() }
    }

    fn no_keys(&mut self) {
        for w in self.windows.values() { w.set_activated(false); w.toplevel().unwrap().send_pending_configure(); }
        if let Some(k) = self.seat.get_keyboard() { k.set_focus(self, None, SERIAL_COUNTER.next_serial()); }
    }

    /// The person pressed this tab in a title bar: a browser stays in front, and one shown instead of it is theirs now.
    pub fn tab_pressed(&mut self, pid: u64) {
        let Some((id, key)) = self.surface.get(pid).map(|p| (p.tab().id, p.tab().key.clone())) else { return };
        if is_browser(&key) { self.browsers.pinned.insert(id); }
        self.browsers.stand_in.remove(&pid);
    }

    /// Where a browser's window may open now (`project`: none is the host's).
    pub fn browser_opens(&self, project: Option<&str>) -> Opens {
        let here = self.keys_project();
        match project {
            // The host's opens with the keyboard, as it always has; not while the person types in a project's, or did when it went.
            None => if here.is_none() && self.browsers.last == Went::Host && !self.typing_for_project() || self.asked(None) { Opens::WithKeys } else { Opens::InFront },
            Some(p) if here.as_deref() == Some(p) => Opens::WithKeys,
            Some(p) if self.chosen_isolated().is_none_or(|s| s.id != p) => Opens::Elsewhere,
            Some(p) => if self.asked(Some(p)) { Opens::WithKeys } else { Opens::InFront },
        }
    }

    /// The panel a browser is in: another one that opens takes its place there, whichever side it is of.
    pub fn browser_panel(&self) -> Option<u64> { self.surface.panels.iter().find(|p| !p.transient && p.tabs.iter().any(|t| is_browser(&t.key))).map(|p| p.pid) }

    /// Whether a dialog that opens goes as a tab beside the window it belongs to: it would come up over another place.
    pub fn dialog_behind(&self, opens: Option<Opens>, parent: Option<u64>) -> bool {
        let seen = parent.is_some_and(|i| self.surface.holding_tab(i).is_some_and(|p| p.tab().id == i));
        match opens { Some(Opens::Elsewhere) => true, Some(Opens::InFront) => !seen, _ => false }
    }

    /// Puts a browser's window that has just got its tab where it may be. `fronts`, `keys`: each panel's front tab, and
    /// the tab with the keyboard, before it opened.
    pub fn place_browser(&mut self, id: u64, pid: u64, opens: Opens, dialog: bool, fronts: &[(u64, u64)], keys: Option<u64>) {
        if dialog { self.browsers.dialogs.insert(id); }
        let before = fronts.iter().find(|(p, _)| *p == pid).map(|(_, t)| *t);
        // (The view is looked at, not typed in.)
        let typed = self.surface.get(pid).and_then(|p| p.tabs.iter().find(|t| Some(t.id) == before)).is_some_and(|t| app_of(&t.key) != view::VIEW_APP);
        let covers = opens == Opens::InFront && typed && before == keys;
        let app = self.surface.get(pid).and_then(|p| p.tabs.iter().find(|t| t.id == id)).map(|t| app_of(&t.key).to_string());
        let front = if opens == Opens::Elsewhere || covers { before } else { Some(id) };
        match front {
            Some(front) => if let Some(p) = self.surface.get_mut(pid) && let Some(i) = p.tabs.iter().position(|t| t.id == front) { p.active = i; },
            // A panel of its own: under the others, not over where the person is.
            None => if let Some(i) = self.surface.panels.iter().position(|p| p.pid == pid) { let p = self.surface.panels.remove(i); self.surface.panels.insert(0, p); },
        }
        if covers { self.browsers.front_due.extend(app); }
        if opens == Opens::InFront { self.refuse_keys(); }
    }

    /// A project asks for its browser to be shown: with its conversation chosen, not under the person's hands, and
    /// never with the keyboard.
    pub fn front_project_browser(&mut self, project: &str) {
        if self.chosen_isolated().is_none_or(|s| s.id != project) { return; }
        let app = view::project_browser_app(project);
        if !self.try_front(&app) { self.browsers.front_due.insert(app); }
    }

    /// The host's browser asked for by a program while the person types for a project: shown with a conversation of the
    /// host's. Whether it waits. One that does not wait (asked for from a panel, as a project's inner login) stays in front.
    pub fn host_browser_waits(&mut self, app: &str) -> bool {
        if app != view::FIREFOX_APP { return false; }
        if self.keys_project().is_some() || self.browsers.last != Went::Host || self.typing_for_project() { self.browsers.front_due.insert(app.to_string()); return true; }
        if let Some(id) = self.surface.panels.iter().find_map(|p| newest(p, app).map(|i| p.tabs[i].id)) { self.browsers.pinned.insert(id); }
        false
    }

    /// The host's browser opens with the keyboard: it is the page asked for, and stays in front.
    pub fn host_browser_opened(&mut self, id: u64) { self.browsers.pinned.insert(id); }

    /// Whether the app is in front now, or has no tab; not where the keyboard is in the tab it would cover.
    fn try_front(&mut self, app: &str) -> bool {
        let Some((pid, i)) = self.surface.panels.iter().find_map(|p| newest(p, app).map(|i| (p.pid, i))) else { return true };
        let Some(p) = self.surface.get(pid) else { return true };
        // Which of its windows shows the page is the app's to know: one in front stays.
        if app_of(&p.tab().key) == app { return true; }
        if self.typed_in(p) { return false; }
        if let Some(p) = self.surface.get_mut(pid) { p.active = i; }
        self.refuse_keys();
        self.redraw = true;
        self.save_due = true;
        true
    }

    /// Whether the person types in the panel's front tab (the view is looked at).
    fn typed_in(&self, p: &Panel) -> bool { self.keys_tab() == Some(p.tab().id) && app_of(&p.tab().key) != view::VIEW_APP }

    /// The browser follows the conversation the person chose, as the view does: no browser of another side is left in
    /// front; where the chosen side has none, the view stands in for it, and it comes back there with its conversation.
    /// By the tab's own note, which a new project tab writes only after its window is here.
    pub fn settle_browsers(&mut self) {
        let side = self.chosen_isolated().map(|s| s.id);
        if side != self.browsers.side {
            self.browsers.side = side.clone();
            self.browsers.pinned.clear();
            self.tell_browser();
        }
        let want = app_of_side(side.as_deref());
        if self.browsers.front_due.contains(&want) && self.try_front(&want) { self.browsers.front_due.remove(&want); }
        let mut b = std::mem::take(&mut self.browsers);
        let mut moves: Vec<(u64, usize)> = Vec::new();
        for p in &self.surface.panels {
            let (id, app) = (p.tab().id, app_of(&p.tab().key));
            if b.pinned.contains(&id) || self.typed_in(p) { continue; }
            let dialog = || p.tabs.iter().rposition(|t| app_of(&t.key) == want && b.dialogs.contains(&t.id));
            let to = if app == want { dialog() } else if is_browser(app) { dialog().or_else(|| newest(p, &want)).or_else(|| instead(p)) }
                else if b.stand_in.get(&p.pid) == Some(&id) { dialog().or_else(|| newest(p, &want)) } else { None };
            if let Some(i) = to.filter(|i| *i != p.active) { moves.push((p.pid, i)); }
        }
        for (pid, i) in &moves {
            let Some(p) = self.surface.get_mut(*pid) else { continue };
            p.active = *i;
            if is_browser(&p.tab().key) { b.stand_in.remove(pid); } else { b.stand_in.insert(*pid, p.tab().id); }
        }
        self.browsers = b;
        if !moves.is_empty() { self.refuse_keys(); self.redraw = true; self.save_due = true; }
    }

    /// A window of this tab is gone.
    pub fn browser_gone(&mut self, id: u64) { self.browsers.pinned.remove(&id); self.browsers.dialogs.remove(&id); }

    /// Which browser the menu's line is of now, for its tick.
    pub fn tell_browser(&self) {
        let app = app_of_side(self.chosen_isolated().map(|s| s.id).as_deref());
        let _ = wb_core::paths::write_atomic(&view::chosen_browser_file(&self.data), app);
    }

    /// The menu's browser line: the browser of the conversation the person is in. Whether that was a project's.
    pub fn toggle_project_browser(&mut self) -> bool {
        let Some(project) = self.chosen_isolated() else { return false };
        let key = view::project_browser_app(&project.id);
        if self.surface.panels.iter().any(|p| newest(p, &key).is_some()) { self.close_app(&key); return true; }
        if self.starting.get(&key).is_some_and(|at| at.elapsed() < PRESSED) { return true; }
        self.ask_browser(&key);
        self.starting.insert(key, Instant::now());
        // Off the event loop: podman takes its time, and its answer is the person's to see.
        let data = self.data.clone();
        std::thread::spawn(move || {
            if let Err(e) = container::show_browser(&project) {
                wb_core::status::say(&data, &e, 10);
                record::warn("browser", "project's browser not shown", &e, &[("project", project.id.clone())]);
            }
        });
        true
    }
}
