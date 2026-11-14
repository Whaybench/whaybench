//! The free surface, without anything Wayland: panels as rectangles with tabs, how they
//! move, stick, share borders, take each other in and are saved. Panels are in stacking order: last on top.
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The title bar's height, from its text size (`set_title`): 28 px for 13 px text. Read everywhere a panel is laid
/// out, set by the compositor when the theme or the person's title zoom changes.
static TITLE_BITS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0x403C_0000_0000_0000);
pub fn title() -> f64 { f64::from_bits(TITLE_BITS.load(std::sync::atomic::Ordering::Relaxed)) }
pub fn set_title(text: f64) { TITLE_BITS.store((text * 28.0 / 13.0).round().to_bits(), std::sync::atomic::Ordering::Relaxed); }
pub const SNAP: f64 = 12.0;
pub const GRIP: f64 = 7.0;
pub const TOUCH: f64 = 2.0;
/// Share of a panel that is the "give way" band on each side, at most SIDE_MAX pixels.
pub const SIDE: f64 = 0.15;
pub const SIDE_MAX: f64 = 40.0;
/// How long the pointer must stay in a band before it counts: passing through changes nothing.
pub const BAND_DWELL: Duration = Duration::from_millis(400);
pub const MIN_W: f64 = 120.0;
pub const MIN_H: f64 = 80.0;
/// How stiffly a moving panel follows: this share of the gap per 1/60 s.
pub const STIFFNESS: f64 = 0.09;
pub const LAYOUT_FORMAT: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect { pub x: f64, pub y: f64, pub w: f64, pub h: f64 }

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self { Rect { x, y, w, h } }
    pub fn right(&self) -> f64 { self.x + self.w }
    pub fn bottom(&self) -> f64 { self.y + self.h }
    pub fn contains(&self, x: f64, y: f64) -> bool { x >= self.x && x < self.right() && y >= self.y && y < self.bottom() }
    pub fn bar(&self) -> Rect { Rect::new(self.x, self.y, self.w, title()) }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tab { pub id: u64, pub key: String, pub name: String }

#[derive(Clone, Debug)]
pub struct Panel {
    pub pid: u64,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub rect: Rect,
    pub tx: f64,
    pub ty: f64,
    /// A dialog: floats over the panels and is not saved.
    pub transient: bool,
    /// Takes the size its app wants (a dialog until the person resizes it); else it is given its rect.
    pub fitted: bool,
    /// A settings window: in front of everything, the joint lines included, and the only one that takes input
    /// while it is open (implies transient).
    pub modal: bool,
    /// Where it came from in the saved layout, to keep the saved stacking.
    pub saved_at: Option<usize>,
}

/// Where on a panel another one held over it would go.
enum Area { Bar, Band(Side) }

fn target_area(r: Rect, x: f64, y: f64) -> Option<Area> {
    if y <= r.y + title() { return Some(Area::Bar); }
    let (bw, bh) = ((SIDE * r.w).min(SIDE_MAX), (SIDE * (r.h - title())).min(SIDE_MAX));
    [(Side::Left, x - r.x, bw), (Side::Right, r.right() - x, bw), (Side::Top, y - r.y - title(), bh), (Side::Bottom, r.bottom() - y, bh)]
        .into_iter().filter(|s| s.1 <= s.2).min_by(|a, b| (a.1 / a.2).total_cmp(&(b.1 / b.2))).map(|s| Area::Band(s.0))
}

impl Panel {
    pub fn new(pid: u64, tab: Tab, rect: Rect) -> Self {
        Panel { pid, tabs: vec![tab], active: 0, rect, tx: rect.x, ty: rect.y, transient: false, fitted: false, modal: false, saved_at: None }
    }
    pub fn tab(&self) -> &Tab { &self.tabs[self.active.min(self.tabs.len() - 1)] }
    pub fn set_rect(&mut self, r: Rect) { self.rect = r; self.tx = r.x; self.ty = r.y; }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side { Left, Right, Top, Bottom }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Edges { pub left: bool, pub right: bool, pub top: bool, pub bottom: bool }

impl Edges {
    pub fn any(&self) -> bool { self.left || self.right || self.top || self.bottom }
    pub fn sides(&self) -> Vec<Side> {
        let mut v = Vec::new();
        if self.left { v.push(Side::Left); }
        if self.right { v.push(Side::Right); }
        if self.top { v.push(Side::Top); }
        if self.bottom { v.push(Side::Bottom); }
        v
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum How { Tab, Side(Side) }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zone { pub target: u64, pub how: How, pub preview: Rect }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Guide { X(f64), Y(f64) }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line { pub x0: f64, pub y0: f64, pub x1: f64, pub y1: f64 }

/// The space a theme gives the frame: around the surface, between panels, around an app.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Metrics { pub outer: f64, pub gap: f64, pub pad: f64 }

#[derive(Default)]
pub struct Surface {
    pub panels: Vec<Panel>,
    pub w: f64,
    pub h: f64,
    pub m: Metrics,
    next_pid: u64,
    /// Panels in the order they were last chosen, the latest last. The stacking order is not this: free panels
    /// stay on top whichever panel was chosen.
    chosen: Vec<u64>,
}

/// Which edges hold together: panel indexes for shared borders, and which panels are on which wall.
struct Holds { vertical: Vec<(usize, usize)>, horizontal: Vec<(usize, usize)>, walls: Vec<(usize, Side)> }

impl Surface {
    pub fn new(w: f64, h: f64) -> Self { Surface { panels: Vec::new(), w, h, m: Metrics::default(), next_pid: 1, chosen: Vec::new() } }
    /// Where the app is in a panel: below the title bar, inside the frame.
    pub fn app_of(&self, r: Rect) -> Rect {
        let pad = self.m.pad;
        Rect::new(r.x + pad, r.y + title(), (r.w - 2.0 * pad).max(1.0), (r.h - title() - pad).max(1.0))
    }
    pub fn new_pid(&mut self) -> u64 { let p = self.next_pid; self.next_pid += 1; p }
    pub fn index(&self, pid: u64) -> Option<usize> { self.panels.iter().position(|p| p.pid == pid) }
    pub fn get(&self, pid: u64) -> Option<&Panel> { self.panels.iter().find(|p| p.pid == pid) }
    pub fn get_mut(&mut self, pid: u64) -> Option<&mut Panel> { self.panels.iter_mut().find(|p| p.pid == pid) }
    pub fn holding_tab(&self, id: u64) -> Option<&Panel> { self.panels.iter().find(|p| p.tabs.iter().any(|t| t.id == id)) }

    pub fn raise(&mut self, pid: u64) {
        if let Some(i) = self.index(pid) {
            let p = self.panels.remove(i);
            self.panels.push(p);
            let panels = &self.panels;
            self.chosen.retain(|c| *c != pid && panels.iter().any(|p| p.pid == *c));
            self.chosen.push(pid);
        }
        self.keep_order();
    }

    /// Panels that hold on to nothing (no shared border, no wall) stay above the ones that do, so a panel taken out of the
    /// layout can always be picked up again, also after a restart. Settings windows and their dialogs stay above all.
    /// Within each group the order stays as it is.
    pub fn keep_order(&mut self) {
        let held = self.lies_flat();
        let mut tagged: Vec<(u8, Panel)> = std::mem::take(&mut self.panels).into_iter().enumerate()
            .map(|(i, p)| (if p.modal { 2 } else if held(i) { 0 } else { 1 }, p)).collect();
        tagged.sort_by_key(|t| t.0);
        self.panels = tagged.into_iter().map(|t| t.1).collect();
    }

    /// For each panel in order: whether it holds on to a wall or shares a border with another.
    fn held(&self) -> impl Fn(usize) -> bool + use<> {
        let h = self.holds();
        move |i| h.walls.iter().any(|w| w.0 == i) || h.vertical.iter().chain(&h.horizontal).any(|(a, b)| *a == i || *b == i)
    }

    /// For each panel in order: whether it lies in the layout. A panel over one below it floats even when one of its
    /// edges happens to touch a wall or a neighbour, so the joint lines under it do not show through.
    fn lies_flat(&self) -> impl Fn(usize) -> bool + use<> {
        let held = self.held();
        let over = |a: &Rect, b: &Rect| a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom();
        // Only a panel below that lies flat counts: a panel of the layout raised over a floating one stays in the layout.
        let mut flat: Vec<bool> = Vec::with_capacity(self.panels.len());
        for (i, p) in self.panels.iter().enumerate() {
            let on_flat = self.panels[..i].iter().zip(&flat).any(|(q, f)| *f && !q.modal && over(&p.rect, &q.rect));
            flat.push(held(i) && !on_flat);
        }
        move |i| flat.get(i).copied().unwrap_or(false)
    }

    /// Panels that float, apart from settings windows: they lie above the joint lines too.
    pub fn free(&self) -> Vec<u64> {
        let held = self.lies_flat();
        self.panels.iter().enumerate().filter(|(i, p)| !p.modal && !held(*i)).map(|(_, p)| p.pid).collect()
    }

    /// Panels, the last chosen first, then the rest from the top down.
    pub fn by_choice(&self) -> impl Iterator<Item = &Panel> {
        let chosen = self.chosen.iter().rev().filter_map(|pid| self.get(*pid));
        let rest = self.panels.iter().rev().filter(|p| !self.chosen.contains(&p.pid));
        chosen.chain(rest)
    }

    /// The panel chosen last that is still there and not a settings window: the one shown as in front.
    pub fn front(&self) -> Option<u64> { self.by_choice().find(|p| !p.modal).map(|p| p.pid) }

    /// The settings window in front, or its dialog (a font or colour chooser) when one is open.
    pub fn modal(&self) -> Option<&Panel> { self.panels.iter().rev().find(|p| p.modal) }

    /// The panel on top at this point.
    pub fn top_at(&self, x: f64, y: f64) -> Option<&Panel> { self.panels.iter().rev().find(|p| p.rect.contains(x, y)) }

    /// One step of movement: each panel closes its share of the gap to its target, by elapsed time, so
    /// a slow frame does not make it lag. True when anything moved.
    pub fn step(&mut self, ms: f64) -> bool {
        let share = 1.0 - (1.0 - STIFFNESS).powf(ms / 16.667);
        let mut moved = false;
        for p in &mut self.panels {
            let (dx, dy) = (p.tx - p.rect.x, p.ty - p.rect.y);
            if dx.abs() <= 0.3 && dy.abs() <= 0.3 {
                if dx != 0.0 || dy != 0.0 { p.rect.x = p.tx; p.rect.y = p.ty; moved = true; }
            } else {
                p.rect.x += dx * share;
                p.rect.y += dy * share;
                moved = true;
            }
        }
        moved
    }

    /// What the top panel at this point would resize: the 7px band inside its edges.
    pub fn edges_at(&self, x: f64, y: f64) -> Option<(u64, Edges)> {
        let p = self.top_at(x, y)?;
        let r = p.rect;
        let e = Edges {
            left: x - r.x <= GRIP, right: r.right() - x <= GRIP,
            top: y - r.y <= GRIP, bottom: r.bottom() - y <= GRIP,
        };
        e.any().then_some((p.pid, e))
    }

    /// Panels whose border touches these edges: they move along.
    pub fn partners(&self, pid: u64, edges: Edges) -> Vec<(u64, Side)> {
        // A dialog resizes alone, and nothing moves it along.
        let Some(r) = self.get(pid).filter(|p| !p.transient).map(|p| p.rect) else { return Vec::new() };
        let gap = self.m.gap;
        let mut found = Vec::new();
        for o in self.panels.iter().filter(|o| o.pid != pid && !o.transient) {
            let q = o.rect;
            let along_y = r.bottom().min(q.bottom()) - r.y.max(q.y) > 0.0;
            let along_x = r.right().min(q.right()) - r.x.max(q.x) > 0.0;
            if edges.right && along_y && (q.x - r.right() - gap).abs() <= TOUCH { found.push((o.pid, Side::Left)); }
            if edges.left && along_y && (r.x - q.right() - gap).abs() <= TOUCH { found.push((o.pid, Side::Right)); }
            if edges.bottom && along_x && (q.y - r.bottom() - gap).abs() <= TOUCH { found.push((o.pid, Side::Top)); }
            if edges.top && along_x && (r.y - q.bottom() - gap).abs() <= TOUCH { found.push((o.pid, Side::Bottom)); }
        }
        found
    }

    /// Drags a border: the panel and everything sharing the border change together.
    pub fn resize(&mut self, pid: u64, edges: Edges, partners: &[(u64, Side)], x: f64, y: f64) {
        let gap = self.m.gap;
        let Some(p) = self.get_mut(pid) else { return };
        // Resized by the person: from now on it keeps this size, not its app's.
        p.fitted = false;
        let mut r = p.rect;
        if edges.right { r.w = (x - r.x).max(MIN_W); }
        if edges.left { let right = r.right(); r.x = x.min(right - MIN_W); r.w = right - r.x; }
        if edges.bottom { r.h = (y - r.y).max(MIN_H); }
        if edges.top { let bottom = r.bottom(); r.y = y.min(bottom - MIN_H); r.h = bottom - r.y; }
        p.set_rect(r);
        for (o, side) in partners {
            let Some(q) = self.get_mut(*o) else { continue };
            let mut n = q.rect;
            match side {
                Side::Left => { let right = n.right(); n.x = (r.right() + gap).min(right - MIN_W); n.w = right - n.x; }
                Side::Right => { n.w = (r.x - gap - n.x).max(MIN_W); }
                Side::Top => { let bottom = n.bottom(); n.y = (r.bottom() + gap).min(bottom - MIN_H); n.h = bottom - n.y; }
                Side::Bottom => { n.h = (r.y - gap - n.y).max(MIN_H); }
            }
            q.set_rect(n);
        }
    }

    /// Where a moving panel rests: edges within 12px stick to the walls' margin, to the gap beside another
    /// panel, or in line with its edges.
    pub fn snapped(&self, pid: u64, x: f64, y: f64) -> (f64, f64, Vec<Guide>) {
        let Some(p) = self.get(pid) else { return (x, y, Vec::new()) };
        let Metrics { outer, gap, .. } = self.m;
        // Lines the panel's start edge may rest on, and lines its end edge may.
        let (mut xa, mut xb) = (vec![outer], vec![self.w - outer]);
        let (mut ya, mut yb) = (vec![outer], vec![self.h - outer]);
        for o in self.panels.iter().filter(|o| o.pid != pid) {
            let q = o.rect;
            xa.extend([q.x, q.right() + gap]);
            xb.extend([q.right(), q.x - gap]);
            ya.extend([q.y, q.bottom() + gap]);
            yb.extend([q.bottom(), q.y - gap]);
        }
        let stick = |start: f64, size: f64, a: &[f64], b: &[f64]| {
            let (mut best, mut near, mut hit) = (start, SNAP + 1.0, None);
            for &line in a {
                if (start - line).abs() < near { best = line; near = (start - line).abs(); hit = Some(line); }
            }
            for &line in b {
                if (start + size - line).abs() < near { best = line - size; near = (start + size - line).abs(); hit = Some(line); }
            }
            (best, hit)
        };
        let (nx, gx) = stick(x, p.rect.w, &xa, &xb);
        let (ny, gy) = stick(y, p.rect.h, &ya, &yb);
        let mut guides = Vec::new();
        if let Some(g) = gx { guides.push(Guide::X(g)); }
        if let Some(g) = gy { guides.push(Guide::Y(g)); }
        (nx, ny, guides)
    }

    /// What dropping `held` here would do. The bands count only after the dwell; `in_band` says how long
    /// the pointer has been in this one.
    pub fn drop_zone(&self, held: u64, x: f64, y: f64, in_band: Duration) -> Option<Zone> {
        let held_rect = self.get(held)?.rect;
        let t = self.panels.iter().rev().find(|p| p.pid != held && !p.transient && p.rect.contains(x, y))?;
        let r = t.rect;
        // Tabs only over the title bar, giving way only in a narrow band at an edge; anywhere else the panel just moves.
        let side = match target_area(r, x, y)? {
            Area::Bar => return Some(Zone { target: t.pid, how: How::Tab, preview: r }),
            Area::Band(side) => side,
        };
        if in_band < BAND_DWELL { return None; }
        // As much as the dropped panel needs, but never more than half of the one giving way.
        let gap = self.m.gap;
        let (half_w, half_h) = ((r.w - gap) / 2.0, (r.h - gap) / 2.0);
        let w = held_rect.w.min(half_w).max(MIN_W.min(half_w));
        let h = held_rect.h.min(half_h).max(MIN_H.min(half_h));
        let preview = match side {
            Side::Left => Rect::new(r.x, r.y, w, r.h),
            Side::Right => Rect::new(r.right() - w, r.y, w, r.h),
            Side::Top => Rect::new(r.x, r.y, r.w, h),
            Side::Bottom => Rect::new(r.x, r.bottom() - h, r.w, h),
        };
        Some(Zone { target: t.pid, how: How::Side(side), preview })
    }

    /// The band the pointer is in over the panel under it, for timing the dwell.
    pub fn band_at(&self, held: u64, x: f64, y: f64) -> Option<(u64, Side)> {
        let t = self.panels.iter().rev().find(|p| p.pid != held && !p.transient && p.rect.contains(x, y))?;
        match target_area(t.rect, x, y)? { Area::Band(side) => Some((t.pid, side)), Area::Bar => None }
    }

    /// Carries out a drop: into tabs (dropped one in front), or the other panel gives way.
    pub fn apply_drop(&mut self, held: u64, zone: Zone) {
        match zone.how {
            How::Tab => {
                let Some(i) = self.index(held) else { return };
                let h = self.panels.remove(i);
                let Some(t) = self.get_mut(zone.target) else { self.panels.push(h); return };
                let mut tabs = h.tabs;
                tabs.append(&mut t.tabs);
                t.tabs = tabs;
                t.active = 0;
                self.raise(zone.target);
            }
            How::Side(side) => {
                let gap = self.m.gap;
                let Some(t) = self.get_mut(zone.target) else { return };
                let (r, half) = (t.rect, zone.preview);
                let rest = match side {
                    Side::Left => Rect::new(half.right() + gap, r.y, r.w - half.w - gap, r.h),
                    Side::Right => Rect::new(r.x, r.y, r.w - half.w - gap, r.h),
                    Side::Top => Rect::new(r.x, half.bottom() + gap, r.w, r.h - half.h - gap),
                    Side::Bottom => Rect::new(r.x, r.y, r.w, r.h - half.h - gap),
                };
                t.set_rect(rest);
                if let Some(h) = self.get_mut(held) {
                    h.rect.w = half.w;
                    h.rect.h = half.h;
                    h.tx = half.x;
                    h.ty = half.y;
                }
            }
        }
    }

    /// A tab pulled out becomes a panel of its own under the pointer. Its new panel's id.
    pub fn tear_out(&mut self, pid: u64, tab: usize, x: f64, y: f64) -> Option<u64> {
        let new = self.new_pid();
        let p = self.get_mut(pid)?;
        if p.tabs.len() < 2 || tab >= p.tabs.len() { return None; }
        let t = p.tabs.remove(tab);
        p.active = p.active.min(p.tabs.len() - 1);
        let r = Rect::new(x - 60.0, y - 12.0, p.rect.w, p.rect.h);
        self.panels.push(Panel::new(new, t, r));
        Some(new)
    }

    /// Takes a closed app out. Its panel goes with its last tab.
    pub fn remove_tab(&mut self, id: u64) -> bool {
        let Some(i) = self.panels.iter().position(|p| p.tabs.iter().any(|t| t.id == id)) else { return false };
        let p = &mut self.panels[i];
        p.tabs.retain(|t| t.id != id);
        if p.tabs.is_empty() { self.panels.remove(i); } else { p.active = p.active.min(p.tabs.len() - 1); }
        true
    }

    /// All panels back to the first layout: tabs of one place in one panel, the rest each on its own. A dialog stays.
    pub fn reset_places(&mut self, slot: impl Fn(&Tab) -> Option<String>, in_front: impl Fn(&str) -> bool) {
        let old = std::mem::take(&mut self.panels);
        let (dialogs, normal): (Vec<Panel>, Vec<Panel>) = old.into_iter().partition(|p| p.transient);
        for tab in normal.into_iter().flat_map(|p| p.tabs) {
            let place = slot(&tab);
            let same = place.as_ref().and_then(|s| self.panels.iter().position(|p| p.tabs.iter().any(|t| slot(t).as_ref() == Some(s))));
            match same {
                Some(i) => {
                    let p = &mut self.panels[i];
                    p.tabs.push(tab);
                    if in_front(&p.tabs.last().unwrap().key) { p.active = p.tabs.len() - 1; }
                }
                None => {
                    let rect = place.as_deref().and_then(|s| self.first_place(s)).unwrap_or_else(|| self.free_place());
                    let pid = self.new_pid();
                    self.panels.push(Panel::new(pid, tab, rect));
                }
            }
        }
        self.panels.extend(dialogs);
    }

    /// Moves a tab into another panel, as its last tab. Its old panel goes with its last tab.
    pub fn move_tab_to(&mut self, id: u64, pid: u64) -> bool {
        let Some(t) = self.panels.iter().filter(|p| p.pid != pid).find_map(|p| p.tabs.iter().find(|t| t.id == id).cloned()) else { return false };
        if self.get(pid).is_none() { return false; }
        self.remove_tab(id);
        self.get_mut(pid).unwrap().tabs.push(t);
        true
    }

    /// Lines that must always show: shared borders, in the middle of the gap, then edges on the
    /// walls, in the middle of the margin.
    pub fn joints(&self) -> (Vec<Line>, Vec<Line>) {
        let (mut shared, mut walls) = (Vec::new(), Vec::new());
        let Metrics { outer, gap, .. } = self.m;
        for (i, a) in self.panels.iter().enumerate() {
            let r = a.rect;
            for b in &self.panels[i + 1..] {
                let q = b.rect;
                let (y0, y1) = (r.y.max(q.y), r.bottom().min(q.bottom()));
                let (x0, x1) = (r.x.max(q.x), r.right().min(q.right()));
                if y1 - y0 > 0.0 {
                    for (left, right) in [(r.right(), q.x), (q.right(), r.x)] {
                        if (right - left - gap).abs() <= TOUCH { let m = (left + right) / 2.0; shared.push(Line { x0: m, y0, x1: m, y1 }); }
                    }
                }
                if x1 - x0 > 0.0 {
                    for (top, bottom) in [(r.bottom(), q.y), (q.bottom(), r.y)] {
                        if (bottom - top - gap).abs() <= TOUCH { let m = (top + bottom) / 2.0; shared.push(Line { x0, y0: m, x1, y1: m }); }
                    }
                }
            }
            let mid = outer / 2.0;
            if (r.x - outer).abs() <= TOUCH { walls.push(Line { x0: mid, y0: r.y, x1: mid, y1: r.bottom() }); }
            if (r.y - outer).abs() <= TOUCH { walls.push(Line { x0: r.x, y0: mid, x1: r.right(), y1: mid }); }
            if (r.right() - (self.w - outer)).abs() <= TOUCH { walls.push(Line { x0: self.w - mid, y0: r.y, x1: self.w - mid, y1: r.bottom() }); }
            if (r.bottom() - (self.h - outer)).abs() <= TOUCH { walls.push(Line { x0: r.x, y0: self.h - mid, x1: r.right(), y1: self.h - mid }); }
        }
        (shared, walls)
    }

    fn holds(&self) -> Holds {
        let Metrics { outer, gap, .. } = self.m;
        let mut h = Holds { vertical: Vec::new(), horizontal: Vec::new(), walls: Vec::new() };
        for (i, a) in self.panels.iter().enumerate() {
            let r = a.rect;
            for (j, b) in self.panels.iter().enumerate() {
                let q = b.rect;
                if i == j { continue; }
                if r.bottom().min(q.bottom()) - r.y.max(q.y) > 0.0 && (q.x - r.right() - gap).abs() <= TOUCH { h.vertical.push((i, j)); }
                if r.right().min(q.right()) - r.x.max(q.x) > 0.0 && (q.y - r.bottom() - gap).abs() <= TOUCH { h.horizontal.push((i, j)); }
            }
            if (r.x - outer).abs() <= TOUCH { h.walls.push((i, Side::Left)); }
            if (r.y - outer).abs() <= TOUCH { h.walls.push((i, Side::Top)); }
            if (r.right() - (self.w - outer)).abs() <= TOUCH { h.walls.push((i, Side::Right)); }
            if (r.bottom() - (self.h - outer)).abs() <= TOUCH { h.walls.push((i, Side::Bottom)); }
        }
        h
    }

    /// Puts panels back on what they held on to, with the present margin and gap.
    fn apply_holds(&mut self, h: &Holds) {
        let Metrics { outer, gap, .. } = self.m;
        let (w, ht) = (self.w, self.h);
        let mut rects: Vec<(f64, f64, f64, f64)> = self.panels.iter().map(|p| (p.rect.x, p.rect.y, p.rect.right(), p.rect.bottom())).collect();
        for (i, side) in &h.walls {
            match side {
                Side::Left => rects[*i].0 = outer,
                Side::Top => rects[*i].1 = outer,
                Side::Right => rects[*i].2 = w - outer,
                Side::Bottom => rects[*i].3 = ht - outer,
            }
        }
        for (a, b) in &h.vertical {
            let m = (rects[*a].2 + rects[*b].0) / 2.0;
            rects[*a].2 = m - gap / 2.0;
            rects[*b].0 = m + gap / 2.0;
        }
        for (a, b) in &h.horizontal {
            let m = (rects[*a].3 + rects[*b].1) / 2.0;
            rects[*a].3 = m - gap / 2.0;
            rects[*b].1 = m + gap / 2.0;
        }
        for (p, r) in self.panels.iter_mut().zip(rects) {
            if p.transient { continue; }
            p.set_rect(Rect::new(r.0, r.1, (r.2 - r.0).max(MIN_W), (r.3 - r.1).max(MIN_H)));
        }
    }

    /// A theme with other margins: panels keep what they held on to.
    pub fn set_metrics(&mut self, m: Metrics) {
        let holds = self.holds();
        self.m = m;
        self.apply_holds(&holds);
    }

    /// The window changed size: everything keeps its share, and walls and shared borders stay.
    pub fn rescale(&mut self, w: f64, h: f64) {
        if self.w <= 0.0 || self.h <= 0.0 || w <= 0.0 || h <= 0.0 { self.w = w; self.h = h; return; }
        let holds = self.holds();
        let (sx, sy) = (w / self.w, h / self.h);
        for p in self.panels.iter_mut().filter(|p| !p.transient) {
            let r = p.rect;
            p.set_rect(Rect::new(r.x * sx, r.y * sy, (r.w * sx).max(MIN_W), (r.h * sy).max(MIN_H)));
        }
        self.w = w;
        self.h = h;
        self.apply_holds(&holds);
    }

    /// A panel's title bar must stay where it can be picked up.
    pub fn keep_reachable(&mut self, pid: u64) {
        let (w, h) = (self.w, self.h);
        if let Some(p) = self.get_mut(pid) {
            p.tx = p.tx.clamp(40.0 - p.rect.w, (w - 40.0).max(40.0 - p.rect.w));
            p.ty = p.ty.clamp(0.0, (h - title()).max(0.0));
        }
    }

    /// Where a product panel goes the first time: left 18% map above sessions, middle 52% conversation,
    /// answer and input, right 30% the view group.
    pub fn first_place(&self, slot: &str) -> Option<Rect> {
        let Metrics { outer, gap, .. } = self.m;
        let (w, h) = (self.w - 2.0 * outer, self.h - 2.0 * outer);
        let left = (w * 0.18).round();
        let right = (w * 0.30).round();
        let middle = w - left - right - 2.0 * gap;
        let (x0, x1, x2) = (outer, outer + left + gap, outer + left + gap + middle + gap);
        let map_h = ((h - gap) * 0.64).round();
        let input_h = 80.0;
        let conversation = ((h - 2.0 * gap - input_h) * 0.68).round();
        Some(match slot {
            "map" => Rect::new(x0, outer, left, map_h),
            "sessions" => Rect::new(x0, outer + map_h + gap, left, h - map_h - gap),
            "term" => Rect::new(x1, outer, middle, conversation),
            "answer" => Rect::new(x1, outer + conversation + gap, middle, h - conversation - input_h - 2.0 * gap),
            "input" => Rect::new(x1, outer + h - input_h, middle, input_h),
            "view" => Rect::new(x2, outer, right, h),
            _ => return None,
        })
    }

    /// Where a panel opens when the layout does not know it.
    pub fn free_place(&self) -> Rect {
        let n = self.panels.iter().filter(|p| !p.transient).count() as f64 % 8.0;
        let (w, h) = ((self.w * 0.55).max(MIN_W), (self.h * 0.65).max(MIN_H));
        let at = self.m.outer.max(20.0) + 30.0 * n;
        Rect::new(at, at, w, h)
    }

    pub fn to_saved(&self, window: (i32, i32)) -> SavedLayout {
        let panels = self.panels.iter().filter(|p| !p.transient).map(|p| {
            // A panel still gliding is saved where it is going.
            SavedPanel {
                x: p.tx / self.w, y: p.ty / self.h, w: p.rect.w / self.w, h: p.rect.h / self.h,
                active: p.tab().key.clone(), tabs: p.tabs.iter().map(|t| t.key.clone()).collect(),
            }
        }).collect();
        SavedLayout { format: LAYOUT_FORMAT, window: Some(window), panels }
    }

    /// Puts a new app where the saved layout had it. False: the layout does not know this key.
    pub fn place_saved(&mut self, saved: &SavedLayout, tab: Tab) -> Result<u64, Tab> {
        let Some(at) = saved.panels.iter().position(|s| s.tabs.contains(&tab.key)) else { return Err(tab) };
        let s = &saved.panels[at];
        if let Some(existing) = self.panels.iter_mut().find(|p| p.saved_at == Some(at)) {
            let order = |k: &String| s.tabs.iter().position(|t| t == k).unwrap_or(usize::MAX);
            existing.tabs.push(tab);
            existing.tabs.sort_by_key(|t| order(&t.key));
            existing.active = existing.tabs.iter().position(|t| t.key == s.active).unwrap_or(0);
            return Ok(existing.pid);
        }
        let pid = self.new_pid();
        let rect = Rect::new(s.x * self.w, s.y * self.h, (s.w * self.w).max(MIN_W), (s.h * self.h).max(MIN_H));
        let mut p = Panel::new(pid, tab, rect);
        p.saved_at = Some(at);
        let before = self.panels.iter().position(|o| o.saved_at.is_some_and(|i| i > at)).unwrap_or(self.panels.len());
        self.panels.insert(before, p);
        Ok(pid)
    }
}

/// `layout.json` in the data directory. Sizes are shares of the surface.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct SavedLayout {
    pub format: u32,
    /// The window's own size last time.
    #[serde(default)]
    pub window: Option<(i32, i32)>,
    #[serde(default)]
    pub panels: Vec<SavedPanel>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SavedPanel { pub x: f64, pub y: f64, pub w: f64, pub h: f64, pub active: String, pub tabs: Vec<String> }

impl SavedLayout {
    /// A file that cannot be read is an empty layout, never a failure. Unknown fields are ignored.
    pub fn parse(text: &str) -> Option<Self> {
        let mut l: Self = serde_json::from_str(text).ok()?;
        // Up to 0.3.1 the notifications panel was called alerts.
        for p in &mut l.panels {
            for k in p.tabs.iter_mut().chain(std::iter::once(&mut p.active)) {
                if let Some(rest) = k.strip_prefix("dev.whaybench.alerts#") { *k = format!("dev.whaybench.notify#{rest}"); }
            }
        }
        l.merge_stacked();
        Some(l)
    }

    /// Panels saved on top of each other, nearly the same rectangle, cannot be told apart on the screen: the lower
    /// one is lost behind the upper. They come back as tabs of one panel, the upper one in front.
    fn merge_stacked(&mut self) {
        let mut i = 0;
        while i < self.panels.len() {
            let same = |a: &SavedPanel, b: &SavedPanel| {
                let w = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
                let h = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
                let both = w.max(0.0) * h.max(0.0);
                let either = a.w * a.h + b.w * b.h - both;
                either > 0.0 && both / either >= 0.9
            };
            if let Some(j) = (i + 1..self.panels.len()).find(|j| same(&self.panels[i], &self.panels[*j])) {
                let lower = self.panels.remove(i);
                let upper = &mut self.panels[j - 1];
                upper.tabs.extend(lower.tabs);
            } else {
                i += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tab(id: u64) -> Tab { Tab { id, key: format!("app#{id}"), name: format!("app{id}") } }
    fn surface(rects: &[Rect]) -> Surface {
        let mut s = Surface::new(1500.0, 900.0);
        for (i, r) in rects.iter().enumerate() {
            let pid = s.new_pid();
            s.panels.push(Panel::new(pid, tab(i as u64 + 1), *r));
        }
        s
    }

    #[test]
    fn the_chosen_panel_is_in_front_even_under_a_free_one() {
        // The left panel is held by the wall; the other one is free and stays on top.
        let mut s = surface(&[Rect::new(0.0, 0.0, 400.0, 400.0), Rect::new(600.0, 300.0, 300.0, 200.0)]);
        let (held, free) = (s.panels[0].pid, s.panels[1].pid);
        s.raise(free);
        s.raise(held);
        assert_eq!(s.panels.last().map(|p| p.pid), Some(free), "the free panel stays on top");
        assert_eq!(s.front(), Some(held), "but the one chosen last is in front");
        assert_eq!(s.by_choice().map(|p| p.pid).collect::<Vec<_>>(), [held, free]);
        s.panels.retain(|p| p.pid != held);
        assert_eq!(s.front(), Some(free), "a closed panel is passed over");
    }

    #[test]
    fn a_layout_from_before_finds_the_notifications_panel_by_its_old_name() {
        let l = SavedLayout::parse(r#"{"format":1,"panels":[{"x":0,"y":0,"w":0.5,"h":0.5,"active":"dev.whaybench.alerts#1","tabs":["dev.whaybench.alerts#1","dev.whaybench.view#1"]}]}"#).unwrap();
        assert_eq!(l.panels[0].tabs, ["dev.whaybench.notify#1", "dev.whaybench.view#1"]);
        assert_eq!(l.panels[0].active, "dev.whaybench.notify#1");
    }
    #[test]
    fn a_floating_panel_stays_on_top_when_a_panel_under_it_is_raised() {
        // Two panels side by side on the walls, and one floating over both, as the input panel lies.
        let mut s = surface(&[Rect::new(0.0, 0.0, 700.0, 900.0), Rect::new(708.0, 0.0, 792.0, 900.0), Rect::new(500.0, 300.0, 400.0, 100.0)]);
        let (right, floating) = (s.panels[1].pid, s.panels[2].pid);
        s.raise(right);
        assert_eq!(s.panels.last().map(|p| p.pid), Some(floating), "a panel opening at start is raised; the floating one stays above");
        assert_eq!(s.free(), vec![floating]);
    }
    #[test]
    fn a_panel_over_others_floats_even_when_an_edge_touches() {
        // Two panels side by side, and one over both whose left edge sits on the wall.
        let s = surface(&[Rect::new(0.0, 0.0, 700.0, 900.0), Rect::new(708.0, 0.0, 792.0, 900.0), Rect::new(0.0, 600.0, 1000.0, 100.0)]);
        assert!(s.held()(2), "its left edge holds on to the wall");
        assert_eq!(s.free(), vec![s.panels[2].pid], "the one on top floats; the two below stay in the layout");
    }

    #[test]
    fn a_panel_holding_on_to_nothing_stays_above_and_settings_above_all() {
        // A free panel first, then one on the left wall; a settings window under both.
        let mut s = surface(&[Rect::new(600.0, 300.0, 300.0, 200.0), Rect::new(0.0, 0.0, 400.0, 400.0), Rect::new(700.0, 100.0, 200.0, 150.0)]);
        s.panels[2].modal = true;
        s.panels[2].transient = true;
        s.panels.swap(0, 2);
        s.keep_order();
        let order: Vec<u64> = s.panels.iter().map(|p| p.tabs[0].id).collect();
        assert_eq!(order, [2, 1, 3], "held, then free, then the settings window");
        s.raise(s.panels[0].pid);
        assert_eq!(s.panels.iter().map(|p| p.tabs[0].id).collect::<Vec<_>>(), [2, 1, 3], "raising a held panel keeps it under the free one");
    }
    #[test]
    fn a_dialog_resized_by_the_person_keeps_its_size_and_moves_no_neighbour() {
        // A panel, and a dialog over it whose left edge touches the panel's right edge.
        let mut s = surface(&[Rect::new(0.0, 0.0, 700.0, 900.0), Rect::new(708.0, 100.0, 300.0, 200.0)]);
        (s.panels[1].transient, s.panels[1].fitted) = (true, true);
        let pid = s.panels[1].pid;
        let edges = s.edges_at(1007.0, 150.0).map(|(p, e)| (p, e.right)).unwrap();
        assert_eq!(edges, (pid, true), "a dialog's edge can be taken");
        assert!(s.partners(pid, Edges { left: true, ..Default::default() }).is_empty(), "nothing moves with a dialog");
        s.resize(pid, Edges { right: true, ..Default::default() }, &[], 1200.0, 150.0);
        assert!(!s.panels[1].fitted && s.panels[1].rect.w > 300.0, "its size is the person's now");
        assert_eq!(s.panels[0].rect.w, 700.0);
    }
    const LONG: Duration = Duration::from_secs(1);

    #[test]
    fn motion_is_by_time_and_stops_on_target() {
        let mut one = surface(&[Rect::new(0.0, 0.0, 200.0, 200.0)]);
        let mut two = surface(&[Rect::new(0.0, 0.0, 200.0, 200.0)]);
        one.panels[0].tx = 100.0;
        two.panels[0].tx = 100.0;
        one.step(33.334);
        two.step(16.667);
        two.step(16.667);
        assert!((one.panels[0].rect.x - two.panels[0].rect.x).abs() < 1e-9, "one long frame equals two short ones");
        assert!((two.panels[0].rect.x - (100.0 * (1.0 - 0.91f64.powi(2)))).abs() < 1e-9);
        for _ in 0..500 { one.step(16.667); }
        assert_eq!(one.panels[0].rect.x, 100.0);
        assert!(!one.step(16.667));
    }

    #[test]
    fn magnet_takes_12px_and_lets_go_beyond() {
        let s = surface(&[Rect::new(100.0, 100.0, 200.0, 200.0), Rect::new(400.0, 100.0, 200.0, 200.0)]);
        let (x, _, g) = s.snapped(1, 195.0, 500.0);
        assert_eq!(x, 200.0, "right edge sticks to the other panel's left");
        assert!(g.contains(&Guide::X(400.0)));
        assert_eq!(s.snapped(1, 187.0, 500.0).0, 187.0, "13px away stays free");
        assert_eq!(s.snapped(1, 8.0, 8.0).0, 0.0, "the wall");
        assert_eq!(s.snapped(1, 8.0, 8.0).1, 0.0);
    }

    #[test]
    fn the_title_bar_makes_a_tab_a_narrow_band_gives_way_and_the_middle_does_nothing() {
        let s = surface(&[Rect::new(600.0, 0.0, 300.0, 300.0), Rect::new(0.0, 0.0, 400.0, 400.0)]);
        let z = s.drop_zone(1, 200.0, 10.0, Duration::ZERO).unwrap();
        assert_eq!(z.how, How::Tab);
        assert_eq!(z.target, 2);
        assert_eq!(s.drop_zone(1, 200.0, 200.0, LONG), None, "the middle only moves the panel");
        // 40px from the left is the band's edge (15% would be 60): nothing before the dwell, a split after it.
        assert_eq!(s.drop_zone(1, 40.0, 200.0, Duration::from_millis(100)), None);
        assert_eq!(s.drop_zone(1, 40.0, 200.0, LONG).unwrap().how, How::Side(Side::Left));
        assert_eq!(s.drop_zone(1, 50.0, 200.0, LONG), None, "past the band");
        assert_eq!(s.drop_zone(1, 200.0, 390.0, LONG).unwrap().how, How::Side(Side::Bottom));
        assert_eq!(s.drop_zone(1, 1000.0, 800.0, LONG), None, "empty surface");
    }

    #[test]
    fn dropping_in_the_middle_makes_tabs_with_the_dropped_one_in_front() {
        let mut s = surface(&[Rect::new(600.0, 0.0, 300.0, 300.0), Rect::new(0.0, 0.0, 400.0, 400.0)]);
        let z = s.drop_zone(1, 200.0, 10.0, LONG).unwrap();
        s.apply_drop(1, z);
        assert_eq!(s.panels.len(), 1);
        let p = &s.panels[0];
        assert_eq!(p.rect, Rect::new(0.0, 0.0, 400.0, 400.0));
        assert_eq!(p.tabs.iter().map(|t| t.id).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(p.tab().id, 1);
    }

    #[test]
    fn giving_way_takes_the_dropped_size_but_at_most_half() {
        let mut s = surface(&[Rect::new(900.0, 0.0, 100.0, 300.0), Rect::new(0.0, 0.0, 800.0, 400.0)]);
        let z = s.drop_zone(1, 790.0, 200.0, LONG).unwrap();
        assert_eq!(z.how, How::Side(Side::Right));
        // Smaller than half of the other: minimum width applies, not half.
        assert_eq!(z.preview, Rect::new(680.0, 0.0, 120.0, 400.0));
        s.apply_drop(1, z);
        assert_eq!(s.get(2).unwrap().rect, Rect::new(0.0, 0.0, 680.0, 400.0));
        let h = s.get(1).unwrap();
        assert_eq!((h.rect.w, h.rect.h, h.tx, h.ty), (120.0, 400.0, 680.0, 0.0));

        let big = surface(&[Rect::new(900.0, 0.0, 600.0, 300.0), Rect::new(0.0, 0.0, 800.0, 400.0)]);
        assert_eq!(big.drop_zone(1, 10.0, 200.0, LONG).unwrap().preview.w, 400.0, "never more than half");
    }

    #[test]
    fn shared_border_moves_both_and_respects_the_minimum() {
        let mut s = surface(&[Rect::new(0.0, 0.0, 300.0, 400.0), Rect::new(301.0, 0.0, 300.0, 400.0), Rect::new(700.0, 0.0, 100.0, 100.0)]);
        let (pid, e) = s.edges_at(297.0, 200.0).unwrap();
        assert_eq!(pid, 1);
        assert!(e.right && !e.left);
        let partners = s.partners(pid, e);
        assert_eq!(partners, [(2, Side::Left)]);
        s.resize(pid, e, &partners, 350.0, 200.0);
        assert_eq!(s.get(1).unwrap().rect.w, 350.0);
        let two = s.get(2).unwrap().rect;
        assert_eq!((two.x, two.right()), (350.0, 601.0));
        s.resize(pid, e, &partners, 900.0, 200.0);
        assert_eq!(s.get(2).unwrap().rect.w, MIN_W);
        s.resize(pid, e, &partners, 5.0, 200.0);
        assert_eq!(s.get(1).unwrap().rect.w, MIN_W);
    }

    #[test]
    fn inside_the_band_only_and_only_the_top_panel() {
        let s = surface(&[Rect::new(0.0, 0.0, 300.0, 300.0), Rect::new(100.0, 100.0, 300.0, 300.0)]);
        assert_eq!(s.edges_at(150.0, 150.0), None, "inside the top panel, away from its edges");
        assert_eq!(s.edges_at(103.0, 200.0).map(|e| e.0), Some(2));
        assert_eq!(s.edges_at(50.0, 50.0), None);
        assert_eq!(s.edges_at(1000.0, 800.0), None);
    }

    #[test]
    fn joints_show_shared_borders_and_walls() {
        let s = surface(&[Rect::new(0.0, 0.0, 300.0, 400.0), Rect::new(301.0, 100.0, 300.0, 400.0)]);
        let (shared, walls) = s.joints();
        assert_eq!(shared.len(), 1);
        assert_eq!((shared[0].y0, shared[0].y1), (100.0, 400.0));
        assert_eq!(walls.len(), 2, "first panel's left and top");
    }

    #[test]
    fn a_tab_pulled_out_is_a_panel_and_a_single_tab_cannot_be() {
        let mut s = surface(&[Rect::new(600.0, 0.0, 300.0, 300.0), Rect::new(0.0, 0.0, 400.0, 400.0)]);
        let z = s.drop_zone(1, 200.0, 10.0, LONG).unwrap();
        s.apply_drop(1, z);
        let new = s.tear_out(2, 0, 500.0, 500.0).unwrap();
        assert_eq!(s.panels.len(), 2);
        assert_eq!(s.get(new).unwrap().tab().id, 1);
        assert_eq!(s.get(2).unwrap().tabs.len(), 1);
        assert_eq!(s.tear_out(2, 0, 1.0, 1.0), None);
    }

    #[test]
    fn closing_the_last_tab_closes_the_panel() {
        let mut s = surface(&[Rect::new(600.0, 0.0, 300.0, 300.0), Rect::new(0.0, 0.0, 400.0, 400.0)]);
        let z = s.drop_zone(1, 200.0, 10.0, LONG).unwrap();
        s.apply_drop(1, z);
        assert!(s.remove_tab(1));
        assert_eq!(s.panels.len(), 1);
        assert!(s.remove_tab(2));
        assert!(s.panels.is_empty());
        assert!(!s.remove_tab(2));
    }

    #[test]
    fn window_resize_keeps_walls_and_borders() {
        let mut s = surface(&[Rect::new(0.0, 0.0, 750.0, 900.0), Rect::new(750.0, 0.0, 750.0, 900.0)]);
        s.rescale(1000.0, 600.0);
        assert_eq!(s.panels[0].rect, Rect::new(0.0, 0.0, 500.0, 600.0));
        assert_eq!(s.panels[1].rect, Rect::new(500.0, 0.0, 500.0, 600.0));
        assert_eq!(s.joints().0.len(), 1);
    }

    fn roomy(rects: &[Rect]) -> Surface {
        let mut s = surface(rects);
        s.m = Metrics { outer: 14.0, gap: 10.0, pad: 5.0 };
        s
    }

    #[test]
    fn with_a_gap_panels_rest_apart_and_still_count_as_together() {
        let s = roomy(&[Rect::new(14.0, 14.0, 300.0, 400.0), Rect::new(500.0, 14.0, 300.0, 400.0)]);
        assert_eq!(s.snapped(2, 330.0, 100.0).0, 324.0, "beside the other panel, leaving the gap");
        assert_eq!(s.snapped(1, 8.0, 8.0).0, 14.0, "on the wall's margin");
        let t = roomy(&[Rect::new(14.0, 14.0, 300.0, 400.0), Rect::new(324.0, 14.0, 300.0, 400.0)]);
        let (shared, walls) = t.joints();
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].x0, 319.0, "in the middle of the gap");
        assert!(walls.iter().any(|l| l.x0 == 7.0), "in the middle of the margin");
        assert_eq!(t.partners(1, Edges { right: true, ..Default::default() }), [(2, Side::Left)]);
    }

    #[test]
    fn a_shared_border_keeps_the_gap() {
        let mut s = roomy(&[Rect::new(14.0, 14.0, 300.0, 400.0), Rect::new(324.0, 14.0, 300.0, 400.0)]);
        let e = Edges { right: true, ..Default::default() };
        let partners = s.partners(1, e);
        s.resize(1, e, &partners, 400.0, 100.0);
        let (a, b) = (s.get(1).unwrap().rect, s.get(2).unwrap().rect);
        assert_eq!(a.right(), 400.0);
        assert_eq!(b.x, 410.0);
        assert_eq!(b.right(), 624.0);
    }

    #[test]
    fn giving_way_leaves_the_gap() {
        let mut s = roomy(&[Rect::new(900.0, 14.0, 600.0, 300.0), Rect::new(14.0, 14.0, 810.0, 400.0)]);
        let z = s.drop_zone(1, 20.0, 200.0, LONG).unwrap();
        assert_eq!(z.preview.w, 400.0, "half of what remains after the gap");
        s.apply_drop(1, z);
        let t = s.get(2).unwrap().rect;
        assert_eq!((t.x, t.w), (424.0, 400.0));
    }

    #[test]
    fn a_new_theme_keeps_what_was_together() {
        let mut s = surface(&[Rect::new(0.0, 0.0, 750.0, 900.0), Rect::new(750.0, 0.0, 750.0, 900.0)]);
        s.set_metrics(Metrics { outer: 14.0, gap: 10.0, pad: 5.0 });
        assert_eq!(s.panels[0].rect, Rect::new(14.0, 14.0, 731.0, 872.0));
        assert_eq!(s.panels[1].rect, Rect::new(755.0, 14.0, 731.0, 872.0));
        assert_eq!(s.joints().0.len(), 1);
        s.set_metrics(Metrics::default());
        assert_eq!(s.panels[0].rect, Rect::new(0.0, 0.0, 750.0, 900.0));
    }

    #[test]
    fn a_resized_window_keeps_gap_and_margin() {
        let mut s = roomy(&[Rect::new(14.0, 14.0, 731.0, 872.0), Rect::new(755.0, 14.0, 731.0, 872.0)]);
        s.rescale(1000.0, 600.0);
        let (a, b) = (s.panels[0].rect, s.panels[1].rect);
        assert_eq!((a.x, a.y, b.right(), b.bottom()), (14.0, 14.0, 986.0, 586.0));
        assert!((b.x - a.right() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn the_app_sits_inside_the_frame() {
        let s = roomy(&[]);
        assert_eq!(s.app_of(Rect::new(100.0, 100.0, 400.0, 300.0)), Rect::new(105.0, 128.0, 390.0, 267.0));
    }

    #[test]
    fn the_first_layout_fills_the_surface_and_leaves_the_gaps() {
        let mut s = Surface::new(1500.0, 900.0);
        s.m = Metrics { outer: 14.0, gap: 10.0, pad: 5.0 };
        let r = |n: &str| s.first_place(n).unwrap();
        assert_eq!(r("map").x, 14.0);
        assert_eq!(r("view").right(), 1486.0);
        assert_eq!(r("term").x, r("map").right() + 10.0);
        assert_eq!(r("view").x, r("term").right() + 10.0);
        assert_eq!(r("sessions").y, r("map").bottom() + 10.0);
        assert_eq!(r("sessions").bottom(), 886.0);
        assert_eq!(r("answer").y, r("term").bottom() + 10.0);
        assert_eq!(r("input").y, r("answer").bottom() + 10.0);
        assert_eq!(r("input").bottom(), 886.0);
        assert!(s.first_place("other").is_none());
    }

    #[test]
    fn bars_stay_reachable() {
        let mut s = surface(&[Rect::new(0.0, 0.0, 300.0, 300.0)]);
        s.panels[0].tx = 5000.0;
        s.panels[0].ty = -50.0;
        s.keep_reachable(1);
        assert_eq!((s.panels[0].tx, s.panels[0].ty), (1460.0, 0.0));
    }

    #[test]
    fn layout_survives_saving_at_another_window_size() {
        let mut s = surface(&[Rect::new(0.0, 0.0, 750.0, 450.0), Rect::new(750.0, 450.0, 300.0, 450.0)]);
        let z = Zone { target: 1, how: How::Tab, preview: Rect::default() };
        s.panels.push(Panel::new(9, Tab { id: 9, key: "firefox#1".into(), name: "web".into() }, Rect::new(10.0, 10.0, 200.0, 200.0)));
        s.apply_drop(9, z);
        let text = serde_json::to_string(&s.to_saved((1500, 900))).unwrap();
        let saved = SavedLayout::parse(&text).unwrap();
        assert_eq!(saved.format, LAYOUT_FORMAT);
        assert_eq!(saved.window, Some((1500, 900)));

        let mut back = Surface::new(1000.0, 600.0);
        // Apps come back in any order; the top panel's app first.
        let two = back.place_saved(&saved, tab(2)).unwrap();
        let one = back.place_saved(&saved, tab(1)).unwrap();
        let web = back.place_saved(&saved, Tab { id: 3, key: "firefox#1".into(), name: "web".into() }).unwrap();
        assert_eq!(one, web, "the same tabs again");
        assert_eq!(back.panels.iter().map(|p| p.pid).collect::<Vec<_>>(), [two, one], "the same stacking: the drop raised the first panel");
        let p = back.get(one).unwrap();
        assert_eq!(p.rect, Rect::new(0.0, 0.0, 500.0, 300.0));
        assert_eq!(p.tabs.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(), ["firefox#1", "app#1"]);
        assert_eq!(p.tab().key, "firefox#1");
        assert!(back.place_saved(&saved, tab(77)).is_err(), "not in the layout");
    }

    #[test]
    fn a_broken_or_future_layout_file_does_not_fail() {
        assert!(SavedLayout::parse("{ not json").is_none());
        let future = SavedLayout::parse(r#"{"format":9,"panels":[],"something":1}"#).unwrap();
        assert_eq!(future.format, 9);
    }

    #[test]
    fn panels_saved_on_each_other_come_back_as_tabs() {
        let text = r#"{"format":1,"panels":[
            {"x":0.01,"y":0.640,"w":0.177,"h":0.344,"active":"files#1","tabs":["files#1"]},
            {"x":0.5,"y":0.0,"w":0.2,"h":0.2,"active":"small#1","tabs":["small#1"]},
            {"x":0.01,"y":0.639,"w":0.176,"h":0.345,"active":"sessions#1","tabs":["sessions#1"]},
            {"x":0.0,"y":0.0,"w":1.0,"h":1.0,"active":"big#1","tabs":["big#1"]}
        ]}"#;
        let l = SavedLayout::parse(text).unwrap();
        assert_eq!(l.panels.len(), 3, "a small panel over a big one stays its own");
        let joined = l.panels.iter().find(|p| p.active == "sessions#1").unwrap();
        assert_eq!(joined.tabs, vec!["sessions#1", "files#1"]);
    }

    #[test]
    fn a_tab_moves_into_another_panel_and_an_empty_panel_goes() {
        let mut s = surface(&[Rect::new(0.0, 0.0, 400.0, 400.0), Rect::new(500.0, 0.0, 400.0, 400.0)]);
        let (a, b) = (s.panels[0].pid, s.panels[1].pid);
        let moved = s.panels[0].tabs[0].id;
        assert!(s.move_tab_to(moved, b));
        assert!(s.get(a).is_none());
        assert_eq!(s.get(b).unwrap().tabs.len(), 2);
        assert!(!s.move_tab_to(moved, b), "already there");
    }

    #[test]
    fn the_first_layout_comes_back() {
        let mut s = surface(&[Rect::new(0.0, 0.0, 400.0, 400.0), Rect::new(500.0, 0.0, 400.0, 400.0), Rect::new(900.0, 400.0, 300.0, 300.0)]);
        let ids: Vec<u64> = s.panels.iter().map(|p| p.tabs[0].id).collect();
        let slot = |t: &Tab| if t.id == ids[0] || t.id == ids[1] { Some("view".to_string()) } else if t.id == ids[2] { Some("map".to_string()) } else { None };
        let second = s.panels[1].tabs[0].key.clone();
        s.reset_places(slot, |k| k == second);
        assert_eq!(s.panels.len(), 2);
        assert_eq!(s.panels[0].tabs.len(), 2, "one place, one panel");
        assert_eq!(s.panels[0].tab().key, second, "the one meant to be in front");
        assert_eq!(s.panels[0].rect, s.first_place("view").unwrap());
        assert_eq!(s.panels[1].rect, s.first_place("map").unwrap());
    }
}
