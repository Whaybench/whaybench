//! Screenshots asked for with `whaybench ctl shot`: the window or one panel drawn off-screen and saved as PNG.
use super::*;

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
        let rgb: Vec<u8> = rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
        writer.write_image_data(&rgb).map_err(|x| x.to_string())?;
    }
    ctl::write_atomic(out, &bytes).map_err(|x| x.to_string())
}

pub(super) fn take_shots(state: &mut App, renderer: &mut GlesRenderer, surface: Size<i32, Physical>) {
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
            // The view showing a project's page or a recovery key: no picture of it (spec 3.22).
            (Some(p), _) if p.tabs.get(p.active).is_some_and(|t| t.key.starts_with(&format!("{}#", wb_core::view::VIEW_APP)))
                && std::env::var_os(wb_core::paths::ENV_DATA_DIR).is_some_and(|r| { let d = wb_core::paths::Data::new(std::path::PathBuf::from(r));
                    wb_core::paths::list_dir(&d.cache()).iter().any(|f| f.file_name().is_some_and(|n| n.to_string_lossy().starts_with("view-protected-"))) }) =>
                Err(wb_core::i18n::t("briefsafe-view-not-read").to_string()),
            (Some(p), _) => {
                let a = state.surface.app_of(p.rect);
                let (w, h) = (a.w.round() as i32, a.h.round() as i32);
                let mut e = app_elements(state, &p, renderer, Rect::new(0.0, 0.0, w as f64, h as f64), 1.0);
                let (mut ids, mut kept) = (Vec::new(), Vec::new());
                let mut f = Frame { out: Vec::new(), ids: &mut ids, n: 0, commit: CommitCounter::default(), kept: &mut kept, k: 0, screen: (w as f64, h as f64), light: Light::of(&state.theme), alpha: 1.0 };
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
