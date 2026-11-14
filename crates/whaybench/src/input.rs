use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent, InputTime,
        KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
    },
    input::{
        keyboard::FilterResult,
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    utils::SERIAL_COUNTER,
};
use wb_core::log as record;

use crate::{layout::{How, TITLE}, render::{BarHit, bar_hit}, state::{Act, App}};


/// The left mouse button: only it picks up panels.
const LEFT: u32 = 0x110;

impl App {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event, .. } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time(&event);

                let key_state = event.state();
                // No shortcuts of the program's own yet: they are to be settable and checked against the usual ones first.
                self.seat.get_keyboard().unwrap().input::<(), _>(self, event.key_code(), key_state, serial, time, |_, _, _| FilterResult::Forward);
            }
            InputEvent::PointerMotion { .. } => {}
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let output = self.space.outputs().next().unwrap();
                let output_geo = self.space.output_geometry(output).unwrap();
                let pos = event.position_transformed(output_geo.size) + output_geo.loc.to_f64();
                self.pointer = (pos.x, pos.y);
                self.pointer_moved(pos.x, pos.y);
                // During a drag the apps do not see the pointer.
                let under = if matches!(self.act, Act::Idle) { self.surface_under(pos) } else { None };
                let serial = SERIAL_COUNTER.next_serial();
                let pointer = self.seat.get_pointer().unwrap();
                pointer.motion(self, under, &MotionEvent { location: pos, serial, time: event.time() });
                pointer.frame(self);
            }
            InputEvent::PointerButton { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                let serial = SERIAL_COUNTER.next_serial();
                let button = event.button_code();
                let button_state = event.state();
                // Menus close on a press outside them even while the pointer is held by a grab, which skips the rest.
                if button_state == ButtonState::Pressed { let at = self.pointer; self.close_popups_unless_at(at.into()); }
                let forward = match button_state {
                    ButtonState::Pressed if !pointer.is_grabbed() => self.pointer_pressed(button),
                    ButtonState::Released if !matches!(self.act, Act::Idle) => { self.pointer_released(); false }
                    _ => true,
                };
                if forward {
                    pointer.button(self, &ButtonEvent { button, state: button_state, serial, time: event.time() });
                    pointer.frame(self);
                }
            }
            InputEvent::PointerAxis { event, .. } => {
                let source = event.source();

                let horizontal_amount = event
                    .amount(Axis::Horizontal)
                    .unwrap_or_else(|| event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.);
                let vertical_amount = event
                    .amount(Axis::Vertical)
                    .unwrap_or_else(|| event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.);
                let horizontal_amount_discrete = event.amount_v120(Axis::Horizontal);
                let vertical_amount_discrete = event.amount_v120(Axis::Vertical);

                let mut frame = AxisFrame::new(event.time()).source(source);
                if horizontal_amount != 0.0 {
                    frame = frame.value(Axis::Horizontal, horizontal_amount);
                    if let Some(discrete) = horizontal_amount_discrete {
                        frame = frame.v120(Axis::Horizontal, discrete as i32);
                    }
                }
                if vertical_amount != 0.0 {
                    frame = frame.value(Axis::Vertical, vertical_amount);
                    if let Some(discrete) = vertical_amount_discrete {
                        frame = frame.v120(Axis::Vertical, discrete as i32);
                    }
                }

                if source == AxisSource::Finger {
                    if event.amount(Axis::Horizontal) == Some(0.0) {
                        frame = frame.stop(Axis::Horizontal);
                    }
                    if event.amount(Axis::Vertical) == Some(0.0) {
                        frame = frame.stop(Axis::Vertical);
                    }
                }

                let pointer = self.seat.get_pointer().unwrap();
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }
}

impl App {
    /// `whaybench ctl pointer`, for tests.
    pub fn test_pointer(&mut self, x: f64, y: f64) {
        self.pointer = (x, y);
        self.pointer_moved(x, y);
        let under = if matches!(self.act, Act::Idle) { self.surface_under((x, y).into()) } else { None };
        let pointer = self.seat.get_pointer().unwrap();
        pointer.motion(self, under, &MotionEvent { location: (x, y).into(), serial: SERIAL_COUNTER.next_serial(), time: InputTime::from_micros(0) });
        pointer.frame(self);
    }

    /// `whaybench ctl press|release`, for tests.
    pub fn test_button(&mut self, down: bool, button: u32) {
        let forward = if down { self.pointer_pressed(button) } else if !matches!(self.act, Act::Idle) { self.pointer_released(); false } else { true };
        if forward {
            let pointer = self.seat.get_pointer().unwrap();
            let state = if down { ButtonState::Pressed } else { ButtonState::Released };
            pointer.button(self, &ButtonEvent { button, state, serial: SERIAL_COUNTER.next_serial(), time: InputTime::from_micros(0) });
            pointer.frame(self);
        }
    }

    /// One line per panel, bottom one first.
    pub fn describe(&self) -> String {
        self.surface.panels.iter().map(|p| {
            let tabs: Vec<&str> = p.tabs.iter().map(|t| t.key.as_str()).collect();
            let r = p.rect;
            format!("{} {:.0},{:.0} {:.0}x{:.0} front={} tabs={}\n", p.pid, r.x, r.y, r.w, r.h, p.tab().key, tabs.join(","))
        }).collect()
    }

    fn pointer_moved(&mut self, x: f64, y: f64) {
        match &self.act {
            Act::Hold { .. } => self.update_hold(),
            Act::Resize { pid, edges, partners } => {
                let (pid, edges, partners) = (*pid, *edges, partners.clone());
                self.surface.resize(pid, edges, &partners, x, y);
            }
            Act::TabPull { pid, tab, x: sx, y: sy } => {
                let (pid, tab, sx, sy) = (*pid, *tab, *sx, *sy);
                if (y - sy).abs() > TITLE || (x - sx).abs() > 60.0 {
                    if let Some(new) = self.surface.tear_out(pid, tab, x, y) {
                        record::decide("screen", "tab pulled out", "a tab dragged out of the title bar", &[("panel", new.to_string())]);
                        self.act = Act::Hold { pid: new, gx: 60.0, gy: 12.0 };
                        self.focus_panel(new);
                    } else {
                        self.act = Act::Swallow;
                    }
                }
            }
            Act::Swallow => {}
            Act::Idle => {
                self.hover = self.surface.edges_at(x, y).map(|(pid, e)| (pid, e, self.surface.partners(pid, e)));
            }
        }
    }

    /// True: the press belongs to the app under the pointer.
    fn pointer_pressed(&mut self, button: u32) -> bool {
        let (x, y) = self.pointer;
        self.close_popups_unless_at((x, y).into());
        // While a settings window is open, a press elsewhere does nothing.
        if let Some(m) = self.surface.modal().map(|m| (m.pid, m.rect))
            && !m.1.contains(x, y) { self.focus_panel(m.0); self.act = Act::Swallow; return false; }
        if self.menu_under((x, y).into()).is_some() { return true; }
        // A press on an open menu is the menu's alone: raising the panel under it would take the keyboard from the
        // menu, and GTK then drops the menu before the item is chosen.
        if self.surface_under((x, y).into()).is_some_and(|(s, _)| self.popups.find_popup(&s).is_some()) { return true; }
        let Some(top) = self.surface.top_at(x, y).map(|p| p.pid) else { return true };
        if button == LEFT {
            if let Some((pid, edges)) = self.surface.edges_at(x, y) {
                let partners = self.surface.partners(pid, edges);
                self.act = Act::Resize { pid, edges, partners };
                self.focus_panel(pid);
                return false;
            }
            let hit = bar_hit(self, top, x, y);
            match hit {
                Some(BarHit::Close) => {
                    let front = self.surface.get(top).map(|p| p.tab().id);
                    if let Some(w) = front.and_then(|f| self.windows.get(&f)) { w.toplevel().unwrap().send_close(); }
                    self.act = Act::Swallow;
                    return false;
                }
                Some(BarHit::CloseTab(i)) => {
                    let id = self.surface.get(top).and_then(|p| p.tabs.get(i)).map(|t| t.id);
                    if let Some(w) = id.and_then(|t| self.windows.get(&t)) { w.toplevel().unwrap().send_close(); }
                    self.act = Act::Swallow;
                    return false;
                }
                Some(BarHit::Tab(i)) => {
                    if let Some(p) = self.surface.get_mut(top) { p.active = i; }
                    self.act = Act::TabPull { pid: top, tab: i, x, y };
                    self.focus_panel(top);
                    self.note_saved_soon();
                    return false;
                }
                Some(BarHit::Bar) => {
                    let (gx, gy) = { let p = self.surface.get(top).unwrap(); (x - p.rect.x, y - p.rect.y) };
                    self.act = Act::Hold { pid: top, gx, gy };
                    self.focus_panel(top);
                    return false;
                }
                None => {}
            }
        }
        if self.surface.get(top).is_some_and(|p| p.rect.bar().contains(x, y)) { return false; }
        self.focus_panel(top);
        true
    }

    fn pointer_released(&mut self) {
        match std::mem::replace(&mut self.act, Act::Idle) {
            Act::Hold { pid, .. } => {
                match self.zone.take() {
                    Some(zone) => {
                        let how = match zone.how { How::Tab => "as a tab", How::Side(_) => "gave way" };
                        record::decide("screen", "panel dropped", how, &[("panel", pid.to_string()), ("target", zone.target.to_string())]);
                        self.surface.apply_drop(pid, zone);
                        let front = if self.surface.get(pid).is_some() { pid } else { zone.target };
                        self.focus_panel(front);
                    }
                    None => self.surface.keep_reachable(pid),
                }
                self.guides.clear();
            }
            Act::Resize { .. } | Act::TabPull { .. } | Act::Swallow | Act::Idle => {}
        }
        self.band = (None, std::time::Instant::now());
        self.note_saved_soon();
    }
}
