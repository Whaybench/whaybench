//! The window on the person's desktop and what is drawn in it each frame: top to bottom, because the
//! renderer takes the frontmost element first.
use std::{cell::{Cell, RefCell}, rc::Rc, time::{Duration, Instant}};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, ExportMem, Offscreen,
            damage::OutputDamageTracker,
            element::{
                utils::{Relocate, RelocateRenderElement},
                AsRenderElements, Id, Kind, memory::MemoryRenderBufferRenderElement, render_elements,
                solid::SolidColorRenderElement, surface::WaylandSurfaceRenderElement, utils::CropRenderElement,
            },
            gles::{GlesPixelProgram, GlesRenderer, GlesTexture, Uniform, UniformName, UniformType, element::PixelShaderElement},
            utils::CommitCounter,
        },
        winit::{self, WinitEvent},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{calloop::{EventLoop, timer::{TimeoutAction, Timer}}, winit::{cursor::CursorIcon, dpi::LogicalSize, window::WindowAttributes}},
    utils::{Buffer, Logical, Physical, Rectangle, Scale, Size, Transform},
};
use wb_core::{log as record, theme::premultiplied};

use crate::{App, ctl, layout::{Guide, How, Line, Panel, Rect, Side, TITLE}, state::{Act, FIRST_WINDOW}, };

render_elements! {
    pub PanelElement<=GlesRenderer>;
    App=CropRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    Solid=SolidColorRenderElement,
    Text=MemoryRenderBufferRenderElement<GlesRenderer>,
    Shader=PixelShaderElement,
}

// What goes to the window: the surface moved down below the menu bar, and the menu bar with its menus over everything.
render_elements! {
    pub WindowElement<=GlesRenderer>;
    Surface=RelocateRenderElement<PanelElement>,
    Menu=WaylandSurfaceRenderElement<GlesRenderer>,
}

const CLOSE: f64 = 22.0;
const TAB_CLOSE: f64 = 18.0;
/// The drop preview's fill and the tint of a panel about to be carried along.
const CARRIED: [f32; 4] = [0.45, 0.43, 0.38, 0.45];

/// A rounded box with its border, an optional title bar with a gradient, and a soft shadow around it.
/// `v_coords` runs 0..1 over the element and `size` is the element's size in pixels.
const CARD: &str = r#"
precision highp float;
varying vec2 v_coords;
uniform vec2 size;
uniform float alpha;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
uniform float margin;
uniform float radius;
uniform float border;
uniform float bar;
uniform vec4 fill;
uniform vec4 bar_top;
uniform vec4 bar_bottom;
uniform vec4 line;
uniform vec4 shadow;

float box(vec2 p, vec2 half_size, float r) {
    vec2 d = abs(p) - half_size + vec2(r);
    return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0) - r;
}

void main() {
    vec2 px = v_coords * size;
    vec2 inner = size - vec2(margin * 2.0);
    vec2 centre = px - size / 2.0;
    float d = box(centre, inner / 2.0, radius);

    float y = px.y - margin;
    vec4 body = fill;
    if (bar > 0.0 && y < bar) body = mix(bar_top, bar_bottom, clamp(y / bar, 0.0, 1.0));
    if (border > 0.0) body = mix(body, line, smoothstep(-border - 0.5, -border + 0.5, d));

    float inside = 1.0 - smoothstep(-0.5, 0.5, d);
    vec4 colour = body * inside;
    if (margin > 0.0) {
        float fall = clamp(1.0 - max(d, 0.0) / margin, 0.0, 1.0);
        colour += shadow * fall * fall * (1.0 - inside);
    }
    gl_FragColor = colour * alpha;
}
"#;

/// The surface behind the panels: a vertical gradient.
const BACK: &str = r#"
precision highp float;
varying vec2 v_coords;
uniform vec2 size;
uniform float alpha;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
uniform vec4 top;
uniform vec4 bottom;
void main() { gl_FragColor = mix(top, bottom, v_coords.y) * alpha; }
"#;

pub struct Shaders { card: GlesPixelProgram, back: GlesPixelProgram }

fn compile(renderer: &mut GlesRenderer) -> Result<Shaders, String> {
    let n = |s: &'static str, t| UniformName::new(s, t);
    let card = renderer.compile_custom_pixel_shader(CARD, &[
        n("margin", UniformType::_1f), n("radius", UniformType::_1f), n("border", UniformType::_1f), n("bar", UniformType::_1f),
        n("fill", UniformType::_4f), n("bar_top", UniformType::_4f), n("bar_bottom", UniformType::_4f), n("line", UniformType::_4f), n("shadow", UniformType::_4f),
    ]).map_err(|e| format!("{e:?}"))?;
    let back = renderer.compile_custom_pixel_shader(BACK, &[n("top", UniformType::_4f), n("bottom", UniformType::_4f)]).map_err(|e| format!("{e:?}"))?;
    Ok(Shaders { card, back })
}

fn four(name: &'static str, c: [f32; 4]) -> Uniform<'static> { Uniform::new(name, (c[0], c[1], c[2], c[3])) }

/// What a card looks like, in the colours as drawing wants them.
struct Card { radius: f64, border: f64, bar: f64, margin: f64, fill: [f32; 4], top: [f32; 4], bottom: [f32; 4], line: [f32; 4], shadow: [f32; 4] }

impl Card {
    fn plain(radius: f64, fill: [f32; 4]) -> Self {
        Card { radius, border: 0.0, bar: 0.0, margin: 0.0, fill, top: fill, bottom: fill, line: fill, shadow: [0.0; 4] }
    }
}

/// Where the pointer is in a panel's title bar.
pub enum BarHit { Close, CloseTab(usize), Tab(usize), Bar }

/// Each tab's area in the bar, as many as fit: left of the close button.
fn tab_rects(state: &mut App, p: &Panel) -> Vec<Rect> {
    // With tabs each has its own close button, so the bar has none: nothing closes a whole group by accident.
    let limit = p.rect.right() - 6.0;
    let mut x = p.rect.x + 6.0;
    let mut out = Vec::new();
    for t in &p.tabs {
        let w = state.chrome.width(&t.name) + 22.0 + TAB_CLOSE;
        let w = w.min(limit - x);
        if w < 24.0 { break; }
        out.push(Rect::new(x, p.rect.y + 3.0, w, TITLE - 6.0));
        x += w + 4.0;
    }
    out
}

pub fn bar_hit(state: &mut App, pid: u64, x: f64, y: f64) -> Option<BarHit> {
    let p = state.surface.get(pid)?.clone();
    if !p.rect.bar().contains(x, y) { return None; }
    if p.tabs.len() > 1 {
        if let Some((i, r)) = tab_rects(state, &p).iter().enumerate().find(|(_, r)| r.contains(x, y)) {
            return Some(if x >= r.right() - TAB_CLOSE { BarHit::CloseTab(i) } else { BarHit::Tab(i) });
        }
        return Some(BarHit::Bar);
    }
    if x >= p.rect.right() - CLOSE { return Some(BarHit::Close); }
    Some(BarHit::Bar)
}

fn phys(r: Rect) -> Rectangle<i32, Physical> {
    let (x0, y0) = (r.x.round() as i32, r.y.round() as i32);
    let (x1, y1) = (r.right().round() as i32, r.bottom().round() as i32);
    Rectangle::new((x0, y0).into(), (x1 - x0, y1 - y0).into())
}

/// Adds elements in order, front first. Solid colours reuse ids, and shaded boxes are kept while they look the same, so
/// frames are comparable and only what changed is sent.
struct Frame<'a> { out: Vec<PanelElement>, ids: &'a mut Vec<(Id, [f32; 4])>, n: usize, commit: CommitCounter, kept: &'a mut Vec<(String, PixelShaderElement)>, k: usize }

impl Frame<'_> {
    fn solid(&mut self, r: Rect, colour: [f32; 4]) {
        // A new colour is a new element: the same one in another colour would not count as changed.
        if self.n == self.ids.len() { self.ids.push((Id::new(), colour)); }
        else if self.ids[self.n].1 != colour { self.ids[self.n] = (Id::new(), colour); }
        let id = self.ids[self.n].0.clone();
        self.n += 1;
        self.out.push(SolidColorRenderElement::new(id, phys(r), self.commit, colour, Kind::Unspecified).into());
    }
    /// A shaded box: the one of last frame at this place in the order if it looks the same, else a new one.
    fn shaded(&mut self, program: &GlesPixelProgram, area: Rectangle<i32, Logical>, uniforms: Vec<Uniform<'static>>, look: String) {
        let make = || PixelShaderElement::new(program.clone(), area, None, 1.0, uniforms, Kind::Unspecified);
        if self.k == self.kept.len() { self.kept.push((look, make())); }
        else if self.kept[self.k].0 != look { self.kept[self.k] = (look, make()); }
        self.out.push(self.kept[self.k].1.clone().into());
        self.k += 1;
    }
    /// A rounded box. Without shaders (they failed to compile), a plain rectangle.
    fn card(&mut self, shaders: Option<&Shaders>, r: Rect, c: Card) {
        let Some(sh) = shaders else {
            self.solid(r, c.fill);
            return;
        };
        let m = c.margin;
        let area = phys(Rect::new(r.x - m, r.y - m, r.w + 2.0 * m, r.h + 2.0 * m));
        let logical: Rectangle<i32, Logical> = Rectangle::new((area.loc.x, area.loc.y).into(), (area.size.w, area.size.h).into());
        let radius = c.radius.min(r.w / 2.0).min(r.h / 2.0);
        let look = format!("card {logical:?} {m} {radius} {} {} {:?} {:?} {:?} {:?} {:?}", c.border, c.bar, c.fill, c.top, c.bottom, c.line, c.shadow);
        self.shaded(&sh.card, logical, vec![
            Uniform::new("margin", m as f32), Uniform::new("radius", radius as f32),
            Uniform::new("border", c.border as f32), Uniform::new("bar", c.bar as f32),
            four("fill", c.fill), four("bar_top", c.top), four("bar_bottom", c.bottom), four("line", c.line), four("shadow", c.shadow),
        ], look);
    }
    fn line(&mut self, l: Line, width: f64, colour: [f32; 4]) {
        let half = width / 2.0;
        self.solid(Rect::new(l.x0 - half, l.y0 - half, (l.x1 - l.x0).abs() + width, (l.y1 - l.y0).abs() + width), colour);
    }
}

fn add_text(state: &mut App, renderer: &mut GlesRenderer, f: &mut Vec<PanelElement>, text: &str, colour: [u8; 3], area: Rect) {
    let Some(t) = state.chrome.text(text, colour) else { return };
    let x = area.x + ((area.w - t.w as f64) / 2.0).max(0.0);
    let y = area.y + (area.h - t.h as f64) / 2.0;
    let src = (t.w as f64 > area.w).then(|| Rectangle::new((0.0, 0.0).into(), (area.w, t.h as f64).into()));
    let size = src.map(|_| (area.w as i32, t.h).into());
    if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(renderer, (x.round(), y.round()), &t.buffer, None, src, size, Kind::Unspecified) {
        f.push(e.into());
    }
}

/// The panel's front app, cut to its area: nothing an app draws reaches outside its panel.
fn app_elements(state: &App, p: &Panel, renderer: &mut GlesRenderer, area: Rect) -> Vec<PanelElement> {
    let Some(w) = state.windows.get(&p.tab().id) else { return Vec::new() };
    let g = w.geometry().loc;
    let at = (area.x.round() as i32 - g.x, area.y.round() as i32 - g.y);
    let elements = AsRenderElements::<GlesRenderer>::render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(w, renderer, at.into(), Scale::from(1.0), 1.0);
    let crop = phys(area);
    elements.into_iter().filter_map(|e| CropRenderElement::from_element(e, 1.0, crop)).map(PanelElement::from).collect()
}

/// One panel: tint if it will be carried along, bar text, its app, then its card. Pushed first is drawn on top.
/// `own_shadow`: the panel casts its shadow over what lies under it (a settings window); panels side by side get theirs
/// from `shadows`, under all of them, so one in front does not darken its neighbours.
fn draw_panel(state: &mut App, renderer: &mut GlesRenderer, f: &mut Frame, shaders: Option<&Shaders>, p: &Panel, front: bool, carried: &[u64], own_shadow: bool) {
    let t = state.theme.clone();
    let radius = t.radius;
    let app = state.surface.app_of(p.rect);
    if carried.contains(&p.pid) { f.card(shaders, p.rect, Card::plain(radius, CARRIED)); }
    if p.tabs.len() == 1 {
        add_text(state, renderer, &mut f.out, "×", t.ink, Rect::new(p.rect.right() - CLOSE, p.rect.y, CLOSE, TITLE));
    }
    if p.tabs.len() > 1 {
        let rects = tab_rects(state, p);
        for (i, r) in rects.iter().enumerate() {
            let ink = if i == p.active { t.ink } else { t.ink_soft };
            add_text(state, renderer, &mut f.out, "×", ink, Rect::new(r.right() - TAB_CLOSE, r.y, TAB_CLOSE, r.h));
            add_text(state, renderer, &mut f.out, &p.tabs[i].name.clone(), ink, Rect::new(r.x, r.y, (r.w - TAB_CLOSE).max(1.0), r.h));
        }
        if let Some(r) = rects.get(p.active) { f.card(shaders, *r, Card::plain(radius.min(7.0), premultiplied(t.tab_front))); }
    } else {
        // The name at the bar's left; its width only as far as the close button.
        let wide = state.chrome.width(&p.tab().name) + 2.0;
        let left = 8.0 + (radius / 2.0).max(0.0);
        let area = Rect::new(p.rect.x + left, p.rect.y, wide.min((p.rect.w - CLOSE - left - 6.0).max(1.0)), TITLE);
        add_text(state, renderer, &mut f.out, &p.tab().name.clone(), if front { t.ink } else { t.ink_soft }, area);
    }
    f.out.extend(app_elements(state, p, renderer, app));
    f.solid(app, premultiplied(t.panel));
    f.card(shaders, p.rect, Card {
        radius, border: t.border_width, bar: TITLE, margin: if own_shadow { t.shadow_size } else { 0.0 },
        fill: premultiplied(t.panel),
        top: premultiplied(if front { t.front_top } else { t.bar_top }),
        bottom: premultiplied(if front { t.front_bottom } else { t.bar_bottom }),
        line: premultiplied(if front { t.front_border } else { t.border }),
        shadow: if own_shadow { premultiplied(t.shadow) } else { [0.0; 4] },
    });
}

fn surface_elements(state: &mut App, renderer: &mut GlesRenderer) -> Vec<PanelElement> {
    let mut ids = std::mem::take(&mut state.solid_ids);
    let mut kept = std::mem::take(&mut state.shaded);
    let shaders = state.shaders.take();
    let status_elements = state.status.clone().map(|w| {
        let g = w.geometry().loc;
        let area = Rect::new(0.0, state.surface.h, state.surface.w, crate::state::STATUS_H as f64);
        let at = (-g.x, area.y.round() as i32 - g.y);
        let e = AsRenderElements::<GlesRenderer>::render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(&w, renderer, at.into(), Scale::from(1.0), 1.0);
        (e, area)
    });
    let t = state.theme.clone();
    let mut f = Frame { out: Vec::new(), ids: &mut ids, n: 0, commit: CommitCounter::default(), kept: &mut kept, k: 0 };
    let accent = premultiplied(t.accent);
    let radius = t.radius;
    if let Some((elements, area)) = status_elements {
        let crop = phys(area);
        f.out.extend(elements.into_iter().filter_map(|e| CropRenderElement::from_element(e, 1.0, crop)).map(PanelElement::from));
        f.solid(area, premultiplied(t.panel));
    }

    // A settings window is above everything else, the joint lines included.
    let modal: Vec<Panel> = state.surface.panels.iter().rev().filter(|p| p.modal).cloned().collect();
    let top_modal = modal.first().map(|p| p.pid);
    for p in &modal { draw_panel(state, renderer, &mut f, shaders.as_ref(), p, Some(p.pid) == top_modal, &[], true); }

    // Above everything: the drop preview and the guides.
    if let Some(z) = state.zone {
        let name = state.surface.get(z.target).map(|p| p.tab().name.clone()).unwrap_or_default();
        let label = if z.how == How::Tab { wb_core::i18n::f("as-a-tab-of", &[("name", (name).to_string())]) } else { wb_core::i18n::f("beside", &[("name", (name).to_string())]) };
        let ink = [(t.accent[0] * 255.0) as u8, (t.accent[1] * 255.0) as u8, (t.accent[2] * 255.0) as u8];
        add_text(state, renderer, &mut f.out, &label, ink, z.preview);
        let mut c = Card::plain(radius, [accent[0] * 0.24, accent[1] * 0.24, accent[2] * 0.24, 0.24]);
        c.border = 2.0;
        c.line = accent;
        f.card(shaders.as_ref(), z.preview, c);
    }
    let (sw, sh) = (state.surface.w, state.surface.h);
    let shared_colour = premultiplied(t.shared);
    for g in &state.guides {
        let (mut at, end) = (0.0, match g { Guide::X(_) => sh, Guide::Y(_) => sw });
        while at < end {
            let piece = match g { Guide::X(x) => Rect::new(*x, at, 1.0, 5.0), Guide::Y(y) => Rect::new(at, *y, 5.0, 1.0) };
            f.solid(piece, shared_colour);
            at += 10.0;
        }
    }
    // The border about to be dragged, then the lines that always show which panels hold together.
    let active = match &state.act { Act::Resize { pid, edges, partners } => Some((*pid, *edges, partners.clone())), _ => state.hover.clone() };
    if let Some((pid, edges, _)) = &active {
        if let Some(r) = state.surface.get(*pid).map(|p| p.rect) {
            for s in edges.sides() {
                let l = match s {
                    Side::Left => Line { x0: r.x, y0: r.y, x1: r.x, y1: r.bottom() },
                    Side::Right => Line { x0: r.right(), y0: r.y, x1: r.right(), y1: r.bottom() },
                    Side::Top => Line { x0: r.x, y0: r.y, x1: r.right(), y1: r.y },
                    Side::Bottom => Line { x0: r.x, y0: r.bottom(), x1: r.right(), y1: r.bottom() },
                };
                f.line(l, 2.0, premultiplied(t.edge));
            }
        }
    }
    // Panels that hold on to nothing lie over the joint lines; the others under them.
    let carried: Vec<u64> = active.as_ref().map(|(_, _, ps)| ps.iter().map(|(p, _)| *p).collect()).unwrap_or_default();
    let top = state.surface.front();
    let free = state.surface.free();
    let panels: Vec<Panel> = state.surface.panels.iter().rev().filter(|p| !p.modal).cloned().collect();
    for p in panels.iter().filter(|p| free.contains(&p.pid)) {
        draw_panel(state, renderer, &mut f, shaders.as_ref(), p, Some(p.pid) == top, &carried, false);
    }
    let busy = active.is_some() || !matches!(state.act, Act::Idle);
    if t.joint_width > 0.0 && t.joint_show.visible(busy) {
        let (shared, walls) = state.surface.joints();
        for l in shared { f.line(l, t.joint_width, shared_colour); }
        for l in walls { f.line(l, t.joint_width, premultiplied(t.wall)); }
    }

    // The held panels, top one first. Each: tint if it will be carried along, bar text, app, then its card.
    for p in panels.iter().filter(|p| !free.contains(&p.pid)) {
        draw_panel(state, renderer, &mut f, shaders.as_ref(), p, Some(p.pid) == top, &carried, false);
    }
    // Their shadows, under every one of them: only the box's outside is drawn.
    if t.shadow_size > 0.0 {
        let none = [0.0; 4];
        for p in panels.iter().filter(|p| !p.modal) {
            f.card(shaders.as_ref(), p.rect, Card { radius, border: 0.0, bar: 0.0, margin: t.shadow_size, fill: none, top: none, bottom: none, line: none, shadow: premultiplied(t.shadow) });
        }
    }
    // The surface itself, under everything.
    if let Some(sh2) = &shaders {
        if t.surface_top != t.surface_bottom {
            let area: Rectangle<i32, Logical> = Rectangle::new((0, 0).into(), (sw.round() as i32, sh.round() as i32).into());
            let (top, bottom) = (premultiplied(t.surface_top), premultiplied(t.surface_bottom));
            f.shaded(&sh2.back, area, vec![four("top", top), four("bottom", bottom)], format!("back {area:?} {top:?} {bottom:?}"));
        }
    }
    let used = f.k;
    let out = f.out;
    kept.truncate(used);
    state.shaded = kept;
    state.solid_ids = ids;
    state.shaders = shaders;
    out
}

fn window_elements(state: &mut App, renderer: &mut GlesRenderer) -> Vec<WindowElement> {
    let down = state.menu_height();
    let mut out: Vec<WindowElement> = Vec::new();
    if let Some(w) = state.menu.clone() {
        let g = w.geometry().loc;
        let at = (-g.x, -g.y);
        out.extend(AsRenderElements::<GlesRenderer>::render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(&w, renderer, at.into(), Scale::from(1.0), 1.0).into_iter().map(WindowElement::from));
    }
    let surface = surface_elements(state, renderer);
    out.extend(surface.into_iter().map(|e| WindowElement::from(RelocateRenderElement::from_element(e, (0, down), Relocate::Relative))));
    out
}

/// Draws the elements off-screen and saves them. Must run before the frame's own drawing: reading after
/// it makes the next swap of the window fail with ContextLost.
fn save<E: smithay::backend::renderer::element::RenderElement<GlesRenderer>>(renderer: &mut GlesRenderer, elements: &[E], w: i32, h: i32, clear: [f32; 4], out: &std::path::Path) -> Result<(), String> {
    let size: Size<i32, Buffer> = (w, h).into();
    let mut texture: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, size).map_err(|e| e.to_string())?;
    let mut fb = renderer.bind(&mut texture).map_err(|e| e.to_string())?;
    let mut tracker = OutputDamageTracker::new((w, h), 1.0, Transform::Normal);
    tracker.render_output(renderer, &mut fb, 0, elements, clear).map_err(|e| format!("{e:?}"))?;
    let map = renderer.copy_framebuffer(&fb, Rectangle::from_size(size), Fourcc::Abgr8888).map_err(|e| e.to_string())?;
    let rgba = renderer.map_texture(&map).map_err(|e| e.to_string())?;
    write_png(out, w as u32, h as u32, rgba)
}

pub fn write_png(out: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    let mut bytes = Vec::new();
    {
        let mut e = png::Encoder::new(&mut bytes, w, h);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        let mut writer = e.write_header().map_err(|x| x.to_string())?;
        let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        writer.write_image_data(&rgb).map_err(|x| x.to_string())?;
    }
    ctl::write_atomic(out, &bytes).map_err(|x| x.to_string())
}

fn take_shots(state: &mut App, renderer: &mut GlesRenderer, surface: Size<i32, Physical>) {
    for shot in std::mem::take(&mut state.shots) {
        let began = Instant::now();
        // By app: that app's window, even in a tab behind another; the view panel is read whatever covers it.
        let by_app = shot.app.as_ref().and_then(|app| {
            let prefix = format!("{app}#");
            state.surface.panels.iter().find_map(|p| p.tabs.iter().position(|t| t.key.starts_with(&prefix)).map(|i| { let mut q = p.clone(); q.active = i; q }))
        });
        let chosen = by_app.or_else(|| shot.panel.and_then(|i| state.surface.panels.get(i).cloned()));
        let missing = shot.app.is_some() || shot.panel.is_some();
        let done = match (chosen, missing) {
            (None, false) => { let e = window_elements(state, renderer); let c = premultiplied(state.theme.surface_top); save(renderer, &e, surface.w, surface.h, c, &shot.out) }
            (None, true) => Err(wb_core::i18n::t("no-such-panel").to_string()),
            (Some(p), _) => {
                let a = state.surface.app_of(p.rect);
                let (w, h) = (a.w.round() as i32, a.h.round() as i32);
                let mut e = app_elements(state, &p, renderer, Rect::new(0.0, 0.0, w as f64, h as f64));
                let (mut ids, mut kept) = (Vec::new(), Vec::new());
                let mut f = Frame { out: Vec::new(), ids: &mut ids, n: 0, commit: CommitCounter::default(), kept: &mut kept, k: 0 };
                let panel = premultiplied(state.theme.panel);
                f.solid(Rect::new(0.0, 0.0, w as f64, h as f64), panel);
                e.extend(f.out);
                save(renderer, &e, w, h, panel, &shot.out)
            }
        };
        match done {
            Ok(()) => record::detail("screen", "panel shot", &[("panel", format!("{:?}", shot.panel)), ("took", format!("{}ms", began.elapsed().as_millis()))]),
            Err(e) => {
                record::warn("screen", "panel not shot", &e, &[("panel", format!("{:?}", shot.panel))]);
                let _ = std::fs::write(ctl::error_file(&shot.out), e);
            }
        }
    }
}

pub fn init_window(event_loop: &mut EventLoop<App>, state: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let (w, h) = state.saved_window().map(|(w, h)| (w as f64, h as f64)).unwrap_or(FIRST_WINDOW);
    let attributes = WindowAttributes::default().with_title("Whaybench").with_surface_size(LogicalSize::new(w, h)).with_visible(true);
    let (backend, winit) = winit::init_from_attributes::<GlesRenderer>(attributes)?;

    let mode = Mode { size: backend.window_size(), refresh: 60_000 };
    let output = Output::new(
        "whaybench".to_string(),
        PhysicalProperties { size: (0, 0).into(), subpixel: Subpixel::Unknown, make: "Whaybench".into(), model: "Surface".into(), serial_number: "1".into() },
    );
    let _global = output.create_global::<App>(&state.display_handle);
    output.change_current_state(Some(mode), Some(Transform::Flipped180), None, Some((0, 0).into()));
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));
    state.surface.rescale(mode.size.w as f64, mode.size.h as f64);
    state.window_size = (mode.size.w, mode.size.h);

    let scale = backend.window().scale_factor();
    record::header(&format!("environment+ · screen={}x{} scale={scale}", mode.size.w, mode.size.h));

    let mut damage_tracker = OutputDamageTracker::from_output(&output);
    let mut last = Instant::now();
    let mut cursor = CursorIcon::Default;
    let mut shaders_failed = false;
    let (mut times, mut since, mut sent_frames) = (Vec::with_capacity(400), Instant::now(), 0usize);
    #[allow(deprecated)] // The newer call needs a capability set.
    backend.window().set_ime_allowed(true);

    // Only what changed goes to the desktop. A frame with nothing new is not sent at all: sending the whole window
    // every frame makes the desktop redraw it and its title bar 60 times a second, which shows as a flicker there.
    // Without a sent frame nothing paces the next one, so a timer asks for it instead.
    let backend = Rc::new(RefCell::new(backend));
    let idle = Rc::new(Cell::new(false));
    {
        let (backend, idle) = (backend.clone(), idle.clone());
        event_loop.handle().insert_source(Timer::from_duration(Duration::from_millis(16)), move |_, _, _| {
            if idle.replace(false) { backend.borrow().window().request_redraw(); }
            TimeoutAction::ToDuration(Duration::from_millis(16))
        })?;
    }

    event_loop.handle().insert_source(winit, move |event, _, state| { let mut backend = backend.borrow_mut(); match event {
        WinitEvent::Resized { size, .. } => {
            output.change_current_state(Some(Mode { size, refresh: 60_000 }), None, None, None);
            state.window_size = (size.w, size.h);
            state.fit_strips();
            state.note_saved_soon();
        }
        WinitEvent::Input(event) => state.process_input_event(event),
        WinitEvent::Ime(ime) => state.handle_ime(ime),
        WinitEvent::Focus(focused) => record::detail("screen", "window focus", &[("focused", focused.to_string())]),
        WinitEvent::Redraw => {
            let now = Instant::now();
            let ms = now.duration_since(last).as_secs_f64() * 1000.0;
            last = now;
            state.frame(ms);
            times.push(ms);
            if since.elapsed() >= Duration::from_secs(5) {
                let mut sorted = std::mem::take(&mut times);
                sorted.sort_by(f64::total_cmp);
                let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q) as usize];
                record::detail("screen", "frames", &[("count", sorted.len().to_string()), ("sent", std::mem::take(&mut sent_frames).to_string()), ("median", format!("{:.1}ms", at(0.5))), ("p99", format!("{:.1}ms", at(0.99))), ("max", format!("{:.1}ms", at(1.0)))]);
                since = now;
            }

            let wanted = match (&state.act, &state.hover) {
                (Act::Hold { .. }, _) => CursorIcon::Grabbing,
                (Act::Resize { edges, .. }, _) => edge_cursor(edges),
                (_, Some((_, edges, _))) => edge_cursor(edges),
                _ => CursorIcon::Default,
            };
            if wanted != cursor {
                cursor = wanted;
                backend.window().set_cursor(cursor.into());
            }

            let size = backend.window_size();
            let age = backend.buffer_age().unwrap_or(0);
            let damage = {
                let (renderer, mut framebuffer) = backend.bind().unwrap();
                if state.shaders.is_none() && !shaders_failed {
                    match compile(renderer) {
                        Ok(sh) => state.shaders = Some(sh),
                        Err(e) => { shaders_failed = true; record::error("screen", "shader failed", &[("error", e)]); }
                    }
                }
                if !state.shots.is_empty() { take_shots(state, renderer, size); }
                let elements = window_elements(state, renderer);
                let clear = premultiplied(state.theme.surface_top);
                damage_tracker.render_output(renderer, &mut framebuffer, age, &elements, clear).unwrap().damage.cloned().unwrap_or_default()
            };
            let sent = !damage.is_empty();
            if sent { backend.submit(Some(&damage)).unwrap(); sent_frames += 1; }

            state.space.elements().for_each(|window| {
                window.send_frame(&output, state.start_time.elapsed(), Some(Duration::ZERO), |_, _| Some(output.clone()))
            });
            state.space.refresh();
            state.popups.cleanup();
            let _ = state.display_handle.flush_clients();
            if sent { backend.window().request_redraw(); } else { idle.set(true); }
        }
        WinitEvent::CloseRequested => {
            state.end_reason = "window closed".into();
            state.loop_signal.stop();
        }
    }})?;
    Ok(())
}

fn edge_cursor(e: &crate::layout::Edges) -> CursorIcon {
    let (h, v) = (e.left || e.right, e.top || e.bottom);
    if h && v {
        if (e.left && e.top) || (e.right && e.bottom) { CursorIcon::NwseResize } else { CursorIcon::NeswResize }
    } else if h { CursorIcon::EwResize } else { CursorIcon::NsResize }
}
