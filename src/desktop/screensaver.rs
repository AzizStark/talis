//! Observe actual screensaver activation, not idle hints or lock requests.
//! Calls use unique owners and NO_AUTO_START; unknown/expired status fails open.
use gtk::{gio, glib};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

const PROVIDERS: &[(&str, &str)] = &[
    ("org.cinnamon.ScreenSaver", "/org/cinnamon/ScreenSaver"),
    ("org.gnome.ScreenSaver", "/org/gnome/ScreenSaver"),
    ("org.mate.ScreenSaver", "/org/mate/ScreenSaver"),
    ("org.xfce.ScreenSaver", "/org/xfce/ScreenSaver"),
    (
        "org.freedesktop.ScreenSaver",
        "/org/freedesktop/ScreenSaver",
    ),
    ("org.freedesktop.ScreenSaver", "/ScreenSaver"),
];
#[derive(Default)]
struct Provider {
    owner: RefCell<Option<String>>,
    revision: Cell<u64>,
    status: Cell<Option<(bool, Instant)>>,
}
pub struct Monitor {
    providers: Vec<Rc<Provider>>,
    connection: RefCell<Option<gio::DBusConnection>>,
    signals: RefCell<Vec<gio::SignalSubscriptionId>>,
    task: RefCell<Option<glib::JoinHandle<()>>>,
    calls: RefCell<Vec<glib::JoinHandle<()>>>,
    stopped: Cell<bool>,
}
impl Monitor {
    pub fn start() -> Rc<Self> {
        let monitor = Rc::new(Self {
            providers: PROVIDERS
                .iter()
                .map(|_| Rc::new(Provider::default()))
                .collect(),
            connection: RefCell::new(None),
            signals: RefCell::new(vec![]),
            task: RefCell::new(None),
            calls: RefCell::new(vec![]),
            stopped: Cell::new(false),
        });
        let weak = Rc::downgrade(&monitor);
        *monitor.task.borrow_mut() = Some(glib::spawn_future_local(async move {
            loop {
                let Some(monitor) = weak.upgrade().filter(|m| !m.stopped.get()) else {
                    break;
                };
                let connection = monitor.connection.borrow().clone();
                let connection = connection.filter(|c| !c.is_closed());
                if connection.is_none() {
                    monitor.disconnect();
                }
                drop(monitor);
                let connection = match connection {
                    Some(connection) => connection,
                    None => match crate::session_bus::connect().await {
                        Ok(connection) => {
                            let Some(monitor) = weak.upgrade().filter(|m| !m.stopped.get()) else {
                                break;
                            };
                            monitor.subscribe(&connection);
                            connection
                        }
                        Err(error) => {
                            tracing::debug!("Screensaver bus unavailable; retrying: {error}");
                            glib::timeout_future(crate::session_bus::RETRY).await;
                            continue;
                        }
                    },
                };
                let Some(monitor) = weak.upgrade().filter(|m| !m.stopped.get()) else {
                    break;
                };
                monitor.poll(&connection);
                drop(monitor);
                glib::timeout_future(Duration::from_secs(2)).await;
            }
        }));
        monitor
    }
    fn subscribe(self: &Rc<Self>, connection: &gio::DBusConnection) {
        let monitor = self;
        *monitor.connection.borrow_mut() = Some(connection.clone());
        for (index, &(name, path)) in PROVIDERS.iter().enumerate() {
            let provider = monitor.providers[index].clone();
            let signal_provider = provider.clone();
            let weak = Rc::downgrade(self);
            #[allow(deprecated)]
            let id = connection.signal_subscribe(
                None,
                Some(name),
                Some("ActiveChanged"),
                Some(path),
                None,
                gio::DBusSignalFlags::NONE,
                move |connection, sender, _, _, _, parameters| {
                    if connection.is_closed() || weak.upgrade().is_none_or(|m| m.stopped.get()) {
                        return;
                    }
                    if signal_provider.owner.borrow().as_deref() == Some(sender) {
                        if let Some((active,)) = parameters.get::<(bool,)>() {
                            signal_provider
                                .revision
                                .set(signal_provider.revision.get().wrapping_add(1));
                            signal_provider.status.set(Some((active, Instant::now())));
                        }
                    }
                },
            );
            monitor.signals.borrow_mut().push(id);
            let weak = Rc::downgrade(self);
            #[allow(deprecated)]
            let id = connection.signal_subscribe(
                Some("org.freedesktop.DBus"),
                Some("org.freedesktop.DBus"),
                Some("NameOwnerChanged"),
                Some("/org/freedesktop/DBus"),
                Some(name),
                gio::DBusSignalFlags::NONE,
                move |connection, _, _, _, _, parameters| {
                    if connection.is_closed() || weak.upgrade().is_none_or(|m| m.stopped.get()) {
                        return;
                    }
                    if let Some((_, _, new_owner)) = parameters.get::<(String, String, String)>() {
                        provider
                            .revision
                            .set(provider.revision.get().wrapping_add(1));
                        provider.status.set(None);
                        *provider.owner.borrow_mut() = (!new_owner.is_empty()).then_some(new_owner);
                    }
                },
            );
            monitor.signals.borrow_mut().push(id);
        }
    }
    fn poll(self: &Rc<Self>, connection: &gio::DBusConnection) {
        // Also owns child tasks: shutdown/disconnect cancels every call.
        self.cancel_calls();
        // Futures run concurrently: a broken service cannot stall other
        // providers or GTK. A single bounded polling cycle runs at once.
        for (index, &(name, path)) in PROVIDERS.iter().enumerate() {
            let connection = connection.clone();
            let provider = self.providers[index].clone();
            let weak = Rc::downgrade(self);
            self.calls
                .borrow_mut()
                .push(glib::spawn_future_local(async move {
                    let revision = provider.revision.get();
                    let owner = connection
                        .call_future(
                            Some("org.freedesktop.DBus"),
                            "/org/freedesktop/DBus",
                            "org.freedesktop.DBus",
                            "GetNameOwner",
                            Some(&(name,).to_variant()),
                            None,
                            gio::DBusCallFlags::NO_AUTO_START,
                            500,
                        )
                        .await
                        .ok()
                        .and_then(|v| v.get::<(String,)>())
                        .map(|v| v.0);
                    if revision != provider.revision.get()
                        || connection.is_closed()
                        || weak.upgrade().is_none_or(|m| m.stopped.get())
                    {
                        return;
                    }
                    *provider.owner.borrow_mut() = owner.clone();
                    let Some(owner) = owner else {
                        provider.status.set(None);
                        return;
                    };
                    let active = connection
                        .call_future(
                            Some(&owner),
                            path,
                            name,
                            "GetActive",
                            None,
                            None,
                            gio::DBusCallFlags::NO_AUTO_START,
                            500,
                        )
                        .await
                        .ok()
                        .and_then(|v| v.get::<(bool,)>())
                        .map(|v| v.0);
                    // A newer signal or owner change must win over late replies.
                    if revision == provider.revision.get()
                        && !connection.is_closed()
                        && weak.upgrade().is_some_and(|m| !m.stopped.get())
                        && provider.owner.borrow().as_deref() == Some(&owner)
                    {
                        provider
                            .status
                            .set(active.map(|active| (active, Instant::now())));
                    }
                }));
        }
    }
    pub fn active(&self, now: Instant) -> bool {
        if self.stopped.get()
            || self
                .connection
                .borrow()
                .as_ref()
                .is_some_and(|c| c.is_closed())
        {
            return false;
        }
        self.providers.iter().any(|provider| {
            provider.status.get().is_some_and(|(active, at)| {
                active && now.saturating_duration_since(at) <= Duration::from_secs(6)
            })
        })
    }
    pub fn stop(&self) {
        self.stopped.set(true);
        if let Some(task) = self.task.borrow_mut().take() {
            task.abort();
        }
        self.disconnect();
    }
    fn cancel_calls(&self) {
        for task in self.calls.borrow_mut().drain(..) {
            task.abort();
        }
    }
    #[allow(deprecated)] // Preserve the GTK/GIO floor used by the existing app.
    fn disconnect(&self) {
        self.cancel_calls();
        if let Some(connection) = self.connection.borrow_mut().take() {
            for id in self.signals.borrow_mut().drain(..) {
                connection.signal_unsubscribe(id);
            }
        }
        for provider in &self.providers {
            provider
                .revision
                .set(provider.revision.get().wrapping_add(1));
            provider.owner.borrow_mut().take();
            provider.status.set(None);
        }
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop();
    }
}
use glib::variant::ToVariant;

#[cfg(test)]
mod tests;
