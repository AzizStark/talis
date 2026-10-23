//! GLArea realization, rendering, diagnostics and context-current teardown.
use super::{metrics::Metrics, state::AppState};
use crate::{preview::PreviewAsset, rendering::Renderer};
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

pub(super) type SharedRenderer = Rc<RefCell<Option<Renderer>>>;
pub(super) type Failure = Rc<RefCell<Option<anyhow::Error>>>;

pub(super) struct Lifecycle {
    pub pending_asset: Rc<RefCell<Option<PreviewAsset>>>,
    pub state: Rc<RefCell<AppState>>,
    pub metrics: Rc<RefCell<Metrics>>,
    pub failure: Failure,
    pub stats: gtk::Label,
    pub capture: Option<PathBuf>,
    pub artwork: crate::artwork::Loader,
    pub header_artwork: Option<super::thumbnail::HeaderArtwork>,
    pub scene_ui: Option<super::scenes::SceneUi>,
    pub running: Rc<Cell<bool>>,
}

fn report_error(area: &gtk::GLArea, failure: &Failure, error: anyhow::Error, shell_surface: bool) {
    let stop = shell_surface || error.is::<crate::graphics::ContextInitializationError>();
    let message = format!("{error:#}");
    tracing::error!("{message}");
    *failure.borrow_mut() = Some(error);
    area.set_error(Some(&glib::Error::new(
        gtk::gdk::GLError::NotAvailable,
        &message,
    )));
    // A context-less startup cannot render. Return its typed error to main for
    // recovery (or a terminal error); Shell also retains its static fallback.
    if stop {
        if let Some(app) = area
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok())
            .and_then(|window| window.application())
        {
            app.quit();
        }
    }
}

pub(super) fn attach(area: &gtk::GLArea, context: Lifecycle) -> SharedRenderer {
    let renderer: SharedRenderer = Rc::new(RefCell::new(None));
    let frames = Rc::new(Cell::new(0u64));
    connect_realize(
        area,
        renderer.clone(),
        context.state.clone(),
        context.pending_asset,
        context.failure.clone(),
    );
    if let Some(ui) = context.scene_ui {
        super::scenes::attach(
            area,
            ui,
            context.state.clone(),
            context.running.clone(),
            renderer.clone(),
            context.artwork.clone(),
        );
    }
    connect_render(
        area,
        renderer.clone(),
        frames.clone(),
        FrameContext {
            state: context.state,
            metrics: context.metrics,
            failure: context.failure,
            stats: context.stats,
            capture: context.capture,
            artwork: context.artwork,
            header_artwork: context.header_artwork,
            running: context.running,
        },
    );
    connect_unrealize(area, renderer.clone(), frames);
    super::presenter::bind(area, &renderer);
    renderer
}

fn connect_realize(
    area: &gtk::GLArea,
    renderer: SharedRenderer,
    state: Rc<RefCell<AppState>>,
    pending: Rc<RefCell<Option<PreviewAsset>>>,
    failure: Failure,
) {
    let initialized = Cell::new(false);
    area.connect_realize(move |area| {
        area.make_current();
        let config = state.borrow().config.clone();
        if let Some(error) = area.error() {
            let error = if initialized.get() {
                anyhow::anyhow!("OpenGL context recreation failed: {error}")
            } else {
                crate::graphics::ContextInitializationError::new(
                    error.to_string(),
                    match gtk::prelude::WidgetExt::display(area).type_().name() {
                        "GdkX11Display" => crate::graphics::DisplayBackend::X11,
                        "GdkWaylandDisplay" => crate::graphics::DisplayBackend::Wayland,
                        _ => crate::graphics::DisplayBackend::Other,
                    },
                )
                .into()
            };
            report_error(area, &failure, error, state.borrow().shell_surface);
            return;
        }
        let asset = pending
            .borrow_mut()
            .take()
            .map(Ok)
            .unwrap_or_else(|| PreviewAsset::load(&config));
        let result = asset.and_then(|asset| unsafe { Renderer::new(asset) });
        match result {
            Ok(mut value) => {
                initialized.set(true);
                let s = state.borrow();
                if s.benchmark || s.gpu_profile {
                    unsafe {
                        value.enable_gpu_timing(s.gpu_profile);
                    }
                }
                drop(s);
                tracing::info!(mode=?config.scene.mode, "Renderer ready");
                state.borrow_mut().vinyl_ready = value.vinyl_supported();
                state.borrow_mut().artwork_ready = value.record_artwork_supported();
                if area.is::<super::presenter::RegionalArea>() {
                    if let Err(error) = unsafe { value.set_artwork(0, None, &config.artwork) } {
                        report_error(
                            area,
                            &failure,
                            error.context("Initial artwork settings failed"),
                            state.borrow().shell_surface,
                        );
                        unsafe {
                            value.destroy();
                        }
                        return;
                    }
                }
                *renderer.borrow_mut() = Some(value);
                super::presenter::queue_render(area);
            }
            Err(error) => report_error(
                area,
                &failure,
                error.context("OpenGL initialization failed"),
                state.borrow().shell_surface,
            ),
        }
    });
}

struct FrameContext {
    state: Rc<RefCell<AppState>>,
    metrics: Rc<RefCell<Metrics>>,
    failure: Failure,
    stats: gtk::Label,
    capture: Option<PathBuf>,
    artwork: crate::artwork::Loader,
    header_artwork: Option<super::thumbnail::HeaderArtwork>,
    running: Rc<Cell<bool>>,
}

fn connect_render(
    area: &gtk::GLArea,
    renderer: SharedRenderer,
    frames: Rc<Cell<u64>>,
    context: FrameContext,
) {
    let capture = RefCell::new(context.capture);
    let first_frame = Cell::new(true);
    let telemetry_at = Cell::new(Instant::now());
    area.connect_render(move |area, _| {
        let Some((animate, resume)) = super::visibility::begin_frame(area) else {
            return glib::Propagation::Stop;
        };
        if let Some(renderer) = renderer.borrow_mut().as_mut() {
            frames.set(frames.get() + 1);
            let mut s = context.state.borrow_mut();
            let now = Instant::now();
            if resume { s.resume_at(now); }
            if s.mode == crate::config::PreviewMode::Arm {
                s.sync_playback(now);
                let delta = if first_frame.get() { Duration::ZERO } else {
                    now.saturating_duration_since(s.previous_frame)
                };
                if animate { s.advance_arm(delta); }
            }
            s.previous_frame = now;
            let scale = area.scale_factor();
            let (width, height) = (area.width() * scale, area.height() * scale);
            if first_frame.get() {
                tracing::info!(width, height, scale, mode=?s.mode, "Scene framebuffer dimensions (physical pixels)");
            }
            let started = Instant::now();
            renderer.set_sleeve_visible(s.playback.borrow().sleeve_visible());
            context.artwork.sync(&s.playback.borrow());
            let (revision, image, _) = context.artwork.snapshot();
            let (previous_revision, previous_image) = context.artwork.previous_snapshot();
            if let Err(error) = unsafe { renderer.set_previous_artwork(previous_revision, previous_image.as_deref()) } {
                report_error(area, &context.failure, error.context("Previous artwork upload failed"), s.shell_surface);
                return glib::Propagation::Stop;
            }
            if let Some(thumbnail) = &context.header_artwork {
                thumbnail.sync(revision, image.as_deref());
            }
            // A missing/failed cover uses center-only default ink, not a loader
            // source: stand/header/previous-track artwork remain unchanged.
            let fallback = image.is_none() && s.config.artwork.enabled && s.playback.borrow().sleeve_visible();
            renderer.set_record_fallback_enabled(fallback);
            s.artwork_ready = renderer.record_artwork_supported();
            s.vinyl_ready = renderer.vinyl_supported();
            if !animate { s.artwork_rotation.resume_at(now); }
            s.sync_artwork(now);
            if animate { s.artwork_rotation.advance(now); }
            renderer.set_artwork_angle(s.artwork_rotation.angle());
            renderer.set_arm_cache_target(s.arm_cache_target(now));
            if let Err(error) = unsafe { renderer.set_artwork(revision, image.as_deref(), &s.config.artwork) } {
                report_error(area, &context.failure, error.context("Artwork upload failed"), s.shell_surface);
                return glib::Propagation::Stop;
            }
            let result = if let Some(region) = super::presenter::region(area) {
                unsafe { renderer.render_regional(width, height, s.arm.position, region) }
                    .map(|background_changed| {
                        if background_changed { super::presenter::refresh_background(area); }
                    })
            } else {
                unsafe { renderer.render(width, height, s.arm.position) }
            };
            if let Err(error) = result {
                report_error(area, &context.failure, error.context("Scene rendering failed"), s.shell_surface);
                return glib::Propagation::Stop;
            }
            if s.mode == crate::config::PreviewMode::Arm {
                tracing::trace!(progress=s.arm.position, target=?s.arm_motion.target(), animate,
                    "Rendered arm pose");
                tracing::trace!(angle=s.artwork_rotation.angle(),ready=s.artwork_ready,"Rendered cover art");
            }
            let cpu = started.elapsed().as_secs_f64() * 1000.0;
            if s.benchmark {
                if let Some(text) = context.metrics.borrow_mut().sample(cpu, renderer.gpu_ms) {
                    if s.mode == crate::config::PreviewMode::Arm {
                        tracing::info!(progress=s.arm.position, start=s.arm.start, end=s.arm.end,
                            demo_playing=s.arm.playing, now_playing=s.playback.borrow().enabled, "Arm timing state");
                    }
                    let label = context.stats.downgrade();
                    glib::idle_add_local_once(move || {
                        if let Some(label) = label.upgrade() { label.set_text(&text); }
                    });
                }
            }
            let mut result = if first_frame.replace(false) {
                unsafe { renderer.check_error() }
            } else { Ok(()) };
            if result.is_ok() {
                if let Some(path) = capture.borrow_mut().take() {
                    result = unsafe { renderer.capture(width, height, &path) };
                }
            }
            if let Err(error) = result {
                report_error(area, &context.failure, error.context("Scene rendering failed"), s.shell_surface);
            } else if s.shell_surface {
                if frames.get() == 1 {
                    println!("READY protocol=2 size={}x{}", area.width(), area.height());
                    println!("FRAME {}", frames.get());
                } else if now.saturating_duration_since(telemetry_at.get()) >= Duration::from_secs(2) {
                    println!("FRAME {}", frames.get());
                    telemetry_at.set(now);
                }
            }
        }
        if resume {
            super::scheduling::ensure_animation(area, context.state.clone(), context.running.clone());
        }
        glib::Propagation::Stop
    });
}

fn connect_unrealize(area: &gtk::GLArea, renderer: SharedRenderer, frames: Rc<Cell<u64>>) {
    area.connect_unrealize(move |area| {
        tracing::info!(frames = frames.get(), "Preview closed");
        area.make_current();
        if let Some(renderer) = renderer.borrow_mut().take() {
            if area.error().is_none() {
                unsafe {
                    renderer.destroy();
                }
            } else {
                // Context loss: avoid deleting resources without a current, valid GL context.
                std::mem::forget(renderer);
            }
        }
    });
}

#[cfg(test)]
mod tests;
