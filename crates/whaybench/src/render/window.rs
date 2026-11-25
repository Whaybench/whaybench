//! The window Whaybench draws in: the GPU offered to the apps, the event loop that draws each frame, and the cursor.
use super::*;
use super::shots::take_shots;

/// With nothing happening and nothing moving, how often the picture is still looked at.
const LOOK_EVERY: Duration = Duration::from_millis(100);
/// Looks at the picture asked for after a change come no closer than this.
const FRAME: Duration = Duration::from_millis(16);

/// Offers the window's GPU to the apps (linux-dmabuf with default feedback, as anvil's winit backend does), so they
/// draw on it and hand over their buffers without a copy. Without a render node they keep shared memory.
fn offer_gpu(state: &mut App, backend: &Rc<RefCell<winit::WinitGraphicsBackend<GlesRenderer>>>) {
    let mut b = backend.borrow_mut();
    let node = EGLDevice::device_for_display(b.renderer().egl_context().display()).map_err(|e| e.to_string())
        .and_then(|d| d.try_get_render_node().map_err(|e| e.to_string()))
        .and_then(|n| n.ok_or_else(|| "no render node".to_string()));
    let formats = b.renderer().dmabuf_formats();
    let feedback = node.and_then(|n| DmabufFeedbackBuilder::new(n.dev_id(), formats).build().map(|f| (n, f)).map_err(|e| e.to_string()));
    match feedback {
        Ok((node, feedback)) => {
            let mut dmabuf = DmabufState::new();
            dmabuf.create_global_with_default_feedback::<App>(&state.display_handle, &feedback);
            state.dmabuf = Some(dmabuf);
            state.gpu = Some(backend.clone());
            crate::start::GPU_APPS.store(true, std::sync::atomic::Ordering::Relaxed);
            record::decide("screen", "apps on the GPU", "the window's GPU has a render node", &[("node", format!("{node}"))]);
        }
        Err(e) => record::decide("screen", "apps on the CPU", "no GPU device to offer the apps", &[("why", e)]),
    }
}

/// `own_frame`: the menu bar runs and serves as the title strip, so the desktop's frame is left out.
/// What it returns is to run after every turn of the event loop: it asks for a frame when something happened.
pub fn init_window(event_loop: &mut EventLoop<'static, App>, state: &mut App, own_frame: bool) -> Result<impl FnMut(&mut App) + 'static, Box<dyn std::error::Error>> {
    let (w, h) = state.saved_window().map(|(w, h)| (w as f64, h as f64)).unwrap_or(FIRST_WINDOW);
    // No frame from the desktop: the menu bar is the title strip, with the window's buttons drawn at its right.
    let attributes = WindowAttributes::default().with_decorations(!own_frame).with_transparent(true).with_title("Whaybench").with_surface_size(LogicalSize::new(w, h)).with_visible(true);
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
    let mut frame_failing = false;
    let (mut times, mut since, mut sent_frames) = (Vec::with_capacity(400), Instant::now(), 0usize);
    #[allow(deprecated)] // The newer call needs a capability set.
    backend.window().set_ime_allowed(!state.builtin_ime);

    // Only what changed goes to the desktop. A frame with nothing new is not sent at all: sending the whole window
    // every frame makes the desktop redraw it and its title bar 60 times a second, which shows as a flicker there.
    // So a frame is asked for, as a game or a compositor does, only when something happened (an app's request, input, a
    // command) or something moves by itself; else a few times a second, for what is looked at by the clock (a theme
    // file changed, the agent's state). Building the picture 60 times a second with nothing new cost several percent
    // of a core.
    let backend = Rc::new(RefCell::new(backend));
    offer_gpu(state, &backend);
    let idle = Rc::new(Cell::new(false));
    let looked = Rc::new(Cell::new(Instant::now()));
    let look = {
        let (backend, idle, looked) = (backend.clone(), idle.clone(), looked.clone());
        move || if idle.replace(false) { looked.set(Instant::now()); backend.borrow().window().request_redraw(); }
    };
    {
        let (look, looked) = (look.clone(), looked.clone());
        event_loop.handle().insert_source(Timer::from_duration(LOOK_EVERY), move |_, _, _| {
            // What happened since is seen after this turn of the loop.
            if looked.get().elapsed() >= LOOK_EVERY { look(); }
            TimeoutAction::ToDuration(LOOK_EVERY)
        })?;
    }
    let after_turn = {
        let (idle, handle, waking) = (idle.clone(), event_loop.handle(), Rc::new(Cell::new(false)));
        move |state: &mut App| {
            if !(state.redraw || state.moving.get()) || !idle.get() || waking.get() { return; }
            match FRAME.checked_sub(looked.get().elapsed()).filter(|d| !d.is_zero()) {
                None => look(),
                // Too soon: the loop is woken when the time has come, and this runs again.
                Some(wait) => {
                    let due = waking.clone();
                    let timer = handle.insert_source(Timer::from_duration(wait), move |_, _, _| { due.set(false); TimeoutAction::Drop });
                    waking.set(timer.is_ok());
                }
            }
        }
    };

    event_loop.handle().insert_source(winit, move |event, _, state| { let mut backend = backend.borrow_mut(); match event {
        WinitEvent::Resized { size, .. } => {
            state.redraw = true;
            output.change_current_state(Some(Mode { size, refresh: 60_000 }), None, None, None);
            // The background apps are laid out for the new size and told it (one that sits still would never ask).
            smithay::desktop::layer_map_for_output(&output).arrange();
            state.window_size = (size.w, size.h);
            state.maximized = backend.window().is_maximized();
            state.fit_strips();
            state.note_saved_soon();
        }
        WinitEvent::Input(event) => {
            state.process_input_event(event);
            if let Some(cmd) = state.window_cmd.take() {
                use crate::state::WindowCmd;
                let w = backend.window();
                let done = match cmd {
                    WindowCmd::Move => w.drag_window().map_err(|e| e.to_string()),
                    WindowCmd::Resize(d) => w.drag_resize_window(d).map_err(|e| e.to_string()),
                    WindowCmd::Minimize => { w.set_minimized(true); Ok(()) }
                    WindowCmd::Maximize => { w.set_maximized(!w.is_maximized()); Ok(()) }
                    WindowCmd::Close => { state.end_reason = "window closed".into(); state.loop_signal.stop(); Ok(()) }
                };
                if let Err(e) = done { record::warn("screen", "window not moved", &e, &[("asked", format!("{cmd:?}"))]); }
            }
        }
        WinitEvent::Ime(ime) => state.handle_ime(ime),
        WinitEvent::Focus(focused) => {
            state.redraw = true;
            record::detail("screen", "window focus", &[("focused", focused.to_string())]);
            state.host_focused = focused;
            if !focused { state.release_keys(); }
            // Back from the desktop: what was copied there comes in, where the desktop cannot be watched.
            if focused { state.clip.read_once(); }
        }
        WinitEvent::Redraw => {
            // What this frame finds (an event since, something moving) decides when the next is drawn.
            state.redraw = false;
            state.moving.set(false);
            let now = Instant::now();
            let ms = now.duration_since(last).as_secs_f64() * 1000.0;
            last = now;
            // After a quiet spell the gap is long: a glide that starts now starts from its first step, not halfway.
            state.frame(ms.min(50.0));
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
                (Act::Idle, None) => state.window_edge_at(state.pointer.0, state.pointer.1).map_or(CursorIcon::Default, |d| {
                    use smithay::reexports::winit::window::ResizeDirection as D;
                    match d { D::East | D::West => CursorIcon::EwResize, D::North | D::South => CursorIcon::NsResize, D::NorthWest | D::SouthEast => CursorIcon::NwseResize, _ => CursorIcon::NeswResize }
                }),
                _ => CursorIcon::Default,
            };
            if wanted != cursor {
                cursor = wanted;
                backend.window().set_cursor(cursor.into());
            }

            let size = backend.window_size();
            let age = backend.buffer_age().unwrap_or(0);
            // A frame that cannot be drawn is left out and tried again: stopping here would take every panel with it.
            let drawn: Result<_, String> = 'frame: {
                let (renderer, mut framebuffer) = match backend.bind() { Ok(b) => b, Err(e) => break 'frame Err(format!("{e:?}")) };
                if state.shaders.is_none() && !shaders_failed {
                    match compile(renderer) {
                        Ok(sh) => state.shaders = Some(sh),
                        Err(e) => { shaders_failed = true; record::error("screen", "shader failed", &[("error", e)]); }
                    }
                }
                if !state.shots.is_empty() { take_shots(state, renderer, size); }
                let elements = window_elements(state, renderer);
                // Behind a see-through window the desktop shows: the surface's own colour only as opaque as the theme says.
                // When the surface shader draws the see-through surface, the clear is fully transparent: both at the
                // theme's opacity would lay it twice.
                let see_through = state.theme.window_opacity < 1.0 && state.shaders.is_some();
                let clear = if see_through { [0.0; 4] } else { premultiplied(state.theme.surface_top) };
                damage_tracker.render_output(renderer, &mut framebuffer, age, &elements, clear).map(|r| r.damage.cloned().unwrap_or_default()).map_err(|e| format!("{e:?}"))
            };
            let drawn = drawn.and_then(|damage| if damage.is_empty() { Ok(false) } else { backend.submit(Some(&damage)).map(|_| true).map_err(|e| format!("{e:?}")) });
            let sent = match drawn {
                Ok(sent) => { frame_failing = false; sent }
                Err(e) => {
                    if !std::mem::replace(&mut frame_failing, true) { record::error("screen", "frame not drawn", &[("error", e)]); }
                    idle.set(true);
                    return;
                }
            };
            if sent { sent_frames += 1; }

            state.space.elements().for_each(|window| {
                window.send_frame(&output, state.start_time.elapsed(), Some(Duration::ZERO), |_, _| Some(output.clone()))
            });
            for layer in smithay::desktop::layer_map_for_output(&output).layers() {
                layer.send_frame(&output, state.start_time.elapsed(), Some(Duration::ZERO), |_, _| Some(output.clone()));
            }
            state.space.refresh();
            state.popups.cleanup();
            let _ = state.display_handle.flush_clients();
            // Something moving is paced by the desktop, frame by frame. A still picture is not drawn again at once: the buffer
            // behind the one just shown is old, so the same picture would be sent a second time (the next look does that).
            if sent && state.moving.get() { backend.window().request_redraw(); } else { idle.set(true); }
        }
        WinitEvent::CloseRequested => {
            state.end_reason = "window closed".into();
            state.loop_signal.stop();
        }
    }})?;
    Ok(after_turn)
}

fn edge_cursor(e: &crate::layout::Edges) -> CursorIcon {
    let (h, v) = (e.left || e.right, e.top || e.bottom);
    if h && v {
        if (e.left && e.top) || (e.right && e.bottom) { CursorIcon::NwseResize } else { CursorIcon::NeswResize }
    } else if h { CursorIcon::EwResize } else { CursorIcon::NsResize }
}
