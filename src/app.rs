//! Preview application orchestration; UI, scheduling and GL lifecycle live in submodules.
mod about;
mod artwork;
mod controls;
mod defaults;
mod feedback;
mod gnome_setup;
mod instance;
mod launch;
mod lifecycle;
mod media;
mod metrics;
mod native_patch;
mod preferences;
mod presenter;
mod scenes;
mod scheduling;
mod settings;
mod shell_bridge;
mod startup;
mod state;
mod style;
mod thumbnail;
mod transport;
mod tray;
mod visibility;
mod window;

use crate::playback::{PlaybackFeed, mpris::Monitor};
use crate::{arm::ArmState, config::Config, preview::PreviewAsset};
use anyhow::{Result, ensure};
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::Instant,
};

/// First-launch setup only: no scene decoding, player observation or scene GL context.
pub fn run_gnome_setup() -> Result<()> {
    gnome_setup::run()
}

#[derive(Clone)]
pub struct Options {
    /// Explicit diagnostic/config launch; never reuse another desktop instance.
    pub independent: bool,
    pub fullscreen: bool,
    pub desktop: bool,
    pub experimental_wayland: bool,
    pub monitor: u32,
    pub smoke_seconds: Option<u32>,
    pub benchmark: bool,
    pub gpu_profile: bool,
    pub scene_only: bool,
    /// Owned GNOME client; Shell forwards bounded input through its private pipe.
    pub shell_surface: bool,
    pub regional_presenter: bool,
    pub native_patch: bool,
    pub start_minimized: bool,
    pub size: Option<(i32, i32)>,
    pub capture: Option<PathBuf>,
    pub arm_progress: f32,
    pub arm_demo: bool,
    pub now_playing: bool,
    pub player: Option<String>,
}

/// Full scene target, never the smaller native animated patch or Settings window.
fn resolution_target(options: &Options) -> (i32, i32) {
    let monitor = gtk::gdk::Display::default().and_then(|display| {
        display
            .monitors()
            .item(options.monitor)
            .and_downcast::<gtk::gdk::Monitor>()
    });
    let (w, h) = if options.desktop {
        monitor
            .as_ref()
            .map(|m| (m.geometry().width(), m.geometry().height()))
            .unwrap_or((1280, 720))
    } else {
        options.size.unwrap_or((1280, 720))
    };
    let scale = monitor.map_or(1, |m| m.scale_factor());
    (w.saturating_mul(scale), h.saturating_mul(scale))
}

fn initialize_owned_wayland() -> Result<()> {
    // libwayland consumes/unsets WAYLAND_SOCKET while GTK opens its display.
    // Validate the inherited launch descriptor before init, then validate the
    // actual backend afterward. Never require the consumed variable to remain.
    ensure!(
        std::env::var("WAYLAND_SOCKET")
            .ok()
            .and_then(|value| value.parse::<i32>().ok())
            .is_some_and(|fd| fd >= 0),
        "Shell surfaces require an inherited native Wayland socket"
    );
    gtk::init()?;
    ensure!(
        gtk::gdk::Display::default().is_some_and(|d| d.type_().name() == "GdkWaylandDisplay"),
        "Shell surfaces must use a native Wayland display"
    );
    Ok(())
}

pub fn run_with_preferences(
    mut config: Config,
    mut options: Options,
    preferences: Option<crate::preferences::Session>,
) -> Result<()> {
    options.apply_desktop_defaults(config.scene.mode);
    if options.shell_surface {
        ensure!(
            !options.desktop && !options.fullscreen && !options.native_patch,
            "Shell surfaces cannot use standalone desktop placement"
        );
        initialize_owned_wayland()?;
    }
    gtk::init()?;
    let instance = if options.normal_desktop() {
        match instance::Instance::claim(&options)? {
            instance::Claim::Primary(instance) => Some(Rc::new(instance)),
            instance::Claim::Activated => return Ok(()),
        }
    } else {
        None
    };
    let placement = if options.desktop {
        let host = crate::desktop::DesktopHost::new_for_session(
            options.monitor,
            options.experimental_wayland,
        )?;
        host.set_initial_mate_overlay(config.render.experimental_mate_overlay);
        Some(host)
    } else {
        None
    };
    tracing::info!(
        gtk=format!("{}.{}.{}", gtk::major_version(), gtk::minor_version(), gtk::micro_version()),
        session=?crate::desktop::Session::detect(), "Starting GTK desktop preview"
    );
    if let Some(host) = &placement {
        tracing::info!(backend=?host.kind(), capabilities=?host.capabilities(), placement=%host.description(), "Desktop host selected");
    }
    // Keep lower-resolution displays on their lighter package. Custom paths
    // remain authoritative, including explicitly selected diagnostic profiles.
    if !options.shell_surface {
        let (w, h) = resolution_target(&options);
        if config.scene.resolve_bundled_resolution(w, h)? {
            tracing::info!(resolution=?config.scene.resolution, "Selected bundled scene resolution");
        }
    }
    let pending = Rc::new(RefCell::new(Some(PreviewAsset::load(&config)?)));
    let app = gtk::Application::builder()
        .application_id("io.github.azizstark.Talis")
        // Desktop admission happens above; keep the GTK identity unchanged on
        // every monitor and allow independent previews/benchmarks to coexist.
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    // The admission endpoint is a separate Gio application. The real GTK app
    // retains its stable identity for icons, window grouping and desktop files.
    app.set_default();
    app.connect_startup(|app| {
        // GApplication can acquire the shared bus before any optional observer.
        // Protect scene-only/no-MPRIS launches too; normal quit/shutdown and
        // display-session teardown remain managed by GTK/the desktop session.
        if let Some(connection) = app.dbus_connection() {
            connection.set_exit_on_close(false);
        }
    });
    let failed: lifecycle::Failure = Rc::new(RefCell::new(None));
    let failure = failed.clone();
    app.connect_shutdown(|app| {
        // Both windows close together, releasing GL resources while current.
        for window in app.windows() {
            window.destroy();
        }
    });
    app.connect_activate(move |app| {
        let now = Instant::now();
        let mut arm = ArmState::default();
        arm.set_range(config.render.arm_range_start, config.render.arm_range_end);
        arm.scrub(options.arm_progress);
        arm.play(options.arm_demo);
        let mut feed = PlaybackFeed {
            preferred: options.player.clone(),
            ..PlaybackFeed::default()
        };
        feed.set_enabled(options.now_playing);
        let feed = Rc::new(RefCell::new(feed));
        let state = Rc::new(RefCell::new(state::AppState {
            config: config.clone(),
            previous_frame: now,
            mode: config.scene.mode,
            benchmark: options.benchmark,
            gpu_profile: options.gpu_profile,
            shell_surface: options.shell_surface,
            arm,
            arm_motion: crate::arm::motion::ArmMotion::new(
                options.arm_progress,
                config.render.arm_response_seconds,
            ),
            playback: feed.clone(),
            artwork_rotation: Default::default(),
            artwork_ready: false,
            vinyl_ready: false,
        }));
        let running = Rc::new(Cell::new(false));
        let metrics = Rc::new(RefCell::new(metrics::Metrics::default()));
        let preview = window::build(app, &options, state.clone(), running.clone());
        let art_area = preview.area.downgrade();
        let artwork = crate::artwork::Loader::new(move || {
            if let Some(area) = art_area.upgrade() {
                presenter::queue_content_render(&area);
            }
        });
        let art_shutdown = artwork.clone();
        app.connect_shutdown(move |_| art_shutdown.stop());
        let renderer = lifecycle::attach(
            &preview.area,
            lifecycle::Lifecycle {
                pending_asset: pending.clone(),
                state: state.clone(),
                metrics,
                failure: failure.clone(),
                stats: preview.stats.clone(),
                capture: options.capture.clone(),
                artwork: artwork.clone(),
                header_artwork: preview.media_ui.as_ref().map(|ui| ui.thumbnail.clone()),
                scene_ui: preview.scene_ui.clone(),
                running: running.clone(),
            },
        );
        if let Some(placement) = &placement {
            if let Err(error) = placement.configure_scene(&preview.window) {
                let message = format!("Desktop placement failed: {error:#}");
                tracing::error!("{message}");
                *failure.borrow_mut() = Some(anyhow::anyhow!(message));
                app.quit();
                return;
            }
        }
        if options.native_patch {
            let result = placement
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Native patch requires a desktop host"))
                .and_then(|host| native_patch::install(&preview.area, &preview.window, host));
            if let Err(error) = result {
                tracing::warn!("Native patch unavailable; using regional fallback: {error:#}");
            }
        }
        if let Some(host) = &placement {
            window::bind_mate_overlay(&preview, host, state.clone());
        }
        if let Some(session) = &preferences {
            match preferences::attach(&preview, &state.borrow(), session.clone()) {
                Ok(Some(saver)) => {
                    app.connect_shutdown(move |_| saver.stop());
                }
                Ok(None) => (),
                Err(error) => {
                    preview
                        .preferences_error
                        .set_text(&format!("Preferences writer unavailable: {error:#}"));
                    preview.preferences_error.set_visible(true);
                    tracing::warn!("Preferences writer unavailable: {error:#}");
                }
            }
        }
        // Preview, capture and controlled benchmarks must remain deterministic.
        let visibility = placement
            .as_ref()
            .filter(|_| !options.benchmark && options.capture.is_none())
            .map(|host| {
                visibility::Controller::attach(
                    &preview.area,
                    &preview.window,
                    state.clone(),
                    running.clone(),
                    host.clone(),
                )
            });
        if let Some(controller) = visibility.clone() {
            app.connect_shutdown(move |_| controller.stop());
        }
        let shell_visibility = options.shell_surface.then(|| {
            visibility::ShellController::attach(&preview.area, state.clone(), running.clone())
        });
        if let Some(controller) = shell_visibility.clone() {
            app.connect_shutdown(move |_| controller.stop());
        }
        scheduling::ensure_animation(&preview.area, state.clone(), running.clone());
        window::present(&preview, &options);
        if failure.borrow().as_ref().is_some_and(|error| {
            options.shell_surface || error.is::<crate::graphics::ContextInitializationError>()
        }) {
            return;
        }
        if let Some(tray) = &preview.tray_ui {
            tray.start();
            let tray = tray.clone();
            app.connect_shutdown(move |_| tray.stop());
        }
        if let Some(instance) = &instance {
            instance.bind_settings(&preview);
        }
        if config.scene.mode == crate::config::PreviewMode::Arm {
            let transport_state = state.clone();
            let transport_ui = Rc::new(RefCell::new(None::<std::rc::Weak<transport::TransportUi>>));
            let feedback = transport_ui.clone();
            let area = preview.area.downgrade();
            let arm_ui = preview.arm_ui.clone();
            let media_ui = preview.media_ui.clone();
            let sleeve_visible = Cell::new(feed.borrow().sleeve_visible());
            let monitor = Monitor::start(feed.clone(), move || {
                let Some(area) = area.upgrade() else {
                    return;
                };
                let mut s = state.borrow_mut();
                s.sync_playback(Instant::now());
                artwork.sync(&s.playback.borrow());
                if visibility::suspended(&area) {
                    s.artwork_rotation.resume_at(Instant::now());
                }
                s.sync_artwork(Instant::now());
                if let Some(ui) = &media_ui {
                    ui.sync(&s.playback.borrow());
                }
                if let Some(ui) = &arm_ui {
                    ui.sync(&s.arm);
                }
                let visible = s.playback.borrow().sleeve_visible();
                drop(s);
                // A stopped/duration-less player may have no artwork callback
                // or animation timer. Present this discrete background change
                // once, without redrawing for unchanged MPRIS polls.
                if sleeve_visible.replace(visible) != visible {
                    presenter::queue_content_render(&area);
                }
                if let Some(ui) = feedback.borrow().as_ref().and_then(|ui| ui.upgrade()) {
                    ui.sync();
                }
                // Only the destination changes here. The frame callback owns
                // visible motion and renders its final pose before going idle.
                scheduling::ensure_animation(&area, state.clone(), running.clone());
            });
            let ui = match transport::TransportUi::attach(
                &preview.area,
                &preview.window,
                transport_state,
                renderer.clone(),
                monitor.clone(),
                placement.as_ref(),
            ) {
                Ok(ui) => ui,
                Err(error) => {
                    let message = format!("Transport input initialization failed: {error:#}");
                    tracing::error!("{message}");
                    *failure.borrow_mut() = Some(anyhow::anyhow!(message));
                    app.quit();
                    return;
                }
            };
            *transport_ui.borrow_mut() = Some(Rc::downgrade(&ui));
            ui.set_settings(preview.controls.as_ref(), preview.tray_ui.as_ref());
            if options.shell_surface {
                let controller = shell_visibility.as_ref().expect("Shell gate installed");
                controller.set_transport(&ui);
                match shell_bridge::Bridge::attach(
                    &ui,
                    preview.controls.as_ref(),
                    controller.clone(),
                ) {
                    Ok(bridge) => app.connect_shutdown(move |_| bridge.stop()),
                    Err(error) => {
                        *failure.borrow_mut() =
                            Some(error.context("Shell input initialization failed"));
                        app.quit();
                        return;
                    }
                };
            }
            if let Some(controller) = &visibility {
                controller.set_transport(&ui);
            }
            app.connect_shutdown(move |_| monitor.stop());
        }
    });
    let status = app.run_with_args(&["talis"]);
    if let Some(error) = failed.borrow_mut().take() {
        return Err(error);
    }
    ensure!(
        status == gtk::glib::ExitCode::SUCCESS,
        "GTK application exited unsuccessfully"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "Requires an isolated Wayland compositor and inherited WAYLAND_SOCKET"]
    fn owned_wayland_startup_accepts_consumed_launch_socket() {
        assert!(
            std::env::var("WAYLAND_SOCKET")
                .ok()
                .and_then(|value| value.parse::<i32>().ok())
                .is_some_and(|fd| fd >= 0),
            "Fixture must pass a real inherited compositor connection"
        );
        super::initialize_owned_wayland().unwrap();
        assert!(
            std::env::var_os("WAYLAND_SOCKET").is_none(),
            "Fixture must exercise libwayland consuming the socket variable"
        );
        assert_eq!(
            gtk::gdk::Display::default().unwrap().type_().name(),
            "GdkWaylandDisplay"
        );
    }

    use gtk::prelude::*;
}
