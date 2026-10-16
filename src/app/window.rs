//! Separate scene and controls windows; no widgets overlay the rendered scene.
#[cfg(test)]
use super::settings::hidden_supported;
use super::{Options, controls::ArmUi, media::MediaUi, state::AppState, style};
use crate::config::PreviewMode;
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub(super) struct AppWindows {
    pub window: gtk::ApplicationWindow,
    pub controls: Option<gtk::ApplicationWindow>,
    pub area: gtk::GLArea,
    pub stats: gtk::Label,
    pub arm_ui: Option<ArmUi>,
    pub media_ui: Option<MediaUi>,
    pub scene_ui: Option<super::scenes::SceneUi>,
    pub tray_ui: Option<Rc<super::tray::TrayUi>>,
    pub mate_overlay: Option<gtk::Switch>,
    pub preferences_error: gtk::Label,
    pub defaults_ui: Option<super::defaults::DefaultsUi>,
}

fn status_text(mode: PreviewMode, benchmark: bool) -> &'static str {
    if benchmark {
        "Benchmark: two-second warmup…"
    } else if mode == PreviewMode::Arm {
        "Arm follows now-playing timing when enabled"
    } else {
        "Static scene — rendering on demand"
    }
}

fn close_together(window: &gtk::ApplicationWindow) {
    window.connect_close_request(|window| {
        if let Some(app) = window.application() {
            app.quit();
        }
        glib::Propagation::Stop
    });
}

pub(super) fn bind_mate_overlay(
    preview: &AppWindows,
    host: &crate::desktop::DesktopHost,
    state: Rc<RefCell<AppState>>,
) {
    let Some(switch) = &preview.mate_overlay else {
        return;
    };
    let host = host.clone();
    let scene = preview.window.downgrade();
    let area = preview.area.downgrade();
    let changing = Cell::new(false);
    switch.connect_active_notify(move |switch| {
        if changing.get() {
            return;
        }
        let (Some(scene), Some(area)) = (scene.upgrade(), area.upgrade()) else {
            return;
        };
        let enabled = switch.is_active();
        let patch = super::presenter::native_window(&area);
        match host.set_mate_overlay(&scene, patch.as_ref(), enabled) {
            Ok(()) => {
                state.borrow_mut().config.render.experimental_mate_overlay = enabled;
                super::presenter::queue_layout_render(&area);
                tracing::info!(
                    enabled,
                    "Experimental MATE placement changed; wallpaper preferences unchanged"
                );
            }
            Err(error) => {
                tracing::warn!("Cannot change MATE placement: {error:#}");
                changing.set(true);
                switch.set_active(state.borrow().config.render.experimental_mate_overlay);
                changing.set(false);
            }
        }
    });
}

pub(super) fn build(
    app: &gtk::Application,
    options: &Options,
    state: Rc<RefCell<AppState>>,
    running: Rc<Cell<bool>>,
) -> AppWindows {
    let config = state.borrow().config.clone();
    style::install_app_icon(&gtk::gdk::Display::default().expect("GTK display"));
    let mode = config.scene.mode;
    let (width, height) = options.size.unwrap_or((1280, 720));
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Talis · Scene")
        .icon_name(style::APP_ICON_NAME)
        .default_width(width)
        .default_height(height)
        .build();
    style::bind_window_icon(&window);
    if options.shell_surface {
        window.set_decorated(false);
        window.set_focusable(false);
        window.set_resizable(false);
        // This owned, concealed source is cloned onto the desktop, not used as
        // an ordinary window. A default size alone is reduced to the work area
        // by Mutter; its minimum must retain the full camera-sized framebuffer.
        // Do not use fullscreen: it can affect panels and other applications.
        window.set_size_request(width, height);
    }
    let area = if options.regional_presenter || options.native_patch {
        super::presenter::new()
    } else {
        gtk::GLArea::builder()
            .hexpand(true)
            .vexpand(true)
            .auto_render(false)
            .has_depth_buffer(false)
            .build()
    };
    area.set_required_version(3, 3);
    area.set_use_es(false);
    if options.desktop || options.shell_surface {
        area.set_focusable(false);
        area.set_can_target(false);
    }
    window.set_child(Some(&area));
    close_together(&window);
    let stats = gtk::Label::new(Some(if options.benchmark {
        "Benchmark: waiting for rendered frames…"
    } else {
        status_text(mode, false)
    }));
    stats.set_wrap(true);
    let controls = if options.scene_only {
        None
    } else {
        Some(
            gtk::ApplicationWindow::builder()
                .application(app)
                .title("Talis · Settings")
                .icon_name(style::APP_ICON_NAME)
                .default_width(684)
                .default_height(493)
                .build(),
        )
    };
    let super::settings::SettingsUi {
        arm_ui,
        media_ui,
        scene_ui,
        tray_ui,
        mate_overlay,
        defaults_ui,
        preferences_error,
    } = super::settings::build(
        super::settings::Context {
            app,
            controls: controls.as_ref(),
            area: &area,
            stats: &stats,
            options,
        },
        state,
        running,
    );
    AppWindows {
        window,
        controls,
        area,
        stats,
        arm_ui,
        media_ui,
        scene_ui,
        tray_ui,
        mate_overlay,
        preferences_error,
        defaults_ui,
    }
}

/// A desktop scene is shown without activation; only the controls request focus.
pub(super) fn present(preview: &AppWindows, options: &Options) {
    if options.desktop || options.shell_surface {
        preview.window.set_visible(true);
    } else {
        if options.fullscreen {
            preview.window.fullscreen();
        }
        preview.window.present();
    }
    if let Some(controls) = &preview.controls {
        controls.connect_map(|_| tracing::debug!("Settings window mapped"));
        if options.shell_surface {
            // Created now, mapped only after an explicit Shell Settings request.
        } else if options.start_minimized {
            controls.set_visible(true);
            controls.minimize();
        } else {
            controls.present();
        }
    }
    if let Some(seconds) = options.smoke_seconds {
        if let Some(app) = preview.window.application() {
            let weak = app.downgrade();
            glib::timeout_add_local_once(
                std::time::Duration::from_secs(seconds as u64),
                move || {
                    if let Some(app) = weak.upgrade() {
                        app.quit();
                    }
                },
            );
        }
    }
}

#[cfg(test)]
mod tests;
