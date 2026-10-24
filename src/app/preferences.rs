//! Event-driven autosave of actual UI edits; no reads of borrowed renderer state.
use super::{state::AppState, style, window::AppWindows};
use crate::preferences::{Document, Patch, Session, Worker};
use anyhow::Result;
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
    time::Duration,
};

pub(super) struct Autosave {
    worker: Worker,
    observed: RefCell<Patch>,
    dirty: RefCell<Patch>,
    requests: RefCell<BTreeMap<u64, Patch>>,
    serial: Cell<u64>,
    debounce: RefCell<Option<glib::SourceId>>,
    completion: RefCell<Option<glib::SourceId>>,
    error: glib::WeakRef<gtk::Label>,
    stopped: Cell<bool>,
}
impl Autosave {
    fn new(
        store: crate::preferences::Store,
        snapshot: Document,
        error: &gtk::Label,
    ) -> Result<Rc<Self>> {
        Ok(Rc::new(Self {
            worker: Worker::new(store)?,
            observed: RefCell::new(snapshot.values()),
            dirty: RefCell::new(Patch::new()),
            requests: RefCell::new(BTreeMap::new()),
            serial: Cell::new(0),
            debounce: RefCell::new(None),
            completion: RefCell::new(None),
            error: error.downgrade(),
            stopped: Cell::new(false),
        }))
    }
    fn update(self: &Rc<Self>, patch: Patch) {
        if self.stopped.get() {
            return;
        }
        let mut changed = false;
        for (key, value) in patch {
            if self.observed.borrow().get(&key) != Some(&value) {
                self.observed
                    .borrow_mut()
                    .insert(key.clone(), value.clone());
                self.dirty.borrow_mut().insert(key, value);
                changed = true;
            }
        }
        if !changed {
            return;
        }
        if let Some(source) = self.debounce.borrow_mut().take() {
            source.remove();
        }
        let weak = Rc::downgrade(self);
        *self.debounce.borrow_mut() = Some(glib::timeout_add_local_once(
            Duration::from_millis(500),
            move || {
                if let Some(saver) = weak.upgrade() {
                    saver.debounce.borrow_mut().take();
                    saver.dispatch();
                }
            },
        ));
    }
    fn dispatch(self: &Rc<Self>) {
        let patch = self.dirty.borrow().clone();
        if patch.is_empty() {
            return;
        }
        let id = self.serial.get() + 1;
        self.serial.set(id);
        if let Err(error) = self.worker.save(id, patch.clone()) {
            self.report(Err(format!("{error:#}")));
            return;
        }
        self.requests.borrow_mut().insert(id, patch);
        if self.completion.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        *self.completion.borrow_mut() = Some(glib::timeout_add_local(
            Duration::from_millis(25),
            move || {
                let Some(saver) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                saver.drain();
                if saver.requests.borrow().is_empty() {
                    saver.completion.borrow_mut().take();
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            },
        ));
    }
    /// Explicit reset must overwrite saved overrides even when the runtime was
    /// already at defaults (for example because of command-line overrides).
    fn restore(self: &Rc<Self>, patch: Patch) {
        if self.stopped.get() {
            return;
        }
        self.observed.borrow_mut().extend(patch.clone());
        self.dirty.borrow_mut().extend(patch);
        if let Some(source) = self.debounce.borrow_mut().take() {
            source.remove();
        }
        self.dispatch();
    }
    fn drain(&self) {
        while let Some((id, result)) = self.worker.poll() {
            if let Some(patch) = self.requests.borrow_mut().remove(&id) {
                if result.is_ok() {
                    for (key, value) in patch {
                        if self.dirty.borrow().get(&key) == Some(&value) {
                            self.dirty.borrow_mut().remove(&key);
                        }
                    }
                }
            }
            if id == self.serial.get() {
                self.report(result);
            }
        }
    }
    fn report(&self, result: std::result::Result<(), String>) {
        if let Some(error) = self.error.upgrade() {
            match result {
                Ok(()) => error.set_visible(false),
                Err(message) => {
                    tracing::warn!("Preferences not saved: {message}");
                    error.set_text(&format!("Settings not saved: {message}"));
                    error.set_visible(true);
                }
            }
        }
    }
    pub(super) fn stop(self: &Rc<Self>) {
        if self.stopped.replace(true) {
            return;
        }
        if let Some(source) = self.debounce.borrow_mut().take() {
            source.remove();
        }
        self.dispatch();
        let result = self.worker.flush().map_err(|error| format!("{error:#}"));
        self.drain();
        if let Err(error) = result {
            tracing::warn!("Preferences shutdown flush failed: {error}");
        }
        if let Some(source) = self.completion.borrow_mut().take() {
            source.remove();
        }
    }
}
impl Drop for Autosave {
    fn drop(&mut self) {
        if let Some(source) = self.debounce.get_mut().take() {
            source.remove();
        }
        if let Some(source) = self.completion.get_mut().take() {
            source.remove();
        }
    }
}
fn one(key: &str, value: impl Into<toml::Value>) -> Patch {
    Patch::from_iter([(key.into(), value.into())])
}

pub(super) fn attach(
    preview: &AppWindows,
    state: &AppState,
    session: Session,
) -> Result<Option<Rc<Autosave>>> {
    if let Some(issue) = session.issue {
        tracing::warn!("{issue}");
        preview.preferences_error.set_text(&issue);
        preview.preferences_error.set_visible(true);
    }
    let (Some(store), Some(window)) = (session.store, preview.controls.as_ref()) else {
        return Ok(None);
    };
    let saver = Autosave::new(
        store,
        Document::from_config(&state.config, state.playback.borrow().preferred.as_deref()),
        &preview.preferences_error,
    )?;
    let root = window.clone().upcast::<gtk::Widget>();
    for (name, key, scale) in [
        ("artwork-size", "label_size_percent", 1.0),
        ("artwork-speed", "rotation_rpm", 1.0),
        ("vinyl-opacity", "surface_opacity", 100.0),
        ("vinyl-brightness", "surface_brightness", 100.0),
        ("sleeve-shadow-lift", "sleeve_shadow_lift", 100.0),
    ] {
        if let Some(spin) =
            style::find_widget(&root, name).and_then(|w| w.downcast::<gtk::SpinButton>().ok())
        {
            let weak = Rc::downgrade(&saver);
            spin.connect_value_changed(move |spin| {
                if let Some(saver) = weak.upgrade() {
                    saver.update(one(key, spin.value() / scale));
                }
            });
        }
    }
    for (name, key) in [
        ("pause-when-hidden", "pause_when_hidden"),
        ("minimize-to-tray", "minimize_to_tray"),
        ("experimental-mate-overlay", "experimental_mate_overlay"),
    ] {
        if let Some(toggle) =
            style::find_widget(&root, name).and_then(|w| w.downcast::<gtk::Switch>().ok())
        {
            let weak = Rc::downgrade(&saver);
            toggle.connect_active_notify(move |toggle| {
                if toggle.is_sensitive() {
                    if let Some(saver) = weak.upgrade() {
                        saver.update(one(key, toggle.is_active()));
                    }
                }
            });
        }
    }
    if let Some(combo) =
        style::find_widget(&root, "frame-rate").and_then(|w| w.downcast::<gtk::ComboBoxText>().ok())
    {
        let weak = Rc::downgrade(&saver);
        combo.connect_changed(move |combo| {
            if let (Some(saver), Some(fps)) = (
                weak.upgrade(),
                combo.active_id().and_then(|id| id.parse::<i64>().ok()),
            ) {
                saver.update(one("fps", fps));
            }
        });
    }
    if let Some(combo) = style::find_widget(&root, "desktop-layer")
        .and_then(|w| w.downcast::<gtk::ComboBoxText>().ok())
    {
        let weak = Rc::downgrade(&saver);
        combo.connect_changed(move |combo| {
            if combo.is_sensitive() {
                if let (Some(saver), Some(id)) = (weak.upgrade(), combo.active_id()) {
                    saver.update(one("wayland_layer", id.as_str()));
                }
            }
        });
    }
    if let Some(ui) = &preview.arm_ui {
        for name in ["arm-range-start", "arm-range-end"] {
            if let Some(spin) =
                style::find_widget(&root, name).and_then(|w| w.downcast::<gtk::SpinButton>().ok())
            {
                let weak = Rc::downgrade(&saver);
                let read = ui.range_reader();
                spin.connect_value_changed(move |_| {
                    if let (Some(saver), Some((start, end))) = (weak.upgrade(), read()) {
                        let mut patch = one("arm_range_start", start as f64);
                        patch.extend(one("arm_range_end", end as f64));
                        saver.update(patch);
                    }
                });
            }
        }
    }
    if let Some(ui) = &preview.media_ui {
        if let Some(combo) = style::find_widget(&root, "playback-source")
            .and_then(|w| w.downcast::<gtk::ComboBoxText>().ok())
        {
            let weak = Rc::downgrade(&saver);
            let read = ui.selection_reader();
            combo.connect_changed(move |_| {
                if let (Some(saver), Some(player)) = (weak.upgrade(), read()) {
                    saver.update(one("preferred_player", player));
                }
            });
        }
    }
    if let Some(ui) = &preview.scene_ui {
        let weak = Rc::downgrade(&saver);
        *ui.committed.borrow_mut() = Some(Box::new(move |scene, resolution| {
            if let Some(saver) = weak.upgrade() {
                let mut patch = one("scene", scene.id());
                patch.extend(one("scene_resolution", resolution.id()));
                saver.restore(patch);
            }
        }));
    }
    if let Some(ui) = &preview.defaults_ui {
        let weak = Rc::downgrade(&saver);
        *ui.committed.borrow_mut() = Some(Box::new(move |patch| {
            if let Some(saver) = weak.upgrade() {
                saver.restore(patch);
            }
        }));
    }
    Ok(Some(saver))
}
#[cfg(test)]
mod tests;
