//! The window on the person's desktop and what is drawn in it each frame: top to bottom, because the
//! renderer takes the frontmost element first.
use std::{cell::{Cell, RefCell}, rc::Rc, time::{Duration, Instant}};

use smithay::{
    backend::{
        allocator::Fourcc,
        egl::EGLDevice,
        renderer::{
            Bind, ExportMem, ImportDma, Offscreen,
            damage::OutputDamageTracker,
            element::{
                utils::{Relocate, RelocateRenderElement},
                AsRenderElements, Id, Kind, memory::MemoryRenderBufferRenderElement, render_elements,
                solid::SolidColorRenderElement, surface::WaylandSurfaceRenderElement, utils::CropRenderElement,
            },
            gles::{GlesPixelProgram, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName, UniformType, element::{PixelShaderElement, TextureShaderElement}},
            element::texture::TextureRenderElement, Renderer,
            utils::CommitCounter,
        },
        winit::{self, WinitEvent},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{calloop::{EventLoop, timer::{TimeoutAction, Timer}}, winit::{cursor::CursorIcon, dpi::LogicalSize, window::WindowAttributes}},
    wayland::dmabuf::{DmabufFeedbackBuilder, DmabufState},
    utils::{Buffer, Logical, Physical, Rectangle, Scale, Size, Transform},
};
use wb_core::{log as record, theme::{Theme, premultiplied}};

use crate::{App, ctl, layout::{Guide, How, Line, Panel, Rect, Side, targets, title}, state::{Act, FIRST_WINDOW}, };

render_elements! {
    pub PanelElement<=GlesRenderer>;
    App=CropRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    Solid=SolidColorRenderElement,
    Text=MemoryRenderBufferRenderElement<GlesRenderer>,
    Shader=PixelShaderElement,
    Glass=Steady,
}

// What goes to the window: the surface moved down below the menu bar, and the menu bar with its menus over everything.
render_elements! {
    pub WindowElement<=GlesRenderer>;
    Surface=RelocateRenderElement<PanelElement>,
    Menu=WaylandSurfaceRenderElement<GlesRenderer>,
}

pub(crate) mod shots;
mod window;
pub use window::init_window;

const CLOSE: f64 = 22.0;
const TAB_CLOSE: f64 = 18.0;
/// The drop preview's fill and the tint of a panel about to be carried along.
const CARRIED: [f32; 4] = [0.45, 0.43, 0.38, 0.45];

/// One light above the window's top middle, shared by the surface and the cards: how lit a point is, and how dark
/// towards the edges. Distances in pixels over the shorter side, so the light stays round whatever the window's shape.
macro_rules! scene { () => { r#"
float scene_light(vec2 uv, vec2 sz) {
    vec2 p = (uv - vec2(0.5, -0.1)) * sz / min(sz.x, sz.y);
    return exp(-dot(p, p) * 1.6);
}
float scene_dark(vec2 uv, vec2 sz) {
    vec2 e = (uv - vec2(0.5)) * sz / min(sz.x, sz.y);
    return smoothstep(0.35, 1.1, length(e));
}
"# } }

/// A rounded box with its border, an optional title bar with a gradient, and a soft shadow around it.
/// `v_coords` runs 0..1 over the element and `size` is the element's size in pixels.
const CARD: &str = concat!(r#"
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
uniform float lift;
uniform float drop;
uniform float gloss;
uniform float shine_at;
uniform float foot;
uniform float panel_light;
uniform float panel_shade;
uniform float rim_light;
uniform float rim_shade;
uniform vec2 at;
uniform vec2 screen;
"#, scene!(), r#"
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
    if (bar > 0.0 && y < bar) {
        body = mix(bar_top, bar_bottom, clamp(y / bar, 0.0, 1.0));
        // A glossy bar: its upper half catches the light, and a thin shade at its foot parts it from the body.
        body = mix(body, vec4(body.a), gloss * (1.0 - smoothstep(0.0, bar * 0.55, y)));
        body = mix(body, vec4(0.0, 0.0, 0.0, body.a), foot * smoothstep(bar - 3.0, bar, y));
    }
    // A bright line just inside the rim along the top, following the rounded corners.
    float shine = smoothstep(-border - 2.5, -border - 1.0, d) * (1.0 - smoothstep(-border - 0.5, -border, d));
    float top = 1.0 - clamp(y / max(radius * 2.0, 1.0), 0.0, 1.0);
    if (bar > 0.0) body = mix(body, vec4(body.a), shine_at * shine * top * top);
    if (border > 0.0) {
        // Light from above: the rim is lighter at the top and darker at the bottom (premultiplied colours).
        float down = clamp(y / max(inner.y, 1.0), 0.0, 1.0);
        vec4 rim = mix(line, vec4(line.a), rim_light * (1.0 - down) * (1.0 - down));
        rim = mix(rim, vec4(0.0, 0.0, 0.0, line.a), rim_shade * down * down);
        body = mix(body, rim, smoothstep(-border - 0.5, -border + 0.5, d));
    }

    // The same light as the surface behind, fainter: one light over the whole window.
    vec2 uv = (at + px) / max(screen, vec2(1.0));
    body = mix(body, vec4(body.a), panel_light * scene_light(uv, screen));
    body = mix(body, vec4(0.0, 0.0, 0.0, body.a), panel_shade * scene_dark(uv, screen));

    float inside = 1.0 - smoothstep(-0.5, 0.5, d);
    vec4 colour = body * inside;
    if (margin > 0.0) {
        // Two shadows, as an object lit from above casts: a wide soft one all round and a tighter one that falls
        // below. The higher the card (lift), the further the second falls.
        float fall = clamp(1.0 - max(d, 0.0) / margin, 0.0, 1.0);
        float k = box(centre - vec2(0.0, margin * drop * lift), inner / 2.0, radius);
        float key = clamp(1.0 - max(k, 0.0) / (margin * 0.75), 0.0, 1.0);
        float cast_shadow = drop > 0.0 ? 0.5 * fall * fall + lift * key * key * key : fall * fall;
        colour += shadow * cast_shadow * (1.0 - inside);
    }
    gl_FragColor = colour * alpha;
}
"#);

/// The surface behind the panels: a vertical gradient.
const BACK: &str = concat!(r#"
precision highp float;
varying vec2 v_coords;
uniform vec2 size;
uniform float alpha;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
uniform vec4 top;
uniform vec4 bottom;
uniform float light;
uniform float vignette;
"#, scene!(), r#"void main() {
    vec4 c = mix(top, bottom, v_coords.y);
    // Lit from a light above the top middle, darker towards the edges (a vignette). Distances in pixels over the
    // shorter side, so the light stays round whatever the window's shape.
    c = mix(c, vec4(c.a), light * scene_light(v_coords, size));
    c = mix(c, vec4(0.0, 0.0, 0.0, c.a), vignette * scene_dark(v_coords, size));
    gl_FragColor = c * alpha;
}
"#);

/// Glass: what lies under a floating panel, blurred, inside the panel's rounded box. `area` is the panel's place in the
/// texture (x, y, w, h in texture coordinates), `px` its size in pixels.
const GLASS: &str = r#"
//_DEFINES_
precision highp float;
uniform sampler2D tex;
uniform float alpha;
varying vec2 v_coords;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
uniform vec4 area;
uniform vec2 px;
uniform vec2 texel;
uniform float blur;
uniform float radius;

float box(vec2 p, vec2 half_size, float r) {
    vec2 d = abs(p) - half_size + vec2(r);
    return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0) - r;
}

void main() {
    // Two rings of eight around the middle, the outer at the blur's reach: soft enough at a glance, and cheap.
    vec4 sum = texture2D(tex, v_coords) * 0.2;
    for (int i = 0; i < 8; i++) {
        float a = float(i) * 0.785398 + 0.392699;
        vec2 d = vec2(cos(a), sin(a)) * texel * blur;
        sum += texture2D(tex, v_coords + d * 0.5) * 0.06;
        sum += texture2D(tex, v_coords + d) * 0.04;
    }
    vec2 p = (v_coords - area.xy) / area.zw * px - px / 2.0;
    float inside = 1.0 - smoothstep(-0.5, 0.5, box(p, px / 2.0, radius));
    gl_FragColor = sum * inside * alpha;
}
"#;

/// A lens: what lies under a docking target, magnified towards its middle like a drop of water. `area` is the
/// target's square in texture coordinates; `strength` how much the middle is enlarged.
const LENS: &str = r#"
//_DEFINES_
precision highp float;
uniform sampler2D tex;
uniform float alpha;
varying vec2 v_coords;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
uniform vec4 area;
uniform float strength;

void main() {
    vec2 centre = area.xy + area.zw / 2.0;
    vec2 p = (v_coords - centre) / (area.zw / 2.0);
    float r = length(p);
    float inside = 1.0 - smoothstep(0.94, 1.0, r);
    vec2 uv = centre + (v_coords - centre) * (1.0 - strength * (1.0 - r * r));
    gl_FragColor = texture2D(tex, uv) * inside * alpha;
}
"#;

/// A disc: a solid dot with a smooth edge, or a soft target that fades out from its middle with a faint rim.
const DISC: &str = r#"
precision highp float;
varying vec2 v_coords;
uniform vec2 size;
uniform float alpha;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
uniform vec4 colour;
uniform float hard;
void main() {
    float d = length(v_coords - vec2(0.5)) * 2.0;
    float edge = 1.0 - smoothstep(1.0 - 2.0 / size.x, 1.0, d);
    float soft = (1.0 - smoothstep(0.0, 1.0, d)) * 0.6 + (smoothstep(0.7, 0.82, d) - smoothstep(0.88, 1.0, d)) * 0.4;
    gl_FragColor = colour * (hard > 0.5 ? edge : soft) * alpha;
}
"#;

pub struct Shaders { card: GlesPixelProgram, back: GlesPixelProgram, disc: GlesPixelProgram, glass: GlesTexProgram, lens: GlesTexProgram, glass_slots: Vec<Under>, lens_under: Under }

/// What lies under a glass or lens, drawn off-screen and kept between frames, with the element that shows it. The
/// element keeps its id and only counts as changed when what lies under it changed, so a still window is not redrawn.
#[derive(Default)]
struct Under { texture: Option<GlesTexture>, tracker: Option<OutputDamageTracker>, id: Option<Id>, commit: CommitCounter, look: String }

/// A texture shader element under an id and change count of our own: smithay's makes a new id each time.
#[derive(Debug)]
pub struct Steady { inner: TextureShaderElement, id: Id, commit: CommitCounter }

impl smithay::backend::renderer::element::Element for Steady {
    fn id(&self) -> &Id { &self.id }
    fn current_commit(&self) -> CommitCounter { self.commit }
    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> { self.inner.geometry(scale) }
    fn transform(&self) -> Transform { self.inner.transform() }
    fn src(&self) -> Rectangle<f64, Buffer> { self.inner.src() }
    fn alpha(&self) -> f32 { self.inner.alpha() }
    fn kind(&self) -> Kind { self.inner.kind() }
}

impl smithay::backend::renderer::element::RenderElement<GlesRenderer> for Steady {
    fn draw(&self, frame: &mut smithay::backend::renderer::gles::GlesFrame<'_, '_>, src: Rectangle<f64, Buffer>, dst: Rectangle<i32, Physical>, damage: &[Rectangle<i32, Physical>], opaque: &[Rectangle<i32, Physical>], cache: Option<&smithay::utils::user_data::UserDataMap>) -> Result<(), smithay::backend::renderer::gles::GlesError> {
        self.inner.draw(frame, src, dst, damage, opaque, cache)
    }
}

fn compile(renderer: &mut GlesRenderer) -> Result<Shaders, String> {
    let n = |s: &'static str, t| UniformName::new(s, t);
    let card = renderer.compile_custom_pixel_shader(CARD, &[
        n("margin", UniformType::_1f), n("radius", UniformType::_1f), n("border", UniformType::_1f), n("bar", UniformType::_1f),
        n("fill", UniformType::_4f), n("bar_top", UniformType::_4f), n("bar_bottom", UniformType::_4f), n("line", UniformType::_4f), n("shadow", UniformType::_4f),
        n("lift", UniformType::_1f), n("rim_light", UniformType::_1f), n("rim_shade", UniformType::_1f),
        n("at", UniformType::_2f), n("screen", UniformType::_2f), n("drop", UniformType::_1f), n("gloss", UniformType::_1f),
        n("shine_at", UniformType::_1f), n("foot", UniformType::_1f), n("panel_light", UniformType::_1f), n("panel_shade", UniformType::_1f),
    ]).map_err(|e| format!("{e:?}"))?;
    let back = renderer.compile_custom_pixel_shader(BACK, &[n("top", UniformType::_4f), n("bottom", UniformType::_4f), n("light", UniformType::_1f), n("vignette", UniformType::_1f)]).map_err(|e| format!("{e:?}"))?;
    let disc = renderer.compile_custom_pixel_shader(DISC, &[n("colour", UniformType::_4f), n("hard", UniformType::_1f)]).map_err(|e| format!("{e:?}"))?;
    let glass = renderer.compile_custom_texture_shader(GLASS, &[
        n("area", UniformType::_4f), n("px", UniformType::_2f), n("texel", UniformType::_2f), n("blur", UniformType::_1f), n("radius", UniformType::_1f),
    ]).map_err(|e| format!("{e:?}"))?;
    let lens = renderer.compile_custom_texture_shader(LENS, &[n("area", UniformType::_4f), n("strength", UniformType::_1f)]).map_err(|e| format!("{e:?}"))?;
    Ok(Shaders { card, back, disc, glass, lens, glass_slots: Vec::new(), lens_under: Under::default() })
}

fn four(name: &'static str, c: [f32; 4]) -> Uniform<'static> { Uniform::new(name, (c[0], c[1], c[2], c[3])) }

/// The theme's light and depth, each a share from 0 to 1, as the card shader takes them.
#[derive(Debug, Default, Clone, Copy)]
struct Light { drop: f32, rim_light: f32, rim_shade: f32, gloss: f32, shine: f32, foot: f32, panel_light: f32, panel_shade: f32 }

impl Light {
    fn of(t: &wb_core::theme::Theme) -> Self {
        let f = |x: f64| x as f32;
        Light { drop: f(t.shadow_drop), rim_light: f(t.border_light), rim_shade: f(t.border_shade), gloss: f(t.bar_gloss),
            shine: f(t.bar_shine), foot: f(t.bar_shade), panel_light: f(t.panel_light), panel_shade: f(t.panel_shade) }
    }
}

/// What a card looks like, in the colours as drawing wants them.
struct Card { radius: f64, border: f64, bar: f64, margin: f64, fill: [f32; 4], top: [f32; 4], bottom: [f32; 4], line: [f32; 4], shadow: [f32; 4], lift: f32 }

impl Card {
    fn plain(radius: f64, fill: [f32; 4]) -> Self {
        Card { radius, border: 0.0, bar: 0.0, margin: 0.0, fill, top: fill, bottom: fill, line: fill, shadow: [0.0; 4], lift: 0.0 }
    }
}

/// Where the pointer is in a panel's title bar.
pub enum BarHit { Close, CloseTab(usize), Tab(usize), Undo, Back(Back), Bar }

/// A way back on a main window's bar: the browser's (the conversations' panel), or a panel's that is out (the input
/// panel), by its place in the list.
#[derive(Clone, Copy)]
pub enum Back { Browser, Panel(usize) }

/// Each tab's area in the bar, as many as fit: left of the close button.
/// The share of a title bar its tabs may take.
const TABS_SHARE: f64 = 0.7;

/// How wide a tab may be so that all fit in `room`: the short ones stay whole, the long ones share what is left.
fn fair_width(wants: &[f64], room: f64) -> f64 {
    let mut sorted = wants.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mut left = room;
    for (i, w) in sorted.iter().enumerate() {
        let share = left / (sorted.len() - i) as f64;
        if *w > share { return share; }
        left -= w;
    }
    f64::INFINITY
}

/// The name as far as it fits in `w`, with an ellipsis.
fn fitting(state: &mut App, name: &str, w: f64) -> String {
    let all = state.chrome.width(name);
    if all <= w { return name.to_string(); }
    let chars: Vec<char> = name.chars().collect();
    // From a little above its share of the letters: a name is measured a few times a frame, not once a letter.
    let from = ((chars.len() as f64 * w / all) as usize + 3).min(chars.len());
    for n in (1..from).rev() {
        let cut = format!("{}…", chars[..n].iter().collect::<String>().trim_end());
        if state.chrome.width(&cut) <= w { return cut; }
    }
    "…".into()
}

fn tab_rects(state: &mut App, p: &Panel) -> Vec<Rect> {
    // With tabs each has its own close button, so the bar has none: nothing closes a whole group by accident.
    let mut x = p.rect.x + 6.0;
    // Part of the bar stays free of tabs: that is where the panel is held.
    let limit = (marks_left(state, p) - 6.0).min(p.rect.x + p.rect.w * TABS_SHARE);
    let wants: Vec<f64> = p.tabs.iter().map(|t| state.chrome.width(&t.name) + 22.0 + TAB_CLOSE).collect();
    let most = fair_width(&wants, limit - x - 4.0 * (wants.len() - 1) as f64);
    let mut out = Vec::new();
    for w in wants {
        let w = w.min(most).min(limit - x);
        if w < 24.0 { break; }
        out.push(Rect::new(x, p.rect.y + 3.0, w, title() - 6.0));
        x += w + 4.0;
    }
    out
}

/// Whether the pointer is on the docking dot of this panel's title bar: in the middle of a bar with one tab (with
/// several, tabs are there), within reach of the dot. A dialog has none: it never joins the panels.
pub fn on_dot(t: &Theme, p: &Panel, x: f64, y: f64) -> bool {
    let reach = (title() * 0.8).max(t.dock_dot_size);
    let (dx, dy) = crate::state::dot_at(p);
    !p.transient && p.tabs.len() == 1 && p.rect.bar().contains(x, y) && (x - dx).hypot(y - dy) <= reach / 2.0 + 4.0
}

/// Where the marks at the end of this panel's bar are: the ↩ of its last move, and on the conversations' panel the way
/// back for the browser while it is out; left of the close button, or at the end of a bar of tabs.
fn undo_rect(state: &App, p: &Panel) -> Option<Rect> {
    state.undo_of(p.pid)?;
    Some(Rect::new(marks_right(p) - CLOSE, p.rect.y, CLOSE, title()))
}

/// The ways back this bar shows, right to left from its ↩: "↩ Browser" on the conversations' panel, "↩ <name>" for each
/// panel out, on the input panel.
fn back_rects(state: &mut App, p: &Panel) -> Vec<(Back, String, Rect)> {
    let mut want: Vec<(Back, String)> = Vec::new();
    if state.browser_back_panel() == Some(p.pid) { want.push((Back::Browser, wb_core::i18n::t("browser-back").to_string())); }
    if p.tabs.iter().any(|t| t.key.starts_with("dev.whaybench.input")) {
        want.extend(state.outside.iter().enumerate().map(|(i, o)| (Back::Panel(i), format!("↩ {}", o.name))));
    }
    let mut right = marks_right(p) - if state.undo_of(p.pid).is_some() { CLOSE } else { 0.0 };
    // Only what fits on the bar, past room for its name: one drawn outside it could not be clicked (or would click
    // the panel beside). What does not fit is a bare ↩ each, and if not even that, the rest wait for room.
    let floor = p.rect.x + 40.0;
    let mut out = Vec::new();
    for (b, label) in want {
        let w = state.chrome.width(&label) + 12.0;
        let (label, w) = if right - w >= floor { (label, w) } else { ("↩".to_string(), CLOSE) };
        if right - w < floor { break; }
        right -= w;
        out.push((b, label, Rect::new(right, p.rect.y, w, title())));
    }
    out
}

fn marks_right(p: &Panel) -> f64 { if p.tabs.len() == 1 { p.rect.right() - CLOSE } else { p.rect.right() } }

/// Where the bar's own content ends: before the marks.
fn marks_left(state: &mut App, p: &Panel) -> f64 {
    back_rects(state, p).last().map(|b| b.2).or_else(|| undo_rect(state, p)).map_or(marks_right(p), |r| r.x)
}

pub fn bar_hit(state: &mut App, pid: u64, x: f64, y: f64) -> Option<BarHit> {
    let p = state.surface.get(pid)?.clone();
    if !p.rect.bar().contains(x, y) { return None; }
    if undo_rect(state, &p).is_some_and(|r| r.contains(x, y)) { return Some(BarHit::Undo); }
    if let Some((b, _, _)) = back_rects(state, &p).into_iter().find(|b| b.2.contains(x, y)) { return Some(BarHit::Back(b)); }
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
struct Frame<'a> { out: Vec<PanelElement>, ids: &'a mut Vec<(Id, [f32; 4])>, n: usize, commit: CommitCounter, kept: &'a mut Vec<(String, PixelShaderElement)>, k: usize, screen: (f64, f64), light: Light, alpha: f32 }

impl Frame<'_> {
    fn solid(&mut self, r: Rect, colour: [f32; 4]) {
        // A new colour is a new element: the same one in another colour would not count as changed.
        let colour = colour.map(|c| c * self.alpha);
        if self.n == self.ids.len() { self.ids.push((Id::new(), colour)); }
        else if self.ids[self.n].1 != colour { self.ids[self.n] = (Id::new(), colour); }
        let id = self.ids[self.n].0.clone();
        self.n += 1;
        self.out.push(SolidColorRenderElement::new(id, phys(r), self.commit, colour, Kind::Unspecified).into());
    }
    /// A shaded box: the one of last frame at this place in the order if it looks the same, else a new one.
    fn shaded(&mut self, program: &GlesPixelProgram, area: Rectangle<i32, Logical>, uniforms: Vec<Uniform<'static>>, look: String) {
        let (alpha, look) = (self.alpha, format!("{look} {}", self.alpha));
        let make = || PixelShaderElement::new(program.clone(), area, None, alpha, uniforms, Kind::Unspecified);
        if self.k == self.kept.len() { self.kept.push((look, make())); }
        else if self.kept[self.k].0 != look { self.kept[self.k] = (look, make()); }
        self.out.push(self.kept[self.k].1.clone().into());
        self.k += 1;
    }
    /// A disc of diameter `d` around (cx, cy), in a colour as drawing wants it. Without shaders, a square.
    fn disc(&mut self, shaders: Option<&Shaders>, cx: f64, cy: f64, d: f64, colour: [f32; 4], hard: bool) {
        let r = Rect::new(cx - d / 2.0, cy - d / 2.0, d, d);
        let Some(sh) = shaders else { return self.solid(r, colour) };
        let p = phys(r);
        let area = Rectangle::new((p.loc.x, p.loc.y).into(), (p.size.w, p.size.h).into());
        self.shaded(&sh.disc, area, vec![four("colour", colour), Uniform::new("hard", if hard { 1.0f32 } else { 0.0 })], format!("disc {p:?} {colour:?} {hard}"));
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
        let look = format!("card {logical:?} {m} {radius} {} {} {:?} {:?} {:?} {:?} {:?} {} {:?} {:?}", c.border, c.bar, c.fill, c.top, c.bottom, c.line, c.shadow, c.lift, self.screen, self.light);
        let l = self.light;
        self.shaded(&sh.card, logical, vec![
            Uniform::new("margin", m as f32), Uniform::new("radius", radius as f32),
            Uniform::new("border", c.border as f32), Uniform::new("bar", c.bar as f32),
            four("fill", c.fill), four("bar_top", c.top), four("bar_bottom", c.bottom), four("line", c.line), four("shadow", c.shadow),
            Uniform::new("lift", c.lift), Uniform::new("rim_light", l.rim_light), Uniform::new("rim_shade", l.rim_shade),
            Uniform::new("drop", l.drop), Uniform::new("gloss", l.gloss), Uniform::new("shine_at", l.shine), Uniform::new("foot", l.foot),
            Uniform::new("panel_light", l.panel_light), Uniform::new("panel_shade", l.panel_shade),
            Uniform::new("at", (r.x as f32 - m as f32, r.y as f32 - m as f32)), Uniform::new("screen", (self.screen.0 as f32, self.screen.1 as f32)),
        ], look);
    }
    fn line(&mut self, l: Line, width: f64, colour: [f32; 4]) {
        let half = width / 2.0;
        self.solid(Rect::new(l.x0 - half, l.y0 - half, (l.x1 - l.x0).abs() + width, (l.y1 - l.y0).abs() + width), colour);
    }
}

fn add_text(state: &mut App, renderer: &mut GlesRenderer, f: &mut Vec<PanelElement>, alpha: f32, text: &str, colour: [u8; 3], area: Rect) {
    let Some(t) = state.chrome.text(text, colour) else { return };
    let x = area.x + ((area.w - t.w as f64) / 2.0).max(0.0);
    let y = area.y + (area.h - t.h as f64) / 2.0;
    let src = (t.w as f64 > area.w).then(|| Rectangle::new((0.0, 0.0).into(), (area.w, t.h as f64).into()));
    let size = src.map(|_| (area.w as i32, t.h).into());
    if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(renderer, (x.round(), y.round()), &t.buffer, Some(alpha), src, size, Kind::Unspecified) {
        f.push(e.into());
    }
}

/// The panel's front app, cut to its area: nothing an app draws reaches outside its panel.
fn app_elements(state: &App, p: &Panel, renderer: &mut GlesRenderer, area: Rect, alpha: f32) -> Vec<PanelElement> {
    let Some(w) = state.windows.get(&p.tab().id) else { return Vec::new() };
    let g = w.geometry().loc;
    let at = (area.x.round() as i32 - g.x, area.y.round() as i32 - g.y);
    let elements = AsRenderElements::<GlesRenderer>::render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(w, renderer, at.into(), Scale::from(1.0), alpha);
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
    // A see-through panel: its frame and fill; its text and app stay solid. The panels behind the front one can be more so.
    f.alpha = (t.panel_opacity * if front { 1.0 } else { t.back_opacity }) as f32;
    if carried.contains(&p.pid) { f.card(shaders, p.rect, Card::plain(radius, CARRIED)); }
    if p.tabs.len() == 1 {
        add_text(state, renderer, &mut f.out, 1.0, "×", t.ink, Rect::new(p.rect.right() - CLOSE, p.rect.y, CLOSE, title()));
    }
    let ink = [(t.accent[0] * 255.0) as u8, (t.accent[1] * 255.0) as u8, (t.accent[2] * 255.0) as u8];
    if let Some(r) = undo_rect(state, p) { add_text(state, renderer, &mut f.out, 1.0, "↩", ink, r); }
    for (_, label, r) in back_rects(state, p) { add_text(state, renderer, &mut f.out, 1.0, &label, ink, r); }
    if p.tabs.len() > 1 {
        let rects = tab_rects(state, p);
        for (i, r) in rects.iter().enumerate() {
            let ink = if i == p.active { t.ink } else { t.ink_soft };
            add_text(state, renderer, &mut f.out, 1.0, "×", ink, Rect::new(r.right() - TAB_CLOSE, r.y, TAB_CLOSE, r.h));
            let room = (r.w - TAB_CLOSE).max(1.0);
            let name = fitting(state, &p.tabs[i].name.clone(), room - 10.0);
            add_text(state, renderer, &mut f.out, 1.0, &name, ink, Rect::new(r.x, r.y, room, r.h));
        }
        if let Some(r) = rects.get(p.active) { f.card(shaders, *r, Card::plain(radius.min(7.0), premultiplied(t.tab_front))); }
    } else {
        // The name at the bar's left; its width only as far as the close button.
        let wide = state.chrome.width(&p.tab().name) + 2.0;
        let left = 8.0 + (radius / 2.0).max(0.0);
        let end = marks_left(state, p);
        let area = Rect::new(p.rect.x + left, p.rect.y, wide.min((end - p.rect.x - left - 6.0).max(1.0)), title());
        add_text(state, renderer, &mut f.out, 1.0, &p.tab().name.clone(), if front { t.ink } else { t.ink_soft }, area);
    }
    f.out.extend(app_elements(state, p, renderer, app, 1.0));
    // The card fills the panel too; see-through, a second fill under the app would make it more opaque than asked.
    if f.alpha >= 1.0 { f.solid(app, premultiplied(t.panel)); }
    f.card(shaders, p.rect, Card {
        radius, border: t.border_width, bar: title(), margin: if own_shadow { t.shadow_size } else { 0.0 },
        fill: premultiplied(t.panel),
        top: premultiplied(if front { t.front_top } else { t.bar_top }),
        bottom: premultiplied(if front { t.front_bottom } else { t.bar_bottom }),
        line: premultiplied(if front { t.front_border } else { t.border }),
        shadow: if own_shadow { premultiplied(t.shadow) } else { [0.0; 4] },
        // The front panel stands higher than the others.
        lift: if front { 1.0 } else { t.back_lift as f32 },
    });
    f.alpha = 1.0;
}

fn surface_elements(state: &mut App, renderer: &mut GlesRenderer) -> Vec<PanelElement> {
    let mut ids = std::mem::take(&mut state.solid_ids);
    let mut kept = std::mem::take(&mut state.shaded);
    let mut shaders = state.shaders.take();
    let status_elements = state.status.clone().map(|w| {
        let g = w.geometry().loc;
        let area = Rect::new(0.0, state.surface.h, state.surface.w, crate::state::STATUS_H as f64);
        let at = (-g.x, area.y.round() as i32 - g.y);
        let e = AsRenderElements::<GlesRenderer>::render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(&w, renderer, at.into(), Scale::from(1.0), 1.0);
        (e, area)
    });
    let t = state.theme.clone();
    let mut f = Frame { out: Vec::new(), ids: &mut ids, n: 0, commit: CommitCounter::default(), kept: &mut kept, k: 0, screen: (state.surface.w, state.surface.h), light: Light::of(&state.theme), alpha: 1.0 };
    let accent = premultiplied(t.accent);
    let radius = t.radius;
    // The window's own frame: its buttons and the empty title strip right of the menus, over the menu bar's row.
    if state.menu.is_some() {
        let (w, top) = (state.surface.w, -(crate::state::MENU_H as f64));
        let h = crate::state::MENU_H as f64;
        let (px, py) = state.pointer;
        let under = state.control_at(px, py);
        for (c, n, glyph) in [(crate::state::Control::Close, 1.0, "✕"), (crate::state::Control::Maximize, 2.0, if state.maximized { "❐" } else { "☐" }), (crate::state::Control::Minimize, 3.0, "—")] {
            let r = Rect::new(w - n * crate::state::CONTROL_W, top, crate::state::CONTROL_W, h);
            let lit = under == Some(c);
            // Close goes red under the pointer, as on Windows; the others take a touch of the accent.
            let ink = if lit && c == crate::state::Control::Close { [255, 255, 255] } else { t.ink };
            add_text(state, renderer, &mut f.out, 1.0, glyph, ink, r);
            if lit {
                let bg = if c == crate::state::Control::Close { [0.77, 0.17, 0.11, 1.0] } else { [t.accent[0], t.accent[1], t.accent[2], 0.15] };
                f.solid(r, premultiplied(bg));
            }
        }
        let from = state.menu_width() as f64;
        // The menu bar's own colour (its style sheet paints it in bar_bottom), so the strip reads as one bar.
        f.solid(Rect::new(from, top, (w - from).max(0.0), h), premultiplied(t.bar_bottom));
    }
    if let Some((elements, area)) = status_elements {
        let crop = phys(area);
        f.out.extend(elements.into_iter().filter_map(|e| CropRenderElement::from_element(e, 1.0, crop)).map(PanelElement::from));
        f.solid(area, premultiplied(t.panel));
    }

    // A settings window is above everything else, the joint lines included.
    let modal: Vec<Panel> = state.surface.panels.iter().rev().filter(|p| p.modal).cloned().collect();
    let top_modal = modal.first().map(|p| p.pid);
    // Floating panels with glass: where each one's elements end, so what lies under it can go right there.
    let mut glass: Vec<(usize, Rect)> = Vec::new();
    for p in &modal { draw_panel(state, renderer, &mut f, shaders.as_ref(), p, Some(p.pid) == top_modal, &[], true); glass.push((f.out.len(), p.rect)); }

    // Above everything: the docking dot, the targets, the drop preview and the guides.
    let mut lenses: Vec<Rect> = Vec::new();
    let (dot, ring) = (premultiplied(t.dock_dot), t.dock_ring);
    let (px, py) = state.pointer;
    match state.act {
        Act::Hold { pid, dock: true, .. } if let Some(held) = state.surface.get(pid).filter(|p| !p.transient).cloned() => {
            let (dx, dy) = crate::state::dot_at(&held);
            f.disc(shaders.as_ref(), dx, dy, t.dock_dot_size, dot, true);
            let zone = state.zone;
            let lit = |target: u64, how: How| zone.is_some_and(|z| z.target == target && z.how == how);
            let panels: Vec<Panel> = state.surface.panels.iter().filter(|p| p.pid != pid && !p.transient).cloned().collect();
            // It can go out: a soft band along each edge of the window, the one the dot is in stronger.
            if crate::state::can_go_out(&held) {
                let (sw, sh, b) = (state.surface.w, state.surface.h, t.dock_ring_size);
                let at = crate::state::out_edge(sw, sh, b, dx, dy).filter(|_| state.out_zone);
                for (side, r) in [(Side::Top, Rect::new(0.0, 0.0, sw, b)), (Side::Bottom, Rect::new(0.0, sh - b, sw, b)), (Side::Left, Rect::new(0.0, b, b, sh - 2.0 * b)), (Side::Right, Rect::new(sw - b, b, b, sh - 2.0 * b))] {
                    let a = if at == Some(side) { (ring[3] * 1.4).min(1.0) } else { ring[3] * 0.4 };
                    if at == Some(side) {
                        let ink = [(t.accent[0] * 255.0) as u8, (t.accent[1] * 255.0) as u8, (t.accent[2] * 255.0) as u8];
                        add_text(state, renderer, &mut f.out, f.alpha, wb_core::i18n::t("browser-detach-here"), ink, r);
                    }
                    f.solid(r, premultiplied([ring[0], ring[1], ring[2], a]));
                }
            }
            for p in &panels {
                for (how, cx, cy) in targets(p.rect, t.dock_ring_size, state.surface.m.gap) {
                    // The one the dot is in stands out; the others stay soft over what is behind them.
                    let c = if lit(p.pid, how) { [ring[0], ring[1], ring[2], (ring[3] * 1.8).min(1.0)] } else { ring };
                    f.disc(shaders.as_ref(), cx, cy, t.dock_ring_size, premultiplied(c), false);
                    lenses.push(Rect::new(cx - t.dock_ring_size / 2.0, cy - t.dock_ring_size / 2.0, t.dock_ring_size, t.dock_ring_size));
                }
            }
        }
        Act::Idle => if let Some(p) = state.surface.top_at(px, py).cloned() && on_dot(&t, &p, px, py) {
            let (dx, dy) = crate::state::dot_at(&p);
            f.disc(shaders.as_ref(), dx, dy, t.dock_dot_size, dot, true);
        },
        _ => {}
    }
    if let Some(z) = state.zone {
        let name = state.surface.get(z.target).map(|p| p.tab().name.clone()).unwrap_or_default();
        let label = if z.how == How::Tab { wb_core::i18n::f("as-a-tab-of", &[("name", (name).to_string())]) } else { wb_core::i18n::f("beside", &[("name", (name).to_string())]) };
        let ink = [(t.accent[0] * 255.0) as u8, (t.accent[1] * 255.0) as u8, (t.accent[2] * 255.0) as u8];
        add_text(state, renderer, &mut f.out, f.alpha, &label, ink, z.preview);
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
    if let Some((pid, edges, _)) = &active
        && let Some(r) = state.surface.get(*pid).map(|p| p.rect) {
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
    let lens_at = f.out.len();
    // Panels that hold on to nothing lie over the joint lines; the others under them.
    let carried: Vec<u64> = active.as_ref().map(|(_, _, ps)| ps.iter().map(|(p, _)| *p).collect()).unwrap_or_default();
    let top = state.surface.front();
    let free = state.surface.free();
    let panels: Vec<Panel> = state.surface.panels.iter().rev().filter(|p| !p.modal).cloned().collect();
    for p in panels.iter().filter(|p| free.contains(&p.pid)) {
        draw_panel(state, renderer, &mut f, shaders.as_ref(), p, Some(p.pid) == top, &carried, false);
        glass.push((f.out.len(), p.rect));
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
        for p in &panels {
            f.card(shaders.as_ref(), p.rect, Card { radius, border: 0.0, bar: 0.0, margin: t.shadow_size, fill: none, top: none, bottom: none, line: none, shadow: premultiplied(t.shadow), lift: if Some(p.pid) == top { 1.0 } else { t.back_lift as f32 } });
        }
    }
    // The background apps (wlr-layer-shell): the surface behind the panels.
    f.out.extend(background_apps(state, renderer, (sw, sh)));
    // The surface itself, under everything.
    if let Some(sh2) = &shaders
        && (t.surface_top != t.surface_bottom || t.surface_light > 0.0 || t.surface_vignette > 0.0 || t.window_opacity < 1.0) {
            let area: Rectangle<i32, Logical> = Rectangle::new((0, 0).into(), (sw.round() as i32, sh.round() as i32).into());
            let (top, bottom) = (premultiplied(t.surface_top), premultiplied(t.surface_bottom));
            let (light, vignette) = (t.surface_light as f32, t.surface_vignette as f32);
            f.alpha = t.window_opacity as f32;
            f.shaded(&sh2.back, area, vec![four("top", top), four("bottom", bottom), Uniform::new("light", light), Uniform::new("vignette", vignette)],
                format!("back {area:?} {top:?} {bottom:?} {light} {vignette}"));
            f.alpha = 1.0;
        }
    let used = f.k;
    let mut out = f.out;
    let screen = (state.surface.w, state.surface.h);
    let mut lens_at = lens_at;
    if t.glass_blur > 0.0 && !glass.is_empty() && let Some(sh) = shaders.as_mut() {
        // A glass put in above the lenses' place (a settings window's) moves that place down by one.
        lens_at += glass_under(renderer, sh, &mut out, &glass, screen, t.glass_blur, radius).iter().filter(|at| **at <= lens_at).count();
    }
    if t.dock_refraction > 0.0 && !lenses.is_empty() && let Some(sh) = shaders.as_mut() {
        lens_under(renderer, sh, &mut out, lens_at, &lenses, screen, t.dock_refraction);
    }
    kept.truncate(used);
    state.shaded = kept;
    state.solid_ids = ids;
    state.shaders = shaders;
    out
}

/// Puts glass under each floating panel: what lies under that panel (the elements after its own, lower floating panels
/// included), blurred. Returns where glass went in. A panel whose under cannot be drawn goes without.
#[allow(clippy::too_many_arguments)]
fn glass_under(renderer: &mut GlesRenderer, sh: &mut Shaders, out: &mut Vec<PanelElement>, glass: &[(usize, Rect)], screen: (f64, f64), blur: f64, radius: f64) -> Vec<usize> {
    let size = (screen.0.round().max(1.0) as i32, screen.1.round().max(1.0) as i32);
    sh.glass_slots.resize_with(glass.len(), Under::default);
    let mut put = Vec::new();
    // Lowest first: each upper panel's glass then shows the lower panels with theirs.
    for (i, (at, r)) in glass.iter().enumerate().rev() {
        let slot = &mut sh.glass_slots[i];
        let Some(texture) = draw_under(renderer, slot, &out[*at..], size) else { continue };
        let area = ((r.x / screen.0) as f32, (r.y / screen.1) as f32, (r.w / screen.0) as f32, (r.h / screen.1) as f32);
        let uniforms = vec![
            Uniform::new("area", area), Uniform::new("px", (r.w as f32, r.h as f32)), Uniform::new("texel", (1.0 / size.0 as f32, 1.0 / size.1 as f32)),
            Uniform::new("blur", blur as f32), Uniform::new("radius", radius.min(r.w / 2.0).min(r.h / 2.0) as f32),
        ];
        let look = format!("{r:?} {blur} {radius}");
        if slot.look != look { slot.look = look; slot.commit.increment(); }
        out.insert(*at, steady(renderer, slot, texture, *r, sh.glass.clone(), uniforms).into());
        put.push(*at);
    }
    put
}

/// The element showing a slot's texture over `r`, under the slot's id and change count.
fn steady(renderer: &GlesRenderer, slot: &mut Under, texture: GlesTexture, r: Rect, program: GlesTexProgram, uniforms: Vec<Uniform<'static>>) -> Steady {
    let src = Rectangle::new((r.x, r.y).into(), (r.w, r.h).into());
    let e = TextureRenderElement::from_static_texture(Id::new(), renderer.context_id(), (r.x.round(), r.y.round()), texture, 1, Transform::Normal, None, Some(src), Some((r.w.round() as i32, r.h.round() as i32).into()), None, Kind::Unspecified);
    Steady { inner: TextureShaderElement::new(e, program, uniforms), id: slot.id.get_or_insert_with(Id::new).clone(), commit: slot.commit }
}

/// Draws the elements into the slot's texture of the surface's size, kept for the next frame, and counts the slot as
/// changed when anything under it changed. None when it cannot.
fn draw_under(renderer: &mut GlesRenderer, slot: &mut Under, elements: &[PanelElement], (w, h): (i32, i32)) -> Option<GlesTexture> {
    let size: Size<i32, Buffer> = (w, h).into();
    let fresh = slot.texture.as_ref().is_none_or(|t| smithay::backend::renderer::Texture::size(t) != size);
    if fresh {
        slot.texture = renderer.create_buffer(Fourcc::Abgr8888, size).ok();
        slot.tracker = Some(OutputDamageTracker::new((w, h), 1.0, Transform::Normal));
    }
    let mut texture = slot.texture.clone()?;
    let tracker = slot.tracker.as_mut()?;
    let under: Vec<&PanelElement> = elements.iter().collect();
    // Nothing under it changed: the texture stays and is not bound at all. Binding it takes the window's surface off
    // the context, its next frame then has no age, and every frame would be drawn and sent whole.
    let changed = fresh || tracker.damage_output(1, &under).is_ok_and(|(d, _)| d.is_some_and(|d| !d.is_empty()));
    if !changed { return Some(texture); }
    let drawn = (|| -> Result<(), String> {
        let mut fb = renderer.bind(&mut texture).map_err(|e| e.to_string())?;
        // The tracker has taken in this frame already: the whole texture is drawn again.
        tracker.render_output(renderer, &mut fb, 0, &under, [0.0; 4]).map_err(|e| format!("{e:?}"))?;
        Ok(())
    })();
    match drawn {
        Ok(()) => { slot.commit.increment(); Some(texture) }
        Err(e) => { record::warn("screen", "under not drawn", &e, &[]); None }
    }
}

/// Puts a lens under each docking target: what lies under it, magnified towards the middle.
fn lens_under(renderer: &mut GlesRenderer, sh: &mut Shaders, out: &mut Vec<PanelElement>, at: usize, lenses: &[Rect], screen: (f64, f64), strength: f64) {
    let size = (screen.0.round().max(1.0) as i32, screen.1.round().max(1.0) as i32);
    let Some(texture) = draw_under(renderer, &mut sh.lens_under, &out[at..], size) else { return };
    for r in lenses {
        let uniforms = vec![
            Uniform::new("area", ((r.x / screen.0) as f32, (r.y / screen.1) as f32, (r.w / screen.0) as f32, (r.h / screen.1) as f32)),
            Uniform::new("strength", (strength * 0.5) as f32),
        ];
        // Each target its own id: they share the slot's texture but not its place.
        let mut slot = Under { id: Some(Id::new()), commit: sh.lens_under.commit, ..Under::default() };
        out.insert(at, steady(renderer, &mut slot, texture.clone(), *r, sh.lens.clone(), uniforms).into());
    }
}

/// What the background apps (wlr-layer-shell) show, at their place on the surface (the window less the menu bar),
/// the bottom layer over the background one.
fn background_apps(state: &App, renderer: &mut GlesRenderer, (sw, sh): (f64, f64)) -> Vec<PanelElement> {
    use smithay::wayland::shell::wlr_layer::Layer;
    let Some(output) = state.space.outputs().next() else { return Vec::new() };
    let map = smithay::desktop::layer_map_for_output(output);
    let down = state.menu_height();
    let crop: Rectangle<i32, Physical> = Rectangle::new((0, 0).into(), (sw.round() as i32, sh.round() as i32).into());
    let mut out = Vec::new();
    for layer in [Layer::Bottom, Layer::Background] {
        // The newest on top, as Smithay's Space draws them (the list is front to back).
        for l in map.layers_on(layer).rev() {
            let Some(g) = map.layer_geometry(l) else { continue };
            let at = (g.loc.x, g.loc.y - down);
            let e = AsRenderElements::<GlesRenderer>::render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(l, renderer, at.into(), Scale::from(1.0), 1.0);
            out.extend(e.into_iter().filter_map(|e| CropRenderElement::from_element(e, 1.0, crop)).map(PanelElement::from));
        }
    }
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

#[cfg(test)]
mod tests {
    #[test]
    fn tabs_share_the_room_they_have() {
        use super::fair_width;
        assert_eq!(fair_width(&[100.0, 120.0], 300.0), f64::INFINITY, "they fit");
        assert_eq!(fair_width(&[400.0, 300.0], 300.0), 150.0, "two long ones halve it");
        assert_eq!(fair_width(&[60.0, 400.0, 400.0], 300.0), 120.0, "a short one stays whole");
    }
}
