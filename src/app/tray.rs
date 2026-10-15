//! Main-thread StatusNotifier tray. No GTK3 dependency or rendering loop.
mod protocol;
use super::{state::AppState, style};
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub(super) struct TrayUi {
    pub widget: gtk::Box,
    window: glib::WeakRef<gtk::ApplicationWindow>,
    toggle: gtk::Switch,
    status: gtk::Label,
    enabled: Cell<bool>,
    available: Cell<bool>,
    hidden: Cell<bool>,
    restoring: Cell<bool>,
    restore_generation: Cell<u64>,
    weak: std::rc::Weak<Self>,
    stopped: Cell<bool>,
    started: Cell<bool>,
    generation: Cell<u64>,
    backend: RefCell<Option<protocol::Backend>>,
    task: RefCell<Option<glib::JoinHandle<()>>>,
    registration_task: RefCell<Option<glib::JoinHandle<()>>>,
}

impl TrayUi {
    pub fn new(window: &gtk::ApplicationWindow, state: Rc<RefCell<AppState>>) -> Rc<Self> {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let toggle = gtk::Switch::new();
        toggle.set_widget_name("minimize-to-tray");
        let enabled = state.borrow().config.minimize_to_tray;
        toggle.set_active(enabled);
        toggle.set_sensitive(enabled);
        let (row, status) =
            style::row_with_detail("Minimize to tray", "No tray · use the taskbar", &toggle);
        widget.append(&row);
        status.set_widget_name("tray-status");
        status.set_single_line_mode(true);
        status.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let ui = Rc::new_cyclic(|weak| Self {
            widget,
            window: window.downgrade(),
            toggle,
            status,
            enabled: Cell::new(enabled),
            available: Cell::new(false),
            hidden: Cell::new(false),
            restoring: Cell::new(false),
            restore_generation: Cell::new(0),
            weak: weak.clone(),
            stopped: Cell::new(false),
            started: Cell::new(false),
            generation: Cell::new(0),
            backend: RefCell::new(None),
            task: RefCell::new(None),
            registration_task: RefCell::new(None),
        });
        let weak = Rc::downgrade(&ui);
        ui.toggle.connect_active_notify(move |toggle| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            ui.enabled.set(toggle.is_active());
            state.borrow_mut().config.minimize_to_tray = toggle.is_active();
            ui.refresh();
            if let Some(backend) = ui.backend.borrow().as_ref() {
                backend.status_changed(ui.enabled.get());
            }
            if ui.enabled.get() {
                ui.hide_if_minimized();
            } else if ui.hidden.get() {
                ui.show();
            }
        });
        // Preserve existing Quit-on-close behavior unless the option is enabled.
        let weak = Rc::downgrade(&ui);
        window.connect_close_request(move |window| {
            if let Some(ui) = weak.upgrade().filter(|ui| ui.enabled.get()) {
                if !ui.hide() {
                    window.minimize();
                }
            } else if let Some(app) = window.application() {
                app.quit();
            }
            glib::Propagation::Stop
        });
        let weak = Rc::downgrade(&ui);
        window.connect_realize(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.watch_surface();
            }
        });
        if window.is_realized() {
            ui.watch_surface();
        }
        ui
    }

    pub fn start(self: &Rc<Self>) {
        if self.stopped.get() || self.started.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        *self.task.borrow_mut() = Some(glib::spawn_future_local(async move {
            loop {
                let Some(ui) = weak.upgrade().filter(|ui| !ui.stopped.get()) else {
                    break;
                };
                let disconnected = ui.backend.borrow().as_ref().is_some_and(|b| b.is_closed());
                if disconnected {
                    ui.cancel_registration();
                    ui.backend.borrow_mut().take();
                    ui.set_available(false);
                }
                let connected = ui.backend.borrow().is_some();
                drop(ui);
                let delay = if connected {
                    std::time::Duration::from_secs(2)
                } else {
                    let connection = crate::session_bus::connect().await;
                    let Some(ui) = weak.upgrade().filter(|ui| !ui.stopped.get()) else {
                        break;
                    };
                    match connection.and_then(|connection| protocol::Backend::new(connection, &ui))
                    {
                        Ok(backend) => {
                            *ui.backend.borrow_mut() = Some(backend);
                            std::time::Duration::from_secs(2)
                        }
                        Err(error) => {
                            tracing::debug!("Tray bus unavailable; retrying: {error}");
                            ui.set_available(false);
                            crate::session_bus::RETRY
                        }
                    }
                };
                glib::timeout_future(delay).await;
            }
        }));
    }

    fn refresh(&self) {
        self.toggle
            .set_sensitive(self.available.get() || self.enabled.get());
        self.status.set_text(if self.available.get() {
            "Quick access from the tray"
        } else {
            "No tray · use the taskbar"
        });
    }

    fn set_available(&self, available: bool) {
        if self.stopped.get() {
            return;
        }
        self.available.set(available);
        self.refresh();
        if !available && self.hidden.get() {
            self.show();
        }
        if available {
            self.hide_if_minimized();
        }
    }

    fn hide_if_minimized(&self) {
        if let Some(window) = self.window.upgrade() {
            let minimized = window
                .surface()
                .and_then(|surface| surface.downcast::<gtk::gdk::Toplevel>().ok())
                .is_some_and(|surface| {
                    surface.state().contains(gtk::gdk::ToplevelState::MINIMIZED)
                });
            // Unmapping clears GDK's cached MINIMIZED bit before the WM has
            // retired its old iconify event. Only a focused, normal-state
            // acknowledgement (or the bounded settle callback) ends restore.
            if !minimized && window.is_active() {
                self.restoring.set(false);
            } else if minimized && !self.restoring.get() {
                self.hide();
            }
        }
    }

    fn watch_surface(self: &Rc<Self>) {
        if let Some(toplevel) = self
            .window
            .upgrade()
            .and_then(|window| window.surface())
            .and_then(|surface| surface.downcast::<gtk::gdk::Toplevel>().ok())
        {
            let weak = Rc::downgrade(self);
            toplevel.connect_state_notify(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.hide_if_minimized();
                }
            });
        }
    }

    fn hide(&self) -> bool {
        if self.stopped.get() || !self.enabled.get() || !self.available.get() {
            return false;
        }
        if self.hidden.get() {
            return true;
        }
        if let Some(window) = self.window.upgrade() {
            self.restore_generation
                .set(self.restore_generation.get().wrapping_add(1));
            self.restoring.set(false);
            self.hidden.set(true);
            window.set_visible(false);
            return true;
        }
        false
    }

    pub(super) fn show(&self) {
        if self.stopped.get() {
            return;
        }
        self.hidden.set(false);
        self.restoring.set(true);
        let generation = self.restore_generation.get().wrapping_add(1);
        self.restore_generation.set(generation);
        if let Some(window) = self.window.upgrade() {
            // GtkWindow remembers minimize_initially even after hiding. Clear
            // it before present() computes the new toplevel layout; otherwise
            // a restore first submits another minimize request to the WM.
            window.unminimize();
            window.present();
        }
        // A one-shot guard, not an animation/polling loop. It also handles
        // already-focused windows that produce no new state acknowledgement.
        let weak = self.weak.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
            if let Some(ui) = weak
                .upgrade()
                .filter(|ui| !ui.stopped.get() && ui.restore_generation.get() == generation)
            {
                ui.restoring.set(false);
                ui.hide_if_minimized();
            }
        });
    }

    fn quit(&self) {
        if let Some(app) = self
            .window
            .upgrade()
            .and_then(|window| window.application())
        {
            app.quit();
        }
    }

    pub fn stop(&self) {
        self.stopped.set(true);
        self.cancel_registration();
        if let Some(task) = self.task.borrow_mut().take() {
            task.abort();
        }
        self.backend.borrow_mut().take();
    }
    fn cancel_registration(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        if let Some(task) = self.registration_task.borrow_mut().take() {
            task.abort();
        }
    }
}

impl Drop for TrayUi {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests;
