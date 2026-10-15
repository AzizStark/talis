//! Dedicated subprocess tests only. Never stop a daemon supplied by the session.
use glib::variant::ToVariant;
use gtk::{gio, glib};
use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub(crate) struct Bus {
    pub address: String,
    child: Option<Child>,
}
impl Bus {
    pub fn new() -> Self {
        assert_eq!(std::env::var("TALIS_SESSION_BUS_TEST").as_deref(), Ok("1"));
        let directory = PathBuf::from(std::env::var("TALIS_SESSION_BUS_TEST_DIRECTORY").unwrap());
        assert_eq!(directory.parent(), Some(std::path::Path::new("/tmp")));
        assert!(
            directory
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("talis-bus-recovery.")
        );
        assert!(directory.is_dir());
        let address = format!("unix:path={}/bus", directory.display());
        assert_eq!(std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap(), address);
        assert!(
            !directory.join("bus").exists(),
            "test must start with its bus absent"
        );
        Self {
            address,
            child: None,
        }
    }
    pub fn start(&mut self) {
        assert!(self.child.is_none());
        let mut child = Command::new("dbus-daemon")
            .args(["--nofork", "--nopidfile", "--print-address=1"])
            .arg(format!(
                "--config-file={}/tests/fixtures/session-bus.conf",
                env!("CARGO_MANIFEST_DIR")
            ))
            .arg(format!("--address={}", self.address))
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut address = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        self.child = Some(child);
        assert!(address.starts_with(&self.address));
    }
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // Only a Child created above can ever be terminated here.
            let _ = child.kill();
            child.wait().unwrap();
        }
    }
    pub fn fixture(&self) -> gio::DBusConnection {
        let connection = gio::DBusConnection::for_address_sync(
            &self.address,
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,
            None::<&gio::Cancellable>,
        )
        .unwrap();
        connection.set_exit_on_close(false);
        connection
    }
}
impl Drop for Bus {
    fn drop(&mut self) {
        self.stop();
    }
}

#[track_caller]
pub(crate) fn wait(check: impl Fn() -> bool) -> impl std::future::Future<Output = ()> {
    let caller = std::panic::Location::caller();
    async move {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !check() {
            assert!(
                Instant::now() < deadline,
                "bus recovery did not settle at {caller}"
            );
            glib::timeout_future(Duration::from_millis(10)).await;
        }
    }
}
pub(crate) async fn name(connection: &gio::DBusConnection, method: &str, name: &str) {
    let args = if method == "RequestName" {
        (name, 0_u32).to_variant()
    } else {
        (name,).to_variant()
    };
    connection
        .call_future(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            method,
            Some(&args),
            None,
            gio::DBusCallFlags::NONE,
            1000,
        )
        .await
        .unwrap();
}
