//! User-selected authored day/night scene; keep the old renderer until a replacement is ready.
use super::{lifecycle::SharedRenderer, scheduling::ensure_animation, state::AppState, style};
use crate::{
    config::{SceneResolution, SceneVariant},
    rendering::Renderer,
    scene_loader::SceneLoader,
};
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(super) struct SceneUi {
    pub widget: gtk::Box,
    pub choice: gtk::ComboBoxText,
    pub resolution: gtk::ComboBoxText,
    pub status: gtk::Label,
    pub committed: SceneCommit,
    updating: Rc<Cell<bool>>,
    restoring: Rc<Cell<bool>>,
    options: super::Options,
}
type SceneCommit = Rc<RefCell<Option<Box<dyn Fn(SceneVariant, SceneResolution)>>>>;
impl SceneUi {
    /// One combined selection: never load an intermediate scene/tier on reset.
    pub(super) fn restore_defaults(&self) {
        if !self.choice.is_sensitive() && !self.resolution.is_sensitive() {
            return;
        }
        let defaults = crate::config::Config::default().scene;
        self.updating.set(true);
        if self.choice.is_sensitive() {
            self.choice.set_active_id(Some(defaults.variant.id()));
        }
        if self.resolution.is_sensitive() {
            self.resolution
                .set_active_id(Some(defaults.resolution.id()));
        }
        self.updating.set(false);
        // Also cancels an older request when the defaults are already loaded.
        self.restoring.set(true);
        self.choice.emit_by_name::<()>("changed", &[]);
        self.restoring.set(false);
    }
}
pub(super) fn controls(state: &AppState, options: &super::Options) -> SceneUi {
    let choice = gtk::ComboBoxText::new();
    choice.set_widget_name("scene-variant");
    choice.append(Some("day"), "Day");
    if state.config.scene.night.is_some() {
        choice.append(Some("night"), "Night");
    }
    choice.set_active_id(Some(state.config.scene.variant.id()));
    let widget = gtk::Box::new(gtk::Orientation::Vertical, 4);
    widget.append(&style::row(
        "Scene",
        "Switch between day and night",
        &choice,
    ));
    let resolution = gtk::ComboBoxText::new();
    resolution.set_widget_name("scene-resolution");
    resolution.append(Some("automatic"), "Automatic");
    resolution.append(Some("1080p"), "1080p");
    resolution.append(Some("1440p"), "1440p");
    resolution.append(Some("4k"), "4K");
    resolution.set_active_id(Some(state.config.scene.resolution.id()));
    resolution
        .set_sensitive(!options.shell_surface && state.config.scene.bundled_resolution_available());
    style::separator(&widget);
    widget.append(&style::row(
        "Resolution",
        if resolution.is_sensitive() {
            "Higher resolution uses more GPU memory"
        } else {
            ""
        },
        &resolution,
    ));
    let status = style::label("", "quiet-status");
    status.set_widget_name("scene-load-status");
    status.add_css_class("settings-feedback-source");
    status.set_wrap(true);
    status.set_visible(false);
    widget.append(&status);
    SceneUi {
        widget,
        choice,
        resolution,
        status,
        committed: Rc::new(RefCell::new(None)),
        updating: Rc::new(Cell::new(false)),
        restoring: Rc::new(Cell::new(false)),
        options: options.clone(),
    }
}

pub(super) fn attach(
    area: &gtk::GLArea,
    ui: SceneUi,
    state: Rc<RefCell<AppState>>,
    running: Rc<Cell<bool>>,
    renderer: SharedRenderer,
    artwork: crate::artwork::Loader,
) {
    let loader = match SceneLoader::new() {
        Ok(loader) => Rc::new(loader),
        Err(error) => {
            ui.choice.set_sensitive(false);
            ui.resolution.set_sensitive(false);
            ui.status.set_text(&format!("{error:#}"));
            ui.status.set_visible(true);
            return;
        }
    };
    let polling = Rc::new(Cell::new(false));
    let updating = ui.updating.clone();
    let restoring = ui.restoring.clone();
    let weak_area = area.downgrade();
    let weak_choice = ui.choice.downgrade();
    let weak_resolution = ui.resolution.downgrade();
    let weak_status = ui.status.downgrade();
    let committed = ui.committed.clone();
    let options = ui.options.clone();
    let changed = Rc::new(move || {
        if updating.get() {
            return;
        }
        if weak_area.upgrade().is_none() {
            return;
        }
        let (Some(choice), Some(resolution)) = (weak_choice.upgrade(), weak_resolution.upgrade())
        else {
            return;
        };
        let variant = match choice.active_id().as_deref() {
            Some("night") => SceneVariant::Night,
            Some("day") => SceneVariant::Day,
            _ => return,
        };
        let mut config = state.borrow().config.clone();
        config.scene.variant = variant;
        config.scene.resolution = match resolution.active_id().as_deref() {
            Some("automatic") => SceneResolution::Automatic,
            Some("1080p") => SceneResolution::FullHd,
            Some("1440p") => SceneResolution::Qhd,
            Some("4k") => SceneResolution::FourK,
            _ => return,
        };
        let (w, h) = super::resolution_target(&options);
        if resolution.is_sensitive() {
            if let Err(error) = config.scene.resolve_bundled_resolution(w, h) {
                // Invalidate any earlier pending day/resolution request too.
                loader.cancel();
                rollback(
                    &choice,
                    &weak_resolution,
                    &weak_status,
                    &state,
                    &updating,
                    &format!("{error:#}"),
                );
                return;
            }
        }
        if config.scene == state.borrow().config.scene {
            // GTK may emit another changed notification after a rollback.
            // Only a genuine pending selection clears the loading notice;
            // leave the failure explanation visible on the retained scene.
            if loader.pending() {
                loader.cancel();
                if let Some(status) = weak_status.upgrade() {
                    status.set_visible(false);
                }
            }
            if restoring.get() {
                if let Some(commit) = committed.borrow().as_ref() {
                    commit(config.scene.variant, config.scene.resolution);
                }
            }
            return;
        }
        if config.scene.selected_package().ok()
            == state.borrow().config.scene.selected_package().ok()
        {
            // Changing Automatic to the already-loaded tier is only a preference
            // edit. Cancel stale requests without allocating another renderer.
            loader.cancel();
            state.borrow_mut().config.scene = config.scene;
            if let Some(commit) = committed.borrow().as_ref() {
                let scene = &state.borrow().config.scene;
                commit(scene.variant, scene.resolution);
            }
            if let Some(status) = weak_status.upgrade() {
                status.set_visible(false);
            }
            return;
        }
        if let Err(error) = loader.request(config) {
            rollback(
                &choice,
                &weak_resolution,
                &weak_status,
                &state,
                &updating,
                &format!("{error:#}"),
            );
            return;
        }
        if let Some(status) = weak_status.upgrade() {
            status.set_visible(false);
            status.set_text("");
        }
        if polling.replace(true) {
            return;
        }
        let (loader, polling, updating, state, running, renderer, artwork) = (
            loader.clone(),
            polling.clone(),
            updating.clone(),
            state.clone(),
            running.clone(),
            renderer.clone(),
            artwork.clone(),
        );
        let (weak_area, weak_choice, weak_resolution, weak_status) = (
            weak_area.clone(),
            weak_choice.clone(),
            weak_resolution.clone(),
            weak_status.clone(),
        );
        let committed = committed.clone();
        glib::timeout_add_local(Duration::from_millis(25), move || {
            let (Some(area), Some(choice)) = (weak_area.upgrade(), weak_choice.upgrade()) else {
                loader.cancel();
                polling.set(false);
                return glib::ControlFlow::Break;
            };
            if !area.is_realized() {
                // A window can close while its worker is still decoding. Never
                // upload/delete GL resources after the context is unrealized.
                loader.cancel();
                polling.set(false);
                updating.set(true);
                choice.set_active_id(Some(state.borrow().config.scene.variant.id()));
                if let Some(resolution) = weak_resolution.upgrade() {
                    resolution.set_active_id(Some(state.borrow().config.scene.resolution.id()));
                }
                updating.set(false);
                if let Some(status) = weak_status.upgrade() {
                    status.set_visible(false);
                }
                return glib::ControlFlow::Break;
            }
            if let Some(result) = loader.poll() {
                area.make_current();
                let result = result.and_then(|(scene, asset)| {
                    if let Some(error) = area.error() {
                        anyhow::bail!("Graphics context unavailable: {error}");
                    }
                    let mut next = unsafe { Renderer::new(asset) }?;
                    let s = state.borrow();
                    if s.benchmark || s.gpu_profile {
                        unsafe {
                            next.enable_gpu_timing(s.gpu_profile);
                        }
                    }
                    drop(s);
                    next.set_sleeve_visible(state.borrow().playback.borrow().sleeve_visible());
                    let vinyl_ready = next.vinyl_supported();
                    let artwork_ready = next.record_artwork_supported();
                    let (revision, image, _) = artwork.snapshot();
                    let (previous_revision, previous_image) = artwork.previous_snapshot();
                    let settings = state.borrow().config.artwork.clone();
                    next.set_record_fallback_enabled(
                        image.is_none()
                            && settings.enabled
                            && state.borrow().playback.borrow().sleeve_visible(),
                    );
                    let checked = unsafe {
                        next.set_artwork(revision, image.as_deref(), &settings)
                            .and_then(|_| {
                                next.set_previous_artwork(
                                    previous_revision,
                                    previous_image.as_deref(),
                                )
                            })
                            .and_then(|_| next.check_error())
                    };
                    if let Err(error) = checked {
                        unsafe {
                            next.destroy();
                        }
                        return Err(error);
                    }
                    let previous = renderer.borrow_mut().replace(next);
                    if let Some(previous) = previous {
                        unsafe {
                            previous.destroy();
                        }
                    }
                    let mut s = state.borrow_mut();
                    s.config.scene = scene;
                    s.vinyl_ready = vinyl_ready;
                    s.artwork_ready = artwork_ready;
                    s.previous_frame = Instant::now();
                    tracing::info!(variant=?s.config.scene.variant, resolution=?s.config.scene.resolution, "Selected scene ready; previous scene released");
                    Ok(())
                });
                if let Err(error) = result {
                    tracing::warn!("Scene switch failed; previous scene retained: {error:#}");
                    rollback(
                        &choice,
                        &weak_resolution,
                        &weak_status,
                        &state,
                        &updating,
                        &format!("Could not load scene: {error:#}"),
                    );
                } else {
                    if let Some(commit) = committed.borrow().as_ref() {
                        let scene = &state.borrow().config.scene;
                        commit(scene.variant, scene.resolution);
                    }
                    if let Some(status) = weak_status.upgrade() {
                        status.set_visible(false);
                    }
                    super::presenter::queue_layout_render(&area);
                    ensure_animation(&area, state.clone(), running.clone());
                }
            }
            if !loader.pending() {
                polling.set(false);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    });
    let on_scene = changed.clone();
    ui.choice.connect_changed(move |_| on_scene());
    ui.resolution.connect_changed(move |_| changed());
}

fn rollback(
    choice: &gtk::ComboBoxText,
    resolution: &glib::WeakRef<gtk::ComboBoxText>,
    status: &glib::WeakRef<gtk::Label>,
    state: &Rc<RefCell<AppState>>,
    updating: &Cell<bool>,
    error: &str,
) {
    updating.set(true);
    choice.set_active_id(Some(state.borrow().config.scene.variant.id()));
    if let Some(resolution) = resolution.upgrade() {
        resolution.set_active_id(Some(state.borrow().config.scene.resolution.id()));
    }
    updating.set(false);
    if let Some(status) = status.upgrade() {
        status.set_text(error);
        status.set_visible(true);
    }
}

#[cfg(test)]
mod tests;
