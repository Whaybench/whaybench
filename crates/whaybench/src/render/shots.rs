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

/// How long a picture taken while a Briefsafe project is open lives: it is a screen test's, then it goes (spec 3.22:
/// the screen is the person's and must stay testable, so pictures are not refused; nothing of a project stays).
pub const PROJECT_SHOT_KEEP: std::time::Duration = std::time::Duration::from_secs(10 * 60);

fn expiring_file(d: &wb_core::paths::Data) -> std::path::PathBuf { d.cache().join("shots-expiring.json") }

/// Whether a picture taken now may hold a project's content: a project is open, or a view shows a protected page.
fn may_hold_project(d: &wb_core::paths::Data) -> bool {
    !wb_core::roots::Roots::new(d.clone()).projects().is_empty()
        || wb_core::paths::list_dir(&d.cache()).iter().any(|f| f.file_name().is_some_and(|n| n.to_string_lossy().starts_with("view-protected-")))
}

/// The file as it was written: removed later only if it is still that file (not one written there since).
fn stamp(path: &std::path::Path) -> Option<(u64, i64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path).ok().filter(|m| m.is_file()).map(|m| (m.len(), m.mtime_nsec() + m.mtime() * 1_000_000_000))
}

/// Notes a picture to be removed after PROJECT_SHOT_KEEP.
fn note_expiring(d: &wb_core::paths::Data, out: &std::path::Path) {
    let Some((len, mtime)) = stamp(out) else { return };
    let mut list: Vec<serde_json::Value> = wb_core::paths::read_kept(&expiring_file(d));
    list.push(serde_json::json!({"path": out, "until": wb_core::clock::now_ms() + PROJECT_SHOT_KEEP.as_millis() as i64, "len": len, "mtime": mtime}));
    let _ = wb_core::paths::write_atomic(&expiring_file(d), serde_json::Value::Array(list).to_string());
    record::decide("screen", "picture to be removed", "taken while a Briefsafe project was open", &[("in", format!("{}s", PROJECT_SHOT_KEEP.as_secs()))]);
}

/// Removes the pictures whose time is up (also after a restart: the list is a file). One changed since is left.
pub fn sweep_expiring(d: &wb_core::paths::Data) {
    let file = expiring_file(d);
    let list: Vec<serde_json::Value> = wb_core::paths::read_kept(&file);
    if list.is_empty() { return; }
    let now = wb_core::clock::now_ms();
    let (due, keep): (Vec<_>, Vec<_>) = list.into_iter().partition(|e| e["until"].as_i64().unwrap_or(0) <= now);
    for e in &due {
        let path = std::path::Path::new(e["path"].as_str().unwrap_or(""));
        if stamp(path) == Some((e["len"].as_u64().unwrap_or(u64::MAX), e["mtime"].as_i64().unwrap_or(-1))) {
            match std::fs::remove_file(path) {
                Ok(()) => record::detail("screen", "picture removed", &[("after", format!("{}s", PROJECT_SHOT_KEEP.as_secs()))]),
                Err(err) => record::warn("screen", "picture not removed", &err.to_string(), &[]),
            }
        }
    }
    if !due.is_empty() { let _ = wb_core::paths::write_atomic(&file, serde_json::Value::Array(keep).to_string()); }
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
        // Taken while a project is open: removed after a while, whatever it shows.
        if done.is_ok() && let Some(r) = std::env::var_os(wb_core::paths::ENV_DATA_DIR) {
            let d = wb_core::paths::Data::new(std::path::PathBuf::from(r));
            if may_hold_project(&d) { note_expiring(&d, &shot.out); }
        }
        match done {
            Ok(()) => record::detail("screen", "panel shot", &[("panel", format!("{:?}", shot.panel)), ("took", format!("{}ms", began.elapsed().as_millis()))]),
            Err(e) => {
                record::warn("screen", "panel not shot", &e, &[("panel", format!("{:?}", shot.panel))]);
                let _ = std::fs::write(ctl::error_file(&shot.out), e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_picture_whose_time_is_up_goes_and_one_written_over_since_stays() {
        let root = std::env::temp_dir().join(format!("wb-shots-{}", std::process::id()));
        let d = wb_core::paths::Data::new(root.clone());
        std::fs::create_dir_all(d.cache()).unwrap();
        let (gone, changed, later) = (root.join("a.png"), root.join("b.png"), root.join("c.png"));
        for p in [&gone, &changed, &later] { std::fs::write(p, "x").unwrap(); }
        let entry = |p: &std::path::Path, until: i64| { let (len, mtime) = stamp(p).unwrap(); serde_json::json!({"path": p, "until": until, "len": len, "mtime": mtime}) };
        let list = serde_json::json!([entry(&gone, 0), entry(&changed, 0), entry(&later, i64::MAX)]);
        std::fs::write(&changed, "written over since").unwrap();
        wb_core::paths::write_atomic(&expiring_file(&d), list.to_string()).unwrap();
        sweep_expiring(&d);
        assert!(!gone.exists(), "its time is up");
        assert!(changed.exists(), "not the picture that was taken");
        assert!(later.exists(), "its time is not up");
        let left: Vec<serde_json::Value> = wb_core::paths::read_kept(&expiring_file(&d));
        assert_eq!(left.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }
}
