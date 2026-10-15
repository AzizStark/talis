//! Nonfatal, cancellable session-bus acquisition shared by desktop observers.
//! Never close the shared connection: other application components use it too.
use gtk::{gio, glib};
use std::cell::RefCell;
use std::time::Duration;

pub(crate) const RETRY: Duration = Duration::from_secs(4);
thread_local! {
    // GIO's shared singleton may remain closed while GtkApplication or another
    // component retains it. Share a fresh recovery connection without pinning it.
    static RECOVERED: RefCell<glib::WeakRef<gio::DBusConnection>> = RefCell::new(glib::WeakRef::new());
}
fn recovered() -> Option<gio::DBusConnection> {
    RECOVERED.with(|weak| weak.borrow().upgrade().filter(|c| !c.is_closed()))
}

pub(crate) async fn connect() -> Result<gio::DBusConnection, glib::Error> {
    if let Some(connection) = recovered() {
        return Ok(connection);
    }
    let future = gio::GioFuture::new(&(), |_, cancellable, result| {
        gio::bus_get(
            gio::BusType::Session,
            Some(cancellable),
            move |connection| {
                // Apply the policy in the completion callback, even if our waiter
                // was cancelled before it received this shared connection.
                if let Ok(connection) = &connection {
                    connection.set_exit_on_close(false);
                }
                result.resolve(connection);
            },
        );
    });
    glib::future_with_timeout(Duration::from_secs(3), async move {
        let connection = future.await?;
        if !connection.is_closed() {
            return Ok(connection);
        }
        // Do not wait for GTK/other users to release the closed singleton.
        // Address discovery can autolaunch on some systems; keep it off GTK's
        // main thread. The asynchronous handshake is cancellable as usual.
        let address = gio::spawn_blocking(|| {
            gio::dbus_address_get_for_bus_sync(gio::BusType::Session, None::<&gio::Cancellable>)
        })
        .await
        .map_err(|_| {
            glib::Error::new(
                gio::IOErrorEnum::Failed,
                "Session bus address lookup failed",
            )
        })??;
        let connection = gio::DBusConnection::for_address_future(
            &address,
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,
        )
        .await?;
        connection.set_exit_on_close(false);
        if connection.is_closed() {
            return Err(glib::Error::new(
                gio::IOErrorEnum::Closed,
                "Session bus disconnected during recovery",
            ));
        }
        // Concurrent observers may have completed another recovery first.
        if let Some(existing) = recovered() {
            return Ok(existing);
        }
        RECOVERED.with(|weak| weak.borrow_mut().set(Some(&connection)));
        Ok(connection)
    })
    .await
    .map_err(|_| glib::Error::new(gio::IOErrorEnum::TimedOut, "Session bus timed out"))?
}

#[cfg(test)]
pub(crate) mod tests;
