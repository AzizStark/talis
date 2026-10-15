//! Desktop admission before asset decoding. IPC never owns the GTK application
//! identity, so per-display/monitor routing cannot change taskbar/icon matching.
mod lease;
use super::{Options, window::AppWindows};
use anyhow::{Context, Result};
use gtk::{gio, prelude::*};
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Default)]
struct Activation {
    pending: Cell<bool>,
    show: RefCell<Option<Box<dyn Fn()>>>,
}
impl Activation {
    fn request(&self) {
        if let Some(show) = self.show.borrow().as_ref() {
            show();
        } else {
            self.pending.set(true);
        }
    }
    fn bind(&self, show: impl Fn() + 'static) {
        *self.show.borrow_mut() = Some(Box::new(show));
        if self.pending.replace(false) {
            self.request();
        }
    }
}

pub(super) struct Instance {
    _application: gio::Application,
    // Retain the OS lease even if the session bus disappears. A second process
    // must never start another renderer merely because IPC is unavailable.
    _lease: lease::Lease,
    activation: Rc<Activation>,
}
pub(super) enum Claim {
    Primary(Instance),
    Activated,
}

fn identity(backend: &str, display: &str, monitor: u32) -> String {
    let mut hash = Sha256::new();
    hash.update(backend.as_bytes());
    hash.update([0]);
    hash.update(display.as_bytes());
    format!(
        "io.github.azizstark.Talis.Instance.d{:x}.m{monitor}",
        hash.finalize()
    )
}

impl Instance {
    pub(super) fn claim(options: &Options) -> Result<Claim> {
        let display = gtk::gdk::Display::default().context("No GTK display")?;
        let id = identity(display.type_().name(), &display.name(), options.monitor);
        Self::register(&id, lease::Lease::acquire(&id)?)
    }

    fn register(id: &str, lease: Option<lease::Lease>) -> Result<Claim> {
        let Some(lease) = lease else {
            activate_existing(id)?;
            tracing::info!("Existing desktop instance activated; no scene assets loaded");
            return Ok(Claim::Activated);
        };
        let activation = Rc::new(Activation::default());
        let application = gio::Application::new(Some(id), gio::ApplicationFlags::FLAGS_NONE);
        let handler = activation.clone();
        application.connect_activate(move |_| handler.request());
        application.connect_startup(|app| {
            if let Some(connection) = app.dbus_connection() {
                connection.set_exit_on_close(false);
            }
        });
        application
            .register(None::<&gio::Cancellable>)
            .context("Register desktop activation endpoint")?;
        if application.is_remote() {
            application.activate();
            if let Some(connection) = application.dbus_connection() {
                connection.set_exit_on_close(false);
                connection.flush_sync(None::<&gio::Cancellable>)?;
            }
            tracing::info!("Existing desktop instance activated; no scene assets loaded");
            return Ok(Claim::Activated);
        }
        Ok(Claim::Primary(Self {
            _application: application,
            _lease: lease,
            activation,
        }))
    }

    pub(super) fn bind_settings(&self, preview: &AppWindows) {
        let window = preview.controls.as_ref().map(|window| window.downgrade());
        let tray = preview.tray_ui.as_ref().map(Rc::downgrade);
        self.activation.bind(move || {
            if let Some(tray) = tray.as_ref().and_then(|tray| tray.upgrade()) {
                tray.show();
            } else if let Some(window) = window.as_ref().and_then(|window| window.upgrade()) {
                window.unminimize();
                window.present();
            }
        });
    }
}

fn activate_existing(id: &str) -> Result<()> {
    use std::{
        collections::HashMap,
        time::{Duration, Instant},
    };
    // Never register another application while somebody owns the OS lease:
    // that could steal the bus name before a simultaneously starting owner.
    let result = (|| -> Result<()> {
        let connection = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>)?;
        connection.set_exit_on_close(false);
        let deadline = Instant::now() + Duration::from_secs(2);
        let owner = loop {
            let reply = connection.call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "GetNameOwner",
                Some(&(id,).to_variant()),
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                500,
                None::<&gio::Cancellable>,
            );
            if let Ok(reply) = reply {
                if let Some((owner,)) = reply.get::<(String,)>() {
                    break owner;
                }
            }
            anyhow::ensure!(
                Instant::now() < deadline && !connection.is_closed(),
                "Desktop activation endpoint unavailable"
            );
            std::thread::sleep(Duration::from_millis(25));
        };
        let path = format!("/{}", id.replace('.', "/"));
        let message = gio::DBusMessage::new_method_call(
            Some(&owner),
            &path,
            Some("org.freedesktop.Application"),
            "Activate",
        );
        message.set_flags(gio::DBusMessageFlags::NO_REPLY_EXPECTED);
        message.set_body(&(HashMap::<String, gtk::glib::Variant>::new(),).to_variant());
        connection.send_message(&message, gio::DBusSendMessageFlags::NONE)?;
        connection.flush_sync(None::<&gio::Cancellable>)?;
        Ok(())
    })();
    result.context("Talis is already running on this display and monitor, but Settings could not be activated; use the tray or turntable logo")
}

#[cfg(test)]
mod tests;
